//! Direct EIP-3009 contribution relay for the first-party wallet review.
use super::*;

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct SponsoredContributionRequest {
    network: String,
    contribution: AutonomousBountyContribution,
    signature: String,
}

fn response(state: &SharedState, attempt: &X402RelayAttempt) -> Response {
    let status = match attempt.status {
        X402RelayStatus::Confirmed => StatusCode::OK,
        X402RelayStatus::Failed => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::ACCEPTED,
    };
    (status, [(header::CACHE_CONTROL, "no-store, private")], Json(serde_json::json!({
        "schema":"agent-bounties/sponsored-contribution-v1", "customer_gas_wei":"0", "solver_paid":false,
        "funding_confirmed":attempt.status == X402RelayStatus::Confirmed, "relay":x402_public_relay(attempt),
        "statusUrl":format!("{}/v1/x402/base/relays/{}",state.public_base_url.trim_end_matches('/'),attempt.id),
        "next_step":"Reconcile the same authorization and bounty. Only canonical FundingAdded proves this contribution."
    }))).into_response()
}

#[utoipa::path(post, path="/v1/base/autonomous-bounties/contribution-relay", request_body = SponsoredContributionRequest, responses((status=200,description="Exact canonical contribution confirmed"),(status=202,description="Contribution reserved or broadcast"),(status=401,description="Invalid contributor signature"),(status=409,description="Funding changed; reconcile the original authorization"),(status=503,description="Sponsorship unavailable; no customer-paid fallback")))]
pub(super) async fn relay(
    State(state): State<SharedState>,
    Json(request): Json<SponsoredContributionRequest>,
) -> Result<Response, StatusCode> {
    let network = request.network.as_str();
    if !["base-mainnet", "base-sepolia"].contains(&network) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let c = &request.contribution;
    let amount = u64::try_from(c.amount.amount).map_err(|_| StatusCode::BAD_REQUEST)?;
    let expiry = c
        .authorization_valid_before
        .ok_or(StatusCode::BAD_REQUEST)?;
    let nonce = c
        .authorization_nonce
        .as_deref()
        .ok_or(StatusCode::BAD_REQUEST)?
        .to_ascii_lowercase();
    let contract =
        normalize_evm_address(&c.bounty_contract).map_err(|_| StatusCode::BAD_REQUEST)?;
    let contributor = normalize_evm_address(&c.contributor).map_err(|_| StatusCode::BAD_REQUEST)?;
    let planner = configured_autonomous_new_risk_planner(network)?;
    let typed = planner
        .plan_contribution(network, c)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .eip3009_authorization
        .ok_or(StatusCode::BAD_REQUEST)?;
    let signature = sponsored_signature::verify(
        &state,
        network,
        &contributor,
        chain_base::creation_authorization_digest(&typed).map_err(|_| StatusCode::BAD_REQUEST)?,
        &request.signature,
    )
    .await?;
    let signature = sponsored_signature::rsv(&signature)?;
    let fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&typed).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    ));
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if let Some(mut attempt) = store
        .get_x402_relay_attempt_by_authorization(network, &contract, &nonce)
        .await
        .map_err(map_x402_db_error)?
    {
        if attempt.request_fingerprint != fingerprint {
            return Err(StatusCode::CONFLICT);
        }
        if attempt.tx_hash.is_some() {
            if attempt.status == X402RelayStatus::Broadcast {
                if sponsored_creation::enabled(&state, network) {
                    // Explicit POST retries may resend only the durable original
                    // bytes. A paused budget/provider still returns the saved hash.
                    let _ = sponsored_verdict::retry_x402_saved(&state, &attempt).await;
                }
                // Returning the saved hash is more useful than losing it to a receipt outage.
                if let Ok(current) = reconcile_x402_relay(&state, attempt.clone()).await {
                    attempt = current;
                }
            }
            return Ok(response(&state, &attempt));
        }
        if attempt.attempt_count >= 3 {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
    }
    if !sponsored_creation::enabled(&state, network) {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let now = Utc::now().timestamp() as u64;
    if expiry <= now.saturating_add(30) || expiry > now.saturating_add(1800) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let item = indexed_autonomous_bounty(&state, network, &contract).await?;
    service_runtime::verifier_checkpoints::require_available(
        state.store.as_ref(),
        network,
        item.terms.as_ref().ok_or(StatusCode::CONFLICT)?,
    )
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if !item.terms_valid {
        return Err(StatusCode::CONFLICT);
    }
    resolve_x402_funding_amount(
        &item.status,
        &item.target_amount,
        &item.funded_amount,
        Some(amount),
    )?;
    let relayer = state
        .x402_relayer
        .address()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let plan = planner
        .plan_authorized_contribution(network, c, &signature, Some(&relayer))
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    validate_hosted_x402_intent(&plan.relay_transaction, &relayer, &contract)?;
    let attempt = store
        .reserve_x402_relay_attempt(
            &NewX402RelayAttempt {
                id: Uuid::new_v4(),
                idempotency_key: format!("fund:{network}:{contract}:{nonce}"),
                network: network.into(),
                bounty_contract: contract,
                contributor,
                amount,
                authorization_nonce: nonce,
                authorization_valid_before: expiry,
                request_fingerprint: fingerprint,
                relayer_address: relayer,
            },
            state.x402_relayer.max_daily_attempts,
            state.x402_relayer.max_daily_attempts_per_contributor,
        )
        .await
        .map_err(map_x402_db_error)?;
    let attempt = process_x402_hosted_relay(&state, attempt, &plan.relay_transaction).await?;
    Ok(response(&state, &attempt))
}
