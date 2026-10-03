//! Bounded public replay of the same historical selection used by platform metrics.
use super::*;
use db::{platform_payout_snapshot_hash, PlatformPayoutProofRow, PLATFORM_PAYOUT_PROOF_MAX_ROWS};
use sha2::{Digest, Sha256};

const MAX_PAGE_SIZE: usize = 200;
const MAX_RESPONSE_BYTES: usize = 256 * 1024;
const PROOF_PATH: &str = "/v1/metrics/platform/payouts";

#[derive(Debug, Default, Deserialize)]
pub(super) struct PayoutQuery {
    period: Option<String>,
    as_of: Option<String>,
    limit: Option<usize>,
    cursor: Option<String>,
    snapshot: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct PayoutProofMetadata {
    status: String,
    url: Option<String>,
    snapshot: Option<String>,
    policy_hash: String,
    max_rows: usize,
    max_page_size: usize,
    max_response_bytes: usize,
    evidence_boundary: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct PayoutProofRecord {
    protocol: String,
    network: String,
    factory_contract: Option<String>,
    contract_address: String,
    bounty_id: String,
    tx_hash: String,
    block_number: String,
    log_index: String,
    kind: String,
    occurred_at: String,
    is_settlement: bool,
    solver_base_units: String,
    verifier_base_units: String,
    keeper_base_units: String,
    bonus_base_units: String,
    total_base_units: String,
}

impl From<PlatformPayoutProofRow> for PayoutProofRecord {
    fn from(row: PlatformPayoutProofRow) -> Self {
        Self {
            protocol: row.protocol,
            network: row.network,
            factory_contract: row.factory_contract,
            contract_address: row.contract_address,
            bounty_id: row.bounty_id,
            tx_hash: row.tx_hash,
            block_number: row.block_number.to_string(),
            log_index: row.log_index.to_string(),
            kind: row.kind,
            occurred_at: row.occurred_at.to_rfc3339(),
            is_settlement: row.is_settlement,
            solver_base_units: row.solver_base_units,
            verifier_base_units: row.verifier_base_units,
            keeper_base_units: row.keeper_base_units,
            bonus_base_units: row.bonus_base_units,
            total_base_units: row.total_base_units,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct PayoutProofResponse {
    schema_version: String,
    network: String,
    period: String,
    started_at: String,
    ended_at: String,
    generated_at: String,
    snapshot: String,
    policy_hash: String,
    excluded_bounty_contracts: Vec<String>,
    excluded_rows_enumerated: bool,
    total_rows: usize,
    offset: usize,
    next_cursor: Option<String>,
    complete: bool,
    records: Vec<PayoutProofRecord>,
    coverage: String,
    evidence_boundary: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct PayoutProofError {
    code: String,
    restart_required: bool,
}

type ProofError = (StatusCode, Json<PayoutProofError>);
fn error(status: StatusCode, code: &str) -> ProofError {
    (
        status,
        Json(PayoutProofError {
            code: code.into(),
            restart_required: status == StatusCode::CONFLICT,
        }),
    )
}

fn hash(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

fn policy_hash() -> String {
    // Includes every declared policy field, not only payout exclusions.
    hash(PUBLIC_METRICS_POLICY_JSON.as_bytes())
}

pub(super) fn cutoff(value: Option<&str>, now: DateTime<Utc>) -> Result<DateTime<Utc>, StatusCode> {
    let cutoff = match value {
        Some(value) if value.len() <= 64 => DateTime::parse_from_rfc3339(value)
            .map(|time| time.with_timezone(&Utc))
            .map_err(|_| StatusCode::BAD_REQUEST)?,
        Some(_) => return Err(StatusCode::BAD_REQUEST),
        None => DateTime::from_timestamp(now.timestamp(), 0).ok_or(StatusCode::BAD_REQUEST)?,
    };
    if cutoff < parse_public_metrics_timestamp(PLATFORM_LAUNCH_AT)?
        || cutoff > now
        || cutoff.timestamp_subsec_nanos() % 1_000 != 0
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(cutoff)
}

pub(super) fn metadata(
    window: &PlatformMetricWindow,
    snapshot: Option<String>,
) -> PayoutProofMetadata {
    let url = snapshot.as_ref().map(|snapshot| {
        let params = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("period", &window.period)
            .append_pair("as_of", &window.ended_at.to_rfc3339())
            .append_pair("snapshot", snapshot)
            .finish();
        format!("{PROOF_PATH}?{params}")
    });
    PayoutProofMetadata {
        status: if snapshot.is_some() { "available" } else { "capacity_exceeded" }.into(),
        url, snapshot, policy_hash: policy_hash(), max_rows: PLATFORM_PAYOUT_PROOF_MAX_ROWS,
        max_page_size: MAX_PAGE_SIZE, max_response_bytes: MAX_RESPONSE_BYTES,
        evidence_boundary: "Historical indexed selection, including older factories; same window, exclusions and database snapshot as selected headline/day totals. A matching sum does not establish chain freshness, identity or independent participation. Changed data or policy requires restart. Returned bonds and creator awards are excluded.".into(),
    }
}

fn scope_hash(window: &PlatformMetricWindow, policy: &str, snapshot: &str) -> String {
    hash(
        format!(
            "historical-payout-cursor-v1\n{}\n{}\n{}\n{policy}\n{snapshot}",
            window.period,
            window.started_at.to_rfc3339(),
            window.ended_at.to_rfc3339()
        )
        .as_bytes(),
    )
}

fn cursor_offset(cursor: Option<&str>, scope: &str, total: usize) -> Result<usize, ProofError> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    if cursor.len() > 128 {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_cursor"));
    }
    let parts: Vec<_> = cursor.splitn(3, ':').collect();
    if parts.len() != 3 || parts[0] != "v1" {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_cursor"));
    }
    let offset = parts[1]
        .parse::<usize>()
        .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid_cursor"))?;
    if parts[2] != scope {
        return Err(error(StatusCode::CONFLICT, "snapshot_changed"));
    }
    if offset == 0 || offset >= total {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_cursor"));
    }
    Ok(offset)
}

fn valid_hex(value: &str, bytes: usize) -> bool {
    value.len() == 2 + bytes * 2
        && value.starts_with("0x")
        && value[2..].bytes().all(|b| b.is_ascii_hexdigit())
}

fn validate_rows(
    rows: &[PlatformPayoutProofRow],
    window: &PlatformMetricWindow,
) -> Result<(), ProofError> {
    let mut seen = std::collections::HashSet::new();
    for row in rows {
        let expected_settlement = match (row.protocol.as_str(), row.kind.as_str()) {
            ("autonomous-v1" | "open-competition-v1", "bounty_settled")
            | ("open-competition-v2", "competition_settled") => true,
            ("autonomous-v1", "submission_rejected")
            | ("open-competition-v1", "competition_submission_rejected") => false,
            _ => {
                return Err(error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "invalid_indexed_record",
                ))
            }
        };
        if row.network != "base-mainnet"
            || row.block_number < 0
            || row.log_index < 0
            || !valid_hex(&row.contract_address, 20)
            || !valid_hex(&row.bounty_id, 32)
            || !valid_hex(&row.tx_hash, 32)
            || row
                .factory_contract
                .as_ref()
                .is_some_and(|factory| !valid_hex(factory, 20))
            || (row.protocol != "autonomous-v1" && row.factory_contract.is_none())
            || row.occurred_at < window.started_at
            || row.occurred_at >= window.ended_at
            || row.is_settlement != expected_settlement
            || !seen.insert((row.tx_hash.to_ascii_lowercase(), row.log_index))
        {
            return Err(error(
                StatusCode::SERVICE_UNAVAILABLE,
                "invalid_indexed_record",
            ));
        }
        let amounts = [
            &row.solver_base_units,
            &row.verifier_base_units,
            &row.keeper_base_units,
            &row.bonus_base_units,
            &row.total_base_units,
        ];
        let parsed = amounts
            .iter()
            .map(|value| {
                if value.is_empty()
                    || value.len() > 39
                    || !value.bytes().all(|b| b.is_ascii_digit())
                {
                    return Err(error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "invalid_indexed_amount",
                    ));
                }
                value
                    .parse::<u128>()
                    .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "invalid_indexed_amount"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if parsed[..4]
            .iter()
            .try_fold(0_u128, |sum, value| sum.checked_add(*value))
            != Some(parsed[4])
        {
            return Err(error(
                StatusCode::SERVICE_UNAVAILABLE,
                "invalid_indexed_amount",
            ));
        }
    }
    Ok(())
}

fn page(
    rows: Vec<PlatformPayoutProofRow>,
    window: &PlatformMetricWindow,
    policy: &PublicMetricsPolicy,
    query: &PayoutQuery,
) -> Result<PayoutProofResponse, ProofError> {
    if rows.len() > PLATFORM_PAYOUT_PROOF_MAX_ROWS {
        return Err(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "proof_capacity_exceeded",
        ));
    }
    let limit = query.limit.unwrap_or(100);
    if limit == 0 || limit > MAX_PAGE_SIZE {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_limit"));
    }
    validate_rows(&rows, window)?;
    let excluded = historical_platform_metric_exclusions(policy);
    let snapshot = platform_payout_snapshot_hash(
        &rows,
        "base-mainnet",
        window.started_at,
        window.ended_at,
        &excluded,
    )
    .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "snapshot_unavailable"))?;
    if query
        .snapshot
        .as_ref()
        .is_some_and(|expected| expected != &snapshot)
    {
        return Err(error(StatusCode::CONFLICT, "snapshot_changed"));
    }
    let policy_hash = policy_hash();
    let scope = scope_hash(window, &policy_hash, &snapshot);
    let offset = cursor_offset(query.cursor.as_deref(), &scope, rows.len())?;
    let end = (offset + limit).min(rows.len());
    let total_rows = rows.len();
    Ok(PayoutProofResponse {
        schema_version: "agent-bounties/platform-payout-proof-v1".into(), network: "base-mainnet".into(),
        period: window.period.clone(), started_at: window.started_at.to_rfc3339(), ended_at: window.ended_at.to_rfc3339(),
        generated_at: Utc::now().to_rfc3339(), snapshot, policy_hash, excluded_bounty_contracts: excluded,
        excluded_rows_enumerated: false, total_rows, offset,
        next_cursor: (end < total_rows).then(|| format!("v1:{end}:{scope}")), complete: end == total_rows,
        records: rows.into_iter().skip(offset).take(limit).map(Into::into).collect(),
        coverage: "indexed_history_only".into(),
        evidence_boundary: "Only the bounded indexed selection is enumerated; complete means final page, not lifetime or chain completeness. Check aggregate coverage independently. Legacy block times are verified; V2 uses its existing canonical safe-block index. Autonomous historical factory is unavailable in this projection. Policy-excluded contracts are listed, but their rows/counts are not enumerated. No participant identities, returned bonds, creator awards or financial authority.".into(),
    })
}

#[utoipa::path(get, path = "/v1/metrics/platform/payouts",
    params(("period" = Option<String>, Query, description = "7d,28d,90d,lifetime; default7d"),
        ("as_of" = Option<String>, Query, description = "RFC3339 exclusive cutoff; required with cursor; use aggregate window.ended_at"),
        ("limit" = Option<usize>, Query, description = "1..200; default100; total cap5000"),
        ("snapshot" = Option<String>, Query, description = "Expected aggregate payout_proof.snapshot; mismatch returns409"),
        ("cursor" = Option<String>, Query, description = "Opaque continuation bound to the exact window, data and policy")),
    responses((status = 200, body = PayoutProofResponse), (status = 400, body = PayoutProofError),
        (status = 409, body = PayoutProofError), (status = 503, body = PayoutProofError))) ]
pub(super) async fn platform_payouts(
    State(state): State<SharedState>,
    Query(query): Query<PayoutQuery>,
) -> Result<Response, ProofError> {
    if query.cursor.is_some() && query.as_of.is_none() {
        return Err(error(StatusCode::BAD_REQUEST, "cursor_requires_as_of"));
    }
    if query
        .cursor
        .as_ref()
        .is_some_and(|cursor| cursor.len() > 128)
        || query
            .limit
            .is_some_and(|limit| limit == 0 || limit > MAX_PAGE_SIZE)
        || query
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.len() > 71)
    {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_limit_or_snapshot"));
    }
    let ended_at = cutoff(query.as_of.as_deref(), Utc::now())
        .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid_as_of"))?;
    let window = platform_metric_window(query.period.as_deref(), ended_at)
        .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid_period"))?;
    let policy = public_metrics_policy()
        .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "policy_unavailable"))?;
    let store = state
        .store
        .as_ref()
        .ok_or_else(|| error(StatusCode::SERVICE_UNAVAILABLE, "store_unavailable"))?;
    let rows = store
        .platform_payout_proof_rows(
            "base-mainnet",
            window.started_at,
            window.ended_at,
            &historical_platform_metric_exclusions(&policy),
        )
        .await
        .map_err(|err| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                if matches!(err, db::DbError::PayoutProofCapacity) {
                    "proof_capacity_exceeded"
                } else {
                    "proof_unavailable"
                },
            )
        })?;
    let response = page(rows, &window, &policy, &query)?;
    let bytes = serde_json::to_vec(&response)
        .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "proof_unavailable"))?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "proof_response_too_large",
        ));
    }
    Ok((
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::CONTENT_TYPE, "application/json"),
        ],
        bytes,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn window() -> PlatformMetricWindow {
        platform_metric_window(Some("lifetime"), "2026-10-01T00:00:00Z".parse().unwrap()).unwrap()
    }
    fn row(log_index: i64) -> PlatformPayoutProofRow {
        PlatformPayoutProofRow {
            protocol: "open-competition-v2".into(),
            network: "base-mainnet".into(),
            factory_contract: Some(format!("0x{}", "11".repeat(20))),
            contract_address: format!("0x{}", "22".repeat(20)),
            bounty_id: format!("0x{}", "33".repeat(32)),
            tx_hash: format!("0x{}", "44".repeat(32)),
            block_number: 123,
            log_index,
            kind: "competition_settled".into(),
            occurred_at: "2026-08-20T00:00:00Z".parse().unwrap(),
            is_settlement: true,
            solver_base_units: "9007199254740993".into(),
            verifier_base_units: "0".into(),
            keeper_base_units: "7".into(),
            bonus_base_units: "0".into(),
            total_base_units: "9007199254741000".into(),
        }
    }
    #[test]
    fn platform_payout_pages_replay_exact_strings_without_duplicates() {
        let rows = vec![row(0), row(1), row(2)];
        let policy = public_metrics_policy().unwrap();
        let first = page(
            rows.clone(),
            &window(),
            &policy,
            &PayoutQuery {
                limit: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(first.records[0].solver_base_units, "9007199254740993");
        assert_eq!(first.total_rows, 3);
        assert!(!first.complete);
        let second = page(
            rows,
            &window(),
            &policy,
            &PayoutQuery {
                limit: Some(2),
                cursor: first.next_cursor,
                snapshot: Some(first.snapshot),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(second.offset, 2);
        assert_eq!(second.records.len(), 1);
        assert_eq!(second.records[0].log_index, "2");
        assert!(second.complete);
        assert!(second.next_cursor.is_none());
        assert!(!second.excluded_rows_enumerated);
    }
    #[test]
    fn platform_payout_cursor_requires_restart_after_revision_or_scope_change() {
        let rows = vec![row(0), row(1)];
        let policy = public_metrics_policy().unwrap();
        let first = page(
            rows.clone(),
            &window(),
            &policy,
            &PayoutQuery {
                limit: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        let query = PayoutQuery {
            cursor: first.next_cursor.clone(),
            ..Default::default()
        };
        let mut added = rows.clone();
        added.push(row(2));
        assert_eq!(
            page(added, &window(), &policy, &query).unwrap_err().0,
            StatusCode::CONFLICT
        );
        let mut changed = rows.clone();
        changed[0].solver_base_units = "1".into();
        changed[0].total_base_units = "8".into();
        assert_eq!(
            page(changed, &window(), &policy, &query).unwrap_err().0,
            StatusCode::CONFLICT
        );
        let mut other_window = window();
        other_window.ended_at += ChronoDuration::seconds(1);
        assert_eq!(
            page(rows.clone(), &other_window, &policy, &query)
                .unwrap_err()
                .0,
            StatusCode::CONFLICT
        );
        let different_policy = scope_hash(&window(), "different-policy", &first.snapshot);
        assert_eq!(
            cursor_offset(first.next_cursor.as_deref(), &different_policy, 2)
                .unwrap_err()
                .0,
            StatusCode::CONFLICT
        );
        let query = PayoutQuery {
            snapshot: Some("sha256:obsolete".into()),
            ..Default::default()
        };
        assert_eq!(
            page(rows, &window(), &policy, &query).unwrap_err().0,
            StatusCode::CONFLICT
        );
    }
    #[test]
    fn platform_payout_rejects_conflicting_logs_invalid_amounts_and_references() {
        let policy = public_metrics_policy().unwrap();
        assert!(page(
            vec![row(0), row(0)],
            &window(),
            &policy,
            &PayoutQuery::default()
        )
        .is_err());
        for field in [
            "amount",
            "sum",
            "network",
            "transaction",
            "timestamp",
            "settlement",
            "factory",
        ] {
            let mut bad = row(0);
            match field {
                "amount" => bad.solver_base_units = "-1".into(),
                "sum" => bad.total_base_units = "2".into(),
                "network" => bad.network = "base-sepolia".into(),
                "transaction" => bad.tx_hash = "javascript:alert(1)".into(),
                "timestamp" => bad.occurred_at = window().ended_at,
                "settlement" => bad.is_settlement = false,
                "factory" => bad.factory_contract = None,
                _ => unreachable!(),
            }
            assert!(
                page(vec![bad], &window(), &policy, &PayoutQuery::default()).is_err(),
                "{field}"
            );
        }
    }
    #[test]
    fn platform_payout_empty_capacity_and_cursor_errors_are_explicit() {
        let policy = public_metrics_policy().unwrap();
        let empty = page(vec![], &window(), &policy, &PayoutQuery::default()).unwrap();
        assert_eq!(empty.total_rows, 0);
        assert!(empty.complete);
        for limit in [0, 201, usize::MAX] {
            assert_eq!(
                page(
                    vec![],
                    &window(),
                    &policy,
                    &PayoutQuery {
                        limit: Some(limit),
                        ..Default::default()
                    }
                )
                .unwrap_err()
                .0,
                StatusCode::BAD_REQUEST
            );
        }
        assert_eq!(
            page(
                vec![row(0); 5001],
                &window(),
                &policy,
                &PayoutQuery::default()
            )
            .unwrap_err()
            .1
             .0
            .code,
            "proof_capacity_exceeded"
        );
        for cursor in ["garbage", "v1:18446744073709551616:x", "v2:1:x"] {
            assert_eq!(
                cursor_offset(Some(cursor), "x", 2).unwrap_err().0,
                StatusCode::BAD_REQUEST
            );
        }
        assert_eq!(
            cursor_offset(Some("v1:0:x"), "x", 2).unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            cursor_offset(Some("v1:2:x"), "x", 2).unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
    }
    #[test]
    fn platform_payout_cutoff_is_bounded_and_metadata_carries_selection() {
        let now = window().ended_at;
        for value in [
            "not-a-date",
            "2026-10-02T00:00:00Z",
            "2020-01-01T00:00:00Z",
            "2026-09-30T00:00:00.000000001Z",
        ] {
            assert_eq!(cutoff(Some(value), now), Err(StatusCode::BAD_REQUEST));
        }
        assert_eq!(cutoff(None, now).unwrap(), now);
        let metadata = metadata(&window(), Some("sha256:example".into()));
        assert!(metadata.url.unwrap().contains("snapshot=sha256%3Aexample"));
        assert_eq!(super::metadata(&window(), None).status, "capacity_exceeded");
    }
    #[tokio::test]
    #[ignore = "requires disposable PostgreSQL in AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn platform_payout_http_replays_real_database_selection() {
        use chain_base::{OpenCompetitionV2Event, OpenCompetitionV2EventKind};
        use db::OpenCompetitionV2SafeContext;
        use tower::ServiceExt;
        let store =
            PostgresStore::connect(&std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap())
                .await
                .unwrap();
        store.migrate().await.unwrap();
        for number in [901_u64, 902] {
            store.upsert_open_competition_v2_event("base-mainnet", &format!("0x{number:040x}"),
                &OpenCompetitionV2Event {
                    id: Uuid::from_u128(0x7061_796f_7574_7072_6f6f_6600_0000_0000 + u128::from(number)), protocol_version: chain_base::OPEN_COMPETITION_V2_PROTOCOL_VERSION.into(),
                    log_key: format!("0x{number:064x}:0"), tx_hash: format!("0x{number:064x}"),
                    block_number: number, log_index: 0, contract_address: format!("0x{:040x}", number + 10),
                    bounty_id: format!("0x{number:064x}"), kind: OpenCompetitionV2EventKind::CompetitionSettled,
                    data: serde_json::json!({"solver_reward":"3000000", "keeper_reward":"40000"}),
                    occurred_at: "2026-09-03T12:00:00Z".parse().unwrap(),
                }, &OpenCompetitionV2SafeContext { block_hash: format!("0x{number:064x}"),
                    safe_block_number: 1000, safe_block_hash: format!("0x{:064x}", 1000) }).await.unwrap();
        }
        let state = crate::tests::test_state_with_operator_token_and_store(
            BountyNetwork::default(),
            "local-fixture-only",
            store,
        );
        let app = Router::new()
            .route("/v1/metrics/platform", get(platform_metrics))
            .route(PROOF_PATH, get(platform_payouts))
            .with_state(state);
        let request = |uri: String| {
            axum::http::Request::builder()
                .uri(uri)
                .body(axum::body::Body::empty())
                .unwrap()
        };
        let response = app
            .clone()
            .oneshot(request(
                "/v1/metrics/platform?period=7d&as_of=2026-09-04T00%3A00%3A00Z".into(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let aggregate: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), MAX_RESPONSE_BYTES)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            aggregate["marketplace_payout_volume"]["selected"]["usdc_base_units"],
            "6080000"
        );
        let url = aggregate["payout_proof"]["url"].as_str().unwrap();
        let response = app
            .clone()
            .oneshot(request(format!("{url}&limit=1")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let first: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), MAX_RESPONSE_BYTES)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(first["snapshot"], aggregate["payout_proof"]["snapshot"]);
        assert_eq!(first["total_rows"], 2);
        let cursor: String =
            url::form_urlencoded::byte_serialize(first["next_cursor"].as_str().unwrap().as_bytes())
                .collect();
        let response = app
            .clone()
            .oneshot(request(format!("{url}&limit=1&cursor={cursor}")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let second: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), MAX_RESPONSE_BYTES)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(second["complete"], true);
        assert_ne!(
            first["records"][0]["factory_contract"],
            second["records"][0]["factory_contract"]
        );
        for (query, status) in [
            ("limit=0", StatusCode::BAD_REQUEST),
            ("as_of=garbage", StatusCode::BAD_REQUEST),
            ("cursor=bogus", StatusCode::BAD_REQUEST),
            ("snapshot=obsolete", StatusCode::CONFLICT),
        ] {
            let response = app
                .clone()
                .oneshot(request(format!("{PROOF_PATH}?{query}")))
                .await
                .unwrap();
            assert_eq!(response.status(), status);
        }
    }
}
