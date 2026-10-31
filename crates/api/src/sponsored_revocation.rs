//! Sponsor Circle's nonce cancellation. Cancellation never refunds spent USDC.
use super::*;
use alloy::{
    primitives::{B256, U256},
    sol_types::{SolCall, SolStruct},
};

#[derive(Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct SponsoredRevocationRequest {
    network: String,
    authorizer: String,
    nonce: String,
    signature: String,
}
alloy::sol! {
    struct CancelAuthorization { address authorizer; bytes32 nonce; }
    function cancelAuthorization(address authorizer, bytes32 nonce, bytes signature);
}
pub(super) fn digest(network: &str, authorizer: &str, nonce: &str) -> Result<B256, StatusCode> {
    let n = base_network_descriptor(network).map_err(|_| StatusCode::BAD_REQUEST)?;
    let m = CancelAuthorization {
        authorizer: authorizer.parse().map_err(|_| StatusCode::BAD_REQUEST)?,
        nonce: nonce.parse().map_err(|_| StatusCode::BAD_REQUEST)?,
    };
    Ok(m.eip712_signing_hash(&alloy::sol_types::Eip712Domain {
        name: Some(
            if n.chain_id == 8453 {
                "USD Coin"
            } else {
                "USDC"
            }
            .into(),
        ),
        version: Some("2".into()),
        chain_id: Some(U256::from(n.chain_id)),
        verifying_contract: Some(n.native_usdc_token_address.parse().unwrap()),
        salt: None,
    }))
}
async fn reconcile(
    state: &SharedState,
    network: &str,
    authorizer: &str,
    nonce: &str,
    op: &db::SponsoredAction,
) -> Result<bool, StatusCode> {
    let Some(receipt) = sponsored_verdict::confirmed_receipt(state, network, op).await? else {
        return Ok(false);
    };
    let token = base_network_descriptor(network)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .native_usdc_token_address;
    let topic = format!(
        "{:#x}",
        alloy::primitives::keccak256(b"AuthorizationCanceled(address,bytes32)")
    );
    let address_topic = format!("0x{:0>64}", authorizer.trim_start_matches("0x"));
    if !receipt.logs.iter().any(|log| {
        log.address.eq_ignore_ascii_case(&token)
            && log.topics.len() == 3
            && log.topics[0].eq_ignore_ascii_case(&topic)
            && log.topics[1].eq_ignore_ascii_case(&address_topic)
            && log.topics[2].eq_ignore_ascii_case(nonce)
            && log.data == "0x"
    }) {
        return Err(StatusCode::BAD_GATEWAY);
    }
    state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .finish_sponsored_action(op.id, None, "confirmed", None, None)
        .await
        .map_err(map_x402_db_error)?;
    Ok(true)
}

#[utoipa::path(post, path="/v1/base/usdc/authorization-revocation-relay", request_body = SponsoredRevocationRequest, responses((status=200,description="Canonical USDC AuthorizationCanceled confirmed"),(status=202,description="Cancellation reserved or broadcast"),(status=401,description="Invalid authorizer signature"),(status=503,description="Sponsorship unavailable or cancellation failed; retain original operation")))]
pub(super) async fn relay(
    State(state): State<SharedState>,
    Json(request): Json<SponsoredRevocationRequest>,
) -> Result<Response, StatusCode> {
    let network = request.network.as_str();
    if !["base-mainnet", "base-sepolia"].contains(&network) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let authorizer =
        normalize_evm_address(&request.authorizer).map_err(|_| StatusCode::BAD_REQUEST)?;
    let nonce = request.nonce.to_ascii_lowercase();
    let digest = digest(network, &authorizer, &nonce)?;
    let signature =
        sponsored_signature::verify(&state, network, &authorizer, digest, &request.signature)
            .await?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let key = format!("usdc-revocation:{digest:#x}");
    let previous = store
        .find_sponsored_action(network, &key)
        .await
        .map_err(map_x402_db_error)?;
    let mut op = if let Some(op) = previous.filter(|v| v.transaction_hash.is_some()) {
        op
    } else {
        if !sponsored_creation::enabled(&state, network) {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        let op = store
            .reserve_sponsored_action(
                network,
                &key,
                &authorizer,
                state.x402_relayer.max_daily_attempts,
                state.x402_relayer.max_daily_attempts_per_contributor,
            )
            .await
            .map_err(map_x402_db_error)?;
        let intent = EvmTransactionIntent {
            from: state.x402_relayer.address(),
            to: base_network_descriptor(network)
                .unwrap()
                .native_usdc_token_address,
            value_wei: 0,
            function: "cancelAuthorization(address,bytes32,bytes)".into(),
            data: format!(
                "0x{}",
                hex::encode(
                    cancelAuthorizationCall {
                        authorizer: authorizer.parse().unwrap(),
                        nonce: nonce.parse().unwrap(),
                        signature: signature.clone()
                    }
                    .abi_encode()
                )
            ),
        };
        sponsored_verdict::broadcast(&state, network, op, &intent, 350_000).await?
    };
    if sponsored_creation::enabled(&state, network) {
        op = sponsored_verdict::retry_saved(
            &state,
            network,
            op,
            (Utc::now().timestamp() as u64).saturating_add(60),
        )
        .await?;
    }
    let confirmed = tokio::time::timeout(
        Duration::from_secs(state.x402_relayer.rpc_timeout_seconds),
        reconcile(&state, network, &authorizer, &nonce, &op),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .unwrap_or(false);
    op = store
        .sponsored_action(op.id)
        .await
        .map_err(map_x402_db_error)?;
    let status = if confirmed {
        StatusCode::OK
    } else if op.status == "failed" || op.attempts >= 3 && op.transaction_hash.is_none() {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::ACCEPTED
    };
    Ok((status,[(header::CACHE_CONTROL,"no-store, private")],Json(serde_json::json!({
        "schema":"agent-bounties/sponsored-usdc-revocation-v1","network":network,"authorizer":authorizer,"nonce":nonce,
        "authorization_cancelled":confirmed,"customer_gas_wei":"0","solver_paid":false,"transaction_hash":op.transaction_hash,"operation":op,
        "next_step":"Reconcile the original bounty before replacing it. Cancellation disables only this unused authorization; it never refunds USDC already transferred."
    }))).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::signers::{local::PrivateKeySigner, SignerSync};
    #[test]
    fn cancellation_binds_authorizer_nonce_chain_and_circle_domain() {
        let key: PrivateKeySigner = format!("{:064x}", 1).parse().unwrap();
        let authorizer = format!("{:#x}", key.address());
        let nonce = format!("0x{}", "11".repeat(32));
        let sig = key
            .sign_hash_sync(&digest("base-mainnet", &authorizer, &nonce).unwrap())
            .unwrap();
        for (network, wallet, nonce) in [
            ("base-sepolia", authorizer.clone(), nonce.clone()),
            (
                "base-mainnet",
                format!("0x{}", "22".repeat(20)),
                nonce.clone(),
            ),
            (
                "base-mainnet",
                authorizer.clone(),
                format!("0x{}", "22".repeat(32)),
            ),
        ] {
            assert_ne!(
                sig.recover_address_from_prehash(&digest(network, &wallet, &nonce).unwrap())
                    .unwrap(),
                key.address()
            );
        }
    }
}
