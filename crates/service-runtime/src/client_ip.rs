//! Client IP derivation behind reverse proxies.
//!
//! Proxies append to `X-Forwarded-For`, and a client can send its own value first, so the left of
//! the list is attacker-controlled. Only the entry written by the outermost trusted proxy is the
//! address that really connected: it sits `trusted_hops` entries from the right. A wrong hop count
//! fails safe: every client then shares one rate-limit bucket, but nobody can choose their own.

use std::net::IpAddr;

/// The client IP that the outermost of `trusted_hops` proxies recorded, from a comma list such as
/// `X-Forwarded-For`. `None` when the list is shorter than the hop count or the entry is invalid.
pub fn forwarded_client_ip(header_value: &str, trusted_hops: usize) -> Option<IpAddr> {
    if trusted_hops == 0 {
        return None;
    }
    let entries: Vec<&str> = header_value.split(',').map(str::trim).collect();
    let index = entries.len().checked_sub(trusted_hops)?;
    entries[index].parse::<IpAddr>().ok().map(canonical_ip)
}

/// IPv4-mapped IPv6 addresses are their IPv4 address.
pub fn canonical_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(v6)),
        v4 => v4,
    }
}

/// True only for globally routable unicast addresses: never private, loopback, link-local,
/// carrier-grade NAT, multicast, broadcast, documentation, reserved or unspecified ranges.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match canonical_ip(ip) {
        IpAddr::V4(v4) => {
            let [first, second, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast()
                || first == 0
                || first >= 240
                || (first == 100 && (64..128).contains(&second)))
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || first == 0x2001 && v6.segments()[1] == 0x0db8)
        }
    }
}

/// The rate-limit bucket for an address: the IPv4 address, or the IPv6 /64 a single subscriber
/// usually controls.
pub fn rate_limit_bucket(ip: IpAddr) -> String {
    match canonical_ip(ip) {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            format!(
                "{:x}:{:x}:{:x}:{:x}::/64",
                segments[0], segments[1], segments[2], segments[3]
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_trusted_proxy_entry_wins_over_client_supplied_prefixes() {
        let spoofed = "8.8.8.8, 1.1.1.1, 203.0.113.9";
        assert_eq!(
            forwarded_client_ip(spoofed, 1),
            Some("203.0.113.9".parse().unwrap())
        );
        assert_eq!(
            forwarded_client_ip(spoofed, 2),
            Some("1.1.1.1".parse().unwrap())
        );
        assert_eq!(forwarded_client_ip("1.1.1.1", 2), None);
        assert_eq!(forwarded_client_ip("1.1.1.1", 0), None);
        assert_eq!(forwarded_client_ip("x, not-an-ip", 1), None);
        assert_eq!(
            forwarded_client_ip("::ffff:9.9.9.9", 1),
            Some("9.9.9.9".parse().unwrap())
        );
    }

    #[test]
    fn only_globally_routable_unicast_addresses_are_public() {
        for (ip, public) in [
            ("8.8.8.8", true),
            ("2001:4860:4860::8888", true),
            ("10.1.2.3", false),
            ("192.168.1.1", false),
            ("100.64.0.1", false),
            ("127.0.0.1", false),
            ("0.1.2.3", false),
            ("224.0.0.1", false),
            ("240.0.0.1", false),
            ("255.255.255.255", false),
            ("203.0.113.9", false),
            ("::ffff:10.0.0.1", false),
            ("::ffff:8.8.8.8", true),
            ("::1", false),
            ("fd00::1", false),
            ("fe80::1", false),
            ("ff02::1", false),
            ("2001:db8::1", false),
        ] {
            assert_eq!(is_public_ip(ip.parse().unwrap()), public, "{ip}");
        }
    }

    #[test]
    fn ipv6_clients_share_their_slash_64_bucket() {
        assert_eq!(
            rate_limit_bucket("2001:4860:4860:1:2:3:4:5".parse().unwrap()),
            rate_limit_bucket("2001:4860:4860:1:ffff::".parse().unwrap())
        );
        assert_eq!(rate_limit_bucket("8.8.8.8".parse().unwrap()), "8.8.8.8");
    }
}
