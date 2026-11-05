//! Exact signatures for fixed legacy actions. Never truncate contract signatures.
use super::*;
use alloy::primitives::{Address, Bytes, Signature, B256};

pub(super) fn bytes(value: &str) -> Result<Bytes, StatusCode> {
    if !value.starts_with("0x") || value.len() > 8194 || value.len() < 4 {
        return Err(StatusCode::UNAUTHORIZED);
    }
    value.parse::<Bytes>().map_err(|_| StatusCode::UNAUTHORIZED)
}

fn eoa(signer: Address, digest: B256, raw: &Bytes) -> Option<Bytes> {
    let signature = Signature::try_from(raw.as_ref()).ok()?;
    if signature.normalize_s().is_some()
        || signature.recover_address_from_prehash(&digest).ok()? != signer
    {
        return None;
    }
    Some(signature.as_bytes().to_vec().into())
}

pub(super) async fn verify(
    state: &SharedState,
    network: &str,
    signer: &str,
    digest: B256,
    value: &str,
) -> Result<Bytes, StatusCode> {
    let raw = bytes(value)?;
    let signer: Address = signer.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    if let Some(signature) = eoa(signer, digest, &raw) {
        return Ok(signature);
    }
    let (_, rpc) = state
        .base_rpc_urls
        .resolve(network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    verify_contract(&rpc, network, signer, digest, raw).await
}

pub(super) async fn verify_with_rpc(
    rpc: &str,
    network: &str,
    signer: &str,
    digest: B256,
    value: &str,
) -> Result<Bytes, StatusCode> {
    let raw = bytes(value)?;
    let signer: Address = signer.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    if let Some(signature) = eoa(signer, digest, &raw) {
        return Ok(signature);
    }
    verify_contract(rpc, network, signer, digest, raw).await
}

async fn verify_contract(
    rpc: &str,
    network: &str,
    signer: Address,
    digest: B256,
    raw: Bytes,
) -> Result<Bytes, StatusCode> {
    let block = creator_open::live_block(rpc)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    chain_base::verify_contract_wallet_signature(
        &chain_base::ReqwestJsonRpcTransport::default(),
        rpc,
        network,
        signer,
        digest,
        &raw,
        &block,
    )
    .await
    .map_err(|_| StatusCode::UNAUTHORIZED)?;
    Ok(raw)
}

/// The old creation/contribution factory ABI accepts only r/s/v. Contract-wallet
/// bytes must already use its exact encoding; never normalize a contract result.
pub(super) fn rsv(raw: &Bytes) -> Result<AutonomousBountyAuthorizationSignature, StatusCode> {
    if raw.len() != 65 || ![27, 28].contains(&raw[64]) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(AutonomousBountyAuthorizationSignature {
        v: raw[64],
        r: format!("0x{}", hex::encode(&raw[..32])),
        s: format!("0x{}", hex::encode(&raw[32..64])),
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn claim(
    state: &SharedState,
    network: &BaseNetworkDescriptor,
    bounty: &str,
    solver: &str,
    bond: u64,
    nonce: &str,
    deadline: u64,
    signature: &AutonomousBountyAuthorizationSignature,
    fresh: bool,
    kind: payments_x402::Eip3009Kind,
) -> Result<(), AgentClaimProblem> {
    let eoa_result = match (fresh, kind) {
        (true, payments_x402::Eip3009Kind::Transfer) => validate_claim_authorization_signature(
            network, bounty, solver, bond, nonce, deadline, signature,
        ),
        (false, payments_x402::Eip3009Kind::Transfer) => {
            validate_persisted_claim_authorization_signature(
                network, bounty, solver, bond, nonce, deadline, signature,
            )
        }
        (true, _) => validate_claim_authorization_signature_as(
            network,
            bounty,
            solver,
            bond,
            nonce,
            deadline,
            signature,
            kind,
            Utc::now().timestamp().max(0) as u64,
        ),
        (false, _) => validate_persisted_claim_authorization_signature_as(
            network, bounty, solver, bond, nonce, deadline, signature, kind,
        ),
    };
    if eoa_result.is_ok() {
        return eoa_result;
    }
    if fresh && deadline.saturating_sub(Utc::now().timestamp() as u64) < 6 {
        return eoa_result;
    }
    let network_key = canonical_base_network_key(network.chain_id)
        .ok_or_else(|| claim_queue_security::problem(StatusCode::BAD_REQUEST))?;
    let typed = if kind == payments_x402::Eip3009Kind::Receive {
        chain_base::eip3009_receive_typed_data(
            network,
            solver,
            bounty,
            bond.into(),
            0,
            deadline,
            nonce,
        )
    } else {
        configured_autonomous_planner(network_key)
            .and_then(|p| {
                p.plan_claim(
                    network_key,
                    bounty,
                    solver,
                    bond.into(),
                    Some(nonce),
                    Some(deadline),
                )
                .map_err(|_| StatusCode::BAD_REQUEST)
            })
            .and_then(|p| p.eip3009_authorization.ok_or(StatusCode::BAD_REQUEST))
            .map_err(claim_queue_security::problem)?
    };
    let digest = chain_base::creation_authorization_digest(&typed)
        .map_err(|_| claim_queue_security::problem(StatusCode::BAD_REQUEST))?;
    let raw = joined_signature(signature)?;
    verify(state, network_key, solver, digest, &raw).await.map_err(|status| agent_claim_problem(status, "authorization_invalid", "verify_authorization", "The wallet did not validate this exact bounded bond authorization.", "Preserve the prepared claim; retry the same signature after checking the wallet and network."))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::signers::{local::PrivateKeySigner, SignerSync};
    #[test]
    fn exact_signature_bounds_and_eoa_normalization_do_not_truncate_contract_data() {
        let key: PrivateKeySigner = format!("{:064x}", 123).parse().unwrap();
        let digest = B256::repeat_byte(9);
        let sig = key.sign_hash_sync(&digest).unwrap().as_bytes();
        let raw = Bytes::from(sig.to_vec());
        assert!(rsv(&raw).is_ok());
        assert_eq!(eoa(key.address(), digest, &raw).unwrap(), raw);
        let mut compact_v = sig.to_vec();
        compact_v[64] -= 27;
        assert!(rsv(&compact_v.clone().into()).is_err());
        assert_eq!(eoa(key.address(), digest, &compact_v.into()).unwrap(), raw);
        let mut wide = vec![0x7f];
        wide.extend_from_slice(&sig);
        assert!(rsv(&wide.clone().into()).is_err());
        assert!(eoa(key.address(), digest, &wide.into()).is_none());
        for invalid in [
            "0x".into(),
            "0xabc".into(),
            "0xzz".into(),
            format!("0x{}", "11".repeat(4097)),
        ] {
            assert!(bytes(&invalid).is_err());
        }
        assert_eq!(
            bytes(&format!("0x{}", "11".repeat(4096))).unwrap().len(),
            4096
        );
    }
}
