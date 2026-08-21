use super::{x402_payment_required_error, x402_payment_required_response, SharedState};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chain_base::{
    fetch_block_number, fetch_exact_block_identity, fetch_transaction_receipt,
    normalize_evm_address, plan_open_competition_v2_action,
    plan_open_competition_v2_broker_payment, plan_open_competition_v2_creation,
    plan_open_competition_v2_funding, plan_open_competition_v2_proof,
    validate_open_competition_v2_release, ChainBaseError,
    OpenCompetitionV2BrokerPaymentAuthorization, OpenCompetitionV2CreateParams,
    OpenCompetitionV2CreationRequest, OpenCompetitionV2EventKind,
    OpenCompetitionV2ProgramClassification, OpenCompetitionV2ProofSystem, OpenCompetitionV2Release,
    OpenCompetitionV2ScoreDirection, OpenCompetitionV2WinnerMode,
    OPEN_COMPETITION_V2_BASE_SEPOLIA_USDC, OPEN_COMPETITION_V2_BASE_USDC,
    OPEN_COMPETITION_V2_PROTOCOL_VERSION,
};
use chrono::{DateTime, Utc};
use competition_metric_core::{
    execute_public_vector_program, execute_structured_artifact_program,
    structured_artifact_policy_hash_for, ArtifactRequirement, JournalScopeV2, PublicVectorCase,
    PublicVectorMode, PublicVectorProgramInput, StructuredArtifactProgramInput,
    GROTH16_PROOF_SYSTEM, JOURNAL_SCHEMA_HASH, METRIC_PROGRAM_HASH, PLONK_PROOF_SYSTEM,
    STRUCTURED_ARTIFACT_JOURNAL_SCHEMA_HASH, STRUCTURED_ARTIFACT_METRIC_PROGRAM_HASH,
};
use db::{
    OpenCompetitionV2ProofJob, OpenCompetitionV2ProofJobState, OpenCompetitionV2ProofJobUpdate,
};
use payments_x402::{
    base_usdc_exact_service_challenge, decode_payment_signature_header,
    encode_payment_response_header, quote_proof_broker_job, validate_exact_service_payload,
    CompetitionWinnerMode, ProofBrokerQuote, ProofBrokerQuoteRequest, SettlementResponse,
    PAYMENT_RESPONSE_HEADER, PAYMENT_SIGNATURE_HEADER,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, env};
use tokio::time::{timeout, Duration};
use uuid::Uuid;

type ApiResult = Result<Json<Value>, (StatusCode, Json<Value>)>;

pub(crate) fn router() -> Router<SharedState> {
    Router::new()
        .nest(
            "/v1/base/open-competition-v2-beta1",
            Router::new().fallback(beta1_superseded),
        )
        .nest(
            "/v1/base/open-competition-v2-beta2",
            Router::new().fallback(beta2_superseded),
        )
        .route("/v1/base/open-competition-v2-beta3/release", get(release))
        .route("/v1/base/open-competition-v2-beta3/profiles", get(profiles))
        .route(
            "/v1/base/open-competition-v2-beta3/structured-artifact-profile",
            post(prepare_structured_artifact_profile),
        )
        .route(
            "/v1/base/open-competition-v2-beta3/validate",
            post(validate_creation),
        )
        .route(
            "/v1/base/open-competition-v2-beta3/creation-preparation",
            post(prepare_creation),
        )
        .route(
            "/v1/base/open-competition-v2-beta3/funding-preparation",
            post(prepare_funding),
        )
        .route(
            "/v1/base/open-competition-v2-beta3/inventory",
            get(inventory),
        )
        .route("/v1/base/open-competition-v2-beta3/events", get(events))
        .route(
            "/v1/base/open-competition-v2-beta3/proof-quotes",
            post(create_proof_quote),
        )
        .route(
            "/v1/base/open-competition-v2-beta3/proof-preparation",
            post(prepare_proof),
        )
        .route(
            "/v1/base/open-competition-v2-beta3/proof-attribution",
            get(proof_attribution),
        )
        .route(
            "/v1/base/open-competition-v2-beta3/action-preparation",
            post(prepare_action),
        )
        .route(
            "/v1/base/open-competition-v2-beta3/proof-jobs/:job_id",
            get(get_proof_job),
        )
        .route(
            "/v1/base/open-competition-v2-beta3/proof-jobs/:job_id/payment",
            post(pay_proof_job),
        )
        .route(
            "/v1/base/open-competition-v2-beta3/proof-jobs/:job_id/relay-authorization",
            post(authorize_proof_job_relay),
        )
}

async fn beta1_superseded() -> impl IntoResponse {
    superseded_release("Beta1")
}

async fn beta2_superseded() -> impl IntoResponse {
    superseded_release("Beta2")
}

