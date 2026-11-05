//! Process-local negative admission only: no responses, credentials or waiting queue.
//! Other processes/egress users have independent budgets. This is not chain evidence.
use crate::ChainBaseError;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

const MAX_ENDPOINTS: usize = 128;
const INITIAL_SECONDS: u64 = 30;
const MAX_LOCAL_SECONDS: u64 = 300;
pub(super) const PROBE_TIMEOUT_SECONDS: u64 = 12;
type Key = [u8; 32];

#[derive(Default)]
struct Registry {
    entries: HashMap<Key, Entry>,
}
#[derive(Default)]
struct Entry {
    active: usize,
    generation: u64,
    hold: Option<Hold>,
    probe: bool,
}
struct Hold {
    started: Instant,
    delay: Duration,
    failures: u32,
    status: u16,
}
impl Hold {
    fn remaining(&self, now: Instant) -> Duration {
        self.delay
            .saturating_sub(now.saturating_duration_since(self.started))
    }
}
static REGISTRY: OnceLock<Arc<Mutex<Registry>>> = OnceLock::new();

fn retry(status: u16, seconds: u64) -> ChainBaseError {
    ChainBaseError::RpcRetryAfter {
        status,
        retry_after_seconds: seconds.max(1),
    }
}
fn seconds_ceil(duration: Duration) -> u64 {
    duration
        .as_secs()
        .saturating_add(u64::from(duration.subsec_nanos() != 0))
}
fn local_delay(failures: u32, jitter: u64) -> u64 {
    let base = INITIAL_SECONDS
        .saturating_mul(
            1u64.checked_shl(failures.saturating_sub(1).min(63))
                .unwrap_or(u64::MAX),
        )
        .min(MAX_LOCAL_SECONDS);
    base.saturating_add(1 + jitter % (base / 4))
        .min(MAX_LOCAL_SECONDS)
}

