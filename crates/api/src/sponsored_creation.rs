//! Gas-only sponsorship. The creator's USDC goes directly to its CREATE2 bounty.
use super::*;

const MAX_AUTHORIZATION_SECONDS: u64 = 7200;
pub(super) const CREATION_MAX_GAS: u64 = 1_500_000;

pub(super) fn enabled(state: &SharedState, network: &str) -> bool {
    env_flag("ENABLE_SPONSORED_BOUNTY_CREATION")
        && state.x402_relayer.enabled
        && state
            .x402_relayer
            .relayer
            .as_ref()
            .is_some_and(|relayer| relayer.has_gas_budget())
        && state.store.is_some()
        && state.base_rpc_urls.resolve(network).is_ok()
}

#[utoipa::path(get, path = "/v1/base/gas-sponsorship", responses((status = 200, description = "Configured gas sponsorship availability; exact simulation and reserve checks occur at relay time")))]
pub(super) async fn readiness(State(state): State<SharedState>) -> Response {
    let budget = match &state.store {
        Some(store) => store.gas_sponsor_budget_status().await.ok(),
        None => None,
    };
    let available = enabled(&state, "base-mainnet")
        && budget
            .as_ref()
            .is_some_and(|budget| budget.remaining_micro_usdc > 0);
    ([(header::CACHE_CONTROL, "no-store")], Json(serde_json::json!({
        "schema": "agent-bounties/gas-sponsorship-v1", "network": "base-mainnet",
        "creation": {"available": available, "method": "eip3009", "customer_gas_wei": "0",
            "authorization_lifetime_seconds": MAX_AUTHORIZATION_SECONDS,
            "wallet_support": "EOA and deployed ERC-1271 wallets validating an exact 65-byte r/s/v authorization; wider signatures require the creator-open protocol",
            "endpoint": "/v1/base/autonomous-bounties/creation-relay",
            "reason": if available { "ready" } else if budget.as_ref().is_some_and(|value| value.remaining_micro_usdc == 0) { "daily_budget_exhausted" } else { "sponsorship_unavailable" }},
        "silent_customer_paid_fallback": false,
        "creator_verdict": {"available": available, "customer_gas_wei":"0", "endpoint":"/v1/base/autonomous-bounties/creator-verdict-relay"},
        "submission": {"available": available, "customer_gas_wei":"0", "endpoint":"/v1/base/autonomous-bounties/submission-relay"},
        "contribution": {"available": available, "customer_gas_wei":"0", "endpoint":"/v1/base/autonomous-bounties/contribution-relay"},
        "claim": {"available": available, "customer_gas_wei":"0", "endpoint":"/v1/base/autonomous-bounties/claims"},
        "authorization_revocation": {"available": available, "customer_gas_wei":"0", "endpoint":"/v1/base/usdc/authorization-revocation-relay"},
        "daily_budget": budget,
        "universal_coverage": false,
        "coverage": {
            "signed_eoa_creation": "this_endpoint",
            "contribution": "signed_contribution_relay_or_existing_x402",
            "claim": "existing_agent_native_claim_readiness_and_caps",
            "timeout_recovery": "existing_timeout_relay_readiness",
            "creator_verdict": "signed_creator_verdict_relay",
            "submission": "signed_solver_submission_relay",
            "legacy_owner_cancel_and_refund": "requires_sponsored_account_or_protocol_extension",
            "wallet_setup_and_meta_child": "setup_has_separate_release_gate_meta_child_requires_original_caller",
            "setup_capabilities_endpoint": "/v1/base/sponsored-setup/capabilities"
        },
        "evidence_boundary": "Availability is not a funding promise. Exact signature, simulation, quota and gas reserve checks run before broadcast."
    }))).into_response()
}

