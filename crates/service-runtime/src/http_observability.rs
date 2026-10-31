//! Bounded server-error records. Callers may pass only router-matched templates.
//! Request/response bodies, headers, raw URLs and caller correlation IDs are not inputs.
use serde::Serialize;
use serde_json::{json, Value};
use std::io::Write;

#[cfg(test)]
#[path = "http_observability_diagnostic_tests.rs"]
mod diagnostic_tests;

#[derive(Clone, Copy)]
pub enum HttpService {
    Api,
    Mcp,
}

/// Fixed dependency-failure stages, including failures represented by HTTP 200.
/// There is deliberately no raw error, URL, request field or body input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureStage {
    ScopedAutonomousRead,
    ScopedOpenCompetitionRead,
    ScopedOpenCompetitionV2Read,
    LegacyStateLock,
    ClaimPlanCompetitionLookup,
    ClaimPlanIndexedLookup,
    AuthorizedClaimPlanCompetitionLookup,
    AuthorizedClaimPlanIndexedLookup,
    SigningNotConfigured,
    DurableStoreNotConfigured,
    DurableStoreCreate,
    RequestTimeout,
    RequestTransportUnavailable,
    ResponseBodyUnavailable,
    ResponseJsonInvalid,
    UpstreamHttpUnavailable,
    TimeoutIndexedLookup,
    TimeoutRelayerNotConfigured,
    TimeoutPlannerUnavailable,
    TimeoutDurableStoreNotConfigured,
    TimeoutRpcNotConfigured,
    TimeoutLeaseAcquireFailed,
    TimeoutLeaseBusy,
    TimeoutRelayExecutionFailed,
    TimeoutRelayExecutionTimedOut,
    TimeoutLeaseReleaseFailed,
    TimeoutConfirmationUnavailable,
    HeadChainIdRead,
    HeadSafeBlockRead,
    HeadSafeValidation,
    HeadLatestNumberRead,
    HeadLatestBlockRead,
    HeadLatestValidation,
    HeadRpcNotConfigured,
    MarketplaceObservationTimedOut,
    MarketplaceSnapshotTimedOut,
    MarketplaceHeadsTimedOut,
}

fn safe_revision(revision: &str) -> &str {
    if revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        revision
    } else {
        "local"
    }
}

