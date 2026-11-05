//! One bounded, read-only legacy certification attempt. No durable partial progress.
use chain_base::ChainBaseError;
use std::{future::Future, time::Duration};
use tokio::time::Instant;

#[derive(Clone, Copy)]
pub(super) struct BootstrapReadPolicy {
    pub duration: Duration,
    pub interval: Duration,
    pub retry_delay: Duration,
    pub max_retry_delay: Duration,
    pub max_calls: usize,
    pub attempts_per_read: usize,
}

impl Default for BootstrapReadPolicy {
    fn default() -> Self {
        Self {
            duration: Duration::from_secs(20 * 60),
            interval: Duration::from_secs(1),
            retry_delay: Duration::from_secs(5),
            max_retry_delay: Duration::from_secs(60),
            max_calls: 2_000,
            attempts_per_read: 6,
        }
    }
}

pub(super) fn stopped(reason: &str) -> anyhow::Error {
    // A fresh error, not a wrapped transient cause: the outer indexer must keep
    // the child alive in its operator hold, not retry/restart a new full replay.
    anyhow::anyhow!(
        "legacy canonical bootstrap stopped; operator reconciliation required: {reason}"
    )
}

pub(super) struct BootstrapReads {
    policy: BootstrapReadPolicy,
    deadline: Instant,
    next_start: Instant,
    calls: usize,
}

