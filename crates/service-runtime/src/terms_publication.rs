//! Shared input boundary for hosted terms writes; no address string grants authority.
use crate::auth::{self, AuthError};
use domain::AutonomousBountyTermsDocument;
use reqwest::header::HeaderMap;

pub const SINGULAR_HEADERS: &[&str] = &[
    "authorization",
    "cookie",
    "origin",
    "idempotency-key",
    "x-operator-token",
    "x-agent-bounties-acquisition-id",
    "x-agent-bounties-handoff-id",
    "x-agent-bounties-legal-acceptance",
];

pub fn validate_headers(headers: &HeaderMap) -> Result<(), AuthError> {
    for name in SINGULAR_HEADERS {
        if headers.get_all(*name).iter().count() > 1 {
            return Err(AuthError::InvalidInput);
        }
    }
    // Never discard an explicitly supplied operator credential and fall back to
    // a cookie. Operator authority does not prove this publication's wallet.
    if headers.contains_key("x-operator-token") {
        return Err(AuthError::Unauthorized);
    }
    Ok(())
}

pub fn idempotency_key(headers: &HeaderMap) -> Result<&str, AuthError> {
    headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .filter(|v| {
            !v.trim().is_empty() && v.len() <= 200 && !v.bytes().any(|c| c.is_ascii_control())
        })
        .ok_or(AuthError::InvalidInput)
}

/// Only the document's own network/creator select the authority to require.
/// Full current creation semantics are checked separately for a new publication.
pub fn document_identity(
    document: &AutonomousBountyTermsDocument,
    claimed_creator: &str,
) -> Result<(String, String), AuthError> {
    let network = chain_base::base_network_descriptor(
        document.contract_terms["network"]
            .as_str()
            .ok_or(AuthError::InvalidInput)?,
    )
    .map_err(|_| AuthError::InvalidInput)?;
    let network = match network.chain_id {
        8453 => "base-mainnet",
        84532 => "base-sepolia",
        _ => return Err(AuthError::InvalidInput),
    };
    let creator = auth::normalize_wallet(
        document.contract_terms["creator_wallet"]
            .as_str()
            .ok_or(AuthError::InvalidInput)?,
    )?;
    if creator != auth::normalize_wallet(claimed_creator)? {
        return Err(AuthError::InvalidInput);
    }
    Ok((network.into(), creator))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publication_identity_uses_reviewed_network_aliases_without_changing_document() {
        let mut document: AutonomousBountyTermsDocument = serde_json::from_value(serde_json::json!({
            "schema_version":"test", "contract_terms":{"network":"Base", "creator_wallet":"0x1111111111111111111111111111111111111111"},
            "title":"Fixture", "goal":"Fixture", "acceptance_criteria":[], "benchmark":{}, "evidence_schema":{}, "verification_policy":{}
        })).unwrap();
        for (alias, expected) in [
            ("Base", "base-mainnet"),
            ("8453", "base-mainnet"),
            ("mainnet", "base-mainnet"),
            ("base-mainnet", "base-mainnet"),
            ("sepolia", "base-sepolia"),
            ("84532", "base-sepolia"),
            ("base-sepolia", "base-sepolia"),
        ] {
            document.contract_terms["network"] = alias.into();
            assert_eq!(
                document_identity(&document, "0x1111111111111111111111111111111111111111")
                    .unwrap()
                    .0,
                expected
            );
            assert_eq!(document.contract_terms["network"], alias);
        }
        document.contract_terms["network"] = "ethereum".into();
        assert!(
            document_identity(&document, "0x1111111111111111111111111111111111111111").is_err()
        );
        document.contract_terms["network"] = "Base".into();
        assert!(
            document_identity(&document, "0x2222222222222222222222222222222222222222").is_err()
        );
    }
    #[test]
    fn publication_headers_reject_ambiguous_credentials_and_keys() {
        for name in SINGULAR_HEADERS {
            let mut headers = HeaderMap::new();
            headers.append(*name, "one".parse().unwrap());
            headers.append(*name, "two".parse().unwrap());
            assert!(validate_headers(&headers).is_err(), "{name}");
        }
        let mut headers = HeaderMap::new();
        headers.insert("cookie", "session=value".parse().unwrap());
        headers.insert("x-operator-token", "operator".parse().unwrap());
        assert!(validate_headers(&headers).is_err());
        headers.remove("x-operator-token");
        assert!(validate_headers(&headers).is_ok());
        assert!(idempotency_key(&headers).is_err());
        headers.insert("idempotency-key", "stable-key".parse().unwrap());
        assert_eq!(idempotency_key(&headers).unwrap(), "stable-key");
    }
}
