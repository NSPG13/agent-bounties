//! Private, bounded, process-local read diagnostics. Never admission authority.
//! Inputs are fixed events and an opaque key; URLs, payloads and errors are absent.
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::Write,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

type Key = [u8; 32];
const MAX_ENDPOINTS: usize = 128;
const LOG_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Default, Serialize)]
struct Counts {
    reads_admitted: u64,
    read_attempts_finished: u64,
    reads_canceled: u64,
    wire_429: u64,
    wire_503_retry_after: u64,
    admission_denied_429: u64,
    admission_denied_503: u64,
    probes_admitted: u64,
    probes_valid: u64,
    probes_failed: u64,
    probes_timed_out: u64,
    probes_canceled: u64,
}

#[derive(Clone, Copy)]
enum Event {
    Admitted {
        probe: bool,
    },
    Suppressed {
        status: u16,
        remaining_seconds: u64,
    },
    Completed {
        probe: bool,
        hold_status: Option<u16>,
        valid: bool,
        timed_out: bool,
        elapsed_ms: u64,
    },
    Canceled {
        probe: bool,
        elapsed_ms: u64,
    },
}

#[derive(Default)]
struct Summary {
    counts: Counts,
    first_at: Option<Instant>,
    last_emitted_at: Option<Instant>,
    pending: bool,
    last_admission_retry_seconds: Option<u64>,
    last_probe_elapsed_ms: Option<u64>,
    max_probe_elapsed_ms: u64,
    last_probe_outcome: Option<&'static str>,
}

fn increment(value: &mut u64) {
    *value = value.saturating_add(1);
}

impl Summary {
    fn observe(
        &mut self,
        event: Event,
        now: Instant,
        slot: usize,
        revision: &str,
    ) -> Option<Value> {
        let first = *self.first_at.get_or_insert(now);
        match event {
            Event::Admitted { probe } => {
                increment(&mut self.counts.reads_admitted);
                if probe {
                    increment(&mut self.counts.probes_admitted);
                    self.pending = true;
                }
            }
            Event::Suppressed {
                status,
                remaining_seconds,
            } => {
                if status == 429 {
                    increment(&mut self.counts.admission_denied_429);
                } else {
                    increment(&mut self.counts.admission_denied_503);
                }
                self.last_admission_retry_seconds = Some(remaining_seconds);
                self.pending = true;
            }
            Event::Completed {
                probe,
                hold_status,
                valid,
                timed_out,
                elapsed_ms,
            } => {
                increment(&mut self.counts.read_attempts_finished);
                match hold_status {
                    Some(429) => {
                        increment(&mut self.counts.wire_429);
                        self.pending = true;
                    }
                    Some(503) => {
                        increment(&mut self.counts.wire_503_retry_after);
                        self.pending = true;
                    }
                    _ => {}
                }
                if probe {
                    let outcome = if timed_out {
                        increment(&mut self.counts.probes_timed_out);
                        "timed_out"
                    } else if valid {
                        increment(&mut self.counts.probes_valid);
                        "valid_rpc_envelope"
                    } else {
                        increment(&mut self.counts.probes_failed);
                        "failed_or_invalid_response"
                    };
                    self.probe_outcome(outcome, elapsed_ms);
                }
            }
            Event::Canceled { probe, elapsed_ms } => {
                increment(&mut self.counts.reads_canceled);
                if probe {
                    increment(&mut self.counts.probes_canceled);
                    self.probe_outcome("canceled", elapsed_ms);
                }
            }
        }
        if !self.pending
            || self
                .last_emitted_at
                .is_some_and(|at| now.saturating_duration_since(at) < LOG_INTERVAL)
        {
            return None;
        }
        self.pending = false;
        self.last_emitted_at = Some(now);
        Some(json!({
            "schema": "agent-bounties/rpc-read-diagnostics-v1",
            "event": "rpc_read_diagnostics",
            "level": "info",
            "component": "chain_base",
            "action": "read_admission",
            "revision": safe_revision(revision),
            "endpoint_slot": slot,
            "overflow_aggregate": slot == 0,
            "observation_age_ms": milliseconds(now.saturating_duration_since(first)),
            "counts": self.counts,
            "last_admission_retry_seconds": self.last_admission_retry_seconds,
            "last_probe_elapsed_ms": self.last_probe_elapsed_ms,
            "max_probe_elapsed_ms": self.max_probe_elapsed_ms,
            "last_probe_outcome": self.last_probe_outcome,
            "evidence_boundary": "Process-local cumulative read diagnostics, not chain or payment evidence. Admission is permission to attempt a read, not proof it reached the provider. Valid probe means matching RPC envelope only. No retry or recovery authority."
        }))
    }

    fn probe_outcome(&mut self, outcome: &'static str, elapsed_ms: u64) {
        self.last_probe_outcome = Some(outcome);
        self.last_probe_elapsed_ms = Some(elapsed_ms);
        self.max_probe_elapsed_ms = self.max_probe_elapsed_ms.max(elapsed_ms);
        self.pending = true;
    }
}