fn validate_expiry(now: u64, expiry: u64, funding_deadline: u64) -> Result<(), StatusCode> {
    if expiry <= now.saturating_add(30)
        || expiry > now.saturating_add(MAX_AUTHORIZATION_SECONDS)
        || expiry > funding_deadline
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    Ok(())
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/creation-relay", request_body = PlanAutonomousBountyAuthorizedCreationRequest, responses((status = 200, description = "Canonical creation and initial funding confirmed"), (status = 202, description = "Exact signed request reserved or broadcast"), (status = 401, description = "Invalid creator signature"), (status = 429, description = "Gas sponsor capacity reached"), (status = 503, description = "Sponsor unavailable; no customer-paid fallback")))]
pub(super) async fn relay(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousBountyAuthorizedCreationRequest>,
) -> Result<Response, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    if network != "base-mainnet" && network != "base-sepolia" {
        return Err(StatusCode::BAD_REQUEST);
    }
    let expiry = request
        .authorization_valid_before
        .ok_or(StatusCode::BAD_REQUEST)?;
    let creator =
        normalize_evm_address(&request.create.creator).map_err(|_| StatusCode::BAD_REQUEST)?;
    let terms = require_autonomous_creation_terms(&state, network, &request.create).await?;
    let relayer = state
        .x402_relayer
        .address()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let planner = configured_autonomous_new_risk_planner(network)?;
    let unsigned = planner
        .plan_creation_with_expiry(network, &request.create, expiry)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let authorization = unsigned
        .eip3009_authorization
        .as_ref()
        .ok_or(StatusCode::BAD_REQUEST)?;
    if chain_base::verify_creation_authorization(authorization, &request.signature).is_err() {
        let (_, rpc) = state
            .base_rpc_urls
            .resolve(network)
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let block = creator_open::live_block(&rpc)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        chain_base::verify_creation_contract_authorization(
            &chain_base::ReqwestJsonRpcTransport::default(),
            &rpc,
            authorization,
            &request.signature,
            &block,
        )
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    }
    let amount = u64::try_from(request.create.initial_funding.amount)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let total = request
        .create
        .solver_reward
        .amount
        .checked_add(request.create.verifier_reward.amount)
        .ok_or(StatusCode::BAD_REQUEST)?;
    if amount == 0 || request.create.initial_funding.amount != total {
        return Err(StatusCode::BAD_REQUEST);
    }
    let plan = planner
        .plan_authorized_creation_with_expiry(
            network,
            &request.create,
            &request.signature,
            Some(&relayer),
            expiry,
        )
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    // Fingerprint semantic authorization, not ECDSA bytes: a different encoding
    // of the same signature must not cause a second sponsorship reservation.
    let fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&serde_json::json!({
            "action":"create", "network":network, "create":request.create, "expiry":expiry,
        }))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    ));
    let now =
        u64::try_from(Utc::now().timestamp()).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    // A lost reply remains reconcilable after the authorization expires.
    // Only fresh attempts need a still-valid transport signature.
    if let Some(previous) = store
        .get_x402_relay_attempt_by_authorization(
            network,
            &plan.predicted_bounty_contract,
            &request.create.creation_nonce,
        )
        .await
        .map_err(map_x402_db_error)?
    {
        if previous.request_fingerprint != fingerprint {
            return Err(StatusCode::CONFLICT);
        }
        if matches!(
            previous.status,
            X402RelayStatus::Broadcast | X402RelayStatus::Confirmed
        ) {
            let previous = process_x402_hosted_relay_with_gas(
                &state,
                previous,
                &plan.relay_transaction,
                CREATION_MAX_GAS,
            )
            .await?;
            return x402_relay_response(&state, &previous);
        }
    }
    service_runtime::verifier_checkpoints::require_available(state.store.as_ref(), network, &terms)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if !enabled(&state, network) {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    validate_expiry(now, expiry, request.create.funding_deadline)?;
    let attempt = store
        .reserve_x402_relay_attempt(
            &NewX402RelayAttempt {
                id: Uuid::new_v4(),
                idempotency_key: format!(
                    "create:{network}:{}:{}",
                    plan.predicted_bounty_contract,
                    request.create.creation_nonce.to_ascii_lowercase()
                ),
                network: network.to_owned(),
                bounty_contract: plan.predicted_bounty_contract,
                contributor: creator,
                amount,
                authorization_nonce: request.create.creation_nonce,
                authorization_valid_before: expiry,
                request_fingerprint: fingerprint,
                relayer_address: relayer,
            },
            state.x402_relayer.max_daily_attempts,
            state.x402_relayer.max_daily_attempts_per_contributor,
        )
        .await
        .map_err(map_x402_db_error)?;
    if attempt.attempt_count >= 3
        && matches!(
            attempt.status,
            X402RelayStatus::Failed | X402RelayStatus::Relaying
        )
    {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    record_opportunity_creation_progress(&state, network, &terms, "wallet_signed").await?;
    // Uses the same durable replay protection and nonce lease as funding.
    // No principal cap is borrowed from x402 micro-contributions: we pay gas only.
    let attempt = process_x402_hosted_relay_with_gas(
        &state,
        attempt,
        &plan.relay_transaction,
        CREATION_MAX_GAS,
    )
    .await?;
    x402_relay_response(&state, &attempt)
}

pub(super) fn response(state: &SharedState, attempt: &X402RelayAttempt) -> Response {
    let status = match attempt.status {
        X402RelayStatus::Confirmed => StatusCode::OK,
        X402RelayStatus::Failed if attempt.retryable => StatusCode::SERVICE_UNAVAILABLE,
        X402RelayStatus::Failed => StatusCode::UNPROCESSABLE_ENTITY,
        _ => StatusCode::ACCEPTED,
    };
    (status, [(header::CACHE_CONTROL, "no-store, private")], Json(serde_json::json!({
        "schema": "agent-bounties/sponsored-creation-v1", "status": x402_relay_status_name(attempt.status),
        "funding_confirmed": attempt.status == X402RelayStatus::Confirmed, "solver_paid": false,
        "customer_gas_wei": "0", "relay": x402_public_relay(attempt),
        "statusUrl": format!("{}/v1/x402/base/relays/{}", state.public_base_url.trim_end_matches('/'), attempt.id),
        "nextStep": "Reconcile this same bounty and authorization. A transaction hash is not funding or solver payment. Never replace an uncertain authorization to escape a pending operation."
    }))).into_response()
}

#[cfg(test)]
#[path = "sponsored_creation_rehearsal.rs"]
mod rehearsal;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transport_expiry_is_short_and_independent_of_delivery() {
        assert!(validate_expiry(1000, 8200, 9_000_000).is_ok());
        for expiry in [0, 1000, 1030, 8201, 9_000_000] {
            assert!(validate_expiry(1000, expiry, 9_000_000).is_err());
        }
        assert!(validate_expiry(1000, 2000, 1999).is_err());
    }
    #[test]
    fn sponsorship_is_unavailable_without_durable_relayer() {
        let config = X402HostedRelayerConfig::default();
        assert!(!config.enabled);
        assert!(config.relayer.is_none());
    }
}
