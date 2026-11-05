//! Public, read-only wallet observation for remote posting assistants.
use super::*;

#[utoipa::path(get, path="/v1/base/posting-wallets/{address}", params(("address"=String,Path,description="Public Base mainnet wallet address")), responses((status=200,description="Safe-block native USDC balance; no ownership, approval or funding assertion"),(status=400,description="Invalid address"),(status=503,description="Fresh chain observation unavailable")))]
pub(super) async fn wallet(
    State(state): State<SharedState>,
    Path(address): Path<String>,
) -> Result<Response, StatusCode> {
    let address = normalize_evm_address(&address).map_err(|_| StatusCode::BAD_REQUEST)?;
    if address == "0x0000000000000000000000000000000000000000" {
        return Err(StatusCode::BAD_REQUEST);
    }
    let (_, rpc) = state
        .base_rpc_urls
        .resolve("base-mainnet")
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let observation = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        if chain_base::fetch_chain_id(&rpc, 1).await? != 8453 {
            return Err(chain_base::ChainBaseError::InvalidRpcResponse(
                "wrong chain".into(),
            ));
        }
        chain_base::observe_erc20_balance_safe(
            &rpc,
            chain_base::BASE_MAINNET_USDC_TOKEN_ADDRESS,
            &address,
            2,
        )
        .await
    })
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let now = u64::try_from(Utc::now().timestamp()).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if now.saturating_sub(observation.safe_block_timestamp) > 1200
        || observation.safe_block_timestamp > now.saturating_add(30)
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    Ok(([(header::CACHE_CONTROL,"no-store")],Json(serde_json::json!({
        "schema":"agent-bounties/posting-wallet-observation-v1", "network":"base-mainnet", "chain_id":8453,
        "wallet":address,"token":observation.token,"usdc_base_units":observation.balance.to_string(),
        "decimals":6,"safe_block_number":observation.safe_block_number,
        "safe_block_hash":observation.safe_block_hash,"safe_block_timestamp":observation.safe_block_timestamp,
        "observed_at":Utc::now(),"wallet_connected":false,"ownership_verified":false,
        "evidence_boundary":"A public balance is not wallet ownership, draft approval, a reserved balance or funded bounty. Recheck the exact principal and sponsorship at the first-party review."
    }))).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn invalid_wallets_and_missing_rpc_do_not_become_zero_balances() {
        let state = crate::tests::test_state(BountyNetwork::default());
        for address in [
            "not-an-address",
            "0x0000000000000000000000000000000000000000",
        ] {
            assert_eq!(
                wallet(State(state.clone()), Path(address.into()))
                    .await
                    .err(),
                Some(StatusCode::BAD_REQUEST)
            );
        }
        assert_eq!(
            wallet(
                State(state),
                Path("0x1111111111111111111111111111111111111111".into())
            )
            .await
            .err(),
            Some(StatusCode::SERVICE_UNAVAILABLE)
        );
    }
}
