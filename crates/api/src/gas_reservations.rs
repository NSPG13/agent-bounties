//! Reserve gas for the separately signed legacy keeper scripts. This credential
//! can only consume the gas allowance; it cannot sign or submit a transaction.
use super::*;

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct GasReservationRequest {
    network: String,
    relayer_address: String,
    transaction_digest: String,
    maximum_fee_wei: String,
}

fn authorize(headers: &HeaderMap, configured: Option<&str>) -> Result<(), StatusCode> {
    // The public runtime helper historically permits an unconfigured operator.
    // This spending endpoint must fail closed regardless of helper behavior.
    let configured = configured
        .filter(|value| value.len() >= 32 && !value.trim().is_empty())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values.next().and_then(|value| value.to_str().ok());
    if values.next().is_some()
        || !service_runtime::operator_token_is_authorized(Some(configured), None, value)
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

#[utoipa::path(post,path="/v1/operator/gas-sponsorship/reservations",request_body=GasReservationRequest,responses((status=200,description="Gas budget reserved; no transaction submitted"),(status=401,description="Dedicated gas budget credential required"),(status=503,description="Budget or fresh quote unavailable; keeper must not broadcast")))]
pub(super) async fn reserve(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<GasReservationRequest>,
) -> Result<Response, StatusCode> {
    authorize(
        &headers,
        env::var("GAS_SPONSOR_BUDGET_TOKEN")
            .ok()
            .filter(|value| value.len() >= 32)
            .as_deref(),
    )?;
    let chain = base_network_descriptor(&request.network)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .chain_id;
    if ![8453, 84532].contains(&chain)
        || request.transaction_digest.len() != 64
        || !request
            .transaction_digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let sender =
        normalize_evm_address(&request.relayer_address).map_err(|_| StatusCode::BAD_REQUEST)?;
    let (_, rpc_url) = state
        .base_rpc_urls
        .resolve(&request.network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let maximum = request
        .maximum_fee_wei
        .parse::<u128>()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    if maximum == 0 || maximum > 100_000_000_000_000 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    // Legacy processes may share the API treasury key. Hold the same network
    // lease before issuing a spending ticket, so they cannot select its nonce
    // concurrently with any hosted relay. A successful ticket keeps the lease
    // for 45 seconds even if the legacy process loses its response or crashes.
    // The client must start within 30 seconds and limits cast to 10 seconds.
    let started = Utc::now().timestamp();
    let lease = store
        .acquire_x402_relayer_lease(&request.network, 45)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let reservation = tokio::time::timeout(Duration::from_secs(8), async {
        use chain_base::JsonRpcTransport;
        let transport = chain_base::ReqwestJsonRpcTransport::default();
        let chain_response=transport.post_json_value(&rpc_url,&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]})).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
        if chain_response["jsonrpc"]!="2.0" || chain_response["id"]!=1 || chain_response.get("error").is_some() || chain_response["result"]!=format!("0x{chain:x}") {return Err(StatusCode::BAD_GATEWAY);}
        let nonce_response=transport.post_json_value(&rpc_url,&serde_json::json!({"jsonrpc":"2.0","id":2,"method":"eth_getTransactionCount","params":[sender,"pending"]})).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
        if nonce_response["jsonrpc"]!="2.0" || nonce_response["id"]!=2 || nonce_response.get("error").is_some() {return Err(StatusCode::BAD_GATEWAY);}
        let nonce=nonce_response["result"].as_str().and_then(|v|v.strip_prefix("0x")).and_then(|v|u64::from_str_radix(v,16).ok()).filter(|v|*v<=i64::MAX as u64).ok_or(StatusCode::BAD_GATEWAY)?;
        let budget=store.gas_sponsor_budget();
        budget.ensure_sender_nonce_available(chain,&sender,nonce).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
        budget.reserve(chain,maximum).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
        Ok::<u64,StatusCode>(nonce)
    }).await;
    let nonce = match reservation {
        Ok(Ok(nonce)) => nonce,
        _ => {
            let _ = store
                .release_x402_relayer_lease(&request.network, lease)
                .await;
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
    };
    Ok(([(header::CACHE_CONTROL,"no-store, private")],Json(serde_json::json!({
        "schema":"agent-bounties/operator-gas-reservation-v1", "reserved":true,
        "network":request.network,"relayer_address":request.relayer_address,"sender_nonce":nonce,"transaction_digest":request.transaction_digest,"maximum_fee_wei":maximum.to_string(),
        "valid_until":started+30,"signer_lease_expires_at":started+45,"shared_signer_lease":true,"carries_forward":false,"daily_limit_micro_usdc":db::DEFAULT_GAS_BUDGET_MICRO_USDC,
        "transaction_submitted":false
    }))).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn budget_only_credential_fails_closed_and_rejects_duplicate_headers() {
        let token = "dedicated-budget-fixture-credential";
        let mut headers = HeaderMap::new();
        assert_eq!(
            authorize(&headers, Some(token)),
            Err(StatusCode::UNAUTHORIZED)
        );
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        assert_eq!(authorize(&headers, None), Err(StatusCode::UNAUTHORIZED));
        assert_eq!(
            authorize(&headers, Some("different-fixture-token")),
            Err(StatusCode::UNAUTHORIZED)
        );
        assert_eq!(authorize(&headers, Some(token)), Ok(()));
        headers.append(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        assert_eq!(
            authorize(&headers, Some(token)),
            Err(StatusCode::UNAUTHORIZED)
        );
    }
}
