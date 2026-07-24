use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const OPEN_COMPETITION_READINESS_SCHEMA: &str =
    "agent-bounties/open-competition-v1-readiness-v1";
pub const OPEN_COMPETITION_ACTION_SCHEMA: &str = "agent-bounties/open-competition-v1-action-v1";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCompetitionReadinessEvidence {
    pub canonical_factory_configured: bool,
    pub canonical_bounty_runtime: bool,
    pub valid_terms: bool,
    pub fully_funded: bool,
    pub deterministic_verifier_ready: bool,
    pub competition_open: bool,
    pub entry_capacity_available: bool,
    pub safe_commit_reveal_timing: bool,
    pub gas_sponsorship_available: bool,
    pub relay_support_available: bool,
    pub r4_release_evidence_complete: bool,
    pub monitoring_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionReadinessCheck {
    pub name: String,
    pub ready: bool,
    pub observed: String,
    pub required: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionReadinessReport {
    pub schema_version: String,
    pub protocol_version: String,
    pub competition_mode: String,
    pub ready_to_compete: bool,
    pub checks: Vec<OpenCompetitionReadinessCheck>,
    pub blockers: Vec<String>,
    pub first_means: String,
    pub ordering_authority: String,
    pub decision_authority: String,
    pub payment_authority: String,
    pub next_action: String,
    pub fairness_statement: String,
    pub evidence_boundary: String,
}

pub fn open_competition_readiness(
    evidence: &OpenCompetitionReadinessEvidence,
) -> OpenCompetitionReadinessReport {
    let checks = vec![
        check(
            "canonical_factory",
            evidence.canonical_factory_configured,
            evidence.canonical_factory_configured,
            "exact immutable open-competition factory address and runtime hash configured",
        ),
        check(
            "canonical_bounty_runtime",
            evidence.canonical_bounty_runtime,
            evidence.canonical_bounty_runtime,
            "bounty is a canonical clone with the expected implementation runtime",
        ),
        check(
            "valid_terms",
            evidence.valid_terms,
            evidence.valid_terms,
            "content-addressed first-valid terms match every onchain commitment",
        ),
        check(
            "fully_funded",
            evidence.fully_funded,
            evidence.fully_funded,
            "solver and verifier rewards are fully escrowed before entry",
        ),
        check(
            "deterministic_verifier",
            evidence.deterministic_verifier_ready,
            evidence.deterministic_verifier_ready,
            "the exact deterministic verifier is executable and matches the published benchmark",
        ),
        check(
            "competition_open",
            evidence.competition_open,
            evidence.competition_open,
            "competition status is open and its deadline has not elapsed",
        ),
        check(
            "entry_capacity",
            evidence.entry_capacity_available,
            evidence.entry_capacity_available,
            "the bounded entry cap has not been reached and this wallet has not entered",
        ),
        check(
            "commit_reveal_timing",
            evidence.safe_commit_reveal_timing,
            evidence.safe_commit_reveal_timing,
            "at least one later block and enough reveal time remain",
        ),
        check(
            "gas_sponsorship",
            evidence.gas_sponsorship_available,
            evidence.gas_sponsorship_available,
            "bounded gas sponsorship is available for the advertised agent-native path",
        ),
        check(
            "relay_support",
            evidence.relay_support_available,
            evidence.relay_support_available,
            "commit and reveal relay paths are configured with commitment-bound authorization",
        ),
        check(
            "r4_release_evidence",
            evidence.r4_release_evidence_complete,
            evidence.r4_release_evidence_complete,
            "independent review, Sepolia rehearsal, exact bytecode, mainnet fork, and signing approval complete",
        ),
        check(
            "dependency_monitoring",
            evidence.monitoring_active,
            evidence.monitoring_active,
            "runtime, verifier, timing, capacity, relay, and settlement monitors are active",
        ),
    ];
    let blockers = checks
        .iter()
        .filter(|item| !item.ready)
        .map(|item| item.name.clone())
        .collect::<Vec<_>>();
    let ready_to_compete = blockers.is_empty();
    OpenCompetitionReadinessReport {
        schema_version: OPEN_COMPETITION_READINESS_SCHEMA.to_string(),
        protocol_version: "open-competition-v1".to_string(),
        competition_mode: "first_valid_submission".to_string(),
        ready_to_compete,
        checks,
        blockers,
        first_means: "The lowest confirmed onchain submission_sequence whose committed deterministic verification returned pass. It does not prove who discovered the answer first offchain.".to_string(),
        ordering_authority: "Base transaction ordering plus the immutable bounty submission_sequence; verifier response time is not an ordering input.".to_string(),
        decision_authority: "The immutable deterministic verifier module evaluates each reveal atomically. No platform operator or AI response chooses the winner.".to_string(),
        payment_authority: "Only the exact canonical competition contract settles escrow; confirmed canonical BountySettled is payment evidence.".to_string(),
        next_action: if ready_to_compete {
            "Call prepare_open_competition_commit. Keep the salt private, then reveal from the same wallet in a later block.".to_string()
        } else {
            "Do not commit or post a bond. Resolve every blocker and request fresh onchain readiness evidence.".to_string()
        },
        fairness_statement: "Commit/reveal raises copying cost but does not prove offchain discovery time, unrelated wallet ownership, or censorship resistance. One wallet is one protocol entry, not one person.".to_string(),
        evidence_boundary: "A readiness report, commitment, reveal, verifier response, or transaction hash is not payment evidence. Only confirmed canonical BountySettled proves the winner was paid.".to_string(),
    }
}

fn check(name: &str, ready: bool, observed: bool, required: &str) -> OpenCompetitionReadinessCheck {
    OpenCompetitionReadinessCheck {
        name: name.to_string(),
        ready,
        observed: observed.to_string(),
        required: required.to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenCompetitionOperation {
    PrepareOpenCompetitionCommit,
    PrepareOpenCompetitionReveal,
    GetOpenCompetitionStatus,
    WithdrawOpenCompetitionBond,
}

impl OpenCompetitionOperation {
    pub fn requires_new_entry_readiness(self) -> bool {
        matches!(self, Self::PrepareOpenCompetitionCommit)
    }

    pub fn requires_live_reveal_readiness(self) -> bool {
        matches!(self, Self::PrepareOpenCompetitionReveal)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenCompetitionActionPlan {
    pub schema_version: String,
    pub protocol_version: String,
    pub competition_mode: String,
    pub operation: OpenCompetitionOperation,
    pub allowed: bool,
    pub target_contract: Option<String>,
    pub function: Option<String>,
    pub arguments: Value,
    pub blocker: Option<String>,
    pub next_action: String,
    pub evidence_boundary: String,
}

pub fn plan_open_competition_action(
    operation: OpenCompetitionOperation,
    readiness: &OpenCompetitionReadinessReport,
    target_contract: Option<String>,
    function: Option<String>,
    arguments: Value,
) -> OpenCompetitionActionPlan {
    let readiness_required = operation.requires_new_entry_readiness();
    let target_present = target_contract
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let function_present = function
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let canonical_target_ready = [
        "canonical_factory",
        "canonical_bounty_runtime",
        "valid_terms",
    ]
    .iter()
    .all(|name| !readiness.blockers.iter().any(|blocker| blocker == name));
    let live_reveal_ready = [
        "deterministic_verifier",
        "competition_open",
        "commit_reveal_timing",
    ]
    .iter()
    .all(|name| !readiness.blockers.iter().any(|blocker| blocker == name));
    let allowed = canonical_target_ready
        && (!readiness_required || readiness.ready_to_compete)
        && (!operation.requires_live_reveal_readiness() || live_reveal_ready)
        && target_present
        && function_present;
    let blocker = if !canonical_target_ready {
        Some(
            "canonical competition factory, bounty runtime, and terms are not verified".to_string(),
        )
    } else if readiness_required && !readiness.ready_to_compete {
        Some(format!(
            "open competition is not ready for a new entry: {}",
            readiness.blockers.join(", ")
        ))
    } else if operation.requires_live_reveal_readiness() && !live_reveal_ready {
        Some(
            "committed reveal requires the pinned deterministic verifier, a live competition, and safe reveal timing"
                .to_string(),
        )
    } else if !target_present || !function_present {
        Some("canonical competition target and function are not configured".to_string())
    } else {
        None
    };
    OpenCompetitionActionPlan {
        schema_version: OPEN_COMPETITION_ACTION_SCHEMA.to_string(),
        protocol_version: "open-competition-v1".to_string(),
        competition_mode: "first_valid_submission".to_string(),
        operation,
        allowed,
        target_contract,
        function,
        arguments,
        blocker,
        next_action: if allowed {
            "Validate the exact target, commitment or reveal preimage, wallet policy, and live state; then sign and broadcast through the configured wallet.".to_string()
        } else {
            "Do not sign or broadcast. Resolve the blocker and request a fresh plan.".to_string()
        },
        evidence_boundary: "This is an unsigned agent-native action plan. It is not an entry, reveal, verdict, settlement, or payment receipt.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_evidence() -> OpenCompetitionReadinessEvidence {
        OpenCompetitionReadinessEvidence {
            canonical_factory_configured: true,
            canonical_bounty_runtime: true,
            valid_terms: true,
            fully_funded: true,
            deterministic_verifier_ready: true,
            competition_open: true,
            entry_capacity_available: true,
            safe_commit_reveal_timing: true,
            gas_sponsorship_available: true,
            relay_support_available: true,
            r4_release_evidence_complete: true,
            monitoring_active: true,
        }
    }

    #[test]
    fn every_new_entry_dependency_fails_closed() {
        let ready = ready_evidence();
        assert!(open_competition_readiness(&ready).ready_to_compete);
        for name in [
            "canonical_factory",
            "canonical_bounty_runtime",
            "valid_terms",
            "fully_funded",
            "deterministic_verifier",
            "competition_open",
            "entry_capacity",
            "commit_reveal_timing",
            "gas_sponsorship",
            "relay_support",
            "r4_release_evidence",
            "dependency_monitoring",
        ] {
            let mut evidence = ready.clone();
            match name {
                "canonical_factory" => evidence.canonical_factory_configured = false,
                "canonical_bounty_runtime" => evidence.canonical_bounty_runtime = false,
                "valid_terms" => evidence.valid_terms = false,
                "fully_funded" => evidence.fully_funded = false,
                "deterministic_verifier" => evidence.deterministic_verifier_ready = false,
                "competition_open" => evidence.competition_open = false,
                "entry_capacity" => evidence.entry_capacity_available = false,
                "commit_reveal_timing" => evidence.safe_commit_reveal_timing = false,
                "gas_sponsorship" => evidence.gas_sponsorship_available = false,
                "relay_support" => evidence.relay_support_available = false,
                "r4_release_evidence" => evidence.r4_release_evidence_complete = false,
                "dependency_monitoring" => evidence.monitoring_active = false,
                _ => unreachable!(),
            }
            let report = open_competition_readiness(&evidence);
            assert!(!report.ready_to_compete, "{name} did not fail closed");
            assert!(report.blockers.iter().any(|blocker| blocker == name));
        }
    }

    #[test]
    fn recovery_actions_remain_plannable_after_new_entries_close() {
        let mut evidence = OpenCompetitionReadinessEvidence::default();
        evidence.canonical_factory_configured = true;
        evidence.canonical_bounty_runtime = true;
        evidence.valid_terms = true;
        evidence.deterministic_verifier_ready = true;
        evidence.competition_open = true;
        evidence.safe_commit_reveal_timing = true;
        let readiness = open_competition_readiness(&evidence);
        let reveal = plan_open_competition_action(
            OpenCompetitionOperation::PrepareOpenCompetitionReveal,
            &readiness,
            Some("0x1111111111111111111111111111111111111111".to_string()),
            Some("revealSolution".to_string()),
            Value::Null,
        );
        assert!(reveal.allowed);

        let commit = plan_open_competition_action(
            OpenCompetitionOperation::PrepareOpenCompetitionCommit,
            &readiness,
            Some("0x1111111111111111111111111111111111111111".to_string()),
            Some("commitSolution".to_string()),
            Value::Null,
        );
        assert!(!commit.allowed);

        evidence.safe_commit_reveal_timing = false;
        let expired_readiness = open_competition_readiness(&evidence);
        let expired_reveal = plan_open_competition_action(
            OpenCompetitionOperation::PrepareOpenCompetitionReveal,
            &expired_readiness,
            Some("0x1111111111111111111111111111111111111111".to_string()),
            Some("revealSolution".to_string()),
            Value::Null,
        );
        assert!(!expired_reveal.allowed);
    }
}
