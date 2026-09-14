use serde::Serialize;
use std::future::Future;
use worker::{
    indexer_error_is_retryable, redact_operational_error, IndexerRecoveryDecision,
    IndexerRecoveryPolicy,
};

// Inject the poll and interruptible clock so failure recovery can be exercised
// without a database, RPC endpoint, signer, or real sleeps.
pub(super) async fn run<P, PF, R, W, WF, J>(
    once: bool,
    poll_seconds: u64,
    policy: IndexerRecoveryPolicy,
    mut poll: P,
    mut wait_or_shutdown: W,
    mut random: J,
) -> anyhow::Result<()>
where
    P: FnMut() -> PF,
    PF: Future<Output = anyhow::Result<R>>,
    R: Serialize,
    W: FnMut(u64) -> WF,
    WF: Future<Output = anyhow::Result<bool>>,
    J: FnMut() -> u64,
{
    let revision = std::env::var("RENDER_GIT_COMMIT")
        .ok()
        .filter(|value| {
            value.len() == 40 && value.chars().all(|character| character.is_ascii_hexdigit())
        })
        .unwrap_or_else(|| "local".to_string());
    let mut consecutive_failures = 0_u32;
    loop {
        let wait_seconds = match poll().await {
            Ok(report) => {
                consecutive_failures = 0;
                println!("{}", serde_json::to_string(&report)?);
                if once {
                    return Ok(());
                }
                poll_seconds
            }
            Err(error) => {
                consecutive_failures = consecutive_failures.saturating_add(1);
                let retryable = indexer_error_is_retryable(&error);
                let decision = policy.decision(consecutive_failures, retryable);
                let (action, backoff_seconds) = if once {
                    ("exit_after_failed_poll", None)
                } else {
                    match decision {
                        IndexerRecoveryDecision::RetryFromPersistedCursor {
                            backoff_seconds,
                            ..
                        } => (
                            "retry_from_safe_projection",
                            Some(jittered_backoff_seconds(
                                backoff_seconds,
                                poll_seconds.max(policy.initial_backoff_seconds),
                                random(),
                            )),
                        ),
                        IndexerRecoveryDecision::ExitForSupervisorRestart { .. } => {
                            ("exit_for_supervisor_restart", None)
                        }
                        IndexerRecoveryDecision::HaltForOperatorInvestigation { .. } => {
                            ("halt_for_operator_investigation", None)
                        }
                    }
                };
                eprintln!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "schema": "agent-bounties/open-competition-v2-keeper-recovery-v1",
                        "component": "open-competition-v2-keeper",
                        "revision": revision,
                        "error": redact_operational_error(&error.to_string()),
                        "decision": action,
                        "consecutive_failures": consecutive_failures,
                        "retryable": retryable,
                        "backoff_seconds": backoff_seconds,
                        "evidence_boundary": "Keeper retries are permissionless and idempotent at the contract state machine; only safe canonical V2 events prove outcomes."
                    }))?
                );
                // Do not propagate the raw provider error: main would print it
                // again without the URL redaction applied to the recovery log.
                if once {
                    anyhow::bail!("Open Competition V2 keeper poll failed");
                }
                if let Some(backoff_seconds) = backoff_seconds {
                    backoff_seconds
                } else if matches!(
                    decision,
                    IndexerRecoveryDecision::ExitForSupervisorRestart { .. }
                ) {
                    anyhow::bail!("Open Competition V2 keeper exhausted its recovery budget");
                } else {
                    // Match indexer integrity containment: wait for an operator
                    // without another poll or a supervisor restart loop.
                    loop {
                        if wait_or_shutdown(86_400).await? {
                            return Ok(());
                        }
                    }
                }
            }
        };
        if wait_or_shutdown(wait_seconds).await? {
            return Ok(());
        }
    }
}

