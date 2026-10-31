//! Normalize public discovery inputs before querying canonical indexed records.

use chain_base::{base_network_descriptor, ChainBaseError};

pub fn network_key(network: &str) -> Result<&'static str, ChainBaseError> {
    match base_network_descriptor(network)?.chain_id {
        8_453 => Ok("base-mainnet"),
        84_532 => Ok("base-sepolia"),
        _ => Err(ChainBaseError::UnknownNetwork(network.to_string())),
    }
}

pub fn bounty_id(value: &str) -> Result<String, &'static str> {
    let digits = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"));
    if digits
        .is_some_and(|digits| digits.len() == 64 && digits.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        Ok(value.to_ascii_lowercase())
    } else {
        Err("bounty_id must be a 0x-prefixed 32-byte ID from the bounty feed; a contract address is not a bounty ID")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_aliases_use_the_indexed_namespace() {
        for alias in ["base", "base-mainnet", "mainnet", "8453", "BASE"] {
            assert_eq!(network_key(alias).unwrap(), "base-mainnet");
        }
        for alias in ["base-sepolia", "sepolia", "84532", "BASE-SEPOLIA"] {
            assert_eq!(network_key(alias).unwrap(), "base-sepolia");
        }
        for invalid in ["", "ethereum", "8454", "base-mainnett"] {
            assert!(network_key(invalid).is_err());
        }
    }

    #[test]
    fn event_filter_rejects_contracts_and_malformed_ids() {
        let valid = format!("0x{}", "AB".repeat(32));
        assert_eq!(bounty_id(&valid).unwrap(), valid.to_ascii_lowercase());
        for invalid in [
            "".to_string(),
            format!("0x{}", "ab".repeat(20)),
            format!("0x{}", "zz".repeat(32)),
            "ab".repeat(32),
        ] {
            assert!(bounty_id(&invalid).is_err());
        }
    }
}