#[derive(Default)]
struct Registry {
    entries: HashMap<Key, (usize, Summary)>,
    overflow: Summary,
}

impl Registry {
    fn observe(&mut self, key: Key, event: Event, now: Instant, revision: &str) -> Option<Value> {
        if !self.entries.contains_key(&key) {
            if self.entries.len() == MAX_ENDPOINTS {
                // Losing diagnostic detail must never deny an otherwise admitted read.
                return self.overflow.observe(event, now, 0, revision);
            }
            self.entries
                .insert(key, (self.entries.len() + 1, Summary::default()));
        }
        let (slot, summary) = self.entries.get_mut(&key)?;
        summary.observe(event, now, *slot, revision)
    }
}

static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

fn safe_revision(revision: &str) -> &str {
    if revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        revision
    } else {
        "local"
    }
}

fn milliseconds(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn record(key: Key, event: Event) {
    let revision = std::env::var("RENDER_GIT_COMMIT").unwrap_or_default();
    let output = REGISTRY
        .get_or_init(Mutex::default)
        .lock()
        .ok()
        .and_then(|mut registry| registry.observe(key, event, Instant::now(), &revision));
    if let Some(output) = output {
        // Match the existing private app logging pattern. No admission lock is
        // held while writing; a diagnostic failure cannot replace an RPC result.
        let _ = writeln!(std::io::stderr().lock(), "{output}");
    }
}

#[cfg(test)]
pub(super) fn counts_for_test(key: Key) -> Value {
    let registry = REGISTRY.get().unwrap().lock().unwrap();
    json!(registry.entries.get(&key).unwrap().1.counts)
}

pub(super) fn suppressed(key: Key, status: u16, remaining_seconds: u64) {
    record(
        key,
        Event::Suppressed {
            status,
            remaining_seconds,
        },
    );
}

pub(super) struct Attempt {
    key: Key,
    probe: bool,
    started: Instant,
    finished: bool,
}

impl Attempt {
    pub fn new(key: Key, probe: bool) -> Self {
        let attempt = Self {
            key,
            probe,
            started: Instant::now(),
            finished: false,
        };
        record(key, Event::Admitted { probe });
        attempt
    }

    pub fn finish(mut self, hold: Option<(u16, u64)>, valid: bool, timed_out: bool) {
        self.finished = true;
        record(
            self.key,
            Event::Completed {
                probe: self.probe,
                hold_status: hold.map(|(status, _)| status),
                valid,
                timed_out,
                elapsed_ms: milliseconds(self.started.elapsed()),
            },
        );
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        if !self.finished {
            record(
                self.key,
                Event::Canceled {
                    probe: self.probe,
                    elapsed_ms: milliseconds(self.started.elapsed()),
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn real_http_limit_is_counted_once_while_siblings_never_reach_the_wire() {
        use crate::{ChainBaseError, JsonRpcTransport, ReqwestJsonRpcTransport};
        use sha2::{Digest, Sha256};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
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
            connection.write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(250), listener.accept())
                    .await
                    .is_err()
            );
        });
        let request = json!({"jsonrpc":"2.0", "id":1, "method":"eth_chainId", "params":[]});
        let first = tokio::time::timeout(
            Duration::from_secs(2),
            ReqwestJsonRpcTransport::default().post_json_value(&url, &request),
        )
        .await
        .unwrap();
        assert!(matches!(first, Err(ChainBaseError::RpcHttpStatus(429))));
        for _ in 0..10 {
            assert!(matches!(
                ReqwestJsonRpcTransport::default()
                    .post_json_value(&url, &request)
                    .await,
                Err(ChainBaseError::RpcRetryAfter { status: 429, .. })
            ));
        }
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
        let counts = counts_for_test(Sha256::digest(url.as_bytes()).into());
        assert_eq!(counts["reads_admitted"], 1);
        assert_eq!(counts["read_attempts_finished"], 1);
        assert_eq!(counts["wire_429"], 1);
        assert_eq!(counts["admission_denied_429"], 10);
        assert_eq!(counts["probes_admitted"], 0);
    }

    #[test]
    fn one_wire_limit_and_many_suppressed_reads_have_distinct_counts_and_bounded_output() {
        let mut registry = Registry::default();
        let now = Instant::now();
        assert!(registry
            .observe([1; 32], Event::Admitted { probe: false }, now, "local")
            .is_none());
        let first = registry
            .observe(
                [1; 32],
                Event::Completed {
                    probe: false,
                    hold_status: Some(429),
                    valid: false,
                    timed_out: false,
                    elapsed_ms: 10,
                },
                now,
                "local",
            )
            .unwrap();
        assert_eq!(first["counts"]["wire_429"], 1);
        for _ in 0..100 {
            assert!(registry
                .observe(
                    [1; 32],
                    Event::Suppressed {
                        status: 429,
                        remaining_seconds: 30
                    },
                    now,
                    "local"
                )
                .is_none());
        }
        assert!(registry
            .observe(
                [1; 32],
                Event::Suppressed {
                    status: 429,
                    remaining_seconds: 1
                },
                now + Duration::from_secs(59),
                "local"
            )
            .is_none());
        let next = registry
            .observe(
                [1; 32],
                Event::Admitted { probe: true },
                now + LOG_INTERVAL,
                "local",
            )
            .unwrap();
        assert_eq!(next["counts"]["wire_429"], 1);
        assert_eq!(next["counts"]["admission_denied_429"], 101);
        assert_eq!(next["counts"]["probes_admitted"], 1);
        assert_eq!(next["last_admission_retry_seconds"], 1);
    }

    #[test]
    fn recovery_outcomes_and_503_are_not_counted_as_wire_429() {
        let mut summary = Summary::default();
        let now = Instant::now();
        for (i, (valid, timed_out)) in [(true, false), (false, false), (false, true)]
            .into_iter()
            .enumerate()
        {
            summary.observe(
                Event::Completed {
                    probe: true,
                    hold_status: None,
                    valid,
                    timed_out,
                    elapsed_ms: (i as u64 + 1) * 10,
                },
                now,
                1,
                "local",
            );
        }
        summary.observe(
            Event::Canceled {
                probe: true,
                elapsed_ms: 40,
            },
            now,
            1,
            "local",
        );
        let output = summary
            .observe(
                Event::Completed {
                    probe: false,
                    hold_status: Some(503),
                    valid: false,
                    timed_out: false,
                    elapsed_ms: 2,
                },
                now + LOG_INTERVAL,
                1,
                "local",
            )
            .unwrap();
        assert_eq!(output["counts"]["wire_429"], 0);
        assert_eq!(output["counts"]["wire_503_retry_after"], 1);
        for outcome in [
            "probes_valid",
            "probes_failed",
            "probes_timed_out",
            "probes_canceled",
        ] {
            assert_eq!(output["counts"][outcome], 1);
        }
        assert_eq!(output["last_probe_elapsed_ms"], 40);
        assert_eq!(output["max_probe_elapsed_ms"], 40);
        assert_eq!(output["last_probe_outcome"], "canceled");
    }

    #[test]
    fn cardinality_overflow_and_sensitive_input_do_not_expand_the_log_schema() {
        let mut registry = Registry::default();
        let now = Instant::now();
        for n in 0..MAX_ENDPOINTS {
            let mut key = [0; 32];
            key[..8].copy_from_slice(&(n as u64).to_le_bytes());
            let output = registry
                .observe(
                    key,
                    Event::Suppressed {
                        status: 503,
                        remaining_seconds: u64::MAX,
                    },
                    now,
                    "https://private.invalid/?key=secret",
                )
                .unwrap();
            assert_eq!(output["endpoint_slot"], n + 1);
            assert_eq!(output["revision"], "local");
            let fields: std::collections::BTreeSet<_> = output
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                fields,
                [
                    "schema",
                    "event",
                    "level",
                    "component",
                    "action",
                    "revision",
                    "endpoint_slot",
                    "overflow_aggregate",
                    "observation_age_ms",
                    "counts",
                    "last_admission_retry_seconds",
                    "last_probe_elapsed_ms",
                    "max_probe_elapsed_ms",
                    "last_probe_outcome",
                    "evidence_boundary"
                ]
                .into_iter()
                .collect()
            );
            let text = output.to_string();
            assert!(!text.contains("private.invalid") && !text.contains("secret"));
        }
        let overflow = registry
            .observe(
                [255; 32],
                Event::Suppressed {
                    status: 429,
                    remaining_seconds: 1,
                },
                now,
                "local",
            )
            .unwrap();
        assert_eq!(overflow["endpoint_slot"], 0);
        assert_eq!(overflow["overflow_aggregate"], true);
        assert!(registry
            .observe(
                [254; 32],
                Event::Suppressed {
                    status: 429,
                    remaining_seconds: 1
                },
                now,
                "local"
            )
            .is_none());
        assert_eq!(registry.entries.len(), MAX_ENDPOINTS);
    }

    #[test]
    fn counters_saturate_instead_of_wrapping_or_panicking() {
        let mut summary = Summary::default();
        summary.counts.admission_denied_429 = u64::MAX;
        let output = summary
            .observe(
                Event::Suppressed {
                    status: 429,
                    remaining_seconds: u64::MAX,
                },
                Instant::now(),
                1,
                "local",
            )
            .unwrap();
        assert_eq!(output["counts"]["admission_denied_429"], u64::MAX);
        assert_eq!(output["last_admission_retry_seconds"], u64::MAX);
    }
}