fn jittered_backoff_seconds(backoff_seconds: u64, minimum_seconds: u64, random: u64) -> u64 {
    // Keep jitter within the upper quarter of the exponential delay, including
    // at the cap. Never retry faster than the healthy poll or initial backoff.
    let upper = backoff_seconds.max(minimum_seconds).max(1);
    let lower = (backoff_seconds - backoff_seconds / 4)
        .max(minimum_seconds)
        .max(1);
    lower + random % (upper - lower + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chain_base::ChainBaseError;
    use std::{
        cell::{Cell, RefCell},
        collections::VecDeque,
        future::ready,
    };

    struct Trace {
        result: anyhow::Result<()>,
        polls_at: Vec<u64>,
        waits: Vec<u64>,
    }

    async fn scripted_run(
        outcomes: Vec<anyhow::Result<&'static str>>,
        once: bool,
        poll_seconds: u64,
        shutdown_after_waits: usize,
    ) -> Trace {
        let now = Cell::new(0_u64);
        let polls_at = RefCell::new(Vec::new());
        let waits = RefCell::new(Vec::new());
        let mut outcomes = VecDeque::from(outcomes);
        let result = run(
            once,
            poll_seconds,
            IndexerRecoveryPolicy::default(),
            || {
                polls_at.borrow_mut().push(now.get());
                ready(
                    outcomes
                        .pop_front()
                        .expect("unexpected additional keeper poll"),
                )
            },
            |seconds| {
                now.set(now.get() + seconds);
                let mut waits = waits.borrow_mut();
                waits.push(seconds);
                ready(Ok(waits.len() >= shutdown_after_waits))
            },
            || 0,
        )
        .await;
        assert!(
            outcomes.is_empty(),
            "keeper stopped before the expected poll"
        );
        Trace {
            result,
            polls_at: polls_at.into_inner(),
            waits: waits.into_inner(),
        }
    }

    fn throttled() -> anyhow::Result<&'static str> {
        Err(anyhow::Error::new(ChainBaseError::RpcHttpStatus(429)))
    }

    #[tokio::test]
    async fn repeated_429s_back_off_and_exhaust_the_existing_budget() {
        let trace = scripted_run((0..8).map(|_| throttled()).collect(), false, 5, usize::MAX).await;

        assert!(trace.result.unwrap_err().to_string().contains("exhausted"));
        assert_eq!(trace.waits, [5, 8, 15, 30, 60, 90, 90]);
        assert_eq!(trace.polls_at, [0, 5, 13, 28, 58, 118, 208, 298]);
    }

    #[tokio::test]
    async fn success_restores_normal_polling_and_resets_the_retry_budget() {
        let trace = scripted_run(
            vec![
                throttled(),
                throttled(),
                Ok("idle"),
                throttled(),
                Ok("idle"),
            ],
            false,
            5,
            5,
        )
        .await;

        assert!(trace.result.is_ok());
        assert_eq!(trace.waits, [5, 8, 5, 5, 5]);
        assert_eq!(trace.polls_at, [0, 5, 13, 18, 23]);
    }

    #[tokio::test]
    async fn retries_never_accelerate_a_slower_healthy_poll() {
        let trace = scripted_run(vec![throttled(), throttled()], false, 60, 2).await;

        assert!(trace.result.is_ok());
        assert_eq!(trace.waits, [60, 60]);
        assert_eq!(trace.polls_at, [0, 60]);
    }

    #[tokio::test]
    async fn once_mode_returns_failure_without_retry_or_raw_provider_credentials() {
        let error = anyhow::Error::new(ChainBaseError::RpcTransport(
            "https://rpc.invalid/provider-secret".to_string(),
        ));
        let trace = scripted_run(vec![Err(error)], true, 5, usize::MAX).await;

        assert_eq!(
            trace.result.unwrap_err().to_string(),
            "Open Competition V2 keeper poll failed"
        );
        assert_eq!(trace.polls_at, [0]);
        assert!(trace.waits.is_empty());
    }

    #[tokio::test]
    async fn once_mode_success_does_not_sleep() {
        let trace = scripted_run(vec![Ok("idle")], true, 5, usize::MAX).await;

        assert!(trace.result.is_ok());
        assert_eq!(trace.polls_at, [0]);
        assert!(trace.waits.is_empty());
    }

    #[tokio::test]
    async fn malformed_safe_block_response_halts_without_another_poll() {
        let error = anyhow::Error::new(ChainBaseError::InvalidRpcResponse(
            "missing safe block".to_string(),
        ));
        let trace = scripted_run(vec![Err(error)], false, 5, 2).await;

        assert!(trace.result.is_ok());
        assert_eq!(trace.polls_at, [0]);
        assert_eq!(trace.waits, [86_400, 86_400]);
    }

    #[tokio::test]
    async fn unclassified_integrity_failure_halts_without_another_poll() {
        let trace = scripted_run(vec![Err(anyhow::anyhow!("factory mismatch"))], false, 5, 1).await;

        assert!(trace.result.is_ok());
        assert_eq!(trace.polls_at, [0]);
        assert_eq!(trace.waits, [86_400]);
    }

    #[tokio::test]
    async fn shutdown_during_backoff_prevents_another_poll() {
        let trace = scripted_run(vec![throttled()], false, 5, 1).await;

        assert!(trace.result.is_ok());
        assert_eq!(trace.polls_at, [0]);
        assert_eq!(trace.waits, [5]);
    }

    #[test]
    fn jitter_has_both_endpoints_without_exceeding_the_cap_or_minimum() {
        for (base, minimum, low, high) in [
            (5, 5, 5, 5),
            (10, 5, 8, 10),
            (120, 5, 90, 120),
            (5, 60, 60, 60),
            (1, 1, 1, 1),
            (u64::MAX, 5, u64::MAX - u64::MAX / 4, u64::MAX),
        ] {
            assert_eq!(jittered_backoff_seconds(base, minimum, 0), low);
            assert_eq!(jittered_backoff_seconds(base, minimum, high - low), high);
            for random in [0, 1, 31, 255, u64::MAX] {
                assert!((low..=high).contains(&jittered_backoff_seconds(base, minimum, random)));
            }
        }
    }
}
