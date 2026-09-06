use crate::{require_operator, SharedState};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post, put},
    Json, Router,
};
use chrono::Utc;
use db::{
    distribution_acquisition_token_hash, normalize_distribution_exclusion_class,
    DistributionOutcomeStats, DistributionRailOutcomeStats, DistributionWalletExclusion,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

pub(crate) const ACQUISITION_HEADER: &str = "x-agent-bounties-acquisition-id";
pub(crate) const HANDOFF_HEADER: &str = "x-agent-bounties-handoff-id";
const PUBLIC_MINIMUM_EXTERNAL_POSTERS: u64 = 3;

pub(crate) fn router() -> Router<SharedState> {
    Router::new()
        .route("/v1/operator/distribution/report", get(operator_report))
        .route(
            "/v1/operator/distribution/wallet-exclusions",
            put(upsert_wallet_exclusion),
        )
        .route("/v1/distribution/summary", get(public_summary))
        .route(
            "/v1/distribution/handoffs/wallet-review",
            post(mark_wallet_reviewed),
        )
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct DistributionRailMetrics {
    rail: String,
    acquisitions: u64,
    assisted_acquisitions: u64,
    mcp_requests: u64,
    failed_mcp_requests: u64,
    prepared_handoffs: u64,
    wallet_reviewed_handoffs: u64,
    handoff_failure_count: u64,
    handoff_failure_rate_basis_points: u64,
    attributed_terms: u64,
    externally_funded_bounties: u64,
    unique_external_funded_posters: u64,
    externally_funded_claimed_bounties: u64,
    externally_funded_submitted_bounties: u64,
    externally_funded_settled_bounties: u64,
    verified_settlements_with_evidence: u64,
    verified_useful_settlements: Option<u64>,
    mcp_failure_rate_basis_points: u64,
    external_funding_base_units: String,
    settled_gmv_base_units: String,
}

impl From<DistributionRailOutcomeStats> for DistributionRailMetrics {
    fn from(value: DistributionRailOutcomeStats) -> Self {
        let mcp_failure_rate_basis_points = if value.mcp_requests == 0 {
            0
        } else {
            value
                .failed_mcp_requests
                .saturating_mul(10_000)
                .checked_div(value.mcp_requests)
                .unwrap_or(0)
                .min(10_000)
        };
        let handoff_attempts = value
            .prepared_handoffs
            .saturating_add(value.handoff_failure_count);
        let handoff_failure_rate_basis_points = if handoff_attempts == 0 {
            0
        } else {
            value
                .handoff_failure_count
                .saturating_mul(10_000)
                .checked_div(handoff_attempts)
                .unwrap_or(0)
                .min(10_000)
        };
        Self {
            rail: value.rail,
            acquisitions: value.acquisitions,
            assisted_acquisitions: value.assisted_acquisitions,
            mcp_requests: value.mcp_requests,
            failed_mcp_requests: value.failed_mcp_requests,
            prepared_handoffs: value.prepared_handoffs,
            wallet_reviewed_handoffs: value.wallet_reviewed_handoffs,
            handoff_failure_count: value.handoff_failure_count,
            handoff_failure_rate_basis_points,
            attributed_terms: value.attributed_terms,
            externally_funded_bounties: value.externally_funded_bounties,
            unique_external_funded_posters: value.unique_external_funded_posters,
            externally_funded_claimed_bounties: value.externally_funded_claimed_bounties,
            externally_funded_submitted_bounties: value.externally_funded_submitted_bounties,
            externally_funded_settled_bounties: value.externally_funded_settled_bounties,
            verified_settlements_with_evidence: value.verified_settlements_with_evidence,
            verified_useful_settlements: None,
            mcp_failure_rate_basis_points,
            external_funding_base_units: value.external_funding_base_units,
            settled_gmv_base_units: value.settled_gmv_base_units,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct DistributionOperatorReport {
    schema_version: String,
    generated_at: String,
    excluded_wallet_classes: Vec<String>,
    protocol_scope: String,
    unavailable_metrics: Vec<String>,
    rails: Vec<DistributionRailMetrics>,
    total_external_funded_bounties: u64,
    unique_external_funded_posters: u64,
    attributed_external_funded_bounties: u64,
    attribution_coverage_basis_points: u64,
    attribution_coverage_ready: bool,
    evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct PublicDistributionRailMetrics {
    rail: String,
    reported: bool,
    externally_funded_bounties: Option<u64>,
    externally_funded_settled_bounties: Option<u64>,
    external_funding_base_units: Option<String>,
    settled_gmv_base_units: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct DistributionPublicSummary {
    schema_version: String,
    status: String,
    generated_at: String,
    privacy_minimum_external_posters: u64,
    total_external_funded_bounties: Option<u64>,
    attributed_external_funded_bounties: Option<u64>,
    attribution_coverage_basis_points: Option<u64>,
    rails: Vec<PublicDistributionRailMetrics>,
    evidence_boundary: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DistributionWalletExclusionRequest {
    wallet_address: String,
    exclusion_class: String,
    reason: Option<String>,
    active: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct DistributionWalletExclusionResponse {
    schema_version: String,
    wallet_address: String,
    exclusion_class: String,
    reason: Option<String>,
    active: bool,
    created_at: String,
    updated_at: String,
    evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct DistributionWalletReviewResponse {
    schema_version: String,
    recorded: bool,
    wallet_reviewed_at: String,
    evidence_boundary: String,
}

fn report_authorized(state: &SharedState, headers: &HeaderMap) -> Result<(), StatusCode> {
    if state.operator_api_token.is_none() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    require_operator(state, headers).map_err(|_| StatusCode::UNAUTHORIZED)
}

async fn load_stats(state: &SharedState) -> Result<DistributionOutcomeStats, StatusCode> {
    state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .distribution_outcome_stats(&state.distribution_excluded_wallet_classes)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}

#[utoipa::path(
    get,
    path = "/v1/operator/distribution/report",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = DistributionOperatorReport),
        (status = 401, description = "Operator token required"),
        (status = 503, description = "Durable attribution store unavailable")
    )
)]
pub(crate) async fn operator_report(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<DistributionOperatorReport>, StatusCode> {
    report_authorized(&state, &headers)?;
    let stats = load_stats(&state).await?;
    Ok(Json(DistributionOperatorReport {
        schema_version: "agent-bounties/distribution-operator-report-v1".to_string(),
        generated_at: Utc::now().to_rfc3339(),
        excluded_wallet_classes: state.distribution_excluded_wallet_classes.clone(),
        protocol_scope: "agent-bounties/autonomous-v1".to_string(),
        unavailable_metrics: vec![
            "verified_useful_settlements: origin-task completion evidence is not yet persisted in the shared attribution model".to_string(),
        ],
        attribution_coverage_ready: stats.attribution_coverage_basis_points >= 9_500,
        rails: stats.rails.into_iter().map(Into::into).collect(),
        total_external_funded_bounties: stats.total_external_funded_bounties,
        unique_external_funded_posters: stats.unique_external_funded_posters,
        attributed_external_funded_bounties: stats.attributed_external_funded_bounties,
        attribution_coverage_basis_points: stats.attribution_coverage_basis_points,
        evidence_boundary: "This report covers canonical agent-bounties/autonomous-v1 only, not legacy bounties or Open Competition. Acquisitions, handoffs, wallet-review boundaries, and bounded handoff failures are server-observed analytics records. Funding requires confirmed canonical FundingAdded and BountyBecameClaimable events, and non-excluded funding must meet the positive canonical target amount; claims, submissions, and settlement require their confirmed canonical events. verified_settlements_with_evidence additionally requires hash-matched published submission evidence. verified_useful_settlements remains unavailable until origin-task completion is durably joined. This report grants no wallet, verification, or payment authority.".to_string(),
    }))
}

#[utoipa::path(
    post,
    path = "/v1/distribution/handoffs/wallet-review",
    responses(
        (status = 200, body = DistributionWalletReviewResponse),
        (status = 400, description = "Both valid attribution headers are required"),
        (status = 409, description = "Attribution headers do not identify the same handoff"),
        (status = 503, description = "Durable attribution store or signing secret unavailable")
    )
)]
pub(crate) async fn mark_wallet_reviewed(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<DistributionWalletReviewResponse>, StatusCode> {
    let (token_hash, handoff_id) =
        authenticated_handoff(&state, &headers)?.ok_or(StatusCode::BAD_REQUEST)?;
    let handoff = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .mark_distribution_handoff_wallet_reviewed(&token_hash, handoff_id, Utc::now())
        .await
        .map_err(|error| match error {
            db::DbError::DistributionAttributionConflict(_) => StatusCode::CONFLICT,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        })?;
    Ok(Json(DistributionWalletReviewResponse {
        schema_version: "agent-bounties/distribution-wallet-review-v1".to_string(),
        recorded: true,
        wallet_reviewed_at: handoff
            .wallet_reviewed_at
            .expect("successful wallet review update returns its timestamp")
            .to_rfc3339(),
        evidence_boundary: "Analytics-only acknowledgement that the first-party review flow reached its wallet-opening boundary. This does not prove a connected wallet, signature, publication, funding, verification, settlement, or payment.".to_string(),
    }))
}

#[utoipa::path(
    get,
    path = "/v1/distribution/summary",
    responses(
        (status = 200, body = DistributionPublicSummary),
        (status = 503, description = "Durable attribution store unavailable")
    )
)]
pub(crate) async fn public_summary(
    State(state): State<SharedState>,
) -> Result<Json<DistributionPublicSummary>, StatusCode> {
    let stats = load_stats(&state).await?;
    Ok(Json(build_public_summary(stats, Utc::now().to_rfc3339())))
}

fn build_public_summary(
    stats: DistributionOutcomeStats,
    generated_at: String,
) -> DistributionPublicSummary {
    let overall_reported = stats.unique_external_funded_posters >= PUBLIC_MINIMUM_EXTERNAL_POSTERS;
    let rails: Vec<_> = stats
        .rails
        .into_iter()
        .map(|rail| {
            let reported = rail.unique_external_funded_posters >= PUBLIC_MINIMUM_EXTERNAL_POSTERS;
            PublicDistributionRailMetrics {
                rail: rail.rail,
                reported,
                externally_funded_bounties: reported.then_some(rail.externally_funded_bounties),
                externally_funded_settled_bounties: reported
                    .then_some(rail.externally_funded_settled_bounties),
                external_funding_base_units: reported.then_some(rail.external_funding_base_units),
                settled_gmv_base_units: reported.then_some(rail.settled_gmv_base_units),
            }
        })
        .collect();
    let has_suppressed_rail = rails.iter().any(|rail| !rail.reported);
    let global_reported = overall_reported && !has_suppressed_rail;
    DistributionPublicSummary {
        schema_version: "agent-bounties/distribution-summary-v1".to_string(),
        status: if global_reported {
            "ready"
        } else if overall_reported {
            "partial"
        } else {
            "insufficient_sample"
        }
        .to_string(),
        generated_at,
        privacy_minimum_external_posters: PUBLIC_MINIMUM_EXTERNAL_POSTERS,
        total_external_funded_bounties: global_reported
            .then_some(stats.total_external_funded_bounties),
        attributed_external_funded_bounties: global_reported
            .then_some(stats.attributed_external_funded_bounties),
        attribution_coverage_basis_points: global_reported
            .then_some(stats.attribution_coverage_basis_points),
        rails,
        evidence_boundary: "Small rail outcomes are withheld. Reported funding and settlement values come only from confirmed canonical events and exclude configured wallet classes. Acquisition records are not people, authority, funding, verification, settlement, or payment evidence.".to_string(),
    }
}

#[utoipa::path(
    put,
    path = "/v1/operator/distribution/wallet-exclusions",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    request_body = DistributionWalletExclusionRequest,
    responses(
        (status = 200, body = DistributionWalletExclusionResponse),
        (status = 400, description = "Invalid address, class, or reason"),
        (status = 401, description = "Operator token required"),
        (status = 503, description = "Durable attribution store unavailable")
    )
)]
pub(crate) async fn upsert_wallet_exclusion(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<DistributionWalletExclusionRequest>,
) -> Result<Json<DistributionWalletExclusionResponse>, StatusCode> {
    report_authorized(&state, &headers)?;
    let wallet_address = chain_base::normalize_evm_address(&request.wallet_address)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .to_ascii_lowercase();
    let exclusion_class = normalize_distribution_exclusion_class(&request.exclusion_class)
        .ok_or(StatusCode::BAD_REQUEST)?;
    let reason = request
        .reason
        .map(|reason| reason.trim().to_string())
        .filter(|reason| !reason.is_empty());
    if reason
        .as_ref()
        .is_some_and(|reason| reason.chars().count() > 500 || reason.chars().any(char::is_control))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let exclusion = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .upsert_distribution_wallet_exclusion(
            &wallet_address,
            exclusion_class,
            reason.as_deref(),
            request.active,
        )
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(wallet_exclusion_response(exclusion)))
}

fn wallet_exclusion_response(
    exclusion: DistributionWalletExclusion,
) -> DistributionWalletExclusionResponse {
    DistributionWalletExclusionResponse {
        schema_version: "agent-bounties/distribution-wallet-exclusion-v1".to_string(),
        wallet_address: exclusion.wallet_address,
        exclusion_class: exclusion.exclusion_class,
        reason: exclusion.reason,
        active: exclusion.active,
        created_at: exclusion.created_at.to_rfc3339(),
        updated_at: exclusion.updated_at.to_rfc3339(),
        evidence_boundary: "This operator classification affects analytics only. It does not establish identity, ownership, funding, verification, settlement, or payment state.".to_string(),
    }
}

pub(crate) async fn bind_terms_attribution(
    state: &SharedState,
    headers: &HeaderMap,
    terms_hash: &str,
    creator_wallet: &str,
) -> Result<(), StatusCode> {
    let Some((token_hash, handoff_id)) = authenticated_handoff(state, headers)? else {
        return Ok(());
    };
    state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .bind_distribution_handoff_terms(
            &token_hash,
            handoff_id,
            terms_hash,
            creator_wallet,
            Utc::now(),
        )
        .await
        .map(|_| ())
        .map_err(|error| match error {
            db::DbError::DistributionAttributionConflict(_) => StatusCode::CONFLICT,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        })
}

fn authenticated_handoff(
    state: &SharedState,
    headers: &HeaderMap,
) -> Result<Option<(String, Uuid)>, StatusCode> {
    let acquisition = headers
        .get(ACQUISITION_HEADER)
        .map(|value| value.to_str().map(str::to_string))
        .transpose()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let handoff = headers
        .get(HANDOFF_HEADER)
        .map(|value| value.to_str().map(str::to_string))
        .transpose()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let (acquisition, handoff) = match (acquisition, handoff) {
        (None, None) => return Ok(None),
        (Some(acquisition), Some(handoff)) => (acquisition, handoff),
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let signing_secret = state
        .distribution_attribution_signing_secret
        .as_deref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let token_hash = distribution_acquisition_token_hash(&acquisition, signing_secret)
        .ok_or(StatusCode::BAD_REQUEST)?;
    let handoff_id = Uuid::parse_str(handoff.trim()).map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Some((token_hash, handoff_id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rail(rail: &str, posters: u64, funded: u64) -> DistributionRailOutcomeStats {
        DistributionRailOutcomeStats {
            rail: rail.to_string(),
            acquisitions: 99,
            assisted_acquisitions: 1,
            mcp_requests: 120,
            failed_mcp_requests: 2,
            prepared_handoffs: 8,
            wallet_reviewed_handoffs: 6,
            handoff_failure_count: 2,
            attributed_terms: 5,
            externally_funded_bounties: funded,
            unique_external_funded_posters: posters,
            externally_funded_claimed_bounties: funded.saturating_sub(1),
            externally_funded_submitted_bounties: funded.saturating_sub(1),
            externally_funded_settled_bounties: funded.saturating_sub(1),
            verified_settlements_with_evidence: funded.saturating_sub(1),
            external_funding_base_units: "5000000".to_string(),
            settled_gmv_base_units: "4000000".to_string(),
        }
    }

    #[test]
    fn operator_funnel_reports_durable_wallet_review_and_unique_failure_rate() {
        let metrics = DistributionRailMetrics::from(rail("bankr", 3, 4));
        assert_eq!(metrics.wallet_reviewed_handoffs, 6);
        assert_eq!(metrics.handoff_failure_count, 2);
        assert_eq!(metrics.handoff_failure_rate_basis_points, 2_000);
        assert_eq!(metrics.verified_useful_settlements, None);
    }

    #[test]
    fn public_summary_withholds_small_rail_outcomes_and_acquisition_counts() {
        let summary = build_public_summary(
            DistributionOutcomeStats {
                rails: vec![rail("bankr", 2, 2), rail("github", 3, 4)],
                total_external_funded_bounties: 7,
                unique_external_funded_posters: 4,
                attributed_external_funded_bounties: 6,
                attribution_coverage_basis_points: 8_571,
            },
            "2026-09-02T00:00:00Z".to_string(),
        );
        assert_eq!(summary.status, "partial");
        assert_eq!(summary.total_external_funded_bounties, None);
        assert_eq!(summary.attributed_external_funded_bounties, None);
        assert_eq!(summary.attribution_coverage_basis_points, None);
        assert!(!summary.rails[0].reported);
        assert_eq!(summary.rails[0].externally_funded_bounties, None);
        assert!(summary.rails[1].reported);
        assert_eq!(summary.rails[1].externally_funded_bounties, Some(4));
    }

    #[test]
    fn public_summary_withholds_global_totals_before_minimum_sample() {
        let summary = build_public_summary(
            DistributionOutcomeStats {
                rails: vec![rail("bankr", 1, 1)],
                total_external_funded_bounties: 1,
                unique_external_funded_posters: 1,
                attributed_external_funded_bounties: 1,
                attribution_coverage_basis_points: 10_000,
            },
            "2026-09-02T00:00:00Z".to_string(),
        );
        assert_eq!(summary.status, "insufficient_sample");
        assert_eq!(summary.attribution_coverage_basis_points, None);
    }

    #[test]
    fn global_privacy_gate_uses_distinct_posters_not_per_rail_sums() {
        let summary = build_public_summary(
            DistributionOutcomeStats {
                rails: vec![rail("bankr", 2, 3), rail("github", 2, 3)],
                total_external_funded_bounties: 6,
                unique_external_funded_posters: 2,
                attributed_external_funded_bounties: 6,
                attribution_coverage_basis_points: 10_000,
            },
            "2026-09-02T00:00:00Z".to_string(),
        );
        assert_eq!(summary.status, "insufficient_sample");
        assert_eq!(summary.total_external_funded_bounties, None);
        assert_eq!(summary.attribution_coverage_basis_points, None);
    }
}
