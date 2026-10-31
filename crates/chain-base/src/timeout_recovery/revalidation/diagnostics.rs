//! Fixed private failure metadata; never RPC bodies, URLs or transaction bytes.
use super::*;
use std::{future::Future, io::Write};

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Phase {
    ChainIdentity,
    SafeHead,
    SafeBlock,
    SafeScope,
    SafeCode,
    LatestHead,
    LatestCode,
    LatestScope,
    PendingScope,
    TransactionAbsence,
    Simulation,
    GasEstimate,
    FeeBounds,
    Balance,
    SafeBlockRecheck,
    LatestBlockRecheck,
    FinalTransactionAbsence,
    ReadDeadline,
}

pub(super) fn sanitized(error: ChainBaseError) -> ChainBaseError {
    match error {
        ChainBaseError::RpcHttpStatus(status) => ChainBaseError::RpcHttpStatus(status),
        ChainBaseError::RpcRetryAfter {
            status,
            retry_after_seconds,
        } => ChainBaseError::RpcRetryAfter {
            status,
            retry_after_seconds,
        },
        ChainBaseError::RpcTransport(_) => {
            ChainBaseError::RpcTransport("timeout revalidation transport unavailable".into())
        }
        ChainBaseError::RpcProviderError { code, .. } => ChainBaseError::RpcProviderError {
            code,
            message: "timeout revalidation provider unavailable".into(),
        },
        ChainBaseError::InvalidRpcResponse(_) | ChainBaseError::InvalidRpcQuantity(_) => {
            ChainBaseError::InvalidRpcResponse("timeout revalidation response unavailable".into())
        }
        _ => unavailable(),
    }
}

fn event(phase: Phase, provider_slot: Option<u8>, error: &ChainBaseError) -> Value {
    let class = match error {
        ChainBaseError::RpcRetryAfter { status: 429, .. } | ChainBaseError::RpcHttpStatus(429) => {
            "rate_limited"
        }
        ChainBaseError::RpcRetryAfter { .. } | ChainBaseError::RpcHttpStatus(_) => "http_status",
        ChainBaseError::RpcTransport(_) => "transport",
        ChainBaseError::RpcProviderError { .. } => "provider",
        ChainBaseError::InvalidRpcResponse(_) | ChainBaseError::InvalidRpcQuantity(_) => {
            "malformed_response"
        }
        _ => "state_or_policy_unavailable",
    };
    json!({"schema":"agent-bounties/timeout-revalidation-failure-v1",
        "event":"timeout_revalidation_failure", "phase":phase,
        "provider_slot":provider_slot, "failure_class":class,
        "evidence_boundary":"Read-only validation failure; no proof of broadcast, recovery or permission to retry."})
}

pub(super) async fn checked<T>(
    phase: Phase,
    provider_slot: Option<u8>,
    operation: impl Future<Output = Result<T, ChainBaseError>>,
) -> Result<T, ChainBaseError> {
    operation.await.map_err(|error| {
        let safe = sanitized(error);
        // Diagnostic I/O cannot change the guarded result.
        let _ = writeln!(
            std::io::stderr().lock(),
            "{}",
            event(phase, provider_slot, &safe)
        );
        safe
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn revalidation_diagnostics_preserve_infrastructure_classes_and_remove_private_text() {
        let cases = [
            ChainBaseError::RpcHttpStatus(429),
            ChainBaseError::RpcRetryAfter {
                status: 503,
                retry_after_seconds: 37,
            },
            ChainBaseError::RpcTransport("https://private.invalid/key?secret=marker".into()),
            ChainBaseError::RpcProviderError {
                code: -32000,
                message: "secret wallet payload marker".into(),
            },
            ChainBaseError::InvalidRpcResponse("private response marker".into()),
        ];
        for (index, original) in cases.into_iter().enumerate() {
            let result = checked(Phase::PendingScope, Some(2), async {
                Err::<(), _>(original)
            })
            .await
            .unwrap_err();
            assert!(!result.to_string().contains("marker"));
            match index {
                0 => assert!(matches!(result, ChainBaseError::RpcHttpStatus(429))),
                1 => assert!(matches!(
                    result,
                    ChainBaseError::RpcRetryAfter {
                        status: 503,
                        retry_after_seconds: 37
                    }
                )),
                2 => assert!(matches!(result, ChainBaseError::RpcTransport(_))),
                3 => assert!(matches!(
                    result,
                    ChainBaseError::RpcProviderError { code: -32000, .. }
                )),
                _ => assert!(matches!(result, ChainBaseError::InvalidRpcResponse(_))),
            }
            let value = event(Phase::PendingScope, Some(2), &result);
            assert_eq!(value["phase"], "pending_scope");
            assert_eq!(value["provider_slot"], 2);
            assert!(!value.to_string().contains("marker"));
            assert_eq!(value.as_object().unwrap().len(), 6);
        }
        assert_eq!(
            checked(Phase::SafeCode, Some(1), async { Ok(17) })
                .await
                .unwrap(),
            17
        );
    }
}
