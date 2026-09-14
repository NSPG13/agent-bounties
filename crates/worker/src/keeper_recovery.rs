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
                let retryable = keeper_error_is_retryable(&error);
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
                        "evidence_boundary": "Each retry re-reads safe canonical projections; transaction submission errors halt for investigation. Only safe canonical V2 events prove outcomes."
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

fn keeper_error_is_retryable(error: &anyhow::Error) -> bool {
    let Some(chain_base::ChainBaseError::RelayerProvider(message)) =
        error.downcast_ref::<chain_base::ChainBaseError>()
    else {
        return indexer_error_is_retryable(error);
    };
    // The relayer preserves only the sanitized Display text of Alloy errors.
    // Recognize the pinned HTTP/JSON-RPC transport forms, not arbitrary words
    // in a provider body. Invalid URLs, null/malformed responses, local usage,
    // rejected transactions and other unknown errors require investigation.
    // Submission-stage errors have a distinct prefix and cannot match these
    // pre-send forms: the keeper has no pending-transaction reconciliation.
    if let Some(status) = message
        .strip_prefix("HTTP error ")
        .and_then(|value| value.split_once(" with "))
        .and_then(|(status, _)| status.parse::<u16>().ok())
    {
        return matches!(status, 408 | 429 | 500 | 502 | 503 | 504);
    }
    if let Some(code) = message
        .strip_prefix("server returned an error response: error code ")
        .and_then(|value| value.split_once(": "))
        .and_then(|(code, _)| code.parse::<i64>().ok())
    {
        return matches!(code, 429 | -32005 | -32055);
    }
    ["error sending request", "request or response body error"]
        .iter()
        .any(|prefix| {
            message == prefix
                || message
                    .strip_prefix(prefix)
                    .is_some_and(|suffix| suffix.starts_with(" for url ("))
        })
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
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

    async fn actionable_relayer_error(
        failure_method: &'static str,
        failure_status: u16,
        failure_result: serde_json::Value,
    ) -> ChainBaseError {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let rpc_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut methods = Vec::new();
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let request = loop {
                    let mut buffer = [0_u8; 4096];
                    let count = stream.read(&mut buffer).await.unwrap();
                    assert!(count > 0, "incomplete fixture request");
                    bytes.extend_from_slice(&buffer[..count]);
                    assert!(bytes.len() < 65_536, "oversized fixture request");
                    let Some(header_end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") else {
                        continue;
                    };
                    let header = std::str::from_utf8(&bytes[..header_end]).unwrap();
                    let length = header
                        .lines()
                        .filter_map(|line| line.split_once(':'))
                        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .unwrap()
                        .1
                        .trim()
                        .parse::<usize>()
                        .unwrap();
                    if bytes.len() >= header_end + 4 + length {
                        break serde_json::from_slice::<serde_json::Value>(
                            &bytes[header_end + 4..header_end + 4 + length],
                        )
                        .unwrap();
                    }
                };
                let method = request["method"].as_str().unwrap();
                assert!(
                    !method.starts_with("eth_send")
                        || (method == "eth_sendRawTransaction" && failure_method == method),
                    "only the submission-failure fixture may reach the local send stub"
                );
                methods.push(method.to_string());
                let failed = method == failure_method;
                let result = if failed {
                    failure_result.clone()
                } else {
                    match method {
                        "eth_chainId" => serde_json::json!("0x2105"),
                        "eth_call" => serde_json::json!("0x"),
                        "eth_estimateGas" => serde_json::json!("0x5208"),
                        "eth_feeHistory" => serde_json::json!({
                            "oldestBlock": "0x1", "baseFeePerGas": ["0x1", "0x1"],
                            "gasUsedRatio": [0.5], "reward": [["0x1"]]
                        }),
                        "eth_getBalance" => serde_json::json!("0x1000000000000000000"),
                        "eth_getTransactionCount" => serde_json::json!("0x0"),
                        _ => panic!("unexpected fixture method: {method}"),
                    }
                };
                let body = serde_json::json!({
                    "jsonrpc": "2.0", "id": request["id"], "result": result
                })
                .to_string();
                let status = if failed { failure_status } else { 200 };
                let response = format!(
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
                if failed {
                    return methods;
                }
            }
        });
        let error = fixture_relayer_error(&rpc_url).await;
        let methods = tokio::time::timeout(std::time::Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(methods.last().unwrap(), failure_method);
        assert_eq!(
            methods
                .iter()
                .filter(|method| method.starts_with("eth_send"))
                .count(),
            usize::from(failure_method == "eth_sendRawTransaction")
        );
        error
    }

    async fn fixture_relayer_error(rpc_url: &str) -> ChainBaseError {
        // Synthetic local signer. The fixture never forwards any RPC call.
        let relayer =
            chain_base::BaseTransactionRelayer::from_private_key(&format!("0x{}", "11".repeat(32)))
                .unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            relayer.simulate_and_broadcast(
                rpc_url,
                8453,
                &chain_base::EvmTransactionIntent {
                    from: Some(relayer.address()),
                    to: relayer.address(),
                    value_wei: 0,
                    data: "0x12345678".to_string(),
                    function: "keeperFixture()".to_string(),
                },
                100_000,
                100_000_000_000,
            ),
        )
        .await
        .unwrap()
        .unwrap_err()
    }

    #[tokio::test]
    async fn actionable_relayer_connection_failure_retries_without_leaking_url() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let rpc_url = format!("http://{}/fixture-secret", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            // Close the connection during a real read, without returning JSON.
            let (stream, _) = listener.accept().await.unwrap();
            drop(stream);
        });
        let error = fixture_relayer_error(&rpc_url).await;
        server.await.unwrap();
        assert!(
            matches!(&error, ChainBaseError::RelayerProvider(message) if message.starts_with("error sending request"))
        );
        assert!(!error.to_string().contains("fixture-secret"));
        let trace = scripted_run(
            vec![Err(anyhow::Error::new(error)), Ok("idle")],
            false,
            5,
            2,
        )
        .await;
        assert!(trace.result.is_ok());
        assert_eq!(trace.polls_at, [0, 5]);
        assert_eq!(trace.waits, [5, 5]);
    }

    #[tokio::test]
    async fn actionable_relayer_transport_failures_back_off_then_poll_again() {
        for method in [
            "eth_chainId",
            "eth_estimateGas",
            "eth_feeHistory",
            "eth_getBalance",
        ] {
            let error = actionable_relayer_error(method, 429, serde_json::Value::Null).await;
            assert!(
                matches!(&error, ChainBaseError::RelayerProvider(message) if message.starts_with("HTTP error 429 with "))
            );
            let trace = scripted_run(
                vec![
                    Err(anyhow::Error::new(error).context("actionable keeper poll")),
                    Ok("idle"),
                ],
                false,
                5,
                2,
            )
            .await;
            assert!(trace.result.is_ok());
            assert_eq!(trace.polls_at, [0, 5], "{method}");
            assert_eq!(trace.waits, [5, 5], "{method}");
        }
    }

    #[tokio::test]
    async fn actionable_relayer_malformed_response_halts_without_retry() {
        let error =
            actionable_relayer_error("eth_estimateGas", 200, serde_json::json!("not a quantity"))
                .await;
        assert!(
            matches!(&error, ChainBaseError::RelayerProvider(message) if message.starts_with("deserialization error:"))
        );
        let trace = scripted_run(vec![Err(anyhow::Error::new(error))], false, 5, 1).await;
        assert!(trace.result.is_ok());
        assert_eq!(trace.polls_at, [0]);
        assert_eq!(trace.waits, [86_400]);
    }

    #[tokio::test]
    async fn ambiguous_submission_and_filler_failures_halt_without_another_poll() {
        for method in ["eth_getTransactionCount", "eth_sendRawTransaction"] {
            let error = actionable_relayer_error(method, 503, serde_json::Value::Null).await;
            assert!(
                matches!(&error, ChainBaseError::RelayerProvider(message) if message.starts_with("transaction submission failed: HTTP error 503 with "))
            );
            let trace = scripted_run(vec![Err(anyhow::Error::new(error))], false, 5, 1).await;
            assert!(trace.result.is_ok());
            assert_eq!(trace.polls_at, [0], "{method}");
            assert_eq!(trace.waits, [86_400], "{method}");
        }
    }

    #[tokio::test]
    async fn relayer_failures_exhaust_the_same_bounded_budget() {
        let trace = scripted_run(
            (0..8)
                .map(|_| {
                    Err(anyhow::Error::new(ChainBaseError::RelayerProvider(
                        "error sending request for url ([redacted-url])".to_string(),
                    )))
                })
                .collect(),
            false,
            5,
            usize::MAX,
        )
        .await;
        assert!(trace.result.unwrap_err().to_string().contains("exhausted"));
        assert_eq!(trace.waits, [5, 8, 15, 30, 60, 90, 90]);
        assert_eq!(trace.polls_at, [0, 5, 13, 28, 58, 118, 208, 298]);
    }

    #[tokio::test]
    async fn permanent_relayer_failures_halt_without_another_poll() {
        for error in [
            ChainBaseError::RelayerProvider("configured RPC URL is invalid".to_string()),
            ChainBaseError::RelayerProvider("HTTP error 401 with body: retry HTTP error 429".to_string()),
            ChainBaseError::RelayerProvider("server returned a null response when a non-null response was expected".to_string()),
            ChainBaseError::RelayerProvider("server returned an error response: error code 3: execution reverted, data: HTTP error 429".to_string()),
            ChainBaseError::RelayerProvider("local usage error: invalid signature".to_string()),
            ChainBaseError::RelayerSimulation("HTTP error 429 with empty body".to_string()),
            ChainBaseError::RelayerChainMismatch { expected: 8453, observed: 1 },
            ChainBaseError::RelayerGasLimitExceeded { estimated: 2, maximum: 1 },
            ChainBaseError::RelayerFeeCapExceeded { estimated: 2, maximum: 1 },
            ChainBaseError::RelayerInsufficientBalance { balance: 1, required: 2 },
            ChainBaseError::InvalidRelayIntent("invalid action".to_string()),
        ] {
            let trace = scripted_run(vec![Err(anyhow::Error::new(error))], false, 5, 1).await;
            assert!(trace.result.is_ok());
            assert_eq!(trace.polls_at, [0]);
            assert_eq!(trace.waits, [86_400]);
        }
    }

    #[test]
    fn relayer_classifier_accepts_only_known_transport_forms() {
        for (message, expected) in [
            ("HTTP error 408 with empty body", true),
            ("HTTP error 429 with empty body", true),
            ("HTTP error 500 with empty body", true),
            ("HTTP error 502 with empty body", true),
            ("HTTP error 503 with empty body", true),
            ("HTTP error 504 with empty body", true),
            ("HTTP error 501 with empty body", false),
            ("HTTP error 403 with body: too many requests", false),
            (
                "server returned an error response: error code 429: too many requests",
                true,
            ),
            (
                "server returned an error response: error code -32005: limit exceeded",
                true,
            ),
            (
                "server returned an error response: error code -32055: temporarily unavailable",
                true,
            ),
            (
                "server returned an error response: error code -32602: invalid params",
                false,
            ),
            ("error sending request for url ([redacted-url])", true),
            ("request or response body error", true),
            ("deserialization error: expected quantity", false),
            ("error decoding response body", false),
            ("builder error", false),
            ("unknown error", false),
            (
                "transaction submission failed: HTTP error 429 with empty body",
                false,
            ),
            (
                "transaction submission failed: error sending request for url ([redacted-url])",
                false,
            ),
        ] {
            let error = anyhow::Error::new(ChainBaseError::RelayerProvider(message.to_string()));
            assert_eq!(keeper_error_is_retryable(&error), expected, "{message}");
            assert!(
                !indexer_error_is_retryable(&error),
                "shared classifier changed"
            );
        }
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
