use super::*;

/// Only fixed block observations are retained; no provider or customer fields.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum CreatorSafeHeadUnavailable {
    InconsistentSample {
        head_block: u64,
        head_timestamp: u64,
        safe_block: u64,
        safe_timestamp: u64,
    },
    BehindCheckpoint {
        checkpoint_block: u64,
        provider_safe_block: u64,
    },
}

impl fmt::Display for CreatorSafeHeadUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("creator-open provider safe head unavailable")
    }
}
impl std::error::Error for CreatorSafeHeadUnavailable {}

pub(super) fn require_usable_safe_head(
    head: &BaseBlockIdentity,
    safe: &BaseBlockIdentity,
) -> anyhow::Result<()> {
    if safe.number > head.number
        || safe.timestamp > head.timestamp
        || head.timestamp.saturating_sub(safe.timestamp) > 3600
    {
        return Err(CreatorSafeHeadUnavailable::InconsistentSample {
            head_block: head.number,
            head_timestamp: head.timestamp,
            safe_block: safe.number,
            safe_timestamp: safe.timestamp,
        }
        .into());
    }
    Ok(())
}

pub(crate) fn safe_head_observation(error: &anyhow::Error) -> Option<&CreatorSafeHeadUnavailable> {
    error.downcast_ref::<CreatorSafeHeadUnavailable>()
}

/// Keep the diagnostic that previously disappeared behind a recovery decision.
pub fn creator_open_indexer_recovery_report(
    network: &str,
    error: &anyhow::Error,
    decision: IndexerRecoveryDecision,
) -> serde_json::Value {
    let message: String = redact_operational_error(&error.to_string())
        .chars()
        .take(512)
        .collect();
    serde_json::json!({
        "schema": "agent-bounties/creator-open-indexer-recovery-v1",
        "network": network,
        "error": message,
        "safe_head_observation": safe_head_observation(error),
        "decision": decision,
        "evidence_boundary": "An unavailable provider cannot certify a new projection or authorize a notification, signature or payment."
    })
}
