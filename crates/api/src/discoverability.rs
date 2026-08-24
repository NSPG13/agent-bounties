use crate::{require_operator, SharedState};
use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use db::{DiscoverabilitySnapshot, DiscoveryRouteUsageStats, NewDiscoverabilitySnapshot};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use utoipa::ToSchema;

const INGEST_TOKEN_HEADER: &str = "x-agent-bounties-discoverability-ingest";
const PROVIDERS: [&str; 4] = [
    "search_console",
    "github",
    "first_party",
    "external_interfaces",
];
const INGESTED_PROVIDERS: [&str; 3] = ["search_console", "github", "first_party"];
const PUBLIC_STALE_AFTER_DAYS: i64 = 9;

pub(crate) fn router() -> Router<SharedState> {
    Router::new()
        .route(
            "/v1/operator/discoverability/snapshots",
            post(ingest_snapshots),
        )
        .route("/v1/operator/discoverability/report", get(operator_report))
        .route("/v1/discoverability/summary", get(public_summary))
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiscoverabilitySnapshotRequest {
    provider: String,
    observed_at: DateTime<Utc>,
    window_started_at: DateTime<Utc>,
    window_ended_at: DateTime<Utc>,
    data_through: DateTime<Utc>,
    payload_checksum: String,
    payload: Value,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiscoverabilityIngestionRequest {
    snapshots: Vec<DiscoverabilitySnapshotRequest>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct DiscoverabilityIngestionResponse {
    schema_version: String,
    accepted: usize,
    duplicates: usize,
    retention_months: u8,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DiscoverabilityReportQuery {
    window_days: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DiscoverabilityOperatorReport {
    schema_version: String,
    window_days: u32,
    window_started_at: String,
    generated_at: String,
    snapshots: Vec<DiscoverabilitySnapshot>,
    route_interactions: Vec<DiscoveryRouteUsageStats>,
    coverage_gaps: Vec<DiscoverabilityCoverageGap>,
    missing_providers: Vec<String>,
    evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DiscoverabilityCoverageGap {
    provider: String,
    previous_data_through: String,
    next_window_started_at: String,
    gap_days: u32,
    long_gap: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct DiscoverabilitySourceStatus {
    provider: String,
    available: bool,
    stale: bool,
    observed_at: Option<String>,
    data_through: Option<String>,
    window_started_at: Option<String>,
    window_ended_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct HumanReachSummary {
    search_impressions: Option<u64>,
    organic_clicks: Option<u64>,
    google_average_position: Option<f64>,
    github_unique_visitors: Option<u64>,
    captured_chatgpt_referrals: Option<u64>,
    opportunity_feed_clicks: Option<u64>,
    market_to_funded_opportunity_ctr: Option<f64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct AutomationReachSummary {
    a2a_interactions: Option<u64>,
    mcp_interactions: Option<u64>,
    api_cli_interactions: Option<u64>,
    feed_interactions: Option<u64>,
    github_unique_cloners: Option<u64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct DiscoverabilityPublicSummary {
    schema_version: String,
    status: String,
    window_days: u32,
    stale_after_days: u8,
    generated_at: String,
    sources: Vec<DiscoverabilitySourceStatus>,
    human_reach: HumanReachSummary,
    automation_reach: AutomationReachSummary,
    definitions: Vec<String>,
    evidence_boundary: String,
}

fn ingest_authorized(state: &SharedState, headers: &HeaderMap) -> Result<(), StatusCode> {
    let token = state
        .discoverability_ingest_token
        .as_deref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let authorized = service_runtime::operator_token_is_authorized(
        Some(token),
        headers
            .get(INGEST_TOKEN_HEADER)
            .and_then(|value| value.to_str().ok()),
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
    );
    if authorized {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

fn report_authorized(state: &SharedState, headers: &HeaderMap) -> Result<(), StatusCode> {
    if state.operator_api_token.is_none() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    if require_operator(state, headers).is_ok() {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

fn nonnegative_u64(totals: &serde_json::Map<String, Value>, key: &str) -> bool {
    totals.get(key).and_then(Value::as_u64).is_some()
}

fn bounded_rate(totals: &serde_json::Map<String, Value>, key: &str) -> bool {
    totals
        .get(key)
        .and_then(Value::as_f64)
        .is_some_and(|value| value.is_finite() && (0.0..=1.0).contains(&value))
}

fn valid_provider_totals(provider: &str, payload: &Value) -> bool {
    let Some(totals) = payload.get("totals").and_then(Value::as_object) else {
        return false;
    };
    match provider {
        "search_console" => {
            nonnegative_u64(totals, "impressions")
                && nonnegative_u64(totals, "clicks")
                && totals
                    .get("average_position")
                    .and_then(Value::as_f64)
                    .is_some_and(|value| value.is_finite() && value >= 0.0)
        }
        "github" => ["unique_visitors", "unique_cloners", "views", "clones"]
            .into_iter()
            .all(|key| nonnegative_u64(totals, key)),
        "first_party" => {
            [
                "captured_chatgpt_referrals",
                "opportunity_feed_clicks",
                "market_views",
                "funded_bounty_clicks",
            ]
            .into_iter()
            .all(|key| nonnegative_u64(totals, key))
                && bounded_rate(totals, "market_to_funded_opportunity_ctr")
        }
        "external_interfaces" => ["a2a", "mcp", "api", "cli", "feed"]
            .into_iter()
            .all(|key| nonnegative_u64(totals, key)),
        _ => false,
    }
}

fn validate_snapshot(
    request: DiscoverabilitySnapshotRequest,
    now: DateTime<Utc>,
) -> Result<NewDiscoverabilitySnapshot, StatusCode> {
    if !INGESTED_PROVIDERS.contains(&request.provider.as_str())
        || request.window_started_at > request.window_ended_at
        || request.data_through < request.window_started_at
        || request.data_through > request.window_ended_at
        || request.window_ended_at > request.observed_at
        || request.observed_at > now + Duration::minutes(5)
        || request.observed_at < now - Duration::days(550)
        || !valid_provider_totals(&request.provider, &request.payload)
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let checksum = request.payload_checksum.trim().to_ascii_lowercase();
    if checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let canonical = serde_json::to_vec(&request.payload).map_err(|_| StatusCode::BAD_REQUEST)?;
    let actual = hex::encode(Sha256::digest(canonical));
    if actual != checksum {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(NewDiscoverabilitySnapshot {
        provider: request.provider,
        observed_at: request.observed_at,
        window_started_at: request.window_started_at,
        window_ended_at: request.window_ended_at,
        data_through: request.data_through,
        payload_checksum: checksum,
        payload: request.payload,
    })
}

#[utoipa::path(
    post,
    path = "/v1/operator/discoverability/snapshots",
    security(
        ("discoverability_ingest_token" = []),
        ("discoverability_ingest_bearer" = [])
    ),
    request_body = DiscoverabilityIngestionRequest,
    responses(
        (status = 200, body = DiscoverabilityIngestionResponse),
        (status = 400, description = "Invalid provider snapshot"),
        (status = 401, description = "Ingestion token required"),
        (status = 503, description = "Ingestion credential or durable store unavailable")
    )
)]
pub(crate) async fn ingest_snapshots(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<DiscoverabilityIngestionRequest>,
) -> Result<Json<DiscoverabilityIngestionResponse>, StatusCode> {
    ingest_authorized(&state, &headers)?;
    if request.snapshots.is_empty() || request.snapshots.len() > 100 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let now = Utc::now();
    let snapshot_count = request.snapshots.len();
    let snapshots = request
        .snapshots
        .into_iter()
        .map(|snapshot| validate_snapshot(snapshot, now))
        .collect::<Result<Vec<_>, _>>()?;
    let route_observed_at = snapshots
        .iter()
        .map(|snapshot| snapshot.observed_at)
        .max()
        .ok_or(StatusCode::BAD_REQUEST)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let mut accepted = 0usize;
    for snapshot in snapshots {
        if store
            .insert_discoverability_snapshot(&snapshot)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        {
            accepted += 1;
        }
    }
    let route_snapshot_exists = store
        .list_discoverability_snapshots(route_observed_at - Duration::minutes(1))
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .iter()
        .any(|snapshot| {
            snapshot.provider == "external_interfaces" && snapshot.observed_at == route_observed_at
        });
    if !route_snapshot_exists {
        let route_snapshot = build_route_snapshot(store, route_observed_at).await?;
        if store
            .insert_discoverability_snapshot(&route_snapshot)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        {
            accepted += 1;
        }
    }
    let total_snapshots = snapshot_count + 1;
    Ok(Json(DiscoverabilityIngestionResponse {
        schema_version: "agent-bounties/discoverability-ingestion-v1".to_string(),
        accepted,
        duplicates: total_snapshots.saturating_sub(accepted),
        retention_months: 18,
    }))
}

async fn build_route_snapshot(
    store: &db::PostgresStore,
    observed_at: DateTime<Utc>,
) -> Result<NewDiscoverabilitySnapshot, StatusCode> {
    let thirty_day = store
        .discovery_route_usage_stats(observed_at - Duration::days(30))
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let seven_day = store
        .discovery_route_usage_stats(observed_at - Duration::days(7))
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let payload = json!({
        "totals": route_totals(&thirty_day),
        "route_families": thirty_day,
        "comparison": {
            "current_7d": route_totals(&seven_day),
            "baseline_rule": "first complete seven-day snapshot after deployment"
        },
        "coverage": {"collection_epoch_has_no_historical_backfill": true}
    });
    let payload_checksum = hex::encode(Sha256::digest(
        serde_json::to_vec(&payload).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
    ));
    Ok(NewDiscoverabilitySnapshot {
        provider: "external_interfaces".to_string(),
        observed_at,
        window_started_at: observed_at - Duration::days(30),
        window_ended_at: observed_at,
        data_through: observed_at,
        payload_checksum,
        payload,
    })
}

fn route_totals(rows: &[DiscoveryRouteUsageStats]) -> BTreeMap<String, u64> {
    let mut totals = ["a2a", "mcp", "api", "cli", "feed"]
        .into_iter()
        .map(|interface| (interface.to_string(), 0u64))
        .collect::<BTreeMap<_, _>>();
    for row in rows {
        if let Some(total) = totals.get_mut(&row.interface) {
            *total = total.saturating_add(row.interaction_count);
        }
    }
    totals
}

fn window_days(query: DiscoverabilityReportQuery) -> Result<u32, StatusCode> {
    let days = query.window_days.unwrap_or(30);
    if (1..=548).contains(&days) {
        Ok(days)
    } else {
        Err(StatusCode::BAD_REQUEST)
    }
}

fn missing_providers(snapshots: &[DiscoverabilitySnapshot]) -> Vec<String> {
    PROVIDERS
        .iter()
        .filter(|provider| {
            !snapshots
                .iter()
                .any(|snapshot| snapshot.provider == **provider)
        })
        .map(|provider| (*provider).to_string())
        .collect()
}

#[utoipa::path(
    get,
    path = "/v1/operator/discoverability/report",
    security(
        ("operator_api_token" = []),
        ("operator_bearer" = [])
    ),
    params(("window_days" = Option<u32>, Query, description = "Operator lookback from 1 to 548 days")),
    responses(
        (status = 200, description = "Operator-only snapshots and aggregate route interactions"),
        (status = 400, description = "Invalid window"),
        (status = 401, description = "Operator token required"),
        (status = 503, description = "Durable store unavailable")
    )
)]
pub(crate) async fn operator_report(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(query): Query<DiscoverabilityReportQuery>,
) -> Result<Json<DiscoverabilityOperatorReport>, StatusCode> {
    report_authorized(&state, &headers)?;
    let days = window_days(query)?;
    let generated_at = Utc::now();
    let started_at = generated_at - Duration::days(i64::from(days));
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let snapshots = store
        .list_discoverability_snapshots(started_at)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let route_interactions = store
        .discovery_route_usage_stats(started_at)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let coverage_gaps = coverage_gaps(&snapshots);
    Ok(Json(DiscoverabilityOperatorReport {
        schema_version: "agent-bounties/discoverability-operator-report-v1".to_string(),
        window_days: days,
        window_started_at: started_at.to_rfc3339(),
        generated_at: generated_at.to_rfc3339(),
        missing_providers: missing_providers(&snapshots),
        snapshots,
        route_interactions,
        coverage_gaps,
        evidence_boundary: "This operator-only report can include raw Search Console dimensions and GitHub paths or referrers. It measures discovery coverage and interactions, not people, independent agents, funding, claims, verification, settlement, or payment.".to_string(),
    }))
}

fn coverage_gaps(snapshots: &[DiscoverabilitySnapshot]) -> Vec<DiscoverabilityCoverageGap> {
    let mut gaps = Vec::new();
    for provider in PROVIDERS {
        let mut provider_snapshots = snapshots
            .iter()
            .filter(|snapshot| snapshot.provider == provider)
            .collect::<Vec<_>>();
        provider_snapshots.sort_by_key(|snapshot| snapshot.window_started_at);
        let mut covered_through = None;
        for snapshot in provider_snapshots {
            if let Some(previous) = covered_through {
                if snapshot.window_started_at > previous {
                    let gap_days = (snapshot.window_started_at - previous).num_days().max(0) as u32;
                    if gap_days > 0 {
                        gaps.push(DiscoverabilityCoverageGap {
                            provider: provider.to_string(),
                            previous_data_through: previous.to_rfc3339(),
                            next_window_started_at: snapshot.window_started_at.to_rfc3339(),
                            gap_days,
                            long_gap: true,
                        });
                    }
                }
            }
            covered_through = Some(
                covered_through
                    .map(|previous| previous.max(snapshot.data_through))
                    .unwrap_or(snapshot.data_through),
            );
        }
    }
    gaps
}

fn total_u64(snapshot: Option<&&DiscoverabilitySnapshot>, key: &str) -> Option<u64> {
    snapshot?.payload.get("totals")?.get(key)?.as_u64()
}

fn total_f64(snapshot: Option<&&DiscoverabilitySnapshot>, key: &str) -> Option<f64> {
    snapshot?.payload.get("totals")?.get(key)?.as_f64()
}

#[utoipa::path(
    get,
    path = "/v1/discoverability/summary",
    responses(
        (status = 200, body = DiscoverabilityPublicSummary),
        (status = 503, description = "Durable store unavailable")
    )
)]
pub(crate) async fn public_summary(
    State(state): State<SharedState>,
) -> Result<Json<DiscoverabilityPublicSummary>, StatusCode> {
    let generated_at = Utc::now();
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let snapshots = store
        .list_discoverability_snapshots(generated_at - Duration::days(548))
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(build_public_summary(&snapshots, generated_at)))
}

fn build_public_summary(
    snapshots: &[DiscoverabilitySnapshot],
    generated_at: DateTime<Utc>,
) -> DiscoverabilityPublicSummary {
    let mut latest = BTreeMap::<&str, &DiscoverabilitySnapshot>::new();
    for snapshot in snapshots {
        latest
            .entry(snapshot.provider.as_str())
            .and_modify(|current| {
                if snapshot.observed_at > current.observed_at {
                    *current = snapshot;
                }
            })
            .or_insert(snapshot);
    }
    let mut sources = Vec::new();
    let mut ready = true;
    for provider in PROVIDERS {
        let snapshot = latest.get(provider).copied();
        let stale = snapshot.is_some_and(|snapshot| {
            generated_at - snapshot.data_through > Duration::days(PUBLIC_STALE_AFTER_DAYS)
        });
        let available = snapshot.is_some() && !stale;
        ready &= available;
        sources.push(DiscoverabilitySourceStatus {
            provider: provider.to_string(),
            available,
            stale,
            observed_at: snapshot.map(|value| value.observed_at.to_rfc3339()),
            data_through: snapshot.map(|value| value.data_through.to_rfc3339()),
            window_started_at: snapshot.map(|value| value.window_started_at.to_rfc3339()),
            window_ended_at: snapshot.map(|value| value.window_ended_at.to_rfc3339()),
        });
    }
    let search = latest.get("search_console");
    let github = latest.get("github");
    let first_party = latest.get("first_party");
    let interfaces = latest.get("external_interfaces");
    DiscoverabilityPublicSummary {
        schema_version: "agent-bounties/discoverability-summary-v1".to_string(),
        status: if ready { "ready" } else { "unavailable" }.to_string(),
        window_days: 30,
        stale_after_days: PUBLIC_STALE_AFTER_DAYS as u8,
        generated_at: generated_at.to_rfc3339(),
        sources,
        human_reach: HumanReachSummary {
            search_impressions: ready.then(|| total_u64(search, "impressions")).flatten(),
            organic_clicks: ready.then(|| total_u64(search, "clicks")).flatten(),
            google_average_position: ready
                .then(|| total_f64(search, "average_position"))
                .flatten(),
            github_unique_visitors: ready
                .then(|| total_u64(github, "unique_visitors"))
                .flatten(),
            captured_chatgpt_referrals: ready
                .then(|| total_u64(first_party, "captured_chatgpt_referrals"))
                .flatten(),
            opportunity_feed_clicks: ready
                .then(|| total_u64(first_party, "opportunity_feed_clicks"))
                .flatten(),
            market_to_funded_opportunity_ctr: ready
                .then(|| total_f64(first_party, "market_to_funded_opportunity_ctr"))
                .flatten(),
        },
        automation_reach: AutomationReachSummary {
            a2a_interactions: ready.then(|| total_u64(interfaces, "a2a")).flatten(),
            mcp_interactions: ready.then(|| total_u64(interfaces, "mcp")).flatten(),
            api_cli_interactions: ready
                .then(|| {
                    interfaces.is_some().then_some(
                        total_u64(interfaces, "api").unwrap_or(0)
                            .saturating_add(total_u64(interfaces, "cli").unwrap_or(0)),
                    )
                })
                .flatten(),
            feed_interactions: ready.then(|| total_u64(interfaces, "feed")).flatten(),
            github_unique_cloners: ready
                .then(|| total_u64(github, "unique_cloners"))
                .flatten(),
        },
        definitions: vec![
            "Search and browser headlines use rolling 28-day windows; the private Search Console recovery snapshot overlaps 35 days. Interface interactions use 30 days, and GitHub traffic uses GitHub's rolling 14-day window.".to_string(),
            "Interactions are aggregate route observations. They are not unique people or independent agents.".to_string(),
            "GitHub unique visitors and unique cloners are GitHub-measured identities. Clone operations remain an operational volume signal only.".to_string(),
            "Captured ChatGPT referrals require an observed ChatGPT/OpenAI referrer or an explicit tagged ChatGPT link; generic MCP traffic is not inferred as ChatGPT.".to_string(),
        ],
        evidence_boundary: "Discoverability metrics describe measured reach and interactions only. They never prove funding, claimability, verification, settlement, or payment. Only the applicable confirmed canonical settlement event proves solver payment.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(provider: &str, payload: Value) -> DiscoverabilitySnapshotRequest {
        let observed_at = DateTime::parse_from_rfc3339("2026-08-24T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let checksum = hex::encode(Sha256::digest(serde_json::to_vec(&payload).unwrap()));
        DiscoverabilitySnapshotRequest {
            provider: provider.to_string(),
            observed_at,
            window_started_at: observed_at - Duration::days(28),
            window_ended_at: observed_at,
            data_through: observed_at,
            payload_checksum: checksum,
            payload,
        }
    }

    fn stored_snapshot(
        provider: &str,
        data_through: DateTime<Utc>,
        payload: Value,
    ) -> DiscoverabilitySnapshot {
        DiscoverabilitySnapshot {
            provider: provider.to_string(),
            observed_at: data_through,
            window_started_at: data_through - Duration::days(30),
            window_ended_at: data_through,
            data_through,
            payload_checksum: "a".repeat(64),
            payload,
            created_at: data_through,
        }
    }

    fn complete_snapshots(now: DateTime<Utc>) -> Vec<DiscoverabilitySnapshot> {
        vec![
            stored_snapshot(
                "search_console",
                now,
                json!({"totals": {"impressions": 350, "clicks": 5, "average_position": 7.8}}),
            ),
            stored_snapshot(
                "github",
                now,
                json!({"totals": {"unique_visitors": 300, "unique_cloners": 530, "views": 487, "clones": 31808}}),
            ),
            stored_snapshot(
                "first_party",
                now,
                json!({"totals": {"captured_chatgpt_referrals": 38, "opportunity_feed_clicks": 12, "market_views": 100, "funded_bounty_clicks": 6, "market_to_funded_opportunity_ctr": 0.06}}),
            ),
            stored_snapshot(
                "external_interfaces",
                now,
                json!({"totals": {"a2a": 1, "mcp": 2, "api": 3, "cli": 4, "feed": 5}}),
            ),
        ]
    }

    #[test]
    fn validation_rejects_negative_or_malformed_provider_counts() {
        let now = DateTime::parse_from_rfc3339("2026-08-24T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let invalid = request(
            "github",
            json!({"totals": {"unique_visitors": -1, "unique_cloners": 2, "views": 3, "clones": 4}}),
        );
        assert_eq!(
            validate_snapshot(invalid, now),
            Err(StatusCode::BAD_REQUEST)
        );
    }

    #[test]
    fn validation_rejects_checksum_drift_and_future_observations() {
        let now = DateTime::parse_from_rfc3339("2026-08-24T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut invalid = request(
            "search_console",
            json!({"totals": {"impressions": 116, "clicks": 0, "average_position": 7.8}}),
        );
        invalid.payload_checksum = "0".repeat(64);
        assert_eq!(
            validate_snapshot(invalid, now),
            Err(StatusCode::BAD_REQUEST)
        );
        let mut future = request(
            "search_console",
            json!({"totals": {"impressions": 116, "clicks": 0, "average_position": 7.8}}),
        );
        future.observed_at = now + Duration::minutes(6);
        future.window_ended_at = future.observed_at;
        future.data_through = future.observed_at;
        assert_eq!(validate_snapshot(future, now), Err(StatusCode::BAD_REQUEST));
    }

    #[test]
    fn ingestion_rejects_client_supplied_route_snapshots() {
        let now = DateTime::parse_from_rfc3339("2026-08-24T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let external = request(
            "external_interfaces",
            json!({"totals": {"a2a": 1, "mcp": 2, "api": 3, "cli": 4, "feed": 5}}),
        );
        assert_eq!(
            validate_snapshot(external, now),
            Err(StatusCode::BAD_REQUEST)
        );
    }

    #[test]
    fn route_totals_ignore_unknown_interfaces_and_saturate() {
        let now = DateTime::parse_from_rfc3339("2026-08-24T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let rows = vec![
            DiscoveryRouteUsageStats {
                interface: "a2a".to_string(),
                route_family: "agent_card".to_string(),
                attribution_reliability: "observed".to_string(),
                interaction_count: u64::MAX,
                successful_interaction_count: 1,
                first_observed_at: now,
                last_observed_at: now,
            },
            DiscoveryRouteUsageStats {
                interface: "a2a".to_string(),
                route_family: "opportunity_list".to_string(),
                attribution_reliability: "declared".to_string(),
                interaction_count: 3,
                successful_interaction_count: 1,
                first_observed_at: now,
                last_observed_at: now,
            },
            DiscoveryRouteUsageStats {
                interface: "unknown".to_string(),
                route_family: "alerts".to_string(),
                attribution_reliability: "observed".to_string(),
                interaction_count: 100,
                successful_interaction_count: 1,
                first_observed_at: now,
                last_observed_at: now,
            },
        ];
        assert_eq!(route_totals(&rows).get("a2a"), Some(&u64::MAX));
        assert!(!route_totals(&rows).contains_key("unknown"));
    }

    #[test]
    fn coverage_gaps_are_reported_without_interpolation() {
        let now = DateTime::parse_from_rfc3339("2026-08-24T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let older = stored_snapshot("github", now - Duration::days(20), json!({}));
        let newer = DiscoverabilitySnapshot {
            provider: "github".to_string(),
            observed_at: now,
            window_started_at: now - Duration::days(14),
            window_ended_at: now,
            data_through: now,
            payload_checksum: "b".repeat(64),
            payload: json!({}),
            created_at: now,
        };
        let gaps = coverage_gaps(&[older, newer]);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].provider, "github");
        assert_eq!(gaps[0].gap_days, 6);
        assert!(gaps[0].long_gap);
    }

    #[test]
    fn public_fields_are_an_explicit_aggregate_allowlist() {
        let serialized = serde_json::to_value(DiscoverabilityPublicSummary {
            schema_version: "test".to_string(),
            status: "unavailable".to_string(),
            window_days: 30,
            stale_after_days: 9,
            generated_at: "2026-08-24T12:00:00Z".to_string(),
            sources: Vec::new(),
            human_reach: HumanReachSummary {
                search_impressions: None,
                organic_clicks: None,
                google_average_position: None,
                github_unique_visitors: None,
                captured_chatgpt_referrals: None,
                opportunity_feed_clicks: None,
                market_to_funded_opportunity_ctr: None,
            },
            automation_reach: AutomationReachSummary {
                a2a_interactions: None,
                mcp_interactions: None,
                api_cli_interactions: None,
                feed_interactions: None,
                github_unique_cloners: None,
            },
            definitions: Vec::new(),
            evidence_boundary: "test".to_string(),
        })
        .unwrap()
        .to_string();
        for forbidden in ["payload", "query", "path", "referrer", "raw"] {
            assert!(!serialized.contains(forbidden));
        }
    }

    #[test]
    fn public_summary_requires_every_fresh_provider_and_withholds_partial_values() {
        let now = DateTime::parse_from_rfc3339("2026-08-24T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let complete = complete_snapshots(now);
        let ready = build_public_summary(&complete, now);
        assert_eq!(ready.status, "ready");
        assert_eq!(ready.human_reach.search_impressions, Some(350));
        assert_eq!(ready.automation_reach.api_cli_interactions, Some(7));

        let missing = build_public_summary(&complete[..3], now);
        assert_eq!(missing.status, "unavailable");
        assert_eq!(missing.human_reach.search_impressions, None);
        assert_eq!(missing.automation_reach.github_unique_cloners, None);

        let mut stale = complete;
        stale[0].data_through = now - Duration::days(10);
        let unavailable = build_public_summary(&stale, now);
        assert_eq!(unavailable.status, "unavailable");
        assert!(unavailable.sources.iter().any(|source| source.stale));
        assert_eq!(unavailable.human_reach.organic_clicks, None);
    }
}