fn superseded_release(release: &str) -> impl IntoResponse {
    (
        StatusCode::GONE,
        Json(json!({
            "schema_version": "agent-bounties/open-competition-v2-problem-v1",
            "state": "superseded_before_launch",
            "failed_transition": "resolve_release",
            "error_code": "superseded_before_launch",
            "retryable": false,
            "replacement_protocol": OPEN_COMPETITION_V2_PROTOCOL_VERSION,
            "replacement_path": "/v1/base/open-competition-v2-beta3",
            "message": format!("{release} was never deployed. Use Beta3; do not reuse {release} vkeys, proofs, verifier bytecode, or release manifests.")
        })),
    )
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NetworkQuery {
    network: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InventoryQuery {
    network: Option<String>,
    state: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EventQuery {
    network: Option<String>,
    bounty_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreationBody {
    network: Option<String>,
    creator: String,
    creation_nonce: String,
    acknowledged_risk_hash: String,
    initial_funding: String,
    params: CreateParamsBody,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateParamsBody {
    solver_reward: String,
    keeper_reward: String,
    funding_deadline: u64,
    proof_window_seconds: u64,
    winner_mode: OpenCompetitionV2WinnerMode,
    score_direction: OpenCompetitionV2ScoreDirection,
    score_threshold: String,
    proof_system: OpenCompetitionV2ProofSystem,
    program_vkey: String,
    source_hash: String,
    elf_hash: String,
    journal_schema_hash: String,
    metric_program_hash: String,
    execution_policy_hash: String,
    verification_policy_hash: String,
    settlement_policy_hash: String,
    beta_risk_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FundingBody {
    network: Option<String>,
    contributor: String,
    competition_contract: String,
    amount: String,
    acknowledged_risk_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StructuredArtifactProfileBody {
    network: Option<String>,
    threshold: String,
    requirements: Vec<ArtifactRequirement>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProofQuoteBody {
    network: Option<String>,
    competition_contract: String,
    solver: String,
    solver_nonce: String,
    artifact_hash: String,
    relay: bool,
    metric: ProofMetricBody,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProofAttributionQuery {
    network: Option<String>,
    competition_contract: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicVectorMetricBody {
    mode: PublicVectorMode,
    threshold: String,
    vectors: Vec<PublicVectorCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StructuredArtifactMetricBody {
    profile_id: String,
    threshold: String,
    artifact_utf8: String,
    requirements: Vec<ArtifactRequirement>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ProofMetricBody {
    PublicVector(PublicVectorMetricBody),
    StructuredArtifact(StructuredArtifactMetricBody),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProofBody {
    network: Option<String>,
    competition_contract: String,
    solver: String,
    solver_nonce: String,
    proof_system: OpenCompetitionV2ProofSystem,
    public_values: String,
    proof: String,
    authorization_deadline: u64,
    solver_signature: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActionBody {
    network: Option<String>,
    competition_contract: String,
    caller: Option<String>,
    action: String,
    contributor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProofRelayAuthorizationBody {
    authorization_deadline: u64,
    solver_signature: Option<String>,
}

#[utoipa::path(get, path = "/v1/base/open-competition-v2-beta3/profiles", responses((status = 200, description = "Pinned V2 release, SP1 rails, and metric program classifications")))]
pub(crate) async fn profiles(
    State(state): State<SharedState>,
    Query(query): Query<NetworkQuery>,
) -> ApiResult {
    let network = network_or_default(query.network);
    let release = release_from_environment(&network).ok();
    let indexers_agree = match &release {
        Some(release) => current_indexer_agreement(&state, &network, release)
            .await
            .is_ok(),
        None => false,
    };
    Ok(Json(profiles_document(&network, release, indexers_agree)?))
}

fn profiles_document(
    network: &str,
    release: Option<OpenCompetitionV2Release>,
    indexers_agree: bool,
) -> Result<Value, (StatusCode, Json<Value>)> {
    let programs = release
        .as_ref()
        .map(|release| json!(release.metric_programs))
        .unwrap_or_else(|| {
            json!([
                {
                    "profile_id": "public-vector-metric-v1",
                    "classification": "disabled",
                    "reason": "Enable only after two isolated builds reproduce the ELF digest and vkey and the published adversarial corpus passes."
                },
                {
                    "profile_id": "structured-artifact-metric-v1",
                    "classification": "disabled",
                    "reason": "Enable only after two isolated builds reproduce the ELF digest and vkey and the published adversarial corpus passes."
                }
            ])
        });
    Ok(json!({
        "schema_version": "agent-bounties/open-competition-v2-profiles-v1",
        "protocol_version": OPEN_COMPETITION_V2_PROTOCOL_VERSION,
        "network": network,
        "release": release,
        "canonical_rails": {
            "settlement_token": settlement_token(network)?,
            "groth16_verifier": release.as_ref().map(|value| &value.groth16_verifier),
            "plonk_verifier": release.as_ref().map(|value| &value.plonk_verifier),
            "sp1_source_commit": release.as_ref().map(|value| &value.sp1_source_commit),
            "sp1_circuit_version": release.as_ref().map(|value| &value.sp1_circuit_version)
        },
        "programs": programs,
        "proof_broker_enabled": release.as_ref().is_some_and(|release| release.proof_broker_enabled) && indexers_agree,
        "creation_enabled": release.as_ref().is_some_and(|release| release.public_creation_enabled) && indexers_agree,
        "indexers_agree": indexers_agree,
        "next_action": if release.is_some() {
            "Validate a complete immutable profile, then prepare creation."
        } else {
            "Wait for the reviewed release manifest; do not construct a factory call from guessed addresses."
        },
        "evidence_boundary": "A profile catalog or release manifest is not deployment, funding, proof acceptance, settlement, or payment evidence."
    }))
}

#[utoipa::path(post, path = "/v1/base/open-competition-v2-beta3/structured-artifact-profile", responses((status = 200, description = "Exact immutable structured-artifact metric fields for V2 creation"), (status = 400, description = "Invalid deterministic artifact requirements"), (status = 503, description = "Reviewed metric profile is unavailable")))]
pub(crate) async fn prepare_structured_artifact_profile(
    Json(body): Json<StructuredArtifactProfileBody>,
) -> ApiResult {
    let network = network_or_default(body.network);
    let release = release_from_environment(&network)?;
    let threshold = decimal_u128(&body.threshold, "threshold")?;
    let verification_policy_hash =
        structured_artifact_policy_hash_for(threshold, &body.requirements).map_err(|error| {
            bad_request(
                "prepare_structured_artifact_profile",
                "invalid_artifact_requirements",
                format!("{error:?}"),
            )
        })?;
    let profile = release
        .metric_programs
        .iter()
        .find(|profile| {
            profile.profile_id == "structured-artifact-metric-v1"
                && profile.classification == OpenCompetitionV2ProgramClassification::Reviewed
        })
        .ok_or_else(|| {
            service_error(
                "prepare_structured_artifact_profile",
                "reviewed_profile_unavailable",
                "structured-artifact-metric-v1 is not reviewed in the active release",
            )
        })?;
    Ok(Json(json!({
        "schema_version": "agent-bounties/open-competition-v2-structured-artifact-profile-v1",
        "protocol_version": OPEN_COMPETITION_V2_PROTOCOL_VERSION,
        "network": network,
        "profile": profile,
        "verification_policy_hash": bytes32_hex(verification_policy_hash),
        "score_threshold": threshold.to_string(),
        "score_direction": "higher_is_better",
        "requirements": body.requirements,
        "next_action": "Copy the returned profile fields, score threshold, direction, and verification policy hash into validate, then prepare creation.",
        "evidence_boundary": "This deterministic profile calculation is not creation, funding, qualification, settlement, or payment evidence."
    })))
}

#[utoipa::path(get, path = "/v1/base/open-competition-v2-beta3/release", responses((status = 200, description = "Exact immutable Beta3 release identity and activation state"), (status = 503, description = "Beta3 release is not configured or fails validation")))]
pub(crate) async fn release(
    State(state): State<SharedState>,
    Query(query): Query<NetworkQuery>,
) -> ApiResult {
    let network = network_or_default(query.network);
    let release = release_from_environment(&network)?;
    let agreement = current_indexer_agreement(&state, &network, &release)
        .await
        .ok();
    let operational = agreement.is_some();
    Ok(Json(json!({
        "schema_version": "agent-bounties/open-competition-v2-beta3-runtime-manifest-v1",
        "activation_state": if release.public_creation_enabled && operational {
            "public_beta"
        } else {
            "creation_disabled"
        },
        "release": release,
        "indexer_agreement": agreement,
        "next_action": if release.public_creation_enabled && operational {
            "Validate a profile, then prepare creation."
        } else {
            "Wait for every public-beta gate to pass."
        },
        "evidence_boundary": "This endpoint identifies configured code and activation flags. Canonical events remain the only evidence of funding, qualification, settlement, or refund."
    })))
}

#[utoipa::path(post, path = "/v1/base/open-competition-v2-beta3/validate", responses((status = 200, description = "Immutable competition profile is valid"), (status = 400, description = "Machine-readable profile validation failure")))]
pub(crate) async fn validate_creation(Json(body): Json<CreationBody>) -> ApiResult {
    match build_creation_plan(body, false) {
        Ok(plan) => Ok(Json(json!({
            "schema_version": "agent-bounties/open-competition-v2-validation-v1",
            "valid": true,
            "state": "creation_ready",
            "bounty_id": plan.bounty_id,
            "predicted_competition": plan.predicted_competition,
            "funding_target": plan.funding_target,
            "next_action": "Call creation-preparation with the same body."
        }))),
        Err(error) => Err(error),
    }
}

#[utoipa::path(post, path = "/v1/base/open-competition-v2-beta3/creation-preparation", responses((status = 200, description = "Exact unsigned create and optional funding calls"), (status = 503, description = "Public Beta3 creation is gated")))]
pub(crate) async fn prepare_creation(
    State(state): State<SharedState>,
    Json(body): Json<CreationBody>,
) -> ApiResult {
    let network = network_or_default(body.network.clone());
    let release = release_from_environment(&network)?;
    if release.public_creation_enabled {
        current_indexer_agreement(&state, &network, &release).await?;
    }
    let plan = build_creation_plan(body, true)?;
    if !plan.public_inventory_eligible_after_confirmation {
        return Ok(Json(json!({
            "plan": plan,
            "state": "awaiting_funding",
            "next_action": "Execute wallet_calls in order, then pool the exact remaining funding before the funding deadline."
        })));
    }
    Ok(Json(json!({
        "plan": plan,
        "state": "awaiting_wallet_calls",
        "next_action": "Execute wallet_calls in order, then wait for safe-block CompetitionActivatedV2."
    })))
}

#[utoipa::path(post, path = "/v1/base/open-competition-v2-beta3/funding-preparation", responses((status = 200, description = "Exact unsigned pooled-funding calls")))]
pub(crate) async fn prepare_funding(Json(body): Json<FundingBody>) -> ApiResult {
    let network = network_or_default(body.network.clone());
    let release = release_from_environment(&network)?;
    let amount = decimal_u128(&body.amount, "amount")?;
    let calls = plan_open_competition_v2_funding(
        &network,
        &release.settlement_token,
        &body.contributor,
        &body.competition_contract,
        amount,
        &body.acknowledged_risk_hash,
    )
    .map_err(|error| bad_request("prepare_funding", "invalid_funding", error.to_string()))?;
    Ok(Json(json!({
        "schema_version": "agent-bounties/open-competition-v2-funding-plan-v1",
        "protocol_version": OPEN_COMPETITION_V2_PROTOCOL_VERSION,
        "network": network,
        "competition_contract": body.competition_contract,
        "amount": amount.to_string(),
        "wallet_calls": calls,
        "next_action": "Execute wallet_calls in order. Publish funding only after safe-block FundingAddedV2; publish active inventory only after CompetitionActivatedV2.",
        "evidence_boundary": "This unsigned funding plan is not funding evidence."
    })))
}

#[utoipa::path(get, path = "/v1/base/open-competition-v2-beta3/inventory", responses((status = 200, description = "Safe-block V2 competition inventory")))]
pub(crate) async fn inventory(
    State(state): State<SharedState>,
    Query(query): Query<InventoryQuery>,
) -> ApiResult {
    let network = network_or_default(query.network);
    let release = release_from_environment(&network)?;
    let store = state.store.as_ref().ok_or_else(database_unavailable)?;
    let mut records = store
        .list_open_competition_v2_projections(&network, &release.factory_contract)
        .await
        .map_err(|error| {
            service_error("load_inventory", "database_read_failed", error.to_string())
        })?;
    if let Some(filter) = query.state {
        records.retain(|record| {
            serde_json::to_value(record.projection.state)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .is_some_and(|state| state == filter)
        });
    }
    records.sort_by(|left, right| {
        let left_active = matches!(
            left.projection.state,
            chain_base::OpenCompetitionV2ProjectedState::Active
        );
        let right_active = matches!(
            right.projection.state,
            chain_base::OpenCompetitionV2ProjectedState::Active
        );
        let left_open = left_active && left.projection.leader.is_none();
        let right_open = right_active && right.projection.leader.is_none();
        let left_net =
            estimated_hosted_net_prize(&left.projection).unwrap_or(left.projection.solver_reward);
        let right_net =
            estimated_hosted_net_prize(&right.projection).unwrap_or(right.projection.solver_reward);
        right_active
            .cmp(&left_active)
            .then_with(|| right_open.cmp(&left_open))
            .then_with(|| right_net.cmp(&left_net))
            .then_with(|| {
                left.projection
                    .proof_deadline
                    .cmp(&right.projection.proof_deadline)
            })
    });
    let competitions = records
        .into_iter()
        .map(|record| {
            let estimated_net = estimated_hosted_net_prize(&record.projection);
            let proof_fee = configured_proof_fee(&record.projection);
            let relay_fee =
                configured_u128_optional("OPEN_COMPETITION_V2_RELAY_FEE_BASE_UNITS");
            let risk = if !matches!(
                record.projection.state,
                chain_base::OpenCompetitionV2ProjectedState::Active
            ) {
                "not_active"
            } else if record.projection.leader.is_some() {
                "qualifying_leader_exists"
            } else if record.projection.winner_mode.as_deref() == Some("best_score") {
                "best_score_competition"
            } else {
                "no_qualifying_leader_indexed"
            };
            json!({
                "record": record,
                "earning_estimate": {
                    "gross_prize": record.projection.solver_reward.to_string(),
                    "hosted_proof_fee_quote": proof_fee.map(|value| value.to_string()),
                    "hosted_relay_fee_quote": relay_fee.map(|value| value.to_string()),
                    "hosted_net_prize_if_win": estimated_net.map(|value| value.to_string()),
                    "profitable_if_win": estimated_net.map(|value| value > 0),
                    "competition_risk": risk,
                    "relay_fee_excluded": false,
                    "warning": "A positive net prize is conditional on winning and is never guaranteed profit. Request a solver-bound five-minute quote before paying."
                }
            })
        })
        .collect::<Vec<_>>();
    Ok(Json(json!({
        "schema_version": "agent-bounties/open-competition-v2-inventory-v1",
        "protocol_version": OPEN_COMPETITION_V2_PROTOCOL_VERSION,
        "network": network,
        "factory_contract": release.factory_contract,
        "competitions": competitions,
        "sort": ["active_first", "no_qualifying_leader_first", "estimated_hosted_net_prize_desc", "proof_deadline_asc"],
        "evidence_boundary": "Inventory is a replay-safe safe-block projection. Only CompetitionSettledV2 proves solver payment."
    })))
}

#[utoipa::path(get, path = "/v1/base/open-competition-v2-beta3/events", responses((status = 200, description = "Replay-safe canonical V2 event history")))]
pub(crate) async fn events(
    State(state): State<SharedState>,
    Query(query): Query<EventQuery>,
) -> ApiResult {
    let network = network_or_default(query.network);
    let release = release_from_environment(&network)?;
    let store = state.store.as_ref().ok_or_else(database_unavailable)?;
    let mut events = store
        .list_open_competition_v2_events(&network, &release.factory_contract)
        .await
        .map_err(|error| service_error("load_events", "database_read_failed", error.to_string()))?;
    if let Some(bounty_id) = query.bounty_id {
        events.retain(|event| event.bounty_id.eq_ignore_ascii_case(&bounty_id));
    }
    Ok(Json(json!({
        "schema_version": "agent-bounties/open-competition-v2-events-v1",
        "protocol_version": OPEN_COMPETITION_V2_PROTOCOL_VERSION,
        "network": network,
        "factory_contract": release.factory_contract,
        "events": events,
        "payment_event": "competition_settled",
        "evidence_boundary": "CompetitionEntryQualifiedV2 and transaction hashes are not payment. Only a safe-block CompetitionSettledV2 event proves solver payment."
    })))
}

#[utoipa::path(post, path = "/v1/base/open-competition-v2-beta3/proof-quotes", responses((status = 200, description = "Five-minute solver- and artifact-bound x402 proof quote"), (status = 409, description = "Proof SLA cannot fit or competition is not active")))]
pub(crate) async fn create_proof_quote(
    State(state): State<SharedState>,
    Json(body): Json<ProofQuoteBody>,
) -> ApiResult {
    let network = network_or_default(body.network.clone());
    let release = release_from_environment(&network)?;
    if release.proof_broker_enabled {
        current_indexer_agreement(&state, &network, &release).await?;
    }
    let store = state.store.as_ref().ok_or_else(database_unavailable)?;
    let records = store
        .list_open_competition_v2_projections(&network, &release.factory_contract)
        .await
        .map_err(|error| service_error("quote_proof", "database_read_failed", error.to_string()))?;
    let record = records
        .into_iter()
        .find(|record| {
            record
                .projection
                .competition
                .eq_ignore_ascii_case(&body.competition_contract)
        })
        .ok_or_else(|| not_found("quote_proof", "competition_not_indexed"))?;
    if record.projection.state != chain_base::OpenCompetitionV2ProjectedState::Active {
        return Err(conflict(
            "quote_proof",
            "competition_not_active",
            "Select an active safe-block competition.",
        ));
    }
    require_reviewed_broker_profile(&release, &record.projection)?;
    let proof_system = record.projection.proof_system.as_deref().ok_or_else(|| {
        conflict(
            "quote_proof",
            "proof_system_unknown",
            "Wait for index reconciliation.",
        )
    })?;
    let winner_mode = match record.projection.winner_mode.as_deref() {
        Some("first_proven") => CompetitionWinnerMode::FirstProven,
        Some("best_score") => CompetitionWinnerMode::BestScore,
        _ => {
            return Err(conflict(
                "quote_proof",
                "winner_mode_unknown",
                "Wait for index reconciliation.",
            ))
        }
    };
    let (program_input, expected_public_values) =
        build_metric_proof_input(&network, &record.projection, &body)?;
    let now = u64::try_from(Utc::now().timestamp()).map_err(|_| {
        service_error(
            "quote_proof",
            "clock_invalid",
            "system time is before Unix epoch",
        )
    })?;
    let proof_deadline = record.projection.proof_deadline.ok_or_else(|| {
        conflict(
            "quote_proof",
            "proof_deadline_unknown",
            "Wait for activation indexing.",
        )
    })?;
    let proof_fee = configured_u128(&format!(
        "OPEN_COMPETITION_V2_{}_PROOF_FEE_BASE_UNITS",
        proof_system.to_ascii_uppercase()
    ))?;
    let relay_fee = if body.relay {
        configured_u128("OPEN_COMPETITION_V2_RELAY_FEE_BASE_UNITS")?
    } else {
        0
    };
    let proof_sla = configured_u64(&format!(
        "OPEN_COMPETITION_V2_{}_PROOF_SLA_SECONDS",
        proof_system.to_ascii_uppercase()
    ))?;
    let quote = quote_proof_broker_job(ProofBrokerQuoteRequest {
        network: eip155_network(&network)?.to_string(),
        competition_contract: record.projection.competition.clone(),
        solver: body.solver.clone(),
        solver_nonce: body.solver_nonce.clone(),
        artifact_hash: body.artifact_hash.clone(),
        proof_system: proof_system.to_string(),
        gross_prize: record.projection.solver_reward.to_string(),
        proof_fee_quote: proof_fee.to_string(),
        relay_fee_quote: relay_fee.to_string(),
        winner_mode,
        proof_deadline,
        measured_proof_sla_seconds: proof_sla,
        now_unix_seconds: now,
    })
    .map_err(|error| conflict("quote_proof", "proof_quote_unavailable", error.to_string()))?;
    let broker = env::var("OPEN_COMPETITION_V2_BROKER_PAYMENT_ADDRESS").map_err(|_| {
        service_error(
            "quote_proof",
            "proof_broker_disabled",
            "proof broker payment address is not configured",
        )
    })?;
    if !state.x402_relayer.enabled
        || state
            .x402_relayer
            .address()
            .is_none_or(|address| !address.eq_ignore_ascii_case(&broker))
    {
        return Err(service_error(
            "quote_proof",
            "proof_broker_relayer_unavailable",
            "The broker payment address must equal the enabled hosted relayer address.",
        ));
    }
    let quote_expires_at = timestamp(quote.quote_expiration, "quote_expiration")?;
    let proof_sla_deadline = timestamp(quote.proof_sla_deadline, "proof_sla_deadline")?;
    let now_datetime = Utc::now();
    let job = OpenCompetitionV2ProofJob {
        id: Uuid::new_v4(),
        idempotency_key: quote.quote_id.clone(),
        network: network.clone(),
        competition_contract: quote.competition_contract.clone(),
        solver: quote.solver.clone(),
        solver_nonce: quote.solver_nonce.clone(),
        artifact_hash: quote.artifact_hash.clone(),
        program_input,
        expected_public_values,
        requested_relay: body.relay,
        proof_system: quote.proof_system.clone(),
        state: OpenCompetitionV2ProofJobState::Quoted,
        gross_prize: quote.gross_prize.clone(),
        proof_fee_quote: quote.proof_fee_quote.clone(),
        relay_fee_quote: quote.relay_fee_quote.clone(),
        net_prize_if_win: quote.net_prize_if_win.clone(),
        maximum_charge: quote.maximum_charge.clone(),
        winner_mode: match quote.winner_mode {
            CompetitionWinnerMode::FirstProven => "first_proven",
            CompetitionWinnerMode::BestScore => "best_score",
        }
        .to_string(),
        competition_risk: quote.competition_risk.clone(),
        quote_expires_at,
        proof_sla_deadline,
        payer: None,
        payment_authorization_nonce: None,
        payment_authorization: None,
        payment_tx_hash: None,
        payment_block_number: None,
        payment_evidence: None,
        proof_hash: None,
        public_values_hash: None,
        proof: None,
        public_values: None,
        proof_provider_job_id: None,
        solver_authorization_deadline: None,
        solver_signature: None,
        relay_tx_hash: None,
        settlement_event_id: None,
        refund_evidence: None,
        refund_tx_hash: None,
        refund_block_number: None,
        refund_due_at: None,
        failure_code: None,
        failure_message: None,
        attempt_count: 0,
        lease_token: None,
        lease_expires_at: None,
        created_at: now_datetime,
        updated_at: now_datetime,
    };
    let stored = store
        .insert_open_competition_v2_proof_job(&job)
        .await
        .map_err(|error| conflict("quote_proof", "quote_replay_conflict", error.to_string()))?;
    let challenge = proof_job_payment_challenge(&state, &release, &broker, &stored)?;
    Ok(Json(json!({
        "quote": quote,
        "proof_job_id": stored.id,
        "payment_required": challenge,
        "next_action": "Pay the exact x402 challenge before quote_expiration, then poll proof-jobs/{proof_job_id}. Cost overruns are the broker's responsibility.",
        "evidence_boundary": "A quote and x402 challenge are not payment, proof generation, competition entry, settlement, or refund evidence."
    })))
}

#[utoipa::path(post, path = "/v1/base/open-competition-v2-beta3/proof-preparation", responses((status = 200, description = "Direct call and exact EIP-712 relay authorization")))]
pub(crate) async fn prepare_proof(Json(body): Json<ProofBody>) -> ApiResult {
    let network = network_or_default(body.network);
    let solver_nonce = decimal_u128(&body.solver_nonce, "solver_nonce")?;
    let public_values = decode_hex(&body.public_values, 640, "public_values")?;
    let proof = decode_hex_bounded(&body.proof, 1, 4 * 1024 * 1024, "proof")?;
    let signature = body
        .solver_signature
        .as_deref()
        .map(|value| decode_hex_bounded(value, 1, 16 * 1024, "solver_signature"))
        .transpose()?;
    let plan = plan_open_competition_v2_proof(
        &network,
        &body.competition_contract,
        &body.solver,
        solver_nonce,
        body.proof_system,
        &public_values,
        &proof,
        body.authorization_deadline,
        signature.as_deref(),
    )
    .map_err(|error| bad_request("prepare_proof", "invalid_proof_plan", error.to_string()))?;
    Ok(Json(json!({
        "plan": plan,
        "next_action": if signature.is_some() {
            "Submit relay_call_after_signature before authorization_deadline."
        } else {
            "Submit direct_call from the solver, or sign relay_authorization and call this endpoint again with solver_signature."
        }
    })))
}

#[utoipa::path(post, path = "/v1/base/open-competition-v2-beta3/action-preparation", responses((status = 200, description = "Finalization, expiry, cancellation, or permissionless refund call")))]
pub(crate) async fn prepare_action(Json(body): Json<ActionBody>) -> ApiResult {
    let network = network_or_default(body.network);
    let plan = plan_open_competition_v2_action(
        &network,
        &body.competition_contract,
        body.caller.as_deref(),
        &body.action,
        body.contributor.as_deref(),
    )
    .map_err(|error| bad_request("prepare_action", "invalid_action", error.to_string()))?;
    Ok(Json(json!({ "plan": plan })))
}

#[utoipa::path(get, path = "/v1/base/open-competition-v2-beta3/proof-jobs/{job_id}", params(("job_id" = Uuid, Path, description = "Hosted proof job ID")), responses((status = 200, description = "Exact proof job state and next action"), (status = 404, description = "Proof job not found")))]
pub(crate) async fn get_proof_job(
    State(state): State<SharedState>,
    Path(job_id): Path<Uuid>,
) -> ApiResult {
    let store = state.store.as_ref().ok_or_else(database_unavailable)?;
    let mut job = store
        .get_open_competition_v2_proof_job(job_id)
        .await
        .map_err(|error| service_error("get_proof_job", "database_read_failed", error.to_string()))?
        .ok_or_else(|| not_found("get_proof_job", "proof_job_not_found"))?;
    if job.state == OpenCompetitionV2ProofJobState::PaymentPending && job.payment_tx_hash.is_some()
    {
        job = reconcile_proof_job_payment(&state, job)
            .await
            .map_err(|status| {
                service_error(
                    "get_proof_job",
                    "payment_reconciliation_failed",
                    format!("payment reconciliation returned HTTP {status}"),
                )
            })?;
    }
    Ok(Json(json!({
        "schema_version": "agent-bounties/open-competition-v2-proof-job-v1",
        "job": job,
        "next_action": proof_job_next_action(job.state),
        "evidence_boundary": "Only canonical payment and refund evidence attached to the job proves money movement. A hosted state alone does not."
    })))
}

#[utoipa::path(get, path = "/v1/base/open-competition-v2-beta3/proof-attribution", params(("network" = Option<String>, Query, description = "Base network"), ("competition_contract" = String, Query, description = "Exact competition contract")), responses((status = 200, description = "Public hosted-proof, x402, relay, and settlement attribution"), (status = 404, description = "No hosted proof jobs found")))]
pub(crate) async fn proof_attribution(
    State(state): State<SharedState>,
    Query(query): Query<ProofAttributionQuery>,
) -> ApiResult {
    let network = network_or_default(query.network);
    let competition_contract =
        normalize_evm_address(&query.competition_contract).map_err(|_| {
            bad_request(
                "proof_attribution",
                "competition_contract_invalid",
                "Use the exact EVM competition contract address.",
            )
        })?;
    let release = release_from_environment(&network)?;
    current_indexer_agreement(&state, &network, &release).await?;
    let store = state.store.as_ref().ok_or_else(database_unavailable)?;
    let jobs = store
        .list_open_competition_v2_proof_jobs_for_contract(&network, &competition_contract)
        .await
        .map_err(|error| {
            service_error(
                "proof_attribution",
                "proof_job_read_failed",
                error.to_string(),
            )
        })?;
    if jobs.is_empty() {
        return Err(not_found(
            "proof_attribution",
            "hosted_proof_jobs_not_found",
        ));
    }
    let events = store
        .list_open_competition_v2_events_for_contract(&network, &competition_contract)
        .await
        .map_err(|error| {
            service_error(
                "proof_attribution",
                "canonical_event_read_failed",
                error.to_string(),
            )
        })?;
    let relayer = state.x402_relayer.address();
    let attributed = jobs
        .iter()
        .map(|job| {
            let settlement = events.iter().find(|event| {
                event.kind == OpenCompetitionV2EventKind::CompetitionSettled
                    && event
                        .data
                        .get("solver")
                        .and_then(Value::as_str)
                        .is_some_and(|solver| solver.eq_ignore_ascii_case(&job.solver))
            });
            json!({
                "proof_job_id": job.id,
                "solver_wallet": job.solver,
                "state": job.state,
                "x402_payment": {
                    "payer": job.payer,
                    "transaction_hash": job.payment_tx_hash,
                    "canonical_evidence_recorded": job.payment_evidence.is_some()
                },
                "hosted_proof": {
                    "used": job.proof_provider_job_id.is_some(),
                    "provider": "agent-bounties-hosted-sp1",
                    "provider_job_recorded": job.proof_provider_job_id.is_some()
                },
                "hosted_relay": {
                    "requested": job.requested_relay,
                    "used": job.relay_tx_hash.is_some(),
                    "relayer_wallet": relayer,
                    "transaction_hash": job.relay_tx_hash
                },
                "canonical_settlement": settlement.map(|event| json!({
                    "event": "CompetitionSettledV2",
                    "transaction_hash": event.tx_hash,
                    "block_number": event.block_number,
                    "solver_reward": event.data.get("solver_reward"),
                    "keeper_wallet": event.data.get("keeper"),
                    "keeper_reward": event.data.get("keeper_reward")
                })),
                "contactability": "wallet_only_unless_the_solver_registers_a_signed_contact_profile"
            })
        })
        .collect::<Vec<_>>();
    Ok(Json(json!({
        "schema_version": "agent-bounties/open-competition-v2-proof-attribution-v1",
        "protocol_version": OPEN_COMPETITION_V2_PROTOCOL_VERSION,
        "network": network,
        "competition_contract": competition_contract,
        "jobs": attributed,
        "identity_boundary": "A solver wallet is canonical attribution, not a verified person. Contact requires a separately signed contact profile.",
        "evidence_boundary": "The proof-job record attributes hosted services. Only CompetitionSettledV2 proves solver and keeper payment."
    })))
}

#[utoipa::path(post, path = "/v1/base/open-competition-v2-beta3/proof-jobs/{job_id}/payment", params(("job_id" = Uuid, Path, description = "Quoted hosted proof job ID")), responses((status = 200, description = "Canonical Base USDC payment confirmed"), (status = 202, description = "Payment relay is awaiting canonical confirmation"), (status = 402, description = "Exact x402 payment authorization required")))]
pub(crate) async fn pay_proof_job(
    State(state): State<SharedState>,
    Path(job_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let mut job = store
        .get_open_competition_v2_proof_job(job_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if job.state == OpenCompetitionV2ProofJobState::Paid {
        return proof_job_payment_response(&job);
    }
    if job.state == OpenCompetitionV2ProofJobState::PaymentPending && job.payment_tx_hash.is_some()
    {
        job = reconcile_proof_job_payment(&state, job).await?;
        if job.state == OpenCompetitionV2ProofJobState::Paid {
            return proof_job_payment_response(&job);
        }
    }
    if !matches!(
        job.state,
        OpenCompetitionV2ProofJobState::Quoted | OpenCompetitionV2ProofJobState::PaymentPending
    ) {
        return Err(StatusCode::CONFLICT);
    }
    if Utc::now() >= job.quote_expires_at && job.state == OpenCompetitionV2ProofJobState::Quoted {
        return Err(StatusCode::CONFLICT);
    }
    let release = release_from_environment(&job.network).map_err(|(status, _)| status)?;
    let broker = env::var("OPEN_COMPETITION_V2_BROKER_PAYMENT_ADDRESS")
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let challenge = proof_job_payment_challenge(&state, &release, &broker, &job)
        .map_err(|(status, _)| status)?;
    let now =
        u64::try_from(Utc::now().timestamp()).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let authorization = if let Some(header) = headers.get(PAYMENT_SIGNATURE_HEADER) {
        let payload = match header
            .to_str()
            .map_err(|_| payments_x402::X402Error::InvalidBase64)
            .and_then(decode_payment_signature_header)
        {
            Ok(payload) => payload,
            Err(error) => return x402_payment_required_error(challenge, &error.to_string()),
        };
        match validate_exact_service_payload(&payload, &challenge, now) {
            Ok(authorization) => authorization,
            Err(error) => return x402_payment_required_error(challenge, &error.to_string()),
        }
    } else if job.state == OpenCompetitionV2ProofJobState::PaymentPending {
        stored_payment_authorization(&job, now)?
    } else {
        return x402_payment_required_response(challenge);
    };
    if authorization.amount.to_string() != job.maximum_charge {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let pending_update = OpenCompetitionV2ProofJobUpdate {
        payer: Some(authorization.payer.clone()),
        payment_authorization_nonce: Some(authorization.nonce.clone()),
        payment_authorization: Some(json!({
            "payer": authorization.payer.clone(),
            "recipient": authorization.recipient.clone(),
            "amount": authorization.amount.to_string(),
            "valid_before": authorization.valid_before,
            "nonce": authorization.nonce.clone(),
            "v": authorization.v,
            "r": authorization.r.clone(),
            "s": authorization.s.clone()
        })),
        ..Default::default()
    };
    if job.state == OpenCompetitionV2ProofJobState::Quoted {
        job = store
            .transition_open_competition_v2_proof_job(
                job.id,
                OpenCompetitionV2ProofJobState::Quoted,
                OpenCompetitionV2ProofJobState::PaymentPending,
                &pending_update,
            )
            .await
            .map_err(|_| StatusCode::CONFLICT)?;
    } else if job
        .payer
        .as_deref()
        .is_some_and(|payer| !payer.eq_ignore_ascii_case(&authorization.payer))
        || job
            .payment_authorization_nonce
            .as_deref()
            .is_some_and(|nonce| !nonce.eq_ignore_ascii_case(&authorization.nonce))
    {
        return Err(StatusCode::CONFLICT);
    }

    if job.payment_tx_hash.is_none() {
        let lease = store
            .acquire_x402_relayer_lease(&job.network, state.x402_relayer.lease_seconds)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let Some(lease) = lease else {
            return proof_job_payment_response(&job);
        };
        job = store
            .get_open_competition_v2_proof_job(job.id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        if job.payment_tx_hash.is_some() {
            store
                .release_x402_relayer_lease(&job.network, lease)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            job = reconcile_proof_job_payment(&state, job).await?;
            return proof_job_payment_response(&job);
        }
        let result = broadcast_proof_job_payment(&state, &job, &release, &authorization).await;
        let release_result = store
            .release_x402_relayer_lease(&job.network, lease)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
        release_result?;
        let transaction = match result {
            Ok(transaction) => transaction,
            Err(ProofPaymentBroadcastError::Chain(error)) => {
                eprintln!(
                    "open_competition_v2 proof payment relay did not broadcast: job_id={} error={error}",
                    job.id
                );
                if let Some(reason) = retryable_proof_payment_broadcast_error(&error) {
                    return proof_job_payment_retry_response(&job, reason);
                }
                return Err(StatusCode::UNPROCESSABLE_ENTITY);
            }
            Err(ProofPaymentBroadcastError::Status(status)) => return Err(status),
        };
        job = store
            .transition_open_competition_v2_proof_job(
                job.id,
                OpenCompetitionV2ProofJobState::PaymentPending,
                OpenCompetitionV2ProofJobState::PaymentPending,
                &OpenCompetitionV2ProofJobUpdate {
                    payer: Some(authorization.payer),
                    payment_authorization_nonce: Some(authorization.nonce),
                    payment_tx_hash: Some(transaction.tx_hash),
                    ..Default::default()
                },
            )
            .await
            .map_err(|_| StatusCode::CONFLICT)?;
    }
    job = reconcile_proof_job_payment(&state, job).await?;
    proof_job_payment_response(&job)
}

#[utoipa::path(post, path = "/v1/base/open-competition-v2-beta3/proof-jobs/{job_id}/relay-authorization", params(("job_id" = Uuid, Path, description = "Proved hosted job ID")), responses((status = 200, description = "Exact EIP-712 typed data or accepted scoped signature"), (status = 409, description = "Job is not relayable")))]
pub(crate) async fn authorize_proof_job_relay(
    State(state): State<SharedState>,
    Path(job_id): Path<Uuid>,
    Json(body): Json<ProofRelayAuthorizationBody>,
) -> ApiResult {
    let store = state.store.as_ref().ok_or_else(database_unavailable)?;
    let job = store
        .get_open_competition_v2_proof_job(job_id)
        .await
        .map_err(|error| {
            service_error(
                "authorize_proof_relay",
                "database_read_failed",
                error.to_string(),
            )
        })?
        .ok_or_else(|| not_found("authorize_proof_relay", "proof_job_not_found"))?;
    if job.state == OpenCompetitionV2ProofJobState::Relaying {
        if job.solver_authorization_deadline == Some(body.authorization_deadline)
            && body.solver_signature.as_deref().is_some_and(|signature| {
                job.solver_signature
                    .as_deref()
                    .is_some_and(|stored| stored.eq_ignore_ascii_case(signature))
            })
        {
            return Ok(Json(json!({
                "schema_version": "agent-bounties/open-competition-v2-relay-authorization-v1",
                "proof_job_id": job.id,
                "state": job.state,
                "next_action": proof_job_next_action(job.state)
            })));
        }
        return Err(conflict(
            "authorize_proof_relay",
            "relay_authorization_conflict",
            "This job already has a different relay authorization.",
        ));
    }
    if job.state != OpenCompetitionV2ProofJobState::Proved || !job.requested_relay {
        return Err(conflict(
            "authorize_proof_relay",
            "job_not_relayable",
            "Request relay in the quote and wait until the job state is proved.",
        ));
    }
    let now = u64::try_from(Utc::now().timestamp()).map_err(|_| {
        service_error(
            "authorize_proof_relay",
            "clock_invalid",
            "system time is before Unix epoch",
        )
    })?;
    let release = release_from_environment(&job.network)?;
    let record = store
        .list_open_competition_v2_projections(&job.network, &release.factory_contract)
        .await
        .map_err(|error| {
            service_error(
                "authorize_proof_relay",
                "database_read_failed",
                error.to_string(),
            )
        })?
        .into_iter()
        .find(|record| {
            record
                .projection
                .competition
                .eq_ignore_ascii_case(&job.competition_contract)
        })
        .ok_or_else(|| not_found("authorize_proof_relay", "competition_not_indexed"))?;
    let proof_deadline = record.projection.proof_deadline.ok_or_else(|| {
        conflict(
            "authorize_proof_relay",
            "proof_deadline_unknown",
            "Wait for index reconciliation.",
        )
    })?;
    if body.authorization_deadline <= now || body.authorization_deadline > proof_deadline {
        return Err(bad_request(
            "authorize_proof_relay",
            "invalid_authorization_deadline",
            "authorization_deadline must be in the future and no later than the proof deadline",
        ));
    }
    let public_values = decode_hex(
        job.public_values.as_deref().ok_or_else(|| {
            service_error(
                "authorize_proof_relay",
                "stored_proof_incomplete",
                "Stored proof is missing public values.",
            )
        })?,
        640,
        "public_values",
    )?;
    let proof = decode_hex_bounded(
        job.proof.as_deref().ok_or_else(|| {
            service_error(
                "authorize_proof_relay",
                "stored_proof_incomplete",
                "Stored proof bytes are missing.",
            )
        })?,
        1,
        4 * 1024 * 1024,
        "proof",
    )?;
    let signature = body
        .solver_signature
        .as_deref()
        .map(|value| decode_hex_bounded(value, 1, 16 * 1024, "solver_signature"))
        .transpose()?;
    let proof_system = match job.proof_system.as_str() {
        "groth16" => OpenCompetitionV2ProofSystem::Groth16,
        "plonk" => OpenCompetitionV2ProofSystem::Plonk,
        _ => {
            return Err(service_error(
                "authorize_proof_relay",
                "stored_proof_system_invalid",
                "Stored proof system is invalid.",
            ))
        }
    };
    let plan = plan_open_competition_v2_proof(
        &job.network,
        &job.competition_contract,
        &job.solver,
        decimal_u128(&job.solver_nonce, "solver_nonce")?,
        proof_system,
        &public_values,
        &proof,
        body.authorization_deadline,
        signature.as_deref(),
    )
    .map_err(|error| {
        bad_request(
            "authorize_proof_relay",
            "invalid_relay_authorization",
            error.to_string(),
        )
    })?;
    if let Some(signature_hex) = body.solver_signature {
        let job = store
            .transition_open_competition_v2_proof_job(
                job.id,
                OpenCompetitionV2ProofJobState::Proved,
                OpenCompetitionV2ProofJobState::Relaying,
                &OpenCompetitionV2ProofJobUpdate {
                    solver_authorization_deadline: Some(body.authorization_deadline),
                    solver_signature: Some(signature_hex),
                    ..Default::default()
                },
            )
            .await
            .map_err(|error| {
                conflict(
                    "authorize_proof_relay",
                    "proof_job_transition_conflict",
                    error.to_string(),
                )
            })?;
        return Ok(Json(json!({
            "schema_version": "agent-bounties/open-competition-v2-relay-authorization-v1",
            "proof_job_id": job.id,
            "state": job.state,
            "plan": plan,
            "next_action": proof_job_next_action(job.state)
        })));
    }
    Ok(Json(json!({
        "schema_version": "agent-bounties/open-competition-v2-relay-authorization-v1",
        "proof_job_id": job.id,
        "state": job.state,
        "plan": plan,
        "next_action": "Sign the exact plan.relay_authorization EIP-712 typed data with the solver wallet, then call this endpoint again with solver_signature."
    })))
}

fn proof_job_payment_challenge(
    state: &SharedState,
    release: &OpenCompetitionV2Release,
    broker: &str,
    job: &OpenCompetitionV2ProofJob,
) -> Result<payments_x402::PaymentRequired, (StatusCode, Json<Value>)> {
    base_usdc_exact_service_challenge(
        format!(
            "{}/v1/base/open-competition-v2-beta3/proof-jobs/{}/payment",
            state.public_base_url.trim_end_matches('/'),
            job.id
        ),
        eip155_network(&job.network)?,
        &release.settlement_token,
        broker,
        &proof_quote_from_job(job)?,
    )
    .map_err(|error| service_error("pay_proof_job", "x402_challenge_failed", error.to_string()))
}

fn stored_payment_authorization(
    job: &OpenCompetitionV2ProofJob,
    now: u64,
) -> Result<payments_x402::ValidatedExactAuthorization, StatusCode> {
    let value = job
        .payment_authorization
        .as_ref()
        .ok_or(StatusCode::CONFLICT)?;
    let string = |field: &str| {
        value
            .get(field)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)
    };
    let amount = string("amount")?
        .parse::<u64>()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let valid_before = value
        .get("valid_before")
        .and_then(Value::as_u64)
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    if now >= valid_before {
        return Err(StatusCode::CONFLICT);
    }
    let v = value
        .get("v")
        .and_then(Value::as_u64)
        .and_then(|value| u8::try_from(value).ok())
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(payments_x402::ValidatedExactAuthorization {
        payer: string("payer")?,
        recipient: string("recipient")?,
        amount,
        valid_before,
        nonce: string("nonce")?,
        v,
        r: string("r")?,
        s: string("s")?,
    })
}

fn proof_quote_from_job(
    job: &OpenCompetitionV2ProofJob,
) -> Result<ProofBrokerQuote, (StatusCode, Json<Value>)> {
    let winner_mode = match job.winner_mode.as_str() {
        "first_proven" => CompetitionWinnerMode::FirstProven,
        "best_score" => CompetitionWinnerMode::BestScore,
        _ => {
            return Err(service_error(
                "pay_proof_job",
                "invalid_stored_quote",
                "Stored winner mode is invalid.",
            ))
        }
    };
    let net = job.net_prize_if_win.parse::<i128>().map_err(|_| {
        service_error(
            "pay_proof_job",
            "invalid_stored_quote",
            "Stored net prize is invalid.",
        )
    })?;
    Ok(ProofBrokerQuote {
        schema_version: "agent-bounties/open-competition-v2-proof-quote-v1".to_string(),
        quote_id: job.idempotency_key.clone(),
        network: eip155_network(&job.network)?.to_string(),
        competition_contract: job.competition_contract.clone(),
        solver: job.solver.clone(),
        solver_nonce: job.solver_nonce.clone(),
        artifact_hash: job.artifact_hash.clone(),
        proof_system: job.proof_system.clone(),
        gross_prize: job.gross_prize.clone(),
        proof_fee_quote: job.proof_fee_quote.clone(),
        relay_fee_quote: job.relay_fee_quote.clone(),
        net_prize_if_win: job.net_prize_if_win.clone(),
        maximum_charge: job.maximum_charge.clone(),
        profitable_if_win: net > 0,
        winner_mode,
        competition_risk: job.competition_risk.clone(),
        quote_expiration: unix_timestamp(job.quote_expires_at, "quote_expires_at")?,
        proof_sla_deadline: unix_timestamp(job.proof_sla_deadline, "proof_sla_deadline")?,
        evidence_boundary: "This persisted quote is not payment or proof evidence.".to_string(),
    })
}

async fn broadcast_proof_job_payment(
    state: &SharedState,
    job: &OpenCompetitionV2ProofJob,
    release: &OpenCompetitionV2Release,
    authorization: &payments_x402::ValidatedExactAuthorization,
) -> Result<chain_base::BaseRelayedTransaction, ProofPaymentBroadcastError> {
    let relayer = state
        .x402_relayer
        .relayer
        .as_ref()
        .ok_or(ProofPaymentBroadcastError::Status(
            StatusCode::SERVICE_UNAVAILABLE,
        ))?;
    if !authorization
        .recipient
        .eq_ignore_ascii_case(&relayer.address())
    {
        return Err(ProofPaymentBroadcastError::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
        ));
    }
    let intent = plan_open_competition_v2_broker_payment(
        &job.network,
        &release.settlement_token,
        &relayer.address(),
        &OpenCompetitionV2BrokerPaymentAuthorization {
            payer: authorization.payer.clone(),
            recipient: authorization.recipient.clone(),
            amount: authorization.amount,
            valid_before: authorization.valid_before,
            nonce: authorization.nonce.clone(),
            v: authorization.v,
            r: authorization.r.clone(),
            s: authorization.s.clone(),
        },
    )
    .map_err(|_| ProofPaymentBroadcastError::Status(StatusCode::UNPROCESSABLE_ENTITY))?;
    let (_, rpc_url) = state
        .base_rpc_urls
        .resolve(&job.network)
        .map_err(|_| ProofPaymentBroadcastError::Status(StatusCode::SERVICE_UNAVAILABLE))?;
    timeout(
        Duration::from_secs(state.x402_relayer.rpc_timeout_seconds),
        relayer.simulate_and_broadcast(
            &rpc_url,
            network_chain_id(&job.network).map_err(ProofPaymentBroadcastError::Status)?,
            &intent,
            state.x402_relayer.max_gas,
            state.x402_relayer.max_fee_per_gas_wei,
        ),
    )
    .await
    .map_err(|_| ProofPaymentBroadcastError::Status(StatusCode::SERVICE_UNAVAILABLE))?
    .map_err(ProofPaymentBroadcastError::Chain)
}

#[derive(Debug)]
enum ProofPaymentBroadcastError {
    Chain(ChainBaseError),
    Status(StatusCode),
}

fn retryable_proof_payment_broadcast_error(error: &ChainBaseError) -> Option<&'static str> {
    match error {
        ChainBaseError::RelayerSimulation(message)
            if message.contains("transfer amount exceeds balance") =>
        {
            Some("payer_balance_not_yet_visible")
        }
        ChainBaseError::RelayerSimulation(message)
            if message.contains("authorization is not yet valid") =>
        {
            Some("authorization_not_yet_visible")
        }
        ChainBaseError::RelayerSimulation(message)
            if [
                "authorization is expired",
                "invalid signature",
                "authorization is used",
            ]
            .iter()
            .any(|terminal| message.contains(terminal)) =>
        {
            None
        }
        ChainBaseError::RelayerSimulation(_) => Some("relayer_simulation_retry"),
        ChainBaseError::RelayerProvider(_) => Some("relayer_provider_retry"),
        ChainBaseError::RelayerGasLimitExceeded { .. } => Some("gas_limit_retry"),
        ChainBaseError::RelayerFeeCapExceeded { .. } => Some("fee_cap_retry"),
        ChainBaseError::RelayerInsufficientBalance { .. } => Some("gas_reserve_retry"),
        _ => None,
    }
}

async fn reconcile_proof_job_payment(
    state: &SharedState,
    job: OpenCompetitionV2ProofJob,
) -> Result<OpenCompetitionV2ProofJob, StatusCode> {
    if job.state != OpenCompetitionV2ProofJobState::PaymentPending {
        return Ok(job);
    }
    let Some(tx_hash) = job.payment_tx_hash.as_deref() else {
        return Ok(job);
    };
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let release = release_from_environment(&job.network).map_err(|(status, _)| status)?;
    let (_, rpc_url) = state
        .base_rpc_urls
        .resolve(&job.network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let response = fetch_transaction_receipt(&rpc_url, tx_hash, 41)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let Some(receipt) = response.result else {
        return Ok(job);
    };
    if receipt.succeeded().map_err(|_| StatusCode::BAD_GATEWAY)? == Some(false) {
        return store
            .transition_open_competition_v2_proof_job(
                job.id,
                OpenCompetitionV2ProofJobState::PaymentPending,
                OpenCompetitionV2ProofJobState::Quoted,
                &OpenCompetitionV2ProofJobUpdate {
                    failure_code: Some("payment_transaction_reverted".to_string()),
                    failure_message: Some(
                        "No proof-service payment was accepted; request a fresh quote or retry before expiration."
                            .to_string(),
                    ),
                    ..Default::default()
                },
            )
            .await
            .map_err(|_| StatusCode::CONFLICT);
    }
    let block_number = receipt
        .block_number()
        .map_err(|_| StatusCode::BAD_GATEWAY)?
        .ok_or(StatusCode::BAD_GATEWAY)?;
    let latest = fetch_block_number(&rpc_url, 42)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let confirmations = latest.saturating_sub(block_number).saturating_add(1);
    if confirmations < state.x402_relayer.confirmations {
        return Ok(job);
    }
    let confirmed = fetch_transaction_receipt(&rpc_url, tx_hash, 43)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .result
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if confirmed
        .block_number()
        .map_err(|_| StatusCode::BAD_GATEWAY)?
        != Some(block_number)
        || confirmed.succeeded().map_err(|_| StatusCode::BAD_GATEWAY)? != Some(true)
    {
        return Ok(job);
    }
    let payer = job.payer.as_deref().ok_or(StatusCode::BAD_GATEWAY)?;
    let broker = env::var("OPEN_COMPETITION_V2_BROKER_PAYMENT_ADDRESS")
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let amount = job
        .maximum_charge
        .parse::<u64>()
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    let logs = confirmed
        .logs_to_evm_logs()
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    if !logs.iter().any(|log| {
        exact_usdc_transfer(
            log,
            &release.settlement_token,
            payer,
            &broker,
            amount,
            tx_hash,
        )
    }) {
        return Err(StatusCode::BAD_GATEWAY);
    }
    let safe = fetch_exact_block_identity(&rpc_url, latest, 44)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .transition_open_competition_v2_proof_job(
            job.id,
            OpenCompetitionV2ProofJobState::PaymentPending,
            OpenCompetitionV2ProofJobState::Paid,
            &OpenCompetitionV2ProofJobUpdate {
                payment_tx_hash: Some(tx_hash.to_string()),
                payment_block_number: Some(block_number),
                payment_evidence: Some(json!({
                    "schema_version": "agent-bounties/open-competition-v2-proof-payment-evidence-v1",
                    "network": job.network,
                    "asset": release.settlement_token,
                    "payer": payer,
                    "recipient": broker,
                    "amount": amount.to_string(),
                    "transaction_hash": tx_hash,
                    "block_number": block_number,
                    "block_hash": confirmed.block_hash,
                    "safe_block_number": safe.number,
                    "safe_block_hash": safe.hash,
                    "confirmations": confirmations
                })),
                ..Default::default()
            },
        )
        .await
        .map_err(|_| StatusCode::CONFLICT)
}

fn exact_usdc_transfer(
    log: &chain_base::EvmLog,
    token: &str,
    payer: &str,
    recipient: &str,
    amount: u64,
    tx_hash: &str,
) -> bool {
    const TRANSFER_TOPIC: &str =
        "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
    if !log.address.eq_ignore_ascii_case(token)
        || !log.tx_hash.eq_ignore_ascii_case(tx_hash)
        || log.topics.len() != 3
        || !log.topics[0].eq_ignore_ascii_case(TRANSFER_TOPIC)
    {
        return false;
    }
    let topic_address = |address: &str| {
        format!(
            "0x{}{}",
            "0".repeat(24),
            address.trim_start_matches("0x").to_ascii_lowercase()
        )
    };
    if !log.topics[1].eq_ignore_ascii_case(&topic_address(payer))
        || !log.topics[2].eq_ignore_ascii_case(&topic_address(recipient))
    {
        return false;
    }
    let Some(raw) = log.data.strip_prefix("0x") else {
        return false;
    };
    if raw.len() != 64 || !raw[..48].bytes().all(|byte| byte == b'0') {
        return false;
    }
    u64::from_str_radix(&raw[48..], 16).ok() == Some(amount)
}

fn proof_job_payment_response(job: &OpenCompetitionV2ProofJob) -> Result<Response, StatusCode> {
    let status = if job.state == OpenCompetitionV2ProofJobState::Paid {
        StatusCode::OK
    } else {
        StatusCode::ACCEPTED
    };
    let mut response = (
        status,
        Json(json!({
            "schema_version": "agent-bounties/open-competition-v2-proof-payment-v1",
            "proof_job_id": job.id,
            "state": job.state,
            "payment_evidence": job.payment_evidence,
            "next_action": proof_job_next_action(job.state),
            "evidence_boundary": "Only the attached canonical Base USDC evidence proves payment."
        })),
    )
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    if job.state == OpenCompetitionV2ProofJobState::Paid {
        let settlement = SettlementResponse {
            success: true,
            error_reason: None,
            error_message: None,
            payer: job.payer.clone(),
            transaction: job
                .payment_tx_hash
                .clone()
                .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?,
            network: eip155_network(&job.network)
                .map_err(|(status, _)| status)?
                .to_string(),
            amount: Some(job.maximum_charge.clone()),
            extensions: Some(BTreeMap::from([(
                "agent-bounties".to_string(),
                json!({"proofJobId": job.id, "canonicalEvidence": job.payment_evidence}),
            )])),
            extra: None,
        };
        let encoded = encode_payment_response_header(&settlement)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        response.headers_mut().insert(
            HeaderName::from_static(PAYMENT_RESPONSE_HEADER),
            HeaderValue::from_str(&encoded).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        );
    }
    Ok(response)
}

fn proof_job_payment_retry_response(
    job: &OpenCompetitionV2ProofJob,
    reason: &'static str,
) -> Result<Response, StatusCode> {
    let mut response = (
        StatusCode::ACCEPTED,
        Json(json!({
            "schema_version": "agent-bounties/open-competition-v2-proof-payment-v1",
            "proof_job_id": job.id,
            "state": job.state,
            "payment_evidence": job.payment_evidence,
            "retryable": true,
            "retry_after_seconds": 2,
            "pending_reason": reason,
            "next_action": "Retry this exact payment endpoint without signing another authorization.",
            "evidence_boundary": "A retryable relay response is not payment evidence. Only attached canonical Base USDC evidence proves payment."
        })),
    )
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("2"));
    Ok(response)
}

fn unix_timestamp(value: DateTime<Utc>, field: &str) -> Result<u64, (StatusCode, Json<Value>)> {
    u64::try_from(value.timestamp())
        .map_err(|_| service_error("read_quote", "invalid_timestamp", field))
}

fn build_metric_proof_input(
    network: &str,
    projection: &chain_base::OpenCompetitionV2Projection,
    body: &ProofQuoteBody,
) -> Result<(Value, String), (StatusCode, Json<Value>)> {
    match &body.metric {
        ProofMetricBody::PublicVector(metric) => {
            build_public_vector_proof_input(network, projection, body, metric)
        }
        ProofMetricBody::StructuredArtifact(metric) => {
            build_structured_artifact_proof_input(network, projection, body, metric)
        }
    }
}

fn build_public_vector_proof_input(
    network: &str,
    projection: &chain_base::OpenCompetitionV2Projection,
    body: &ProofQuoteBody,
    metric: &PublicVectorMetricBody,
) -> Result<(Value, String), (StatusCode, Json<Value>)> {
    let required = |value: &Option<String>, field: &str| {
        value.clone().ok_or_else(|| {
            conflict(
                "quote_proof",
                "competition_profile_incomplete",
                format!("Safe-block projection is missing {field}; wait for index reconciliation."),
            )
        })
    };
    let threshold = metric.threshold.parse::<i128>().map_err(|_| {
        bad_request(
            "quote_proof",
            "invalid_metric_threshold",
            "metric.threshold must be a signed decimal string within i128",
        )
    })?;
    let contract_threshold = required(&projection.score_threshold, "score_threshold")?
        .parse::<i128>()
        .map_err(|_| {
            service_error(
                "quote_proof",
                "invalid_indexed_threshold",
                "Indexed competition score threshold is invalid.",
            )
        })?;
    if threshold != contract_threshold {
        return Err(bad_request(
            "quote_proof",
            "metric_threshold_mismatch",
            "metric.threshold must equal the competition's immutable score threshold",
        ));
    }
    let expected_direction = match metric.mode {
        PublicVectorMode::AllEqual | PublicVectorMode::MaximizeExactMatches => "higher_is_better",
        PublicVectorMode::MinimizeAbsoluteError => "lower_is_better",
    };
    if projection.score_direction.as_deref() != Some(expected_direction) {
        return Err(bad_request(
            "quote_proof",
            "metric_direction_mismatch",
            format!(
                "{} requires score_direction={expected_direction}",
                metric_mode_name(metric.mode)
            ),
        ));
    }
    if metric.mode == PublicVectorMode::AllEqual {
        let total_weight = metric
            .vectors
            .iter()
            .try_fold(0_i128, |total, vector| {
                total.checked_add(i128::from(vector.weight))
            })
            .ok_or_else(|| {
                bad_request(
                    "quote_proof",
                    "metric_weight_overflow",
                    "metric vector weights overflow i128",
                )
            })?;
        if threshold != total_weight {
            return Err(bad_request(
                "quote_proof",
                "all_equal_threshold_mismatch",
                "all_equal requires threshold equal to the sum of vector weights",
            ));
        }
    }

    let proof_system = match projection.proof_system.as_deref() {
        Some("groth16") => GROTH16_PROOF_SYSTEM,
        Some("plonk") => PLONK_PROOF_SYSTEM,
        _ => {
            return Err(conflict(
                "quote_proof",
                "proof_system_unknown",
                "Wait for index reconciliation.",
            ))
        }
    };
    let metric_program_hash = required(&projection.metric_program_hash, "metric_program_hash")?;
    if !metric_program_hash.eq_ignore_ascii_case(&bytes32_hex(METRIC_PROGRAM_HASH)) {
        return Err(conflict(
            "quote_proof",
            "unsupported_metric_program",
            "The hosted broker supports public-vector-metric-v1 only.",
        ));
    }
    let journal_schema_hash = required(&projection.journal_schema_hash, "journal_schema_hash")?;
    if !journal_schema_hash.eq_ignore_ascii_case(&bytes32_hex(JOURNAL_SCHEMA_HASH)) {
        return Err(conflict(
            "quote_proof",
            "unsupported_journal_schema",
            "The indexed journal schema is not public-vector-metric-v1.",
        ));
    }
    let input = PublicVectorProgramInput {
        scope: JournalScopeV2 {
            chain_id: network_chain_id(network).map_err(|_| {
                bad_request(
                    "quote_proof",
                    "unknown_network",
                    "Use base-mainnet or base-sepolia.",
                )
            })?,
            competition: fixed_hex(&projection.competition, "competition_contract")?,
            bounty_id: fixed_hex(&projection.bounty_id, "bounty_id")?,
            solver: fixed_hex(&body.solver, "solver")?,
            solver_nonce: decimal_u128(&body.solver_nonce, "solver_nonce")?,
            proof_system,
            program_vkey: fixed_hex(
                &required(&projection.program_vkey, "program_vkey")?,
                "program_vkey",
            )?,
            source_hash: fixed_hex(
                &required(&projection.source_hash, "source_hash")?,
                "source_hash",
            )?,
            elf_hash: fixed_hex(&required(&projection.elf_hash, "elf_hash")?, "elf_hash")?,
            execution_policy_hash: fixed_hex(
                &required(&projection.execution_policy_hash, "execution_policy_hash")?,
                "execution_policy_hash",
            )?,
            settlement_policy_hash: fixed_hex(
                &required(&projection.settlement_policy_hash, "settlement_policy_hash")?,
                "settlement_policy_hash",
            )?,
            beta_risk_hash: fixed_hex(
                &required(&projection.beta_risk_hash, "beta_risk_hash")?,
                "beta_risk_hash",
            )?,
        },
        mode: metric.mode,
        threshold,
        vectors: metric.vectors.clone(),
    };
    let output = execute_public_vector_program(&input).map_err(|error| {
        bad_request(
            "quote_proof",
            "invalid_metric_input",
            format!("public-vector-metric-v1 rejected the input: {error:?}"),
        )
    })?;
    let verification_policy = required(
        &projection.verification_policy_hash,
        "verification_policy_hash",
    )?;
    if !verification_policy.eq_ignore_ascii_case(&bytes32_hex(output.verification_policy_hash)) {
        return Err(bad_request(
            "quote_proof",
            "verification_policy_mismatch",
            "Expected vectors, weights, mode, or threshold do not match the immutable verification policy.",
        ));
    }
    if !body
        .artifact_hash
        .eq_ignore_ascii_case(&bytes32_hex(output.submission_hash))
    {
        return Err(bad_request(
            "quote_proof",
            "artifact_hash_mismatch",
            "artifact_hash must equal the canonical hash of the submitted observed vector",
        ));
    }
    Ok((
        serde_json::to_value(input).map_err(|error| {
            service_error(
                "quote_proof",
                "metric_serialization_failed",
                error.to_string(),
            )
        })?,
        format!("0x{}", hex::encode(output.journal)),
    ))
}

fn build_structured_artifact_proof_input(
    network: &str,
    projection: &chain_base::OpenCompetitionV2Projection,
    body: &ProofQuoteBody,
    metric: &StructuredArtifactMetricBody,
) -> Result<(Value, String), (StatusCode, Json<Value>)> {
    if metric.profile_id != "structured-artifact-metric-v1" {
        return Err(bad_request(
            "quote_proof",
            "unsupported_metric_profile",
            "Use profile_id=structured-artifact-metric-v1.",
        ));
    }
    let required = |value: &Option<String>, field: &str| {
        value.clone().ok_or_else(|| {
            conflict(
                "quote_proof",
                "competition_profile_incomplete",
                format!("Safe-block projection is missing {field}; wait for index reconciliation."),
            )
        })
    };
    let threshold = decimal_u128(&metric.threshold, "metric.threshold")?;
    let contract_threshold = required(&projection.score_threshold, "score_threshold")?
        .parse::<u128>()
        .map_err(|_| {
            service_error(
                "quote_proof",
                "invalid_indexed_threshold",
                "Indexed competition score threshold is invalid.",
            )
        })?;
    if threshold != contract_threshold {
        return Err(bad_request(
            "quote_proof",
            "metric_threshold_mismatch",
            "metric.threshold must equal the competition's immutable score threshold",
        ));
    }
    if projection.score_direction.as_deref() != Some("higher_is_better") {
        return Err(bad_request(
            "quote_proof",
            "metric_direction_mismatch",
            "structured-artifact-metric-v1 requires score_direction=higher_is_better",
        ));
    }
    let proof_system = match projection.proof_system.as_deref() {
        Some("groth16") => GROTH16_PROOF_SYSTEM,
        Some("plonk") => PLONK_PROOF_SYSTEM,
        _ => {
            return Err(conflict(
                "quote_proof",
                "proof_system_unknown",
                "Wait for index reconciliation.",
            ))
        }
    };
    if !required(&projection.metric_program_hash, "metric_program_hash")?
        .eq_ignore_ascii_case(&bytes32_hex(STRUCTURED_ARTIFACT_METRIC_PROGRAM_HASH))
    {
        return Err(conflict(
            "quote_proof",
            "unsupported_metric_program",
            "The competition is not pinned to structured-artifact-metric-v1.",
        ));
    }
    if !required(&projection.journal_schema_hash, "journal_schema_hash")?
        .eq_ignore_ascii_case(&bytes32_hex(STRUCTURED_ARTIFACT_JOURNAL_SCHEMA_HASH))
    {
        return Err(conflict(
            "quote_proof",
            "unsupported_journal_schema",
            "The competition journal schema is not structured-artifact-metric-v1.",
        ));
    }
    let input = StructuredArtifactProgramInput {
        scope: JournalScopeV2 {
            chain_id: network_chain_id(network).map_err(|_| {
                bad_request(
                    "quote_proof",
                    "unknown_network",
                    "Use base-mainnet or base-sepolia.",
                )
            })?,
            competition: fixed_hex(&projection.competition, "competition_contract")?,
            bounty_id: fixed_hex(&projection.bounty_id, "bounty_id")?,
            solver: fixed_hex(&body.solver, "solver")?,
            solver_nonce: decimal_u128(&body.solver_nonce, "solver_nonce")?,
            proof_system,
            program_vkey: fixed_hex(
                &required(&projection.program_vkey, "program_vkey")?,
                "program_vkey",
            )?,
            source_hash: fixed_hex(
                &required(&projection.source_hash, "source_hash")?,
                "source_hash",
            )?,
            elf_hash: fixed_hex(&required(&projection.elf_hash, "elf_hash")?, "elf_hash")?,
            execution_policy_hash: fixed_hex(
                &required(&projection.execution_policy_hash, "execution_policy_hash")?,
                "execution_policy_hash",
            )?,
            settlement_policy_hash: fixed_hex(
                &required(&projection.settlement_policy_hash, "settlement_policy_hash")?,
                "settlement_policy_hash",
            )?,
            beta_risk_hash: fixed_hex(
                &required(&projection.beta_risk_hash, "beta_risk_hash")?,
                "beta_risk_hash",
            )?,
        },
        threshold,
        artifact: metric.artifact_utf8.as_bytes().to_vec(),
        requirements: metric.requirements.clone(),
    };
    let output = execute_structured_artifact_program(&input).map_err(|error| {
        bad_request(
            "quote_proof",
            "invalid_metric_input",
            format!("structured-artifact-metric-v1 rejected the input: {error:?}"),
        )
    })?;
    let verification_policy = required(
        &projection.verification_policy_hash,
        "verification_policy_hash",
    )?;
    if !verification_policy.eq_ignore_ascii_case(&bytes32_hex(output.verification_policy_hash)) {
        return Err(bad_request(
            "quote_proof",
            "verification_policy_mismatch",
            "The artifact requirements or threshold do not match the immutable verification policy.",
        ));
    }
    if !body
        .artifact_hash
        .eq_ignore_ascii_case(&bytes32_hex(output.submission_hash))
    {
        return Err(bad_request(
            "quote_proof",
            "artifact_hash_mismatch",
            "artifact_hash must equal the canonical hash of artifact_utf8",
        ));
    }
    let mut program_input = serde_json::to_value(input).map_err(|error| {
        service_error(
            "quote_proof",
            "metric_serialization_failed",
            error.to_string(),
        )
    })?;
    program_input
        .as_object_mut()
        .expect("structured artifact input serializes as an object")
        .insert(
            "_profile_id".to_string(),
            Value::String("structured-artifact-metric-v1".to_string()),
        );
    Ok((program_input, format!("0x{}", hex::encode(output.journal))))
}

fn metric_mode_name(mode: PublicVectorMode) -> &'static str {
    match mode {
        PublicVectorMode::AllEqual => "all_equal",
        PublicVectorMode::MaximizeExactMatches => "maximize_exact_matches",
        PublicVectorMode::MinimizeAbsoluteError => "minimize_absolute_error",
    }
}

fn bytes32_hex(value: [u8; 32]) -> String {
    format!("0x{}", hex::encode(value))
}

fn fixed_hex<const N: usize>(
    value: &str,
    field: &str,
) -> Result<[u8; N], (StatusCode, Json<Value>)> {
    decode_hex(value, N, field)?
        .try_into()
        .map_err(|_| bad_request("quote_proof", "invalid_hex_length", field))
}

fn network_chain_id(network: &str) -> Result<u64, StatusCode> {
    match network {
        "base-mainnet" => Ok(8453),
        "base-sepolia" => Ok(84532),
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

fn build_creation_plan(
    body: CreationBody,
    require_public_creation: bool,
) -> Result<chain_base::OpenCompetitionV2CreationPlan, (StatusCode, Json<Value>)> {
    let network = network_or_default(body.network);
    let release = release_from_environment(&network)?;
    let initial_funding = decimal_u128(&body.initial_funding, "initial_funding")?;
    let params = OpenCompetitionV2CreateParams {
        solver_reward: decimal_u128(&body.params.solver_reward, "solver_reward")?,
        keeper_reward: decimal_u128(&body.params.keeper_reward, "keeper_reward")?,
        funding_deadline: body.params.funding_deadline,
        proof_window_seconds: body.params.proof_window_seconds,
        winner_mode: body.params.winner_mode,
        score_direction: body.params.score_direction,
        score_threshold: body.params.score_threshold,
        proof_system: body.params.proof_system,
        program_vkey: body.params.program_vkey,
        source_hash: body.params.source_hash,
        elf_hash: body.params.elf_hash,
        journal_schema_hash: body.params.journal_schema_hash,
        metric_program_hash: body.params.metric_program_hash,
        execution_policy_hash: body.params.execution_policy_hash,
        verification_policy_hash: body.params.verification_policy_hash,
        settlement_policy_hash: body.params.settlement_policy_hash,
        beta_risk_hash: body.params.beta_risk_hash,
    };
    if require_public_creation && !release.public_creation_enabled {
        return Err(service_error(
            "prepare_creation",
            "beta_creation_disabled",
            "Public V2 creation remains disabled until both canaries and production indexers agree.",
        ));
    }
    plan_open_competition_v2_creation(OpenCompetitionV2CreationRequest {
        release,
        creator: body.creator,
        creation_nonce: body.creation_nonce,
        acknowledged_risk_hash: body.acknowledged_risk_hash,
        initial_funding,
        params,
    })
    .map_err(|error| bad_request("prepare_creation", "invalid_profile", error.to_string()))
}

pub(crate) fn release_from_environment(
    network: &str,
) -> Result<OpenCompetitionV2Release, (StatusCode, Json<Value>)> {
    let prefix = match network {
        "base-mainnet" => "BASE_MAINNET",
        "base-sepolia" => "BASE_SEPOLIA",
        _ => {
            return Err(bad_request(
                "resolve_release",
                "unknown_network",
                "Use base-mainnet or base-sepolia.",
            ));
        }
    };
    let variable = format!("{prefix}_OPEN_COMPETITION_V2_BETA3_RELEASE_MANIFEST_JSON");
    let raw = env::var(&variable).map_err(|_| {
        service_error(
            "resolve_release",
            "release_not_configured",
            format!("{variable} is not configured"),
        )
    })?;
    let release: OpenCompetitionV2Release = serde_json::from_str(&raw).map_err(|_| {
        service_error(
            "resolve_release",
            "release_manifest_invalid",
            format!("{variable} is not valid V2 release JSON"),
        )
    })?;
    if release.network != network
        || release.protocol_version != OPEN_COMPETITION_V2_PROTOCOL_VERSION
    {
        return Err(service_error(
            "resolve_release",
            "release_manifest_mismatch",
            "Configured release network or protocol version does not match the request.",
        ));
    }
    validate_open_competition_v2_release(&release).map_err(|error| {
        service_error(
            "resolve_release",
            "release_manifest_invalid",
            error.to_string(),
        )
    })?;
    Ok(release)
}

pub(crate) async fn current_indexer_agreement(
    state: &SharedState,
    network: &str,
    release: &OpenCompetitionV2Release,
) -> Result<db::OpenCompetitionV2IndexerAgreement, (StatusCode, Json<Value>)> {
    let store = state.store.as_ref().ok_or_else(database_unavailable)?;
    let agreement = store
        .get_open_competition_v2_indexer_agreement(network, &release.factory_contract)
        .await
        .map_err(|error| {
            service_error(
                "resolve_release",
                "indexer_agreement_read_failed",
                error.to_string(),
            )
        })?
        .ok_or_else(|| {
            service_error(
                "resolve_release",
                "indexer_agreement_missing",
                "Primary and shadow V2 indexers have not produced agreement evidence.",
            )
        })?;
    let max_age_seconds = env::var("OPEN_COMPETITION_V2_INDEXER_AGREEMENT_MAX_AGE_SECONDS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(120)
        .clamp(30, 600);
    let max_lag_blocks = env::var("OPEN_COMPETITION_V2_INDEXER_MAX_LAG_BLOCKS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(64)
        .clamp(8, 600);
    let age_seconds = Utc::now()
        .signed_duration_since(agreement.observed_at)
        .num_seconds();
    if !indexer_agreement_is_current(
        release,
        &agreement,
        age_seconds,
        max_age_seconds,
        max_lag_blocks,
    ) {
        return Err(service_error(
            "resolve_release",
            "indexer_agreement_unavailable",
            "Primary and shadow V2 indexer agreement is missing, stale, lagging, or mismatched.",
        ));
    }
    Ok(agreement)
}

fn indexer_agreement_is_current(
    release: &OpenCompetitionV2Release,
    agreement: &db::OpenCompetitionV2IndexerAgreement,
    age_seconds: i64,
    max_age_seconds: i64,
    max_lag_blocks: u64,
) -> bool {
    agreement.agrees
        && agreement.protocol_version == OPEN_COMPETITION_V2_PROTOCOL_VERSION
        && agreement.common_safe_block >= release.deployment_block
        && agreement
            .primary_safe_head
            .saturating_sub(agreement.common_safe_block)
            <= max_lag_blocks
        && agreement
            .shadow_safe_head
            .saturating_sub(agreement.common_safe_block)
            <= max_lag_blocks
        && agreement
            .primary_block_hash
            .eq_ignore_ascii_case(&agreement.shadow_block_hash)
        && (0..=max_age_seconds).contains(&age_seconds)
}

fn require_reviewed_broker_profile(
    release: &OpenCompetitionV2Release,
    projection: &chain_base::OpenCompetitionV2Projection,
) -> Result<(), (StatusCode, Json<Value>)> {
    if !release.proof_broker_enabled {
        return Err(service_error(
            "quote_proof",
            "proof_broker_release_disabled",
            "The pinned release has not enabled hosted proving.",
        ));
    }
    let equals = |indexed: &Option<String>, released: &str| {
        indexed
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case(released))
    };
    let reviewed = release.metric_programs.iter().any(|profile| {
        profile.classification == OpenCompetitionV2ProgramClassification::Reviewed
            && equals(&projection.program_vkey, &profile.program_vkey)
            && equals(&projection.source_hash, &profile.source_hash)
            && equals(&projection.elf_hash, &profile.elf_hash)
            && equals(
                &projection.journal_schema_hash,
                &profile.journal_schema_hash,
            )
            && equals(
                &projection.metric_program_hash,
                &profile.metric_program_hash,
            )
            && is_nonzero_bytes32(&profile.review_evidence_hash)
    });
    if !reviewed {
        return Err(conflict(
            "quote_proof",
            "metric_program_not_reviewed",
            "Use a release-reviewed metric profile for hosted proving, or submit a BYO proof directly.",
        ));
    }
    Ok(())
}

fn is_nonzero_bytes32(value: &str) -> bool {
    value.strip_prefix("0x").is_some_and(|hex| {
        hex.len() == 64
            && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
            && hex.bytes().any(|byte| byte != b'0')
    })
}

fn configured_proof_fee(projection: &chain_base::OpenCompetitionV2Projection) -> Option<u128> {
    let proof_system = projection.proof_system.as_deref()?.to_ascii_uppercase();
    configured_u128_optional(&format!(
        "OPEN_COMPETITION_V2_{proof_system}_PROOF_FEE_BASE_UNITS"
    ))
}

fn estimated_hosted_net_prize(
    projection: &chain_base::OpenCompetitionV2Projection,
) -> Option<u128> {
    projection
        .solver_reward
        .checked_sub(configured_proof_fee(projection)?)?
        .checked_sub(configured_u128_optional(
            "OPEN_COMPETITION_V2_RELAY_FEE_BASE_UNITS",
        )?)
}

fn settlement_token(network: &str) -> Result<&'static str, (StatusCode, Json<Value>)> {
    match network {
        "base-mainnet" => Ok(OPEN_COMPETITION_V2_BASE_USDC),
        "base-sepolia" => Ok(OPEN_COMPETITION_V2_BASE_SEPOLIA_USDC),
        _ => Err(bad_request(
            "resolve_network",
            "unknown_network",
            "Use base-mainnet or base-sepolia.",
        )),
    }
}

fn eip155_network(network: &str) -> Result<&'static str, (StatusCode, Json<Value>)> {
    match network {
        "base-mainnet" => Ok("eip155:8453"),
        "base-sepolia" => Ok("eip155:84532"),
        _ => Err(bad_request(
            "resolve_network",
            "unknown_network",
            "Use base-mainnet or base-sepolia.",
        )),
    }
}

fn configured_u128(name: &str) -> Result<u128, (StatusCode, Json<Value>)> {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u128>().ok())
        .ok_or_else(|| {
            service_error(
                "quote_proof",
                "proof_broker_disabled",
                format!("{name} is not configured as base-unit integer"),
            )
        })
}

fn configured_u128_optional(name: &str) -> Option<u128> {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u128>().ok())
}

fn decimal_u128(value: &str, field: &str) -> Result<u128, (StatusCode, Json<Value>)> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(bad_request(
            "validate_amount",
            "invalid_decimal_amount",
            format!("{field} must be an unsigned decimal string"),
        ));
    }
    value.parse::<u128>().map_err(|_| {
        bad_request(
            "validate_amount",
            "amount_out_of_range",
            format!("{field} exceeds u128"),
        )
    })
}

fn configured_u64(name: &str) -> Result<u64, (StatusCode, Json<Value>)> {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            service_error(
                "quote_proof",
                "proof_broker_disabled",
                format!("{name} is not configured as a positive integer"),
            )
        })
}

fn timestamp(value: u64, field: &str) -> Result<DateTime<Utc>, (StatusCode, Json<Value>)> {
    i64::try_from(value)
        .ok()
        .and_then(|value| DateTime::from_timestamp(value, 0))
        .ok_or_else(|| bad_request("quote_proof", "invalid_timestamp", field))
}

fn decode_hex(
    value: &str,
    expected: usize,
    field: &str,
) -> Result<Vec<u8>, (StatusCode, Json<Value>)> {
    let bytes = decode_hex_bounded(value, expected, expected, field)?;
    if bytes.len() != expected {
        return Err(bad_request(
            "prepare_proof",
            "invalid_hex_length",
            format!("{field} must decode to exactly {expected} bytes"),
        ));
    }
    Ok(bytes)
}

fn decode_hex_bounded(
    value: &str,
    minimum: usize,
    maximum: usize,
    field: &str,
) -> Result<Vec<u8>, (StatusCode, Json<Value>)> {
    let raw = value.strip_prefix("0x").ok_or_else(|| {
        bad_request(
            "prepare_proof",
            "invalid_hex",
            format!("{field} must be 0x-prefixed hex"),
        )
    })?;
    if raw.len() % 2 != 0 || raw.len() / 2 < minimum || raw.len() / 2 > maximum {
        return Err(bad_request(
            "prepare_proof",
            "invalid_hex_length",
            format!("{field} must decode to {minimum}..={maximum} bytes"),
        ));
    }
    hex::decode(raw).map_err(|_| {
        bad_request(
            "prepare_proof",
            "invalid_hex",
            format!("{field} contains non-hex data"),
        )
    })
}

fn proof_job_next_action(state: OpenCompetitionV2ProofJobState) -> &'static str {
    match state {
        OpenCompetitionV2ProofJobState::Quoted => "Pay the exact x402 challenge before quote expiration.",
        OpenCompetitionV2ProofJobState::PaymentPending => "Wait for canonical Base USDC payment confirmation; do not sign another authorization.",
        OpenCompetitionV2ProofJobState::Paid => "Wait for proving to start; retrying payment is unnecessary.",
        OpenCompetitionV2ProofJobState::Proving => "Wait for proof generation or refund_due.",
        OpenCompetitionV2ProofJobState::Proved => "Submit directly or wait for the configured relay.",
        OpenCompetitionV2ProofJobState::Relaying => "Wait for canonical entry or a bounded refund decision.",
        OpenCompetitionV2ProofJobState::Confirmed => "Inspect the attached safe-block CompetitionSettledV2 payment evidence.",
        OpenCompetitionV2ProofJobState::RefundDue => "Wait for canonical USDC refund evidence, due within 30 minutes.",
        OpenCompetitionV2ProofJobState::Refunded => "Confirm the canonical USDC refund evidence.",
        OpenCompetitionV2ProofJobState::LostCompetition => "The proof service completed, but another qualifying entry won; no broker refund is due.",
    }
}

fn network_or_default(network: Option<String>) -> String {
    network
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "base-mainnet".to_string())
}

fn database_unavailable() -> (StatusCode, Json<Value>) {
    service_error(
        "load_canonical_state",
        "database_unavailable",
        "Canonical V2 persistence is not configured.",
    )
}

fn bad_request(
    transition: &str,
    code: &str,
    message: impl Into<String>,
) -> (StatusCode, Json<Value>) {
    problem(StatusCode::BAD_REQUEST, transition, code, false, message)
}

fn conflict(transition: &str, code: &str, message: impl Into<String>) -> (StatusCode, Json<Value>) {
    problem(StatusCode::CONFLICT, transition, code, true, message)
}

fn not_found(transition: &str, code: &str) -> (StatusCode, Json<Value>) {
    problem(StatusCode::NOT_FOUND, transition, code, false, code)
}

fn service_error(
    transition: &str,
    code: &str,
    message: impl Into<String>,
) -> (StatusCode, Json<Value>) {
    problem(
        StatusCode::SERVICE_UNAVAILABLE,
        transition,
        code,
        true,
        message,
    )
}

fn problem(
    status: StatusCode,
    transition: &str,
    code: &str,
    retryable: bool,
    message: impl Into<String>,
) -> (StatusCode, Json<Value>) {
    (
        status,
        Json(json!({
            "schema_version": "agent-bounties/open-competition-v2-problem-v1",
            "state": "failed",
            "failed_transition": transition,
            "error_code": code,
            "retryable": retryable,
            "message": message.into()
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chain_base::{
        OpenCompetitionV2MetricProgramRelease, OpenCompetitionV2ProjectedState,
        OpenCompetitionV2Projection,
    };

    fn hash(byte: u8) -> String {
        format!("0x{}", hex::encode([byte; 32]))
    }

    fn release_fixture() -> OpenCompetitionV2Release {
        OpenCompetitionV2Release {
            protocol_version: OPEN_COMPETITION_V2_PROTOCOL_VERSION.to_string(),
            network: "base-sepolia".to_string(),
            source_commit: "11".repeat(20),
            repository_subject_hash: hash(13),
            sp1_source_commit: "22".repeat(20),
            sp1_circuit_version: "agent-bounties-sp1-safe-v5".to_string(),
            factory_contract: "0x1111111111111111111111111111111111111111".to_string(),
            factory_runtime_code_hash: hash(14),
            implementation_contract: "0x2222222222222222222222222222222222222222".to_string(),
            implementation_runtime_code_hash: hash(15),
            settlement_token: OPEN_COMPETITION_V2_BASE_SEPOLIA_USDC.to_string(),
            groth16_verifier: "0x5555555555555555555555555555555555555555".to_string(),
            groth16_verifier_hash: hash(16),
            groth16_verifier_runtime_code_hash: hash(17),
            groth16_adapter: "0x3333333333333333333333333333333333333333".to_string(),
            groth16_adapter_runtime_code_hash: hash(18),
            plonk_verifier: "0x6666666666666666666666666666666666666666".to_string(),
            plonk_verifier_hash: hash(19),
            plonk_verifier_runtime_code_hash: hash(20),
            plonk_adapter: "0x4444444444444444444444444444444444444444".to_string(),
            plonk_adapter_runtime_code_hash: hash(21),
            deployment_block: 1,
            release_hash: hash(1),
            beta_risk_hash: hash(2),
            public_creation_enabled: false,
            proof_broker_enabled: true,
            metric_programs: vec![OpenCompetitionV2MetricProgramRelease {
                profile_id: "public-vector-metric-v1".to_string(),
                classification: OpenCompetitionV2ProgramClassification::Reviewed,
                program_vkey: hash(3),
                source_hash: hash(4),
                elf_hash: hash(5),
                journal_schema_hash: hash(6),
                metric_program_hash: hash(7),
                review_evidence_hash: hash(8),
            }],
        }
    }

    fn projection() -> OpenCompetitionV2Projection {
        OpenCompetitionV2Projection {
            state: OpenCompetitionV2ProjectedState::Active,
            program_vkey: Some(hash(3)),
            source_hash: Some(hash(4)),
            elf_hash: Some(hash(5)),
            journal_schema_hash: Some(hash(6)),
            metric_program_hash: Some(hash(7)),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn profiles_fail_closed_without_a_release_manifest() {
        let response = profiles_document("base-sepolia", None, false).expect("profiles response");
        assert!(response["canonical_rails"]["sp1_source_commit"].is_null());
        assert!(response["canonical_rails"]["groth16_verifier"].is_null());
        assert_eq!(response["creation_enabled"], false);
    }

    #[test]
    fn hosted_proving_requires_an_exact_reviewed_release_profile() {
        let mut release = release_fixture();
        assert!(require_reviewed_broker_profile(&release, &projection()).is_ok());

        release.metric_programs[0].classification =
            OpenCompetitionV2ProgramClassification::CustomUnreviewed;
        assert!(require_reviewed_broker_profile(&release, &projection()).is_err());

        release = release_fixture();
        release.metric_programs[0].program_vkey = hash(9);
        assert!(require_reviewed_broker_profile(&release, &projection()).is_err());

        release = release_fixture();
        release.proof_broker_enabled = false;
        assert!(require_reviewed_broker_profile(&release, &projection()).is_err());
    }

    #[test]
    fn structured_artifact_quote_derives_and_binds_the_submitted_bytes() {
        let requirements = vec![
            ArtifactRequirement::JsonValid { weight: 1 },
            ArtifactRequirement::JsonPointerStringEquals {
                pointer: "/url".to_string(),
                expected: "https://agentbounties.app/tasks/".to_string(),
                weight: 2,
            },
        ];
        let artifact = r#"{"url":"https://agentbounties.app/tasks/"}"#;
        let scope = JournalScopeV2 {
            chain_id: 84_532,
            competition: [0x11; 20],
            bounty_id: [0x22; 32],
            solver: [0x33; 20],
            solver_nonce: 7,
            proof_system: GROTH16_PROOF_SYSTEM,
            program_vkey: [0x44; 32],
            source_hash: [0x55; 32],
            elf_hash: [0x66; 32],
            execution_policy_hash: [0x77; 32],
            settlement_policy_hash: [0x88; 32],
            beta_risk_hash: [0x99; 32],
        };
        let expected = execute_structured_artifact_program(&StructuredArtifactProgramInput {
            scope,
            threshold: 3,
            artifact: artifact.as_bytes().to_vec(),
            requirements: requirements.clone(),
        })
        .unwrap();
        let projection = OpenCompetitionV2Projection {
            competition: "0x1111111111111111111111111111111111111111".to_string(),
            bounty_id: hash(0x22),
            score_threshold: Some("3".to_string()),
            score_direction: Some("higher_is_better".to_string()),
            proof_system: Some("groth16".to_string()),
            program_vkey: Some(hash(0x44)),
            source_hash: Some(hash(0x55)),
            elf_hash: Some(hash(0x66)),
            journal_schema_hash: Some(bytes32_hex(STRUCTURED_ARTIFACT_JOURNAL_SCHEMA_HASH)),
            metric_program_hash: Some(bytes32_hex(STRUCTURED_ARTIFACT_METRIC_PROGRAM_HASH)),
            execution_policy_hash: Some(hash(0x77)),
            verification_policy_hash: Some(bytes32_hex(expected.verification_policy_hash)),
            settlement_policy_hash: Some(hash(0x88)),
            beta_risk_hash: Some(hash(0x99)),
            ..Default::default()
        };
        let body = ProofQuoteBody {
            network: Some("base-sepolia".to_string()),
            competition_contract: projection.competition.clone(),
            solver: "0x3333333333333333333333333333333333333333".to_string(),
            solver_nonce: "7".to_string(),
            artifact_hash: bytes32_hex(expected.submission_hash),
            relay: true,
            metric: ProofMetricBody::StructuredArtifact(StructuredArtifactMetricBody {
                profile_id: "structured-artifact-metric-v1".to_string(),
                threshold: "3".to_string(),
                artifact_utf8: artifact.to_string(),
                requirements,
            }),
        };
        let (program_input, journal) =
            build_metric_proof_input("base-sepolia", &projection, &body).unwrap();
        assert_eq!(
            program_input["_profile_id"],
            "structured-artifact-metric-v1"
        );
        assert_eq!(journal, format!("0x{}", hex::encode(expected.journal)));
    }

    #[test]
    fn indexer_agreement_rejects_a_fresh_but_lagging_cursor() {
        let release = release_fixture();
        let mut agreement = db::OpenCompetitionV2IndexerAgreement {
            network: release.network.clone(),
            factory_contract: release.factory_contract.clone(),
            protocol_version: release.protocol_version.clone(),
            common_safe_block: 100,
            primary_safe_head: 110,
            shadow_safe_head: 111,
            primary_block_hash: hash(10),
            shadow_block_hash: hash(10),
            canonical_event_count: 0,
            canonical_event_set_hash: hash(11),
            agrees: true,
            failure_code: None,
            observed_at: Utc::now(),
        };
        assert!(indexer_agreement_is_current(
            &release, &agreement, 1, 120, 64
        ));
        agreement.primary_safe_head = 165;
        assert!(!indexer_agreement_is_current(
            &release, &agreement, 1, 120, 64
        ));
        agreement.primary_safe_head = 110;
        assert!(!indexer_agreement_is_current(
            &release, &agreement, 121, 120, 64
        ));
    }

    #[test]
    fn proof_payment_retries_transient_balance_and_provider_observation() {
        assert_eq!(
            retryable_proof_payment_broadcast_error(&ChainBaseError::RelayerSimulation(
                "execution reverted: ERC20: transfer amount exceeds balance".to_string(),
            )),
            Some("payer_balance_not_yet_visible")
        );
        assert_eq!(
            retryable_proof_payment_broadcast_error(&ChainBaseError::RelayerProvider(
                "temporary upstream failure".to_string(),
            )),
            Some("relayer_provider_retry")
        );
        assert_eq!(
            retryable_proof_payment_broadcast_error(&ChainBaseError::RelayerInsufficientBalance {
                balance: 1,
                required: 2,
            }),
            Some("gas_reserve_retry")
        );
        assert_eq!(
            retryable_proof_payment_broadcast_error(&ChainBaseError::RelayerSimulation(
                "execution reverted without decoded reason".to_string(),
            )),
            Some("relayer_simulation_retry")
        );
        assert_eq!(
            retryable_proof_payment_broadcast_error(&ChainBaseError::RelayerGasLimitExceeded {
                estimated: 2,
                maximum: 1,
            }),
            Some("gas_limit_retry")
        );
    }

    #[test]
    fn proof_payment_rejects_terminal_authorization_failures() {
        for message in [
            "FiatTokenV2: authorization is expired",
            "FiatTokenV2: invalid signature",
            "FiatTokenV2: authorization is used",
        ] {
            assert_eq!(
                retryable_proof_payment_broadcast_error(&ChainBaseError::RelayerSimulation(
                    message.to_string(),
                )),
                None
            );
        }
        assert_eq!(
            retryable_proof_payment_broadcast_error(&ChainBaseError::RelayerChainMismatch {
                expected: 8453,
                observed: 1,
            }),
            None
        );
    }
}
