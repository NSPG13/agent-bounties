//! Public operational readiness, separate from process liveness and earning eligibility.
//! No event, wallet, customer, provider URL or raw error is exposed here.
use super::*;

#[cfg(test)]
#[path = "marketplace_readiness_diagnostic_tests.rs"]
mod diagnostic_tests;

// Diagnostic detail only: the accepted interval remains exactly 0..=300 seconds.
fn checked_head_timestamp(timestamp: u64, now: i64) -> Result<i64, FailureClass> {
    let timestamp = i64::try_from(timestamp).map_err(|_| FailureClass::TimeOutOfRange)?;
    let age = now
        .checked_sub(timestamp)
        .ok_or(FailureClass::TimeOutOfRange)?;
    if age < 0 {
        Err(FailureClass::FutureTime)
    } else if age > 300 {
        Err(FailureClass::StaleTime)
    } else {
        Ok(timestamp)
    }
}

fn observed_head_time_failure(
    safe_time: i64,
    latest_time: i64,
    now: i64,
) -> Option<(FailureStage, FailureClass)> {
    for (timestamp, stage) in [
        (safe_time, FailureStage::HeadSafeValidation),
        (latest_time, FailureStage::HeadLatestValidation),
    ] {
        // These times came from successful u64-to-i64 header parsing. Keep
        // unexpected signed inputs fail-closed rather than casting them.
        let checked = u64::try_from(timestamp)
            .map_err(|_| FailureClass::TimeOutOfRange)
            .and_then(|value| checked_head_timestamp(value, now));
        if let Err(class) = checked {
            return Some((stage, class));
        }
    }
    None
}

// Diagnostic progress only. It never extends a deadline or authorizes stale data.
fn observation_timeout_stage(heads_started: bool) -> FailureStage {
    if heads_started {
        FailureStage::MarketplaceHeadsTimedOut
    } else {
        FailureStage::MarketplaceSnapshotTimedOut
    }
}

fn reader_status(
    protocol: &str,
    heartbeat: Option<&BaseIndexerHeartbeat>,
    durable_cursor: Option<u64>,
    safe_block: Option<u64>,
    latest_block: Option<u64>,
    now: DateTime<Utc>,
) -> serde_json::Value {
    let mut result = serde_json::json!({
        "protocol": protocol, "ready": false, "blocker": "heartbeat_missing",
        "heartbeat_age_seconds": null, "cursor_block": durable_cursor,
        "lag_blocks": safe_block.zip(durable_cursor).map(|(safe, cursor)| safe.saturating_sub(cursor))
    });
    let Some(heartbeat) = heartbeat else {
        return result;
    };
    result["heartbeat_age_seconds"] = serde_json::json!(heartbeat
        .completed_at
        .map(|at| now.signed_duration_since(at).num_seconds()));
    // Beta3 may have no new safe blocks while its reader is current. The V1
    // helper recognizes only its own skip wording; normalize only this exact,
    // reviewed Beta3 condition, never an arbitrary skipped poll.
    let mut checked = heartbeat.clone();
    if protocol == "agent-bounties/open-competition-v2-beta3"
        && checked.status == "skipped"
        && checked.skipped_reason.as_deref() == Some("no safe blocks are ready to scan")
    {
        checked.skipped_reason = Some("no confirmed blocks are ready to scan".into());
    }
    let blocker = if heartbeat.status == "failed" || heartbeat.error_message.is_some() {
        Some("reader_failed")
    } else if safe_block.is_none() {
        Some("safe_head_unavailable")
    } else if latest_block.is_none() {
        Some("latest_head_unavailable")
    } else if safe_block
        .zip(latest_block)
        .is_some_and(|(safe, latest)| safe > latest)
    {
        Some("head_order_inconsistent")
    } else if durable_cursor.is_none() {
        Some("durable_cursor_unavailable")
    } else if heartbeat
        .persisted_cursor_block
        .is_none_or(|cursor| durable_cursor.unwrap() < cursor)
    {
        Some("cursor_inconsistent")
    } else if durable_cursor
        .zip(latest_block)
        .is_some_and(|(cursor, latest)| cursor > latest)
    {
        Some("cursor_ahead_of_latest_head")
    } else if heartbeat.completed_at.is_some_and(|at| at > now) {
        Some("heartbeat_in_future")
    } else if !open_competition_monitoring_is_fresh(&checked, safe_block.unwrap(), now) {
        Some("reader_stale_or_behind")
    } else if heartbeat
        .latest_block
        .zip(heartbeat.persisted_cursor_block)
        .is_none_or(|(latest, cursor)| cursor > latest)
    {
        Some("cursor_ahead_of_observed_head")
    } else {
        None
    };
    result["ready"] = serde_json::json!(blocker.is_none());
    result["blocker"] = serde_json::json!(blocker);
    result
}

async fn safe_head_for_chain(rpc: &str, chain_id: u64) -> Option<(u64, i64)> {
    if chain_base::fetch_chain_id(rpc, 92)
        .await
        .inspect_err(|error| {
            record_classified_failure(
                FailureStage::HeadChainIdRead,
                chain_failure_class(error),
                &deployment_revision(),
            );
        })
        .ok()?
        != chain_id
    {
        record_classified_failure(
            FailureStage::HeadChainIdRead,
            FailureClass::WrongChain,
            &deployment_revision(),
        );
        return None;
    }
    let block = fetch_safe_block_identity(rpc, 93)
        .await
        .inspect_err(|error| {
            record_classified_failure(
                FailureStage::HeadSafeBlockRead,
                chain_failure_class(error),
                &deployment_revision(),
            );
        })
        .ok()?;
    let timestamp = checked_head_timestamp(block.timestamp, Utc::now().timestamp())
        .inspect_err(|class| {
            record_classified_failure(
                FailureStage::HeadSafeValidation,
                *class,
                &deployment_revision(),
            );
        })
        .ok()?;
    Some((block.number, timestamp))
}

#[cfg(test)]
async fn base_safe_head(rpc: &str) -> Option<u64> {
    safe_head_for_chain(rpc, 8453)
        .await
        .map(|(number, _)| number)
}