/// The lease covers one wire attempt. Dropping a canceled probe cannot leave a
/// permanent lock or open a stampede: it starts a short hold before another probe.
pub(super) struct ReadLease {
    registry: Arc<Mutex<Registry>>,
    key: Key,
    generation: u64,
    pub probe: bool,
    finished: bool,
    diagnostics: Option<crate::rpc_read_diagnostics::Attempt>,
}
impl ReadLease {
    pub fn acquire(endpoint: &str) -> Result<Self, ChainBaseError> {
        let key = Sha256::digest(endpoint.as_bytes()).into();
        let result = Self::acquire_at(
            REGISTRY
                .get_or_init(|| Arc::new(Mutex::new(Registry::default())))
                .clone(),
            key,
            Instant::now(),
        );
        // Diagnostics run after the admission lock is released and cannot alter
        // its result. Only the opaque in-process key crosses this boundary.
        match result {
            Ok(mut lease) => {
                lease.diagnostics =
                    Some(crate::rpc_read_diagnostics::Attempt::new(key, lease.probe));
                Ok(lease)
            }
            Err(error) => {
                if let ChainBaseError::RpcRetryAfter {
                    status,
                    retry_after_seconds,
                } = &error
                {
                    crate::rpc_read_diagnostics::suppressed(key, *status, *retry_after_seconds);
                }
                Err(error)
            }
        }
    }
    fn acquire_at(
        registry: Arc<Mutex<Registry>>,
        key: Key,
        now: Instant,
    ) -> Result<Self, ChainBaseError> {
        let (generation, probe) = {
            let mut state = registry.lock().map_err(|_| retry(503, INITIAL_SECONDS))?;
            // Never evict an active hold to admit an untracked provider. Healthy
            // entries are removed when their last attempt finishes.
            if !state.entries.contains_key(&key) && state.entries.len() >= MAX_ENDPOINTS {
                return Err(retry(503, INITIAL_SECONDS));
            }
            let entry = state.entries.entry(key).or_default();
            let probe = if let Some(hold) = &entry.hold {
                let remaining = hold.remaining(now);
                if !remaining.is_zero() || entry.probe {
                    return Err(retry(hold.status, seconds_ceil(remaining).max(1)));
                }
                entry.probe = true;
                true
            } else {
                false
            };
            entry.active += 1;
            (entry.generation, probe)
        };
        Ok(Self {
            registry,
            key,
            generation,
            probe,
            finished: false,
            diagnostics: None,
        })
    }
    pub fn finish(mut self, hold: Option<(u16, u64)>, valid_response: bool, probe_timed_out: bool) {
        let jitter = if hold.is_some() {
            let bytes = *uuid::Uuid::new_v4().as_bytes();
            u64::from_le_bytes(bytes[..8].try_into().expect("eight bytes"))
        } else {
            0
        };
        self.finish_at(hold, valid_response, Instant::now(), jitter);
        if let Some(diagnostics) = self.diagnostics.take() {
            diagnostics.finish(hold, valid_response, probe_timed_out);
        }
    }
    fn finish_at(
        &mut self,
        requested: Option<(u16, u64)>,
        valid_response: bool,
        now: Instant,
        jitter: u64,
    ) {
        let Ok(mut state) = self.registry.lock() else {
            self.finished = true;
            return;
        };
        let Some(entry) = state.entries.get_mut(&self.key) else {
            self.finished = true;
            return;
        };
        if let Some((status, requested_seconds)) = requested {
            if entry.generation == self.generation || entry.hold.is_none() {
                let failures = entry
                    .hold
                    .as_ref()
                    .map_or(1, |h| h.failures.saturating_add(1));
                let delay = local_delay(failures, jitter).max(requested_seconds);
                entry.hold = Some(Hold {
                    started: now,
                    delay: Duration::from_secs(delay),
                    failures,
                    status,
                });
                entry.generation = entry.generation.wrapping_add(1);
                entry.probe = false;
            } else if let Some(hold) = &mut entry.hold {
                // A late response may extend a newer hold, never shorten it or
                // count one concurrent wave as many consecutive failed probes.
                let remaining = hold.remaining(now);
                let requested_delay =
                    Duration::from_secs(requested_seconds.max(local_delay(hold.failures, jitter)));
                // Classify the cause of the effective longest hold, retaining
                // the existing cause on ties rather than relabeling it.
                if requested_delay > remaining {
                    hold.status = status;
                }
                hold.delay = remaining.max(requested_delay);
                hold.started = now;
                entry.generation = entry.generation.wrapping_add(1);
            }
        } else if self.probe && entry.generation == self.generation {
            if valid_response {
                entry.hold = None;
            } else if let Some(hold) = &mut entry.hold {
                hold.started = now;
                hold.delay = Duration::from_secs(INITIAL_SECONDS);
            }
        }
        if self.probe {
            entry.probe = false;
        }
        entry.active = entry.active.saturating_sub(1);
        if entry.active == 0 && entry.hold.is_none() {
            state.entries.remove(&self.key);
        }
        self.finished = true;
    }
}
impl Drop for ReadLease {
    fn drop(&mut self) {
        if !self.finished {
            self.finish_at(None, false, Instant::now(), 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn registry() -> Arc<Mutex<Registry>> {
        Arc::new(Mutex::new(Registry::default()))
    }
    fn key(n: u64) -> Key {
        Sha256::digest(n.to_le_bytes()).into()
    }
    fn admit(r: &Arc<Mutex<Registry>>, n: u64, at: Instant) -> Result<ReadLease, ChainBaseError> {
        ReadLease::acquire_at(r.clone(), key(n), at)
    }
    fn limited(result: Result<ReadLease, ChainBaseError>) -> (u16, u64) {
        match result {
            Err(ChainBaseError::RpcRetryAfter {
                status,
                retry_after_seconds,
            }) => (status, retry_after_seconds),
            _ => panic!("expected immediate hold"),
        }
    }
    #[test]
    fn delays_are_bounded_exponential_and_positive_jitter() {
        for n in 1..100 {
            for jitter in [0, 1, 7, u64::MAX] {
                let delay = local_delay(n, jitter);
                let base = 30u64
                    .saturating_mul(1u64.checked_shl(n - 1).unwrap_or(u64::MAX))
                    .min(300);
                assert!(
                    (base.saturating_add(1).min(300)..=base.saturating_add(base / 4).min(300))
                        .contains(&delay)
                );
            }
        }
    }
    #[test]
    fn hold_is_shared_and_only_one_probe_can_enter() {
        let r = registry();
        let now = Instant::now();
        admit(&r, 1, now)
            .unwrap()
            .finish_at(Some((429, 0)), false, now, 0);
        assert_eq!(limited(admit(&r, 1, now)), (429, 31));
        let probe = admit(&r, 1, now + Duration::from_secs(31)).unwrap();
        assert!(probe.probe);
        for _ in 0..100 {
            assert_eq!(
                limited(admit(&r, 1, now + Duration::from_secs(31))),
                (429, 1)
            );
        }
        let mut probe = probe;
        probe.finish_at(None, true, now + Duration::from_secs(31), 0);
        assert!(!admit(&r, 1, now + Duration::from_secs(31)).unwrap().probe);
    }
    #[test]
    fn concurrent_probe_admission_has_one_winner_and_no_wait_queue() {
        let r = registry();
        let now = Instant::now();
        admit(&r, 1, now)
            .unwrap()
            .finish_at(Some((429, 0)), false, now, 0);
        let barrier = Arc::new(std::sync::Barrier::new(17));
        let mut threads = Vec::new();
        for _ in 0..16 {
            let r = r.clone();
            let b = barrier.clone();
            threads.push(std::thread::spawn(move || {
                b.wait();
                admit(&r, 1, now + Duration::from_secs(31))
            }));
        }
        barrier.wait();
        let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(r.lock().unwrap().entries[&key(1)].active, 1);
    }
    #[test]
    fn cancellation_releases_probe_with_a_hold_and_late_success_cannot_clear_it() {
        let r = registry();
        let now = Instant::now();
        let mut older = admit(&r, 1, now).unwrap();
        admit(&r, 1, now)
            .unwrap()
            .finish_at(Some((429, 0)), false, now, 0);
        older.finish_at(None, true, now, 0);
        let probe = admit(&r, 1, now + Duration::from_secs(31)).unwrap();
        drop(probe);
        let state = r.lock().unwrap();
        let e = &state.entries[&key(1)];
        assert!(!e.probe);
        assert_eq!(e.active, 0);
        assert!(e.hold.is_some());
        drop(state);
        assert!(admit(&r, 1, Instant::now()).is_err());
    }
    #[test]
    fn newer_hold_during_probe_survives_that_probes_success_and_drop() {
        for success in [true, false] {
            let r = registry();
            let now = Instant::now();
            let mut old = admit(&r, 1, now).unwrap();
            admit(&r, 1, now)
                .unwrap()
                .finish_at(Some((429, 0)), false, now, 0);
            let mut probe = admit(&r, 1, now + Duration::from_secs(31)).unwrap();
            old.finish_at(Some((429, 7200)), false, now + Duration::from_secs(31), 0);
            probe.finish_at(None, success, now + Duration::from_secs(32), 0);
            assert_eq!(
                limited(admit(&r, 1, now + Duration::from_secs(32))),
                (429, 7199)
            );
            assert!(!r.lock().unwrap().entries[&key(1)].probe);
        }
    }
    #[tokio::test]
    async fn canceled_and_timed_out_http_probes_release_exclusive_admission() {
        for cancel in [true, false] {
            use crate::{JsonRpcTransport, ReqwestJsonRpcTransport};
            use tokio::io::AsyncReadExt;
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let now = Instant::now();
            let mut initial = ReadLease::acquire(&url).unwrap();
            initial.finish_at(Some((429, 0)), false, now - Duration::from_secs(32), 0);
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut connection, _) = listener.accept().await.unwrap();
                let mut buffer = [0; 4096];
                assert!(connection.read(&mut buffer).await.unwrap() > 0);
                started_tx.send(()).unwrap();
                // The canceled reqwest future closes the connection; no response sent.
                loop {
                    if connection.read(&mut buffer).await.unwrap() == 0 {
                        break 0;
                    }
                }
            });
            let task_url = url.clone();
            let task = tokio::spawn(async move {
                ReqwestJsonRpcTransport::default()
                .post_json_value(
                    &task_url,
                    &serde_json::json!({"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}),
                )
                .await
            });
            tokio::time::timeout(Duration::from_secs(2), started_rx)
                .await
                .unwrap()
                .unwrap();
            assert!(
                ReadLease::acquire(&url).is_err(),
                "active probe is exclusive"
            );
            if cancel {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                let result = tokio::time::timeout(Duration::from_secs(15), task)
                    .await
                    .unwrap()
                    .unwrap();
                assert!(
                    matches!(result,Err(ChainBaseError::RpcTransport(ref message)) if message == "read probe timed out")
                );
            }
            assert!(matches!(
                ReadLease::acquire(&url),
                Err(ChainBaseError::RpcRetryAfter {
                    retry_after_seconds: 1..=30,
                    ..
                })
            ));
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(2), server)
                    .await
                    .unwrap()
                    .unwrap(),
                0
            );
            let counts =
                crate::rpc_read_diagnostics::counts_for_test(Sha256::digest(url.as_bytes()).into());
            assert_eq!(counts["probes_admitted"], 1);
            assert_eq!(counts["probes_canceled"], u64::from(cancel));
            assert_eq!(counts["probes_timed_out"], u64::from(!cancel));
            assert_eq!(counts["probes_failed"], 0);
            assert_eq!(counts["wire_429"], 0);
        }
    }
    #[test]
    fn overflow_and_long_provider_delay_are_not_shortened() {
        let r = registry();
        let now = Instant::now();
        admit(&r, 1, now)
            .unwrap()
            .finish_at(Some((429, u64::MAX)), false, now, 0);
        assert_eq!(limited(admit(&r, 1, now)), (429, u64::MAX));
        assert!(limited(admit(&r, 1, now + Duration::from_secs(300))).1 > 300);
    }

