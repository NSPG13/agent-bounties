//! Retire only the exact synchronized authorization, proven inert at finality.
use super::*;
use alloy::{
    primitives::{Address, B256},
    sol_types::SolCall,
};

alloy::sol! {
    function authorizationState(address authorizer, bytes32 nonce) external view returns (bool);
    function balanceOf(address owner) external view returns (uint256);
}

/// This reads chain state only. Caller must bind `saved` to the private account's
/// recorded continuation hash before using it to retire or replace a draft.
pub(super) async fn prove_inert(
    saved: &serde_json::Value,
    rpc: &str,
    network: &str,
) -> Result<serde_json::Value, StatusCode> {
    let descriptor = base_network_descriptor(network).map_err(|_| StatusCode::BAD_REQUEST)?;
    if chain_base::fetch_chain_id(rpc, 0)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        != descriptor.chain_id
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let create: AutonomousBountyCreate =
        serde_json::from_value(saved["create"].clone()).map_err(|_| StatusCode::BAD_REQUEST)?;
    let expiry = saved["authorization_valid_before"]
        .as_u64()
        .ok_or(StatusCode::BAD_REQUEST)?;
    let plan = configured_autonomous_planner(network)?
        .plan_creation_with_expiry(network, &create, expiry)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let authorization = plan
        .eip3009_authorization
        .as_ref()
        .ok_or(StatusCode::BAD_REQUEST)?;
    let signature = sponsored_signature::verify_with_rpc(
        rpc,
        network,
        &create.creator,
        chain_base::creation_authorization_digest(authorization)
            .map_err(|_| StatusCode::BAD_REQUEST)?,
        saved["signature"].as_str().ok_or(StatusCode::BAD_REQUEST)?,
    )
    .await?;
    sponsored_signature::rsv(&signature)?;
    if !saved["bounty_contract"]
        .as_str()
        .is_some_and(|v| v.eq_ignore_ascii_case(&plan.predicted_bounty_contract))
        || !saved["bounty_id"]
            .as_str()
            .is_some_and(|v| v.eq_ignore_ascii_case(&plan.bounty_id))
    {
        return Err(StatusCode::CONFLICT);
    }
    let block = chain_base::fetch_finalized_block_identity(rpc, 1)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let creator: Address = create
        .creator
        .parse()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let nonce: B256 = create
        .creation_nonce
        .parse()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let target: Address = plan
        .predicted_bounty_contract
        .parse()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let token = descriptor.native_usdc_token_address.as_str();
    let code =
        chain_base::fetch_contract_code_at(rpc, &plan.predicted_bounty_contract, block.number, 2)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    // A consumed EIP-3009 authorization may have sent funds to the counterfactual
    // address without deployment. Never abandon those funds as a failed post.
    let balance = chain_base::fetch_contract_uint128_at(
        rpc,
        token,
        &format!(
            "0x{}",
            hex::encode(balanceOfCall { owner: target }.abi_encode())
        ),
        block.number,
        3,
    )
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let used = chain_base::fetch_contract_bool_at(
        rpc,
        token,
        &format!(
            "0x{}",
            hex::encode(
                authorizationStateCall {
                    authorizer: creator,
                    nonce
                }
                .abi_encode()
            )
        ),
        block.number,
        4,
    )
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let rechecked = fetch_exact_block_identity(rpc, block.number, 5)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if rechecked.hash != block.hash {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    if !inert(&code, balance, used, expiry, block.timestamp) {
        return Err(StatusCode::CONFLICT);
    }
    Ok(
        serde_json::json!({"schema":"agent-bounties/inert-creation-proof-v1","network":network,
        "bounty_contract":plan.predicted_bounty_contract,"bounty_id":plan.bounty_id,"authorizer":create.creator,"nonce":create.creation_nonce,
        "authorization_valid_before":expiry,"authorization_used_or_cancelled":used,"contract_code":"0x","usdc_balance":"0",
        "finalized_block":block, "reason":if used {"nonce_disabled_without_deployment_or_funds"} else {"expired_unused_authorization"}}),
    )
}

fn inert(code: &str, balance: u128, used: bool, expiry: u64, finalized_timestamp: u64) -> bool {
    code == "0x" && balance == 0 && (used || expiry > 0 && finalized_timestamp >= expiry)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_index_never_proves_inert_and_counterfactual_funds_stay_recoverable() {
        assert!(!inert("0x", 0, false, 200, 199));
        assert!(inert("0x", 0, false, 200, 200));
        assert!(inert("0x", 0, true, 200, 199));
        assert!(!inert("0x", 1, true, 200, 300));
        assert!(!inert("0x1234", 0, true, 200, 300));
        assert!(!inert("0x", 0, false, 0, 300));
    }
}