/// Independent heads sampled after the reader snapshots. A fresh safe tag may
/// lag any reader's prior checkpoint, including Beta3 on another provider.
/// Latest is the upper bound; safe still bounds projection data and cursor lag.
async fn observed_heads_for_chain(rpc: &str, chain_id: u64) -> Option<(u64, i64, u64)> {
    let (safe, safe_time) = safe_head_for_chain(rpc, chain_id).await?;
    let latest_number = chain_base::fetch_block_number(rpc, 94)
        .await
        .inspect_err(|error| {
            record_classified_failure(
                FailureStage::HeadLatestNumberRead,
                chain_failure_class(error),
                &deployment_revision(),
            );
        })
        .ok()?;
    let latest = chain_base::fetch_exact_block_identity(rpc, latest_number, 95)
        .await
        .inspect_err(|error| {
            record_classified_failure(
                FailureStage::HeadLatestBlockRead,
                chain_failure_class(error),
                &deployment_revision(),
            );
        })
        .ok()?;
    let latest_time = i64::try_from(latest.timestamp)
        .inspect_err(|_| {
            record_classified_failure(
                FailureStage::HeadLatestValidation,
                FailureClass::TimeOutOfRange,
                &deployment_revision(),
            );
        })
        .ok()?;
    let now = Utc::now().timestamp();
    if latest.number != latest_number || safe > latest.number || safe_time > latest_time {
        record_classified_failure(
            FailureStage::HeadLatestValidation,
            FailureClass::InvalidOrder,
            &deployment_revision(),
        );
        return None;
    }
    if let Some((stage, class)) = observed_head_time_failure(safe_time, latest_time, now) {
        record_classified_failure(stage, class, &deployment_revision());
        return None;
    }
    Some((safe, safe_time, latest.number))
}

pub(super) struct ProjectionReadBoundary {
    pub block_number: u64,
    pub safe_block_number: u64,
    expires_at: DateTime<Utc>,
}
pub(super) struct CheckedProjection<T> {
    value: T,
    boundary: ProjectionReadBoundary,
}
impl<T> CheckedProjection<T> {
    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> CheckedProjection<U> {
        CheckedProjection {
            value: map(self.value),
            boundary: self.boundary,
        }
    }
    pub fn into_current(self) -> Result<T, StatusCode> {
        self.boundary.ensure_current()?;
        Ok(self.value)
    }
}

impl<T> CheckedProjection<Vec<T>> {
    pub fn concat(projections: Vec<Self>) -> Result<Self, StatusCode> {
        let mut projections = projections.into_iter();
        let mut combined = projections.next().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        for projection in projections {
            // Preserve the shortest validity window of every included reader.
            combined.boundary.expires_at = combined
                .boundary
                .expires_at
                .min(projection.boundary.expires_at);
            combined.value.extend(projection.value);
        }
        combined.boundary.ensure_current()?;
        Ok(combined)
    }
}