    #[tokio::test]
    async fn probe_diagnostics_distinguish_matching_success_from_provider_error() {
        use crate::{JsonRpcTransport, ReqwestJsonRpcTransport};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for valid in [true, false] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let mut initial = ReadLease::acquire(&url).unwrap();
            initial.finish_at(
                Some((429, 0)),
                false,
                Instant::now() - Duration::from_secs(32),
                0,
            );
            let server = tokio::spawn(async move {
                let (mut connection, _) =
                    tokio::time::timeout(Duration::from_secs(2), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let mut buffer = [0; 4096];
                assert!(
                    tokio::time::timeout(Duration::from_secs(2), connection.read(&mut buffer))
                        .await
                        .unwrap()
                        .unwrap()
                        > 0
                );
                let body = if valid {
                    r#"{"jsonrpc":"2.0","id":1,"result":"0x2105"}"#
                } else {
                    r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"fixture"}}"#
                };
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                connection.write_all(response.as_bytes()).await.unwrap();
            });
            tokio::time::timeout(
                Duration::from_secs(2),
                ReqwestJsonRpcTransport::default().post_json_value(
                    &url,
                    &serde_json::json!({"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}),
                ),
            )
            .await
            .unwrap()
            .unwrap();
            tokio::time::timeout(Duration::from_secs(2), server)
                .await
                .unwrap()
                .unwrap();
            let counts =
                crate::rpc_read_diagnostics::counts_for_test(Sha256::digest(url.as_bytes()).into());
            assert_eq!(counts["probes_admitted"], 1);
            assert_eq!(counts["probes_valid"], u64::from(valid));
            assert_eq!(counts["probes_failed"], u64::from(!valid));
            assert_eq!(counts["probes_canceled"], 0);
            assert_eq!(counts["wire_429"], 0);
            assert_eq!(ReadLease::acquire(&url).is_ok(), valid);
        }
    }
    #[test]
    fn late_rate_response_extends_but_never_shortens_or_multiplies_wave() {
        let r = registry();
        let now = Instant::now();
        let mut older = admit(&r, 1, now).unwrap();
        admit(&r, 1, now)
            .unwrap()
            .finish_at(Some((429, 7200)), false, now, 0);
        older.finish_at(Some((429, 0)), false, now + Duration::from_secs(10), 0);
        assert_eq!(
            limited(admit(&r, 1, now + Duration::from_secs(10))),
            (429, 7190)
        );
        assert_eq!(
            r.lock().unwrap().entries[&key(1)]
                .hold
                .as_ref()
                .unwrap()
                .failures,
            1
        );
    }
    #[test]
    fn late_mixed_status_holds_report_the_effective_longest_deadline_cause() {
        for (first, later) in [(429, 503), (503, 429)] {
            for (first_delay, later_delay, expected) in
                [(0, 7200, later), (7200, 0, first), (7200, 7200, first)]
            {
                let r = registry();
                let now = Instant::now();
                let mut old = admit(&r, 1, now).unwrap();
                admit(&r, 1, now)
                    .unwrap()
                    .finish_at(Some((first, first_delay)), false, now, 0);
                old.finish_at(Some((later, later_delay)), false, now, 0);
                assert_eq!(limited(admit(&r, 1, now)), (expected, 7200));
                assert_eq!(
                    r.lock().unwrap().entries[&key(1)]
                        .hold
                        .as_ref()
                        .unwrap()
                        .failures,
                    1
                );
            }
        }
    }
    #[test]
    fn failed_probes_keep_hold_and_consecutive_limits_increase_delay() {
        let r = registry();
        let now = Instant::now();
        admit(&r, 1, now)
            .unwrap()
            .finish_at(Some((429, 0)), false, now, 0);
        admit(&r, 1, now + Duration::from_secs(31))
            .unwrap()
            .finish_at(Some((429, 0)), false, now + Duration::from_secs(31), 0);
        assert_eq!(
            limited(admit(&r, 1, now + Duration::from_secs(31))),
            (429, 61)
        );
        admit(&r, 1, now + Duration::from_secs(92))
            .unwrap()
            .finish_at(None, false, now + Duration::from_secs(92), 0);
        assert_eq!(
            limited(admit(&r, 1, now + Duration::from_secs(92))),
            (429, 30)
        );
        assert_eq!(
            r.lock().unwrap().entries[&key(1)]
                .hold
                .as_ref()
                .unwrap()
                .failures,
            2
        );
    }
    #[test]
    fn provider_503_hold_is_distinct_and_other_endpoints_and_processes_are_independent() {
        let r = registry();
        let now = Instant::now();
        admit(&r, 1, now)
            .unwrap()
            .finish_at(Some((503, 80)), false, now, 0);
        assert_eq!(limited(admit(&r, 1, now)), (503, 80));
        assert!(admit(&r, 2, now).is_ok());
        assert!(
            admit(&registry(), 1, now).is_ok(),
            "separate process state does not share a rate budget"
        );
    }
    #[test]
    fn cardinality_is_bounded_and_healthy_entries_leave_no_positive_cache() {
        let r = registry();
        let now = Instant::now();
        for n in 0..MAX_ENDPOINTS as u64 {
            admit(&r, n, now)
                .unwrap()
                .finish_at(Some((429, 0)), false, now, 0);
        }
        assert_eq!(r.lock().unwrap().entries.len(), MAX_ENDPOINTS);
        assert_eq!(limited(admit(&r, 999, now)), (503, 30));
        admit(&r, 0, now + Duration::from_secs(31))
            .unwrap()
            .finish_at(None, true, now + Duration::from_secs(31), 0);
        let lease = admit(&r, 999, now).unwrap();
        drop(lease);
        assert_eq!(r.lock().unwrap().entries.len(), MAX_ENDPOINTS - 1);
    }
}
