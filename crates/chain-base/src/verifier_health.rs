//! Offline signer liveness. No payment-shaped message or signature is returned.
use super::*;

pub fn check_regression_signer_health(
    key: &str,
    expected: &str,
    release_id: &str,
) -> Result<(), ChainBaseError> {
    let fail = || {
        ChainBaseError::InvalidVerificationConfiguration("verifier signer diagnostic failed".into())
    };
    let digest = release_id.strip_prefix("sha256:").ok_or_else(fail)?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(fail());
    }
    let signer: PrivateKeySigner = key.parse().map_err(|_| fail())?;
    let expected = parse_alloy_address(expected).map_err(|_| fail())?;
    if signer.address() != expected {
        return Err(fail());
    }
    let message = format!("Agent Bounties offline verifier health only. No transaction, bounty, verdict or payment authority.\n{release_id}\n{}", Uuid::new_v4());
    let hash = B256::from_slice(&Keccak256::digest(message.as_bytes()));
    let signature = signer.sign_hash_sync(&hash).map_err(|_| fail())?;
    if signature
        .recover_address_from_prehash(&hash)
        .map_err(|_| fail())?
        != expected
    {
        return Err(fail());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signer_health_requires_expected_key_and_release_without_revealing_key() {
        let key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let expected = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";
        let release = format!("sha256:{}", "a".repeat(64));
        check_regression_signer_health(key, expected, &release).unwrap();
        for (secret, address, id) in [
            ("bad-secret", expected, release.as_str()),
            (
                key,
                "0x0000000000000000000000000000000000000001",
                release.as_str(),
            ),
            (key, expected, "unbound"),
        ] {
            let error = check_regression_signer_health(secret, address, id)
                .unwrap_err()
                .to_string();
            assert!(!error.contains(secret));
            assert!(!error.contains(key));
        }
    }
}