impl ProjectionReadBoundary {
    pub fn bind<T>(self, value: T) -> CheckedProjection<T> {
        CheckedProjection {
            value,
            boundary: self,
        }
    }
    pub fn ensure_current(&self) -> Result<(), StatusCode> {
        if Utc::now() <= self.expires_at {
            Ok(())
        } else {
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

fn projection_boundary(
    protocol: &str,
    heartbeat: Option<&BaseIndexerHeartbeat>,
    cursor: Option<u64>,
    safe: Option<(u64, i64)>,
    latest: Option<u64>,
    now: DateTime<Utc>,
) -> Result<ProjectionReadBoundary, StatusCode> {
    let status = reader_status(protocol, heartbeat, cursor, safe.map(|s| s.0), latest, now);
    if status["ready"] != true {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let (safe_block, safe_time) = safe.ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let completed = heartbeat
        .and_then(|h| h.completed_at)
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let safe_expires = DateTime::<Utc>::from_timestamp(safe_time, 0)
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        + chrono::Duration::seconds(300);
    if !(0..=300).contains(&(now.timestamp() - safe_time)) {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    Ok(ProjectionReadBoundary {
        block_number: safe_block.min(cursor.ok_or(StatusCode::SERVICE_UNAVAILABLE)?),
        safe_block_number: safe_block,
        expires_at: (completed + chrono::Duration::seconds(90)).min(safe_expires),
    })
}

/// Check only this protocol, never a sibling. Capture the heartbeat before its
/// durable cursor and before data reads; cap those later reads to this boundary.
pub(super) async fn require_current_reader(
    state: &SharedState,
    network: &str,
    protocol: &str,
    factory: &str,
) -> Result<ProjectionReadBoundary, StatusCode> {
    let heads_started = std::sync::atomic::AtomicBool::new(false);
    tokio::time::timeout(Duration::from_secs(12), async {
        let store = state
            .store
            .as_ref()
            .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        let heartbeat = store
            .get_base_indexer_heartbeat(network, factory)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let cursor = store
            .get_base_log_cursor(network, factory)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let (descriptor, rpc) = state
            .marketplace_head_rpc
            .resolve(&state.base_rpc_urls, network)
            .map_err(|_| {
                record_failure_stage(FailureStage::HeadRpcNotConfigured, &deployment_revision());
                StatusCode::SERVICE_UNAVAILABLE
            })?;
        heads_started.store(true, std::sync::atomic::Ordering::Relaxed);
        let heads = observed_heads_for_chain(&rpc, descriptor.chain_id).await;
        projection_boundary(
            protocol,
            heartbeat.as_ref(),
            cursor.map(|c| c.last_scanned_block),
            heads.map(|(safe, time, _)| (safe, time)),
            heads.map(|(_, _, latest)| latest),
            Utc::now(),
        )
    })
    .await
    .map_err(|_| {
        record_failure_stage(
            observation_timeout_stage(heads_started.load(std::sync::atomic::Ordering::Relaxed)),
            &deployment_revision(),
        );
        StatusCode::SERVICE_UNAVAILABLE
    })?
}

struct ReaderSnapshot {
    protocol: &'static str,
    factory: Option<String>,
    heartbeat: Option<BaseIndexerHeartbeat>,
    cursor: Option<u64>,
}

/// One request's captured DB/head evidence, never a cross-request success cache.
/// Each boundary remains bound to its exact protocol, factory and network.
pub(super) struct ProjectionObservation {
    network: String,
    boundaries: Vec<ObservedReader>,
}
struct ObservedReader {
    protocol: &'static str,
    factory: Option<String>,
    boundary: Result<ProjectionReadBoundary, StatusCode>,
}
impl ProjectionObservation {
    pub fn take(
        &mut self,
        network: &str,
        protocol: &str,
        factory: &str,
    ) -> Result<ProjectionReadBoundary, StatusCode> {
        let network = service_runtime::bounty_read_inputs::network_key(network)
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        if network != self.network {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        let index = self
            .boundaries
            .iter()
            .position(|reader| {
                reader.protocol == protocol && reader.factory.as_deref() == Some(factory)
            })
            .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        let boundary = self.boundaries.swap_remove(index).boundary?;
        boundary.ensure_current()?;
        Ok(boundary)
    }
}

async fn capture_reader(
    state: &SharedState,
    network: &str,
    protocol: &'static str,
    factory: Option<String>,
) -> ReaderSnapshot {
    let read = async {
        let store = state.store.as_ref()?;
        let factory = factory.as_deref()?;
        // Writer persists heartbeat after cursor. This order avoids combining
        // an older cursor with a newer heartbeat.
        let heartbeat = store
            .get_base_indexer_heartbeat(network, factory)
            .await
            .ok()
            .flatten();
        let cursor = store
            .get_base_log_cursor(network, factory)
            .await
            .ok()
            .flatten()
            .map(|c| c.last_scanned_block);
        Some((heartbeat, cursor))
    };
    let (heartbeat, cursor) = match tokio::time::timeout(Duration::from_secs(12), read).await {
        Ok(value) => value.unwrap_or_default(),
        Err(_) => {
            record_failure_stage(
                FailureStage::MarketplaceSnapshotTimedOut,
                &deployment_revision(),
            );
            (None, None)
        }
    };
    ReaderSnapshot {
        protocol,
        factory,
        heartbeat,
        cursor,
    }
}

async fn finish_projection_observation(
    network: &str,
    rpc: Option<(&str, u64)>,
    snapshots: Vec<ReaderSnapshot>,
) -> ProjectionObservation {
    // Every represented heartbeat/cursor snapshot is complete before any head
    // request starts. No future from an earlier request can be reused here.
    let heads = if snapshots
        .iter()
        .any(|s| s.heartbeat.is_some() && s.cursor.is_some())
    {
        match rpc {
            Some((url, chain)) => match tokio::time::timeout(
                Duration::from_secs(12),
                observed_heads_for_chain(url, chain),
            )
            .await
            {
                Ok(heads) => heads,
                Err(_) => {
                    record_failure_stage(
                        FailureStage::MarketplaceHeadsTimedOut,
                        &deployment_revision(),
                    );
                    None
                }
            },
            None => {
                record_failure_stage(FailureStage::HeadRpcNotConfigured, &deployment_revision());
                None
            }
        }
    } else {
        None
    };
    let now = Utc::now();
    let boundaries = snapshots
        .into_iter()
        .map(|s| {
            let result = projection_boundary(
                s.protocol,
                s.heartbeat.as_ref(),
                s.cursor,
                heads.map(|(n, t, _)| (n, t)),
                heads.map(|(_, _, n)| n),
                now,
            );
            ObservedReader {
                protocol: s.protocol,
                factory: s.factory,
                boundary: result,
            }
        })
        .collect();
    ProjectionObservation {
        network: network.to_string(),
        boundaries,
    }
}

pub(super) async fn observe_projection_readers(
    state: &SharedState,
    network: &str,
) -> ProjectionObservation {
    let network = service_runtime::bounty_read_inputs::network_key(network).unwrap_or(network);
    let autonomous = configured_autonomous_planner(network)
        .ok()
        .map(|p| p.factory_contract);
    let competition = open_competition_release_from_environment(network)
        .ok()
        .map(|r| r.factory_contract);
    let beta3 = open_competition_v2_api::release_from_environment(network)
        .ok()
        .map(|r| r.factory_contract);
    let autonomous_v2 = configured_autonomous_v2_deployment(network)
        .ok()
        .map(|d| d.factory);
    observe_resolved_projection_readers(
        state,
        network,
        autonomous,
        competition,
        beta3,
        autonomous_v2,
    )
    .await
}

async fn observe_resolved_projection_readers(
    state: &SharedState,
    network: &str,
    autonomous: Option<String>,
    competition: Option<String>,
    beta3: Option<String>,
    autonomous_v2: Option<String>,
) -> ProjectionObservation {
    // Separate bounded snapshots: one unavailable sibling does not erase another.
    let (a, b, c, d) = tokio::join!(
        capture_reader(state, network, AUTONOMOUS_PROTOCOL_VERSION, autonomous),
        capture_reader(
            state,
            network,
            OPEN_COMPETITION_PROTOCOL_VERSION,
            competition
        ),
        capture_reader(
            state,
            network,
            "agent-bounties/open-competition-v2-beta3",
            beta3
        ),
        capture_reader(
            state,
            network,
            AUTONOMOUS_V2_PROTOCOL_VERSION,
            autonomous_v2
        ),
    );
    let rpc = state
        .marketplace_head_rpc
        .resolve(&state.base_rpc_urls, network)
        .ok();
    finish_projection_observation(
        network,
        rpc.as_ref().map(|(d, url)| (url.as_str(), d.chain_id)),
        vec![a, b, c, d],
    )
    .await
}

#[utoipa::path(get, path = "/readiness/marketplace", responses(
    (status = 200, body = serde_json::Value, description = "All canonical readers are current; not payment or verification approval"),
    (status = 503, body = serde_json::Value, description = "One or more readers or their safe head is unavailable or behind")
))]
pub(super) async fn readiness(State(state): State<SharedState>) -> Response {
    let mut factories = vec![
        (
            AUTONOMOUS_PROTOCOL_VERSION,
            autonomous_factory_for_chain(8453),
        ),
        (
            OPEN_COMPETITION_PROTOCOL_VERSION,
            open_competition_release_from_environment("base-mainnet")
                .ok()
                .map(|r| r.factory_contract),
        ),
        (
            "agent-bounties/open-competition-v2-beta3",
            open_competition_v2_api::release_from_environment("base-mainnet")
                .ok()
                .map(|r| r.factory_contract),
        ),
    ];
    if autonomous_projection::protocols("base-mainnet")
        .is_ok_and(|protocols| protocols.contains(&AUTONOMOUS_V2_PROTOCOL_VERSION))
    {
        factories.push((
            AUTONOMOUS_V2_PROTOCOL_VERSION,
            configured_autonomous_v2_deployment("base-mainnet")
                .ok()
                .map(|d| d.factory),
        ));
    }
    // One bound covers database reads and all RPC requests. Snapshot each
    // heartbeat before its cursor, then fetch heads after every snapshot.
    let heads_started = std::sync::atomic::AtomicBool::new(false);
    let observation = async {
        let mut snapshots = Vec::new();
        for (protocol, factory) in &factories {
            let (heartbeat, cursor) = match (state.store.as_ref(), factory.as_deref()) {
                (Some(store), Some(factory)) => {
                    let heartbeat = store
                        .get_base_indexer_heartbeat("base-mainnet", factory)
                        .await;
                    let cursor = store.get_base_log_cursor("base-mainnet", factory).await;
                    (
                        heartbeat.ok().flatten(),
                        cursor.ok().flatten().map(|c| c.last_scanned_block),
                    )
                }
                _ => (None, None),
            };
            snapshots.push((*protocol, heartbeat, cursor));
        }
        heads_started.store(true, std::sync::atomic::Ordering::Relaxed);
        let heads = match state
            .marketplace_head_rpc
            .resolve(&state.base_rpc_urls, "base-mainnet")
        {
            Ok((_, rpc)) => observed_heads_for_chain(&rpc, 8453).await,
            Err(_) => {
                record_failure_stage(FailureStage::HeadRpcNotConfigured, &deployment_revision());
                None
            }
        };
        (heads, snapshots)
    };
    let (heads, snapshots) = tokio::time::timeout(Duration::from_secs(12), observation)
        .await
        .unwrap_or_else(|_| {
            record_failure_stage(
                observation_timeout_stage(heads_started.load(std::sync::atomic::Ordering::Relaxed)),
                &deployment_revision(),
            );
            (
                None,
                factories
                    .iter()
                    .map(|(protocol, _)| (*protocol, None, None))
                    .collect(),
            )
        });
    let now = Utc::now();
    let safe = heads.map(|(safe, _, _)| safe);
    let latest = heads.map(|(_, _, latest)| latest);
    let readers: Vec<_> = snapshots
        .into_iter()
        .map(|(protocol, heartbeat, cursor)| {
            reader_status(protocol, heartbeat.as_ref(), cursor, safe, latest, now)
        })
        .collect();
    let ready = readers.iter().all(|r| r["ready"] == true);
    (
        if ready { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE },
        [(header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({
            "schema": "agent-bounties/marketplace-readiness-v1", "ready": ready,
            "checked_at": now, "safe_block": safe, "latest_block": latest, "readers": readers,
            "limits": { "heartbeat_age_seconds": 90, "lag_blocks": 20 },
            "evidence_boundary": "Reader freshness only. Does not approve verification, prove funding or payment, authorize a restart, or lift a protocol hold."
        })),
    ).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_time_diagnostics_distinguish_rejected_inputs_at_the_exact_boundary() {
        let now = 1_800_000_000_i64;
        for age in [0, 1, 299, 300] {
            let timestamp = now - age;
            assert_eq!(checked_head_timestamp(timestamp as u64, now), Ok(timestamp));
        }
        assert_eq!(
            checked_head_timestamp((now - 301) as u64, now),
            Err(FailureClass::StaleTime)
        );
        assert_eq!(
            checked_head_timestamp((now + 1) as u64, now),
            Err(FailureClass::FutureTime)
        );
        for timestamp in [i64::MAX as u64 + 1, u64::MAX] {
            assert_eq!(
                checked_head_timestamp(timestamp, now),
                Err(FailureClass::TimeOutOfRange)
            );
        }
        assert_eq!(
            checked_head_timestamp(1, i64::MIN),
            Err(FailureClass::TimeOutOfRange)
        );
    }

    #[test]
    fn head_time_diagnostics_preserve_the_existing_acceptance_interval() {
        // Compare the diagnostic helper with the former acceptance predicate,
        // including the current UTC epoch and both interval boundaries.
        for now in [600_i64, 1_800_000_000, i64::MAX - 600] {
            for offset in -600..=600 {
                let timestamp = now + offset;
                let accepted_before = (0..=300).contains(&(now - timestamp));
                assert_eq!(
                    checked_head_timestamp(timestamp as u64, now).is_ok(),
                    accepted_before,
                    "now={now}, timestamp={timestamp}"
                );
            }
        }
    }

    #[test]
    fn safe_head_expiring_during_latest_read_is_attributed_to_safe_head() {
        let first_observation = 1_800_000_000_i64;
        let safe_time = first_observation - 300;
        let latest_time = first_observation - 2;
        assert!(checked_head_timestamp(safe_time as u64, first_observation).is_ok());
        assert_eq!(
            observed_head_time_failure(safe_time, latest_time, first_observation),
            None
        );
        // One second of RPC latency invalidates only the safe head. It must
        // still fail closed, without incorrectly blaming the latest header.
        assert_eq!(
            observed_head_time_failure(safe_time, latest_time, first_observation + 1),
            Some((FailureStage::HeadSafeValidation, FailureClass::StaleTime))
        );
    }

    #[test]
    fn head_time_diagnostics_identify_the_rejected_header() {
        let now = 1_800_000_000_i64;
        assert_eq!(
            observed_head_time_failure(now - 30, now + 1, now),
            Some((FailureStage::HeadLatestValidation, FailureClass::FutureTime))
        );
        assert_eq!(
            observed_head_time_failure(now - 301, now + 1, now),
            Some((FailureStage::HeadSafeValidation, FailureClass::StaleTime))
        );
        assert_eq!(
            observed_head_time_failure(-1, now, now),
            Some((
                FailureStage::HeadSafeValidation,
                FailureClass::TimeOutOfRange
            ))
        );
    }

    #[test]
    fn combined_projection_preserves_every_included_expiry() {
        let projection = |seconds| {
            ProjectionReadBoundary {
                block_number: 998,
                safe_block_number: 998,
                expires_at: Utc::now() + chrono::Duration::seconds(seconds),
            }
            .bind(vec![1])
        };
        assert!(CheckedProjection::<Vec<u8>>::concat(Vec::new()).is_err());
        assert!(CheckedProjection::concat(vec![projection(60), projection(-1)]).is_err());
        assert!(CheckedProjection::concat(vec![projection(-1), projection(60)]).is_err());
        let combined = CheckedProjection::concat(vec![projection(60), projection(30)]).unwrap();
        assert!(combined.boundary.expires_at <= Utc::now() + chrono::Duration::seconds(30));
        assert_eq!(combined.into_current().unwrap(), vec![1, 1]);
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn database_snapshots_finish_before_first_shared_head_postgres() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let parsed = url::Url::parse(&database_url).unwrap();
        assert!(matches!(parsed.host_str(), Some("127.0.0.1" | "localhost")));
        assert_eq!(
            std::env::var("AGENT_BOUNTIES_LOCAL_TEST_DATABASE").as_deref(),
            Ok("1")
        );
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();
        let factories = [
            (
                AUTONOMOUS_PROTOCOL_VERSION,
                format!("0x{:040x}", Uuid::new_v4().as_u128()),
            ),
            (
                OPEN_COMPETITION_PROTOCOL_VERSION,
                format!("0x{:040x}", Uuid::new_v4().as_u128()),
            ),
            (
                "agent-bounties/open-competition-v2-beta3",
                format!("0x{:040x}", Uuid::new_v4().as_u128()),
            ),
            (
                AUTONOMOUS_V2_PROTOCOL_VERSION,
                format!("0x{:040x}", Uuid::new_v4().as_u128()),
            ),
        ];
        for (_, factory) in &factories {
            store
                .upsert_base_log_cursor("base-mainnet", factory, 998, None)
                .await
                .unwrap();
            let mut record = heartbeat(Utc::now());
            record.escrow_contract = factory.clone();
            store.upsert_base_indexer_heartbeat(&record).await.unwrap();
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let changed = store.clone();
        let changed_factories = factories.clone();
        let rpc = Router::new().route("/", post(move |Json(request): Json<Value>| {
            let calls = calls.clone();
            let changed = changed.clone();
            let changed_factories = changed_factories.clone();
            async move {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    assert_eq!(request["method"], "eth_chainId");
                    // Complete actual committed writes before replying to the
                    // first head read. Any later DB snapshot would see them.
                    for (_, factory) in &changed_factories {
                        changed.upsert_base_log_cursor("base-mainnet", factory, 2000, None).await.unwrap();
                        let mut record = heartbeat(Utc::now());
                        record.escrow_contract = factory.clone();
                        record.persisted_cursor_block = Some(2000);
                        record.status = "failed".into();
                        changed.upsert_base_indexer_heartbeat(&record).await.unwrap();
                    }
                }
                let result = match request["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x2105"),
                    "eth_blockNumber" => json!("0x3e8"),
                    "eth_getBlockByNumber" => json!({
                        "number": if request["params"][0] == "safe" { "0x3e6" } else { "0x3e8" },
                        "hash": format!("0x{}", "ab".repeat(32)),
                        "timestamp": format!("0x{:x}", Utc::now().timestamp())
                    }),
                    method => panic!("unexpected fixture RPC: {method}"),
                };
                Json(json!({"jsonrpc":"2.0", "id":request["id"], "result":result}))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let rpc_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, rpc).await.unwrap() });
        let transaction_calls = Arc::new(AtomicUsize::new(0));
        let transaction_observed = transaction_calls.clone();
        let transaction_app = Router::new().route(
            "/",
            post(move || {
                transaction_calls.fetch_add(1, Ordering::SeqCst);
                async { StatusCode::TOO_MANY_REQUESTS }
            }),
        );
        let transaction_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let transaction_url = format!("http://{}", transaction_listener.local_addr().unwrap());
        let transaction_server = tokio::spawn(async move {
            axum::serve(transaction_listener, transaction_app)
                .await
                .unwrap()
        });
        let mut state = crate::tests::test_state_with_operator_token_and_store(
            BountyNetwork::default(),
            "unused",
            store.clone(),
        );
        let config = Arc::get_mut(&mut state).unwrap();
        config.base_rpc_urls.base_mainnet = Some(transaction_url.clone());
        config.marketplace_head_rpc.base_mainnet = Some(rpc_url);
        // Supply only factory selection; the production heartbeat/cursor reads,
        // join and head observation run unchanged against actual PostgreSQL.
        let mut original = observe_resolved_projection_readers(
            &state,
            "base-mainnet",
            Some(factories[0].1.clone()),
            Some(factories[1].1.clone()),
            Some(factories[2].1.clone()),
            Some(factories[3].1.clone()),
        )
        .await;
        assert_eq!(observed.load(Ordering::SeqCst), 4);
        assert_eq!(transaction_observed.load(Ordering::SeqCst), 0);
        assert_eq!(
            state.base_rpc_urls.resolve("base-mainnet").unwrap().1,
            transaction_url
        );
        for (protocol, factory) in &factories {
            let boundary = original.take("base-mainnet", protocol, factory).unwrap();
            assert_eq!(
                boundary.block_number, 998,
                "all four snapshots precede heads"
            );
            assert_eq!(boundary.safe_block_number, 998);
            assert_eq!(
                store
                    .get_base_log_cursor("base-mainnet", factory)
                    .await
                    .unwrap()
                    .unwrap()
                    .last_scanned_block,
                2000
            );
            assert_eq!(
                store
                    .get_base_indexer_heartbeat("base-mainnet", factory)
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                "failed"
            );
        }
        // A subsequent request must read the new invalid rows, not reuse the
        // first request's success or heads as a positive cache.
        let mut newer = observe_resolved_projection_readers(
            &state,
            "base-mainnet",
            Some(factories[0].1.clone()),
            Some(factories[1].1.clone()),
            Some(factories[2].1.clone()),
            Some(factories[3].1.clone()),
        )
        .await;
        assert_eq!(observed.load(Ordering::SeqCst), 8);
        for (protocol, factory) in &factories {
            assert!(newer.take("base-mainnet", protocol, factory).is_err());
        }
        assert!(require_current_reader(
            &state,
            "base-mainnet",
            AUTONOMOUS_PROTOCOL_VERSION,
            &factories[0].1
        )
        .await
        .is_err());
        assert_eq!(observed.load(Ordering::SeqCst), 12);
        assert_eq!(transaction_observed.load(Ordering::SeqCst), 0);
        transaction_server.abort();
        server.abort();
    }

    #[tokio::test]
    async fn one_request_shares_four_heads_without_sharing_protocol_health_or_identity() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let rpc=Router::new().route("/",post(move |Json(request):Json<Value>| {
            let calls=calls.clone(); async move {
                calls.fetch_add(1,Ordering::SeqCst);
                let result=match request["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x2105"),
                    "eth_blockNumber" => json!("0x3e8"),
                    "eth_getBlockByNumber" => json!({"number":if request["params"][0]=="safe" {"0x3e6"} else {"0x3e8"},"hash":format!("0x{}","ab".repeat(32)),"timestamp":format!("0x{:x}",Utc::now().timestamp())}),
                    other => panic!("unexpected RPC {other}"),
                };
                Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, rpc).await.unwrap();
        });
        for failed in [false, true] {
            let now = Utc::now();
            let healthy = heartbeat(now);
            let mut bad = healthy.clone();
            bad.status = "failed".into();
            bad.error_message = Some("private marker".into());
            let snapshots = vec![
                ReaderSnapshot {
                    protocol: AUTONOMOUS_PROTOCOL_VERSION,
                    factory: Some("a".into()),
                    heartbeat: Some(healthy.clone()),
                    cursor: Some(998),
                },
                ReaderSnapshot {
                    protocol: OPEN_COMPETITION_PROTOCOL_VERSION,
                    factory: Some("b".into()),
                    heartbeat: Some(healthy.clone()),
                    cursor: Some(998),
                },
                ReaderSnapshot {
                    protocol: "agent-bounties/open-competition-v2-beta3",
                    factory: Some("c".into()),
                    heartbeat: Some(if failed { bad } else { healthy }),
                    cursor: Some(998),
                },
            ];
            observed.store(0, Ordering::SeqCst);
            let mut observation =
                finish_projection_observation("base-mainnet", Some((&url, 8453)), snapshots).await;
            assert_eq!(
                observed.load(Ordering::SeqCst),
                4,
                "one shared head acquisition for three snapshots"
            );
            assert!(observation
                .take("base-sepolia", AUTONOMOUS_PROTOCOL_VERSION, "a")
                .is_err());
            assert!(observation
                .take("base-mainnet", AUTONOMOUS_PROTOCOL_VERSION, "wrong factory")
                .is_err());
            let autonomous = observation
                .take("base", AUTONOMOUS_PROTOCOL_VERSION, "a")
                .unwrap();
            assert_eq!(autonomous.block_number, 998);
            assert_eq!(autonomous.safe_block_number, 998);
            assert!(
                observation
                    .take("base-mainnet", AUTONOMOUS_PROTOCOL_VERSION, "a")
                    .is_err(),
                "one boundary consumption"
            );
            assert!(observation
                .take("base-mainnet", OPEN_COMPETITION_PROTOCOL_VERSION, "b")
                .is_ok());
            assert_eq!(
                observation
                    .take(
                        "base-mainnet",
                        "agent-bounties/open-competition-v2-beta3",
                        "c"
                    )
                    .is_err(),
                failed
            );
        }
        let no_snapshot = ReaderSnapshot {
            protocol: AUTONOMOUS_PROTOCOL_VERSION,
            factory: Some("a".into()),
            heartbeat: None,
            cursor: None,
        };
        observed.store(0, Ordering::SeqCst);
        let mut unavailable =
            finish_projection_observation("base-mainnet", Some((&url, 8453)), vec![no_snapshot])
                .await;
        assert_eq!(
            observed.load(Ordering::SeqCst),
            0,
            "no valid DB snapshot means no head demand"
        );
        assert!(unavailable
            .take("base-mainnet", AUTONOMOUS_PROTOCOL_VERSION, "a")
            .is_err());
        server.abort();
    }

    #[tokio::test]
    async fn diagnostics_head_rate_limit_stays_unavailable_without_exposing_provider_data() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        for (fail_chain_id, has_retry_after) in
            [(true, true), (false, true), (true, false), (false, false)]
        {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let failed_reads = Arc::new(AtomicUsize::new(0));
            let observed_failed_reads = failed_reads.clone();
            let app = Router::new().route("/", axum::routing::post(move |Json(request): Json<serde_json::Value>| {
                let failed_reads = failed_reads.clone();
                async move {
                    let method = request["method"].as_str().unwrap();
                    if !fail_chain_id && method == "eth_chainId" {
                        return Json(json!({"jsonrpc":"2.0", "id":request["id"], "result":"0x2105"})).into_response();
                    }
                    assert_eq!(method, if fail_chain_id { "eth_chainId" } else { "eth_getBlockByNumber" });
                    failed_reads.fetch_add(1, Ordering::SeqCst);
                    let mut response = (StatusCode::TOO_MANY_REQUESTS, Json(json!({
                        "jsonrpc":"2.0", "id":request["id"], "error":{"code":15,"message":"private-response-marker"}
                    }))).into_response();
                    if has_retry_after {
                        response.headers_mut().insert("retry-after", HeaderValue::from_static("30"));
                    }
                    response
                }
            }));
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            assert!(observed_heads_for_chain(
                &format!("http://{address}/?token=private-query-marker"),
                8453
            )
            .await
            .is_none());
            // Header and no-header rate limits stop after the first wire attempt.
            // Diagnostics observes this policy without adding or removing retries.
            assert_eq!(observed_failed_reads.load(Ordering::SeqCst), 1);
            server.abort();
        }
    }

    fn heartbeat(now: DateTime<Utc>) -> BaseIndexerHeartbeat {
        BaseIndexerHeartbeat {
            network: "base-mainnet".into(),
            escrow_contract: "private-fixture".into(),
            status: "success".into(),
            started_at: now,
            completed_at: Some(now),
            latest_block: Some(1000),
            confirmed_to_block: Some(998),
            from_block: Some(990),
            to_block: Some(998),
            fetched_logs: 0,
            persisted_cursor_block: Some(998),
            skipped_reason: None,
            error_message: None,
            updated_at: now,
        }
    }
    #[test]
    fn independent_latest_bounds_the_durable_cursor_without_rejecting_normal_progress() {
        let now = Utc::now();
        let h = heartbeat(now);
        for protocol in [
            AUTONOMOUS_PROTOCOL_VERSION,
            OPEN_COMPETITION_PROTOCOL_VERSION,
            "agent-bounties/open-competition-v2-beta3",
        ] {
            // Cursor commits before its new heartbeat. Do not compare that newer
            // durable cursor only with the old heartbeat's latest block (1000).
            assert_eq!(
                reader_status(protocol, Some(&h), Some(1008), Some(1000), Some(1010), now)["ready"],
                true
            );
            let impossible =
                reader_status(protocol, Some(&h), Some(2000), Some(1000), Some(1010), now);
            assert_eq!(impossible["ready"], false);
            assert_eq!(impossible["blocker"], "cursor_ahead_of_latest_head");
            for latest in [None, Some(999)] {
                assert_eq!(
                    reader_status(protocol, Some(&h), Some(998), Some(1000), latest, now)["ready"],
                    false
                );
            }
            let boundary = projection_boundary(
                protocol,
                Some(&h),
                Some(1008),
                Some((1000, now.timestamp())),
                Some(1010),
                now,
            )
            .unwrap();
            assert_eq!(boundary.block_number, 1000); // earning projection remains safe-capped
        }
        let mut future = h;
        future.completed_at = Some(now + chrono::Duration::milliseconds(500));
        assert_eq!(
            reader_status(
                "test",
                Some(&future),
                Some(998),
                Some(1000),
                Some(1010),
                now
            )["blocker"],
            "heartbeat_in_future"
        );
    }

    #[test]
    fn older_independent_safe_tag_does_not_invalidate_beta3_completed_progress() {
        let now = Utc::now();
        let mut h = heartbeat(now);
        h.latest_block = Some(52_706_930);
        h.persisted_cursor_block = Some(52_706_913);
        h.confirmed_to_block = Some(52_706_913);
        assert_eq!(
            reader_status(
                "agent-bounties/open-competition-v2-beta3",
                Some(&h),
                Some(52_706_913),
                Some(52_706_877),
                Some(52_706_963),
                now
            )["ready"],
            true
        );
    }

    #[tokio::test]
    async fn independent_heads_reject_wrong_order_identity_or_timestamp() {
        for (safe, latest, returned, safe_offset, latest_offset, expected) in [
            (1000, 1010, 1010, -10, 0, true),
            (1000, 990, 990, -10, 0, false),
            (1000, 1010, 1011, -10, 0, false),
            (1000, 1010, 1010, -10, 10, false),
            (1000, 1010, 1010, 10, 10, false),
            (1000, 1010, 1010, -10, -20, false),
            (1000, 1010, 1010, -400, -350, false),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let router = Router::new().route("/", axum::routing::post(move |Json(request): Json<serde_json::Value>| async move {
                let result = match request["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x2105"),
                    "eth_blockNumber" => json!(format!("0x{latest:x}")),
                    "eth_getBlockByNumber" => {
                        let (number, offset) = if request["params"][0] == "safe" { (safe, safe_offset) }
                            else { assert_eq!(request["params"][0], format!("0x{latest:x}")); (returned, latest_offset) };
                        json!({"number":format!("0x{number:x}"), "hash":format!("0x{}", "ab".repeat(32)),
                            "timestamp":format!("0x{:x}", Utc::now().timestamp() + offset)})
                    }
                    _ => panic!("unexpected read"),
                };
                Json(json!({"jsonrpc":"2.0", "id":request["id"], "result":result}))
            }));
            let server = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            let observed = observed_heads_for_chain(&format!("http://{address}"), 8453).await;
            server.abort();
            assert_eq!(observed.is_some(), expected);
        }
    }

    #[test]
    fn stopped_reader_is_not_hidden_by_a_healthy_process_or_recent_failed_heartbeat() {
        let now = Utc::now();
        let mut h = heartbeat(now);
        assert_eq!(
            reader_status("test", Some(&h), Some(998), Some(998), Some(1000), now)["ready"],
            true
        );
        h.persisted_cursor_block = None;
        h.status = "failed".into();
        h.error_message = Some("secret provider address".into());
        let result = reader_status("test", Some(&h), Some(998), Some(998), Some(1000), now);
        assert_eq!(result["blocker"], "reader_failed");
        assert_eq!(result["cursor_block"], 998);
        assert_eq!(result["lag_blocks"], 0);
        assert!(!result.to_string().contains("secret"));
        assert!(!result.to_string().contains("private-fixture"));
    }
    #[test]
    fn reader_requires_current_heartbeat_and_progress_against_independent_head() {
        let now = Utc::now();
        let mut h = heartbeat(now);
        for head in [None, Some(1019)] {
            assert_eq!(
                reader_status("test", Some(&h), Some(998), head, Some(1030), now)["ready"],
                false
            );
        }
        h.completed_at = Some(now - chrono::Duration::seconds(91));
        assert_eq!(
            reader_status("test", Some(&h), Some(998), Some(998), Some(1000), now)["ready"],
            false
        );
        h.completed_at = Some(now + chrono::Duration::seconds(1));
        assert_eq!(
            reader_status("test", Some(&h), Some(998), Some(998), Some(1000), now)["ready"],
            false
        );
        assert_eq!(
            reader_status("test", None, Some(998), Some(998), Some(1000), now)["ready"],
            false
        );
        h = heartbeat(now);
        assert_eq!(
            reader_status("test", Some(&h), None, Some(998), Some(1000), now)["ready"],
            false
        );
        h.persisted_cursor_block = Some(1001);
        assert_eq!(
            reader_status("test", Some(&h), Some(998), Some(998), Some(1000), now)["ready"],
            false
        );
    }
    #[test]
    fn caught_up_beta3_skip_is_healthy_but_other_skips_are_not() {
        let now = Utc::now();
        let mut h = heartbeat(now);
        h.status = "skipped".into();
        h.skipped_reason = Some("no safe blocks are ready to scan".into());
        let protocol = "agent-bounties/open-competition-v2-beta3";
        assert_eq!(
            reader_status(protocol, Some(&h), Some(998), Some(998), Some(1000), now)["ready"],
            true
        );
        h.skipped_reason = Some("provider unavailable".into());
        assert_eq!(
            reader_status(protocol, Some(&h), Some(998), Some(998), Some(1000), now)["ready"],
            false
        );
    }
    #[test]
    fn projection_boundaries_reject_stale_readers_and_ignore_unrelated_siblings() {
        let now = Utc::now();
        let healthy = heartbeat(now);
        let protocols = [
            AUTONOMOUS_PROTOCOL_VERSION,
            OPEN_COMPETITION_PROTOCOL_VERSION,
            "agent-bounties/open-competition-v2-beta3",
        ];
        for protocol in protocols {
            let boundary = projection_boundary(
                protocol,
                Some(&healthy),
                Some(998),
                Some((998, now.timestamp())),
                Some(1000),
                now,
            )
            .unwrap();
            assert_eq!(boundary.block_number, 998);
            // Rows written above the previously checked safe/cursor boundary must not enter this read.
            assert_eq!(
                [997, 998, 999]
                    .into_iter()
                    .filter(|block| *block <= boundary.block_number)
                    .collect::<Vec<_>>(),
                vec![997, 998]
            );
            let mut stale = healthy.clone();
            stale.completed_at = Some(now - chrono::Duration::seconds(91));
            assert!(projection_boundary(
                protocol,
                Some(&stale),
                Some(998),
                Some((998, now.timestamp())),
                Some(1000),
                now
            )
            .is_err());
            assert!(projection_boundary(
                protocol,
                Some(&healthy),
                Some(998),
                None,
                Some(1000),
                now
            )
            .is_err());
            assert!(projection_boundary(
                protocol,
                Some(&healthy),
                None,
                Some((998, now.timestamp())),
                Some(1000),
                now
            )
            .is_err());
        }
        let mut failed = healthy.clone();
        failed.status = "failed".to_string();
        failed.error_message = Some("private provider error".to_string());
        assert!(projection_boundary(
            protocols[2],
            Some(&failed),
            Some(998),
            Some((998, now.timestamp())),
            Some(1000),
            now
        )
        .is_err());
        assert!(projection_boundary(
            protocols[0],
            Some(&healthy),
            Some(998),
            Some((998, now.timestamp())),
            Some(1000),
            now
        )
        .is_ok());
        assert!(ProjectionReadBoundary {
            block_number: 998,
            safe_block_number: 998,
            expires_at: now - chrono::Duration::seconds(1)
        }
        .ensure_current()
        .is_err());
    }

    #[test]
    fn slow_sibling_cannot_release_an_expired_ready_projection() {
        let expired = ProjectionReadBoundary {
            block_number: 998,
            safe_block_number: 998,
            expires_at: Utc::now() - chrono::Duration::seconds(1),
        };
        let captured = expired.bind(vec!["stale ready row"]).map(|items| items);
        assert!(captured.into_current().is_err());
        let current = ProjectionReadBoundary {
            block_number: 998,
            safe_block_number: 998,
            expires_at: Utc::now() + chrono::Duration::seconds(5),
        };
        assert_eq!(
            current.bind(vec!["healthy row"]).into_current().unwrap(),
            vec!["healthy row"]
        );
    }

    #[tokio::test]
    async fn projection_safe_head_checks_requested_sepolia_chain() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = Router::new().route("/", axum::routing::post(|Json(request): Json<serde_json::Value>| async move {
            let result = if request["method"] == "eth_chainId" { json!("0x14a34") } else {
                json!({"number":"0x3e6", "hash":format!("0x{}", "ab".repeat(32)), "timestamp":format!("0x{:x}", Utc::now().timestamp())})
            };
            Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
        }));
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let rpc = format!("http://{address}");
        assert_eq!(
            safe_head_for_chain(&rpc, 84532).await.map(|head| head.0),
            Some(998)
        );
        assert!(safe_head_for_chain(&rpc, 8453).await.is_none());
        server.abort();
    }