pub fn failure_stage_event(stage: FailureStage, revision: &str) -> Value {
    let (component, action) = match stage {
        FailureStage::ScopedAutonomousRead
        | FailureStage::ScopedOpenCompetitionRead
        | FailureStage::ScopedOpenCompetitionV2Read
        | FailureStage::LegacyStateLock => ("api", "opportunity_projection"),
        FailureStage::ClaimPlanCompetitionLookup | FailureStage::ClaimPlanIndexedLookup => {
            ("api", "claim_plan")
        }
        FailureStage::AuthorizedClaimPlanCompetitionLookup
        | FailureStage::AuthorizedClaimPlanIndexedLookup => ("api", "authorized_claim_plan"),
        FailureStage::SigningNotConfigured
        | FailureStage::DurableStoreNotConfigured
        | FailureStage::DurableStoreCreate => ("api", "create_discovery_subscription"),
        FailureStage::RequestTimeout
        | FailureStage::RequestTransportUnavailable
        | FailureStage::ResponseBodyUnavailable
        | FailureStage::ResponseJsonInvalid
        | FailureStage::UpstreamHttpUnavailable => ("mcp", "hosted_api_request"),
        FailureStage::TimeoutIndexedLookup
        | FailureStage::TimeoutRelayerNotConfigured
        | FailureStage::TimeoutPlannerUnavailable
        | FailureStage::TimeoutDurableStoreNotConfigured
        | FailureStage::TimeoutRpcNotConfigured
        | FailureStage::TimeoutLeaseAcquireFailed
        | FailureStage::TimeoutLeaseBusy
        | FailureStage::TimeoutRelayExecutionFailed
        | FailureStage::TimeoutRelayExecutionTimedOut
        | FailureStage::TimeoutLeaseReleaseFailed
        | FailureStage::TimeoutConfirmationUnavailable => ("api", "timeout_relay"),
        FailureStage::HeadChainIdRead
        | FailureStage::HeadSafeBlockRead
        | FailureStage::HeadSafeValidation
        | FailureStage::HeadLatestNumberRead
        | FailureStage::HeadLatestBlockRead
        | FailureStage::HeadLatestValidation
        | FailureStage::HeadRpcNotConfigured
        | FailureStage::MarketplaceObservationTimedOut
        | FailureStage::MarketplaceHeadsTimedOut => ("api", "marketplace_heads"),
        FailureStage::MarketplaceSnapshotTimedOut => ("api", "marketplace_snapshot"),
    };
    let boundary = if action == "timeout_relay" {
        "Diagnostic only. A relay timeout or provider error may follow a broadcast; reconcile current state and pending transactions before any retry. No recovery authority."
    } else {
        "Dependency availability only; no canonical state or payment conclusion and no recovery authority."
    };
    json!({
        "schema": "agent-bounties/service-failure-stage-v1",
        "event": "service_failure_stage",
        "level": "error",
        "component": component,
        "revision": safe_revision(revision),
        "action": action,
        "attempt": 1,
        "stage": stage,
        "evidence_boundary": boundary,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    Transport,
    TransportTimeout,
    ConnectionFailed,
    LocalPacingTimeout,
    LocalPacingCapacity,
    LocalPacingConfiguration,
    HttpStatus,
    RateLimited,
    Provider,
    MalformedResponse,
    WrongChain,
    InvalidTime,
    StaleTime,
    FutureTime,
    TimeOutOfRange,
    InvalidOrder,
    InsufficientBalance,
    SimulationRejected,
    PolicyRejected,
    Unknown,
}

// These exact, fixed messages originate in our transport/pacer implementation.
// Unknown strings stay generic; no raw error, endpoint or arbitrary message is logged.
fn transport_failure_class(message: &str) -> FailureClass {
    match message {
        "request timed out" | "read probe timed out" => FailureClass::TransportTimeout,
        "connection failed" => FailureClass::ConnectionFailed,
        "RPC read pacing wait deadline elapsed" => FailureClass::LocalPacingTimeout,
        "RPC read pacing endpoint capacity unavailable"
        | "RPC read pacing caller capacity unavailable" => FailureClass::LocalPacingCapacity,
        "RPC read pacing configuration unavailable" | "RPC read pacing registry unavailable" => {
            FailureClass::LocalPacingConfiguration
        }
        _ => FailureClass::Transport,
    }
}

pub fn chain_failure_class(error: &chain_base::ChainBaseError) -> FailureClass {
    use chain_base::ChainBaseError;
    match error {
        ChainBaseError::RpcTransport(message) => transport_failure_class(message),
        ChainBaseError::RpcRetryAfter { status: 429, .. } | ChainBaseError::RpcHttpStatus(429) => {
            FailureClass::RateLimited
        }
        ChainBaseError::RpcRetryAfter { .. } | ChainBaseError::RpcHttpStatus(_) => {
            FailureClass::HttpStatus
        }
        ChainBaseError::RpcProviderError { .. } | ChainBaseError::RelayerProvider(_) => {
            FailureClass::Provider
        }
        ChainBaseError::InvalidRpcResponse(_) | ChainBaseError::InvalidRpcQuantity(_) => {
            FailureClass::MalformedResponse
        }
        ChainBaseError::RelayerChainMismatch { .. } => FailureClass::WrongChain,
        ChainBaseError::RelayerInsufficientBalance { .. } => FailureClass::InsufficientBalance,
        ChainBaseError::RelayerSimulation(_) => FailureClass::SimulationRejected,
        ChainBaseError::InvalidRelayIntent(_)
        | ChainBaseError::RelayerGasLimitExceeded { .. }
        | ChainBaseError::RelayerFeeCapExceeded { .. } => FailureClass::PolicyRejected,
        _ => FailureClass::Unknown,
    }
}

pub fn classified_failure_event(stage: FailureStage, class: FailureClass, revision: &str) -> Value {
    let mut event = failure_stage_event(stage, revision);
    event["failure_class"] = json!(class);
    event
}

pub fn record_classified_failure(stage: FailureStage, class: FailureClass, revision: &str) {
    write_event(&classified_failure_event(stage, class, revision));
}

pub fn record_failure_stage(stage: FailureStage, revision: &str) {
    write_event(&failure_stage_event(stage, revision));
}

fn write_event(event: &Value) {
    // A logging failure must not replace the handler's response or panic.
    let _ = writeln!(std::io::stderr().lock(), "{event}");
}

pub fn http_error_event(
    service: HttpService,
    method: &str,
    matched_route_template: Option<&str>,
    revision: &str,
    status: u16,
) -> Option<Value> {
    if !(500..600).contains(&status) {
        return None;
    }
    let method = match method {
        "GET" => "GET",
        "HEAD" => "HEAD",
        "POST" => "POST",
        "PUT" => "PUT",
        "PATCH" => "PATCH",
        "DELETE" => "DELETE",
        "OPTIONS" => "OPTIONS",
        "CONNECT" => "CONNECT",
        "TRACE" => "TRACE",
        _ => "OTHER",
    };
    let route = matched_route_template
        .filter(|route| {
            route.starts_with('/')
                && route.len() <= 240
                && route.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric()
                        || matches!(byte, b'/' | b'-' | b'_' | b'.' | b':' | b'*' | b'{' | b'}')
                })
        })
        .unwrap_or("unmatched");
    let revision = safe_revision(revision);
    let code = match status {
        500 => "internal_server_error",
        501 => "not_implemented",
        502 => "bad_gateway",
        503 => "service_unavailable",
        504 => "gateway_timeout",
        _ => "server_error",
    };
    Some(json!({
        "event": "http_server_error",
        "level": "error",
        "service": match service { HttpService::Api => "api", HttpService::Mcp => "mcp" },
        "route": route,
        "method": method,
        "status": status,
        "revision": revision,
        "code": code,
    }))
}