impl BootstrapReads {
    pub fn new(policy: BootstrapReadPolicy) -> Self {
        let now = Instant::now();
        Self {
            policy,
            deadline: now + policy.duration,
            next_start: now,
            calls: 0,
        }
    }

    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// The caller MUST use a single-wire-attempt transport. Count admissions
    /// even when the existing shared provider hold prevents a network request.
    /// Reinvoke the same immutable request; earlier successful reads stay in
    /// caller memory only. Dropping this future cancels waits and in-flight I/O.
    pub async fn run<T, F, Fut>(&mut self, mut read: F) -> anyhow::Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, ChainBaseError>>,
    {
        for attempt in 0..self.policy.attempts_per_read {
            if self.calls >= self.policy.max_calls {
                return Err(stopped("read admission budget exhausted"));
            }
            if self.next_start >= self.deadline || Instant::now() >= self.deadline {
                return Err(stopped("absolute observation deadline exhausted"));
            }
            if self.next_start > Instant::now() {
                tokio::time::sleep_until(self.next_start).await;
            }
            if Instant::now() >= self.deadline {
                return Err(stopped("absolute observation deadline exhausted"));
            }
            self.calls += 1;
            self.next_start = Instant::now() + self.policy.interval;
            let result = tokio::time::timeout_at(self.deadline, read()).await;
            // Existing shared admission/pacing may itself delay dispatch. Space
            // from completion as well, so that delay cannot collapse wire gaps.
            self.next_start = Instant::now() + self.policy.interval;
            let error = match result {
                Ok(Ok(value)) => return Ok(value),
                Ok(Err(error)) => error,
                Err(_) => return Err(stopped("absolute observation deadline exhausted")),
            };
            let provider_delay = match &error {
                ChainBaseError::RpcRetryAfter {
                    status: 429 | 503,
                    retry_after_seconds,
                } => Some(Duration::from_secs(*retry_after_seconds)),
                ChainBaseError::RpcTransport(_)
                | ChainBaseError::RpcHttpStatus(429 | 502 | 503 | 504)
                | ChainBaseError::RpcProviderError { code: -32016, .. } => Some(Duration::ZERO),
                // Invalid envelopes, IDs, hashes, ABI data, unclassified errors
                // and unsupported requests never become a retryable verdict.
                _ => None,
            };
            let Some(provider_delay) = provider_delay else {
                return Err(error.into());
            };
            if attempt + 1 >= self.policy.attempts_per_read {
                return Err(stopped("attempt budget for one immutable read exhausted"));
            }
            let backoff = self
                .policy
                .retry_delay
                .saturating_mul(1u32 << attempt.min(31))
                .min(self.policy.max_retry_delay)
                .max(provider_delay);
            let Some(resume) = Instant::now().checked_add(backoff) else {
                return Err(stopped("provider delay exceeds observation budget"));
            };
            // Never shorten Retry-After to fit. There is no next request when
            // the provider's wait cannot fit the original absolute deadline.
            if resume >= self.deadline {
                return Err(stopped(&format!(
                    "provider/retry delay exceeds observation budget (provider delay {} seconds)",
                    provider_delay.as_secs()
                )));
            }
            self.next_start = self.next_start.max(resume);
        }
        Err(stopped("attempt budget for one immutable read exhausted"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        indexer_error_is_retryable, IndexerRecoveryDecision, IndexerRecoveryPolicy,
        IndexerRecoveryState,
    };
    use std::sync::{Arc, Mutex};

    fn policy() -> BootstrapReadPolicy {
        BootstrapReadPolicy {
            duration: Duration::from_secs(2),
            interval: Duration::from_millis(2),
            retry_delay: Duration::from_millis(2),
            max_retry_delay: Duration::from_millis(8),
            max_calls: 2_000,
            attempts_per_read: 6,
        }
    }

    fn assert_operator_hold(error: anyhow::Error) {
        // The running child remains alive in main's existing halt branch, so
        // wait-n does not restart its healthy sibling readers or reset budgets.
        let error = stopped(&error.to_string());
        assert!(!indexer_error_is_retryable(&error));
        let mut state = IndexerRecoveryState::default();
        for _ in 0..10 {
            assert!(matches!(
                state.on_error(&IndexerRecoveryPolicy::default(), &error),
                IndexerRecoveryDecision::HaltForOperatorInvestigation { .. }
            ));
        }
    }

    #[tokio::test]
    async fn throttled_middle_block_keeps_prefix_and_exact_request() {
        let mut reads = BootstrapReads::new(policy());
        let calls = Arc::new(Mutex::new(Vec::new()));
        for block in [10, 20, 30] {
            let mut attempts = 0;
            reads
                .run(|| {
                    calls.lock().unwrap().push(block);
                    attempts += 1;
                    let response = if block == 20 && attempts < 3 {
                        Err(ChainBaseError::RpcRetryAfter {
                            status: 429,
                            retry_after_seconds: 0,
                        })
                    } else {
                        Ok(block)
                    };
                    std::future::ready(response)
                })
                .await
                .unwrap();
        }
        assert_eq!(*calls.lock().unwrap(), vec![10, 20, 20, 20, 30]);
        assert_eq!(reads.calls, 5);
    }

    #[tokio::test]
    async fn pace_every_attempt_including_successes_and_retries() {
        let mut reads = BootstrapReads::new(BootstrapReadPolicy {
            interval: Duration::from_millis(15),
            ..policy()
        });
        let starts = Arc::new(Mutex::new(Vec::new()));
        for _ in 0..3 {
            reads
                .run(|| {
                    starts.lock().unwrap().push(Instant::now());
                    std::future::ready(Ok(()))
                })
                .await
                .unwrap();
        }
        for window in starts.lock().unwrap().windows(2) {
            assert!(window[1].duration_since(window[0]) >= Duration::from_millis(15));
        }
    }

    #[tokio::test]
    async fn delayed_dispatch_cannot_collapse_next_wire_gap() {
        let mut reads = BootstrapReads::new(BootstrapReadPolicy {
            interval: Duration::from_millis(15),
            ..policy()
        });
        reads
            .run(|| async {
                tokio::time::sleep(Duration::from_millis(35)).await;
                Ok(())
            })
            .await
            .unwrap();
        let completed = Instant::now();
        reads.run(|| std::future::ready(Ok(()))).await.unwrap();
        assert!(Instant::now().duration_since(completed) >= Duration::from_millis(15));
    }

    #[tokio::test]
    async fn six_attempts_stop_without_outer_restart() {
        let mut reads = BootstrapReads::new(policy());
        let error = reads
            .run(|| std::future::ready::<Result<(), _>>(Err(ChainBaseError::RpcHttpStatus(429))))
            .await
            .unwrap_err();
        assert_eq!(reads.calls, 6);
        assert!(error.to_string().contains("attempt budget"));
        assert_operator_hold(error);
    }

    #[tokio::test]
    async fn global_budget_is_not_reset_between_successful_reads() {
        let mut reads = BootstrapReads::new(BootstrapReadPolicy {
            interval: Duration::ZERO,
            duration: Duration::from_secs(30),
            ..policy()
        });
        for _ in 0..2_000 {
            reads.run(|| std::future::ready(Ok(()))).await.unwrap();
        }
        let error = reads
            .run(|| async {
                panic!("no call beyond total budget");
                #[allow(unreachable_code)]
                Ok(())
            })
            .await
            .unwrap_err();
        assert_eq!(reads.calls, 2_000);
        assert_operator_hold(error);
    }

    #[tokio::test]
    async fn retry_after_is_not_shortened_or_restarted() {
        let mut reads = BootstrapReads::new(BootstrapReadPolicy {
            duration: Duration::from_millis(30),
            ..policy()
        });
        let error = reads
            .run(|| {
                std::future::ready::<Result<(), _>>(Err(ChainBaseError::RpcRetryAfter {
                    status: 429,
                    retry_after_seconds: 60,
                }))
            })
            .await
            .unwrap_err();
        assert_eq!(reads.calls, 1);
        assert!(error.to_string().contains("60 seconds"));
        assert_operator_hold(error);
    }

    #[tokio::test]
    async fn honors_retry_after_before_retrying_same_read() {
        let mut reads = BootstrapReads::new(policy());
        let mut starts = Vec::new();
        reads
            .run(|| {
                starts.push(Instant::now());
                std::future::ready(if starts.len() == 1 {
                    Err(ChainBaseError::RpcRetryAfter {
                        status: 503,
                        retry_after_seconds: 1,
                    })
                } else {
                    Ok(())
                })
            })
            .await
            .unwrap();
        assert!(starts[1].duration_since(starts[0]) >= Duration::from_secs(1));
    }

    #[tokio::test]
    async fn one_deadline_bounds_hanging_read_and_remaining_reads() {
        let mut reads = BootstrapReads::new(BootstrapReadPolicy {
            duration: Duration::from_millis(30),
            ..policy()
        });
        reads.run(|| std::future::ready(Ok(()))).await.unwrap();
        let deadline = reads.deadline();
        let error = reads
            .run(|| std::future::pending::<Result<(), ChainBaseError>>())
            .await
            .unwrap_err();
        assert_eq!(reads.deadline(), deadline);
        assert_eq!(reads.calls, 2);
        assert_operator_hold(error);
        assert!(reads
            .run(|| async {
                panic!("deadline must not reset");
                #[allow(unreachable_code)]
                Ok(())
            })
            .await
            .is_err());
        assert_eq!(reads.calls, 2);
    }

    #[tokio::test]
    async fn malformed_and_unclassified_errors_are_never_retried() {
        for error in [
            ChainBaseError::InvalidRpcResponse("missing ID".into()),
            ChainBaseError::RpcHttpStatus(403),
            ChainBaseError::RpcProviderError {
                code: -32602,
                message: "invalid parameters".into(),
            },
        ] {
            let mut reads = BootstrapReads::new(policy());
            let mut once = Some(error);
            let error = reads
                .run(|| {
                    std::future::ready::<Result<(), _>>(Err(once.take().expect("must not retry")))
                })
                .await
                .unwrap_err();
            assert_eq!(reads.calls, 1);
            assert_operator_hold(error);
        }
    }

    #[tokio::test]
    async fn cancellation_drops_pending_read_without_replaying() {
        struct DropFlag(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = dropped.clone();
        let task = tokio::spawn(async move {
            let mut reads = BootstrapReads::new(policy());
            reads
                .run(|| {
                    let guard = DropFlag(flag.clone());
                    async move {
                        let _guard = guard;
                        std::future::pending::<Result<(), ChainBaseError>>().await
                    }
                })
                .await
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
    }
}