    #[tokio::test]
    async fn foreign_rpc_chain_cannot_authorize_marketplace_readiness() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = Router::new().route(
            "/",
            axum::routing::post(|Json(request): Json<serde_json::Value>| async move {
                // A wrong chain is rejected before any block can be trusted.
                assert_eq!(request["method"], "eth_chainId");
                Json(serde_json::json!({"jsonrpc":"2.0", "id":request["id"], "result":"0x1"}))
            }),
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        assert_eq!(base_safe_head(&format!("http://{address}")).await, None);
        server.abort();
    }
    #[tokio::test]
    async fn safe_head_requires_fresh_time_from_base() {
        for (offset, expected) in [(0i64, Some(998)), (-400, None), (100, None)] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let router = Router::new().route("/", axum::routing::post(move |Json(request): Json<serde_json::Value>| async move {
                let result = match request["method"].as_str().unwrap() {
                    "eth_chainId" => serde_json::json!("0x2105"),
                    "eth_getBlockByNumber" => {
                        assert_eq!(request["params"], serde_json::json!(["safe", false]));
                        serde_json::json!({"number":"0x3e6", "hash":format!("0x{}", "ab".repeat(32)),
                            "timestamp":format!("0x{:x}", Utc::now().timestamp() + offset)})
                    }
                    _ => panic!("unexpected RPC request")
                };
                Json(serde_json::json!({"jsonrpc":"2.0", "id":request["id"], "result":result}))
            }));
            let server = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            assert_eq!(base_safe_head(&format!("http://{address}")).await, expected);
            server.abort();
        }
    }
}