pub fn record_http_error(
    service: HttpService,
    method: &str,
    matched_route_template: Option<&str>,
    revision: &str,
    status: u16,
) {
    if let Some(event) = http_error_event(service, method, matched_route_template, revision, status)
    {
        write_event(&event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_transport_reasons_separate_local_capacity_from_network_failures() {
        for (message, expected) in [
            ("request timed out", "transport_timeout"),
            ("read probe timed out", "transport_timeout"),
            ("connection failed", "connection_failed"),
            (
                "RPC read pacing wait deadline elapsed",
                "local_pacing_timeout",
            ),
            (
                "RPC read pacing endpoint capacity unavailable",
                "local_pacing_capacity",
            ),
            (
                "RPC read pacing caller capacity unavailable",
                "local_pacing_capacity",
            ),
            (
                "RPC read pacing configuration unavailable",
                "local_pacing_configuration",
            ),
            (
                "RPC read pacing registry unavailable",
                "local_pacing_configuration",
            ),
            (
                "connection failed: https://private/?secret=value",
                "transport",
            ),
            ("request failed", "transport"),
        ] {
            let event = classified_failure_event(
                FailureStage::HeadChainIdRead,
                chain_failure_class(&chain_base::ChainBaseError::RpcTransport(message.into())),
                "https://private/?secret=value",
            );
            assert_eq!(event["failure_class"], expected);
            assert_eq!(event["revision"], "local");
            assert!(!event.to_string().contains("secret"));
            assert!(!event.to_string().contains("private"));
        }
        for stage in [
            FailureStage::MarketplaceSnapshotTimedOut,
            FailureStage::MarketplaceHeadsTimedOut,
        ] {
            let event = failure_stage_event(stage, "local");
            assert_eq!(event["component"], "api");
            assert_eq!(event.as_object().unwrap().len(), 9);
        }
    }

    #[test]
    fn head_time_classes_are_bounded_and_preserve_the_rejected_header() {
        for stage in [
            FailureStage::HeadSafeValidation,
            FailureStage::HeadLatestValidation,
        ] {
            for (class, expected) in [
                (FailureClass::StaleTime, "stale_time"),
                (FailureClass::FutureTime, "future_time"),
                (FailureClass::TimeOutOfRange, "time_out_of_range"),
            ] {
                let event = classified_failure_event(stage, class, "https://private/?secret=value");
                assert_eq!(event["stage"], serde_json::to_value(stage).unwrap());
                assert_eq!(event["failure_class"], expected);
                assert_eq!(event["action"], "marketplace_heads");
                assert_eq!(event["revision"], "local");
                assert_eq!(event.as_object().unwrap().len(), 10);
                assert!(!event.to_string().contains("private"));
                assert!(!event.to_string().contains("secret"));
            }
        }
    }

    #[test]
    fn classified_dependency_errors_preserve_only_typed_classes_and_broadcast_uncertainty() {
        use chain_base::ChainBaseError;
        for (error, expected) in [
            (
                ChainBaseError::RpcTransport("https://private-marker/?secret=value".into()),
                "transport",
            ),
            (ChainBaseError::RpcHttpStatus(429), "rate_limited"),
            (
                ChainBaseError::RpcRetryAfter {
                    status: 429,
                    retry_after_seconds: 123,
                },
                "rate_limited",
            ),
            (ChainBaseError::RpcHttpStatus(503), "http_status"),
            (
                ChainBaseError::RpcRetryAfter {
                    status: 503,
                    retry_after_seconds: 123,
                },
                "http_status",
            ),
            (
                ChainBaseError::RpcProviderError {
                    code: 15,
                    message: "private-marker".into(),
                },
                "provider",
            ),
            (
                ChainBaseError::InvalidRpcResponse("private-marker".into()),
                "malformed_response",
            ),
            (
                ChainBaseError::RelayerProvider("private-marker".into()),
                "provider",
            ),
        ] {
            let event = classified_failure_event(
                FailureStage::HeadChainIdRead,
                chain_failure_class(&error),
                "private-marker",
            );
            assert_eq!(event["failure_class"], expected);
            assert_eq!(event["revision"], "local");
            assert_eq!(event.as_object().unwrap().len(), 10);
            assert!(!event.to_string().contains("private-marker"));
            assert!(!event.to_string().contains("secret"));
        }
        for stage in [
            FailureStage::TimeoutRelayExecutionFailed,
            FailureStage::TimeoutRelayExecutionTimedOut,
            FailureStage::TimeoutLeaseReleaseFailed,
            FailureStage::TimeoutConfirmationUnavailable,
        ] {
            let event = failure_stage_event(stage, "local");
            assert_eq!(event["component"], "api");
            assert_eq!(event["action"], "timeout_relay");
            assert!(event["evidence_boundary"]
                .as_str()
                .unwrap()
                .contains("may follow a broadcast"));
            assert!(event["evidence_boundary"]
                .as_str()
                .unwrap()
                .contains("before any retry"));
        }
    }

    #[test]
    fn dependency_stages_are_structured_bounded_and_do_not_echo_invalid_revisions() {
        let stages = [
            FailureStage::ScopedAutonomousRead,
            FailureStage::ScopedOpenCompetitionRead,
            FailureStage::ScopedOpenCompetitionV2Read,
            FailureStage::LegacyStateLock,
            FailureStage::ClaimPlanCompetitionLookup,
            FailureStage::ClaimPlanIndexedLookup,
            FailureStage::AuthorizedClaimPlanCompetitionLookup,
            FailureStage::AuthorizedClaimPlanIndexedLookup,
            FailureStage::SigningNotConfigured,
            FailureStage::DurableStoreNotConfigured,
            FailureStage::DurableStoreCreate,
            FailureStage::RequestTimeout,
            FailureStage::RequestTransportUnavailable,
            FailureStage::ResponseBodyUnavailable,
            FailureStage::ResponseJsonInvalid,
            FailureStage::UpstreamHttpUnavailable,
        ];
        let revision = "0123456789abcdef0123456789abcdef01234567";
        for stage in stages {
            let event = failure_stage_event(stage, revision);
            let encoded = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<Value>(&encoded).unwrap(), event);
            assert_eq!(event.as_object().unwrap().len(), 9);
            assert_eq!(event["schema"], "agent-bounties/service-failure-stage-v1");
            assert_eq!(event["event"], "service_failure_stage");
            assert_eq!(event["revision"], revision);
            assert_eq!(event["attempt"], 1);
            assert_eq!(event["level"], "error");
            assert!(event["evidence_boundary"]
                .as_str()
                .unwrap()
                .contains("no recovery authority"));
            for bad_revision in [
                "https://private.invalid/?token=secret",
                "private\nheader",
                "",
                &"a".repeat(41),
            ] {
                let redacted = failure_stage_event(stage, bad_revision);
                assert_eq!(redacted["revision"], "local");
                assert!(!redacted.to_string().contains("private"));
                assert!(!redacted.to_string().contains("secret"));
            }
        }
    }

    #[test]
    fn server_errors_have_only_bounded_operational_fields() {
        let event = http_error_event(
            HttpService::Api,
            "GET",
            Some("/v1/bounties/:id"),
            "0123456789abcdef0123456789abcdef01234567",
            503,
        )
        .unwrap();
        assert_eq!(
            event,
            json!({
                "event": "http_server_error", "level": "error", "service": "api", "route": "/v1/bounties/:id",
                "method": "GET", "status": 503, "revision": "0123456789abcdef0123456789abcdef01234567",
                "code": "service_unavailable",
            })
        );
    }

    #[test]
    fn unknown_methods_unmatched_routes_and_invalid_revisions_never_echo_inputs() {
        let long_route = "/".repeat(241);
        for route in [
            None,
            Some("/path?private-query"),
            Some("/path\nprivate-header"),
            Some(long_route.as_str()),
        ] {
            let event = http_error_event(
                HttpService::Mcp,
                "PRIVATE-METHOD",
                route,
                "private-revision",
                500,
            )
            .unwrap();
            assert_eq!(event["route"], "unmatched");
            assert_eq!(event["method"], "OTHER");
            assert_eq!(event["revision"], "local");
            assert!(!event.to_string().to_ascii_lowercase().contains("private"));
        }
    }

    #[test]
    fn success_and_expected_client_errors_are_not_server_errors() {
        for status in [200, 202, 301, 400, 401, 403, 404, 410, 429, 499, 600] {
            assert!(
                http_error_event(HttpService::Mcp, "POST", Some("/mcp"), "local", status).is_none()
            );
        }
        assert_eq!(
            http_error_event(HttpService::Mcp, "POST", Some("/mcp"), "local", 599).unwrap()["code"],
            "server_error"
        );
    }
}
