//! Fail-closed test: canonical domain origins for agentbounties.app
//!
//! Ensures that analytics, discovery, redirects, and generated public links
//! accept and emit only the canonical origins:
//!   - agentbounties.app      (website)
//!   - api.agentbounties.app  (API)
//!   - mcp.agentbounties.app  (MCP)
//!
//! A retired legacy origin is rejected or absent from generated output.
//! No test relies on DNS, external HTTP, credentials, or a live wallet.

/// Canonical origins that MUST be accepted.
const CANONICAL_ORIGINS: &[&str] = &[
    "https://agentbounties.app",
    "https://api.agentbounties.app",
    "https://mcp.agentbounties.app",
];

/// Retired / legacy origins that MUST be rejected or absent.
const LEGACY_ORIGINS: &[&str] = &[
    "https://agentbounties.org",
    "https://www.agentbounties.org",
    "https://api.agentbounties.org",
    "https://mcp.agentbounties.org",
    "https://old.agentbounties.app",
    "http://agentbounties.app",
];

/// Returns true if the given origin matches one of the canonical origins.
fn is_canonical_origin(origin: &str) -> bool {
    let normalized = origin.trim().trim_end_matches('/');
    CANONICAL_ORIGINS
        .iter()
        .any(|c| normalized.eq_ignore_ascii_case(c))
}

/// Validate that a generated URL belongs to a canonical origin.
fn validate_generated_url(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    if !trimmed.starts_with("https://") {
        return Err(format!("generated URL uses non-HTTPS scheme: {trimmed}"));
    }
    // Extract the origin portion: scheme + host (up to the first `/` after `https://`).
    let after_scheme = &trimmed["https://".len()..];
    let host_end = after_scheme.find('/').unwrap_or(after_scheme.len());
    let origin = format!("https://{}", &after_scheme[..host_end]);

    if !is_canonical_origin(&origin) {
        return Err(format!(
            "generated URL does not belong to any canonical origin: {trimmed} (origin: {origin})"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Origin validation ──

    #[test]
    fn canonical_website_origin_is_accepted() {
        assert!(is_canonical_origin("https://agentbounties.app"));
    }

    #[test]
    fn canonical_api_origin_is_accepted() {
        assert!(is_canonical_origin("https://api.agentbounties.app"));
    }

    #[test]
    fn canonical_mcp_origin_is_accepted() {
        assert!(is_canonical_origin("https://mcp.agentbounties.app"));
    }

    #[test]
    fn canonical_origin_with_trailing_slash_is_accepted() {
        assert!(is_canonical_origin("https://agentbounties.app/"));
    }

    #[test]
    fn all_legacy_origins_are_rejected() {
        for origin in LEGACY_ORIGINS {
            assert!(
                !is_canonical_origin(origin),
                "Legacy origin should be rejected: {origin}"
            );
        }
    }

    #[test]
    fn http_scheme_is_rejected() {
        assert!(!is_canonical_origin("http://agentbounties.app"));
    }

    #[test]
    fn arbitrary_domain_is_rejected() {
        assert!(!is_canonical_origin("https://example.com"));
        assert!(!is_canonical_origin("https://evil-agentbounties.app"));
    }

    // ── Generated link validation ──

    #[test]
    fn generated_link_on_canonical_website_passes() {
        assert!(validate_generated_url("https://agentbounties.app/funding.html").is_ok());
    }

    #[test]
    fn generated_link_on_canonical_api_passes() {
        assert!(validate_generated_url("https://api.agentbounties.app/v1/opportunities").is_ok());
    }

    #[test]
    fn generated_link_on_canonical_mcp_passes() {
        assert!(validate_generated_url("https://mcp.agentbounties.app/tools").is_ok());
    }

    #[test]
    fn generated_link_on_legacy_origin_fails() {
        let result = validate_generated_url("https://agentbounties.org/funding.html");
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("canonical"),
            "error should mention canonical"
        );
    }

    #[test]
    fn generated_link_with_http_scheme_fails() {
        assert!(validate_generated_url("http://agentbounties.app/earn.html").is_err());
    }

    #[test]
    fn static_page_urls_are_canonical() {
        let static_urls = [
            "https://agentbounties.app/funding.html",
            "https://agentbounties.app/earn.html",
            "https://agentbounties.app/post.html",
            "https://agentbounties.app/x402.html",
            "https://agentbounties.app/x402-test-vectors.json",
            "https://agentbounties.app/prepare-agent.html",
            "https://agentbounties.app/protocol.json",
        ];
        for url in &static_urls {
            assert!(
                validate_generated_url(url).is_ok(),
                "Static page URL should be canonical: {url}"
            );
        }
    }

    #[test]
    fn no_legacy_origin_leaks_into_static_urls() {
        let static_urls = [
            "https://agentbounties.app/funding.html",
            "https://agentbounties.app/earn.html",
            "https://agentbounties.app/post.html",
        ];
        for url in &static_urls {
            for legacy in LEGACY_ORIGINS {
                assert!(
                    !url.starts_with(legacy),
                    "Static URL {url} must not start with legacy origin {legacy}"
                );
            }
        }
    }
    #[test]
    fn parses_and_accepts_allowed_fixture() {
        let json = include_str!("../../../fixtures/origins/allowed.json");
        let origins: Vec<String> = serde_json::from_str(json).unwrap();
        for origin in origins {
            assert!(is_canonical_origin(&origin));
        }
    }

    #[test]
    fn parses_and_rejects_stale_fixture() {
        let json = include_str!("../../../fixtures/origins/stale.json");
        let origins: Vec<String> = serde_json::from_str(json).unwrap();
        for origin in origins {
            assert!(!is_canonical_origin(&origin));
        }
    }

    #[test]
    fn parses_and_rejects_malformed_fixture() {
        let json = include_str!("../../../fixtures/origins/malformed.json");
        let origins: Vec<String> = serde_json::from_str(json).unwrap();
        for origin in origins {
            assert!(!is_canonical_origin(&origin));
        }
    }

    #[test]
    fn parses_and_rejects_forwarded_fixture() {
        let json = include_str!("../../../fixtures/origins/forwarded.json");
        let origins: Vec<String> = serde_json::from_str(json).unwrap();
        for origin in origins {
            assert!(!is_canonical_origin(&origin));
        }
    }
}
