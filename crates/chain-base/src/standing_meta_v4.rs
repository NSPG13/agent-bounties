use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const STANDING_META_V4_READINESS_SCHEMA: &str = "agent-bounties/standing-meta-v4-readiness-v1";
pub const STANDING_META_V4_ACTION_SCHEMA: &str = "agent-bounties/standing-meta-v4-action-v1";
pub const STANDING_META_V4_PARENT_SOLVER_REWARD: u64 = 2_000_000;
pub const STANDING_META_V4_PARENT_VERIFIER_REWARD: u64 = 10_000;
pub const STANDING_META_V4_PARENT_FUNDING_TARGET: u64 = 2_010_000;
pub const STANDING_META_V4_CHILD_FUNDING_TARGET: u64 = 1_000_000;
pub const STANDING_META_V4_CHILD_SOLVER_REWARD: u64 = 990_000;
pub const STANDING_META_V4_CHILD_VERIFIER_REWARD: u64 = 10_000;
pub const STANDING_META_V4_SUCCESS_MARGIN: u64 = 1_000_000;
pub const STANDING_META_V4_MINIMUM_VERIFIERS: u16 = 8;
pub const STANDING_META_V4_MINIMUM_CHILD_SOLVERS: u16 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StandingMetaV4EconomicsEvidence {
    pub parent_solver_reward: u64,
    pub parent_verifier_reward: u64,
    pub parent_funding_target: u64,
    pub child_funding_target: u64,
    pub child_solver_reward: u64,
    pub child_verifier_reward: u64,
}

impl Default for StandingMetaV4EconomicsEvidence {
    fn default() -> Self {
        Self {
            parent_solver_reward: STANDING_META_V4_PARENT_SOLVER_REWARD,
            parent_verifier_reward: STANDING_META_V4_PARENT_VERIFIER_REWARD,
            parent_funding_target: STANDING_META_V4_PARENT_FUNDING_TARGET,
            child_funding_target: STANDING_META_V4_CHILD_FUNDING_TARGET,
            child_solver_reward: STANDING_META_V4_CHILD_SOLVER_REWARD,
            child_verifier_reward: STANDING_META_V4_CHILD_VERIFIER_REWARD,
        }
    }
}

impl StandingMetaV4EconomicsEvidence {
    pub fn successful_settlement_margin(&self) -> Option<u64> {
        self.parent_solver_reward
            .checked_sub(self.child_funding_target)
    }

    pub fn is_exact_and_profitable(&self) -> bool {
        self.parent_solver_reward == STANDING_META_V4_PARENT_SOLVER_REWARD
            && self.parent_verifier_reward == STANDING_META_V4_PARENT_VERIFIER_REWARD
            && self.parent_funding_target == STANDING_META_V4_PARENT_FUNDING_TARGET
            && self.child_funding_target == STANDING_META_V4_CHILD_FUNDING_TARGET
            && self.child_solver_reward == STANDING_META_V4_CHILD_SOLVER_REWARD
            && self.child_verifier_reward == STANDING_META_V4_CHILD_VERIFIER_REWARD
            && self
                .parent_solver_reward
                .checked_add(self.parent_verifier_reward)
                == Some(self.parent_funding_target)
            && self
                .child_solver_reward
                .checked_add(self.child_verifier_reward)
                == Some(self.child_funding_target)
            && self.successful_settlement_margin() == Some(STANDING_META_V4_SUCCESS_MARGIN)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StandingMetaV4ReadinessEvidence {
    #[serde(default)]
    pub economics: StandingMetaV4EconomicsEvidence,
    pub canonical_components_configured: bool,
    pub valid_terms: bool,
    pub gas_sponsorship_available: bool,
    pub vrf_subscription_funded: bool,
    pub vrf_consumers_authorized: bool,
    pub official_vrf_configuration_revalidated: bool,
    pub eligible_verifier_wallets: u16,
    pub eligible_child_solver_wallets_after_exclusions: u16,
    pub safe_timing: bool,
    pub appeal_path_executable: bool,
    pub r4_release_evidence_complete: bool,
    pub monitoring_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandingMetaV4ReadinessCheck {
    pub name: String,
    pub ready: bool,
    pub observed: String,
    pub required: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandingMetaV4ReadinessReport {
    pub schema_version: String,
    pub protocol_version: String,
    pub ready_to_earn: bool,
    pub successful_settlement_margin_base_units: Option<String>,
    pub checks: Vec<StandingMetaV4ReadinessCheck>,
    pub blockers: Vec<String>,
    pub next_action: String,
    pub decision_authority: String,
    pub payment_authority: String,
    pub fairness_statement: String,
    pub evidence_boundary: String,
}

pub fn standing_meta_v4_readiness(
    evidence: &StandingMetaV4ReadinessEvidence,
) -> StandingMetaV4ReadinessReport {
    let economics_ready = evidence.economics.is_exact_and_profitable();
    let checks = vec![
        check(
            "canonical_components",
            evidence.canonical_components_configured,
            evidence.canonical_components_configured.to_string(),
            "all exact immutable V4 component addresses and code hashes configured",
        ),
        check(
            "valid_terms",
            evidence.valid_terms,
            evidence.valid_terms.to_string(),
            "typed terms validate against the claim-restricted canonical V4 child and parent round",
        ),
        check(
            "profitable_integer_economics",
            economics_ready,
            format!(
                "parent_reward={} child_outlay={} margin={}",
                evidence.economics.parent_solver_reward,
                evidence.economics.child_funding_target,
                evidence
                    .economics
                    .successful_settlement_margin()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "underflow".to_string())
            ),
            "exact micro-USDC amounts and 1000000 base-unit successful-settlement margin",
        ),
        check(
            "gas_sponsorship",
            evidence.gas_sponsorship_available,
            evidence.gas_sponsorship_available.to_string(),
            "platform sponsorship reserve available",
        ),
        check(
            "vrf_subscription_reserve",
            evidence.vrf_subscription_funded,
            evidence.vrf_subscription_funded.to_string(),
            "native-token VRF 2.5 subscription reserve funded",
        ),
        check(
            "vrf_consumer_authorization",
            evidence.vrf_consumers_authorized,
            evidence.vrf_consumers_authorized.to_string(),
            "both immutable sortition coordinators authorized as consumers",
        ),
        check(
            "official_vrf_configuration",
            evidence.official_vrf_configuration_revalidated,
            evidence
                .official_vrf_configuration_revalidated
                .to_string(),
            "official Base coordinator and key hash revalidated before deployment",
        ),
        check(
            "verifier_pool",
            evidence.eligible_verifier_wallets >= STANDING_META_V4_MINIMUM_VERIFIERS,
            evidence.eligible_verifier_wallets.to_string(),
            "at least 8 active, available verifier wallets after exclusions",
        ),
        check(
            "child_solver_pool",
            evidence.eligible_child_solver_wallets_after_exclusions
                >= STANDING_META_V4_MINIMUM_CHILD_SOLVERS,
            evidence
                .eligible_child_solver_wallets_after_exclusions
                .to_string(),
            "at least 3 active, available solver wallets after exclusions",
        ),
        check(
            "safe_timing",
            evidence.safe_timing,
            evidence.safe_timing.to_string(),
            "immediate frozen draw, viable work windows, timeout paths, and no rerolls",
        ),
        check(
            "appeal_path",
            evidence.appeal_path_executable,
            evidence.appeal_path_executable.to_string(),
            "symmetric appeal can select and lock five eligible appellate wallets",
        ),
        check(
            "r4_release_evidence",
            evidence.r4_release_evidence_complete,
            evidence.r4_release_evidence_complete.to_string(),
            "independent review, Sepolia rehearsal, mainnet fork test, bytecode and wallet-policy evidence complete",
        ),
        check(
            "dependency_monitoring",
            evidence.monitoring_active,
            evidence.monitoring_active.to_string(),
            "live suppression monitors active for reserves, pool sizes, assignments, appeals, and settlement",
        ),
    ];
    let blockers = checks
        .iter()
        .filter(|item| !item.ready)
        .map(|item| item.name.clone())
        .collect::<Vec<_>>();
    let ready_to_earn = blockers.is_empty();
    StandingMetaV4ReadinessReport {
        schema_version: STANDING_META_V4_READINESS_SCHEMA.to_string(),
        protocol_version: "standing-meta-v4".to_string(),
        ready_to_earn,
        successful_settlement_margin_base_units: evidence
            .economics
            .successful_settlement_margin()
            .map(|value| value.to_string()),
        checks,
        blockers,
        next_action: if ready_to_earn {
            "Call prepare_standing_meta_v4_claim; never call generic agent_native_claim for a V4 parent."
                .to_string()
        } else {
            "Do not claim or spend funds. Resolve every blocker and obtain fresh onchain readiness evidence."
                .to_string()
        },
        decision_authority: "The VRF-selected primary verifier judges first; the eligible solver or creator may trigger a five-wallet appellate majority. Chainlink selects wallets but does not judge submissions.".to_string(),
        payment_authority: "Only the exact canonical bounty contract settles escrow; confirmed canonical BountySettled is payment evidence.".to_string(),
        fairness_statement: "Anonymous wallets may share an owner. Fixed staking and random assignment raise coordination cost but do not prove identity, unrelated ownership, or organizational independence.".to_string(),
        evidence_boundary: "Readiness is fail-closed and point-in-time. A readiness report, action plan, signature, VRF result, verdict, or transaction hash is not settlement or payment evidence.".to_string(),
    }
}

fn check(
    name: &str,
    ready: bool,
    observed: impl Into<String>,
    required: impl Into<String>,
) -> StandingMetaV4ReadinessCheck {
    StandingMetaV4ReadinessCheck {
        name: name.to_string(),
        ready,
        observed: observed.into(),
        required: required.into(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StandingMetaV4Operation {
    PrepareStandingMetaV4Claim,
    PrepareAnonymousStakeRegistration,
    SetAnonymousStakeAvailability,
    ListVerificationAssignments,
    SubmitPrimaryVerdict,
    WaiveVerificationAppeal,
    OpenVerificationAppeal,
    SubmitAppealVote,
    FinalizeVerificationCase,
}

impl StandingMetaV4Operation {
    pub fn requires_earning_readiness(self) -> bool {
        matches!(self, Self::PrepareStandingMetaV4Claim)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StandingMetaV4ActionPlan {
    pub schema_version: String,
    pub protocol_version: String,
    pub operation: StandingMetaV4Operation,
    pub allowed: bool,
    pub target_contract: Option<String>,
    pub function: Option<String>,
    pub arguments: Value,
    pub blocker: Option<String>,
    pub next_action: String,
    pub evidence_boundary: String,
}

pub fn plan_standing_meta_v4_action(
    operation: StandingMetaV4Operation,
    readiness: &StandingMetaV4ReadinessReport,
    target_contract: Option<String>,
    function: Option<String>,
    arguments: Value,
) -> StandingMetaV4ActionPlan {
    let ready_required = operation.requires_earning_readiness();
    let target_present = target_contract
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let function_present = function
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let allowed =
        (!ready_required || readiness.ready_to_earn) && target_present && function_present;
    let blocker = if ready_required && !readiness.ready_to_earn {
        Some(format!(
            "standing-meta-v4 is not ready to earn: {}",
            readiness.blockers.join(", ")
        ))
    } else if !target_present || !function_present {
        Some("canonical V4 target and function are not configured".to_string())
    } else {
        None
    };
    StandingMetaV4ActionPlan {
        schema_version: STANDING_META_V4_ACTION_SCHEMA.to_string(),
        protocol_version: "standing-meta-v4".to_string(),
        operation,
        allowed,
        target_contract,
        function,
        arguments,
        blocker,
        next_action: if allowed {
            "Validate the exact target, arguments, wallet policy, and live state; then sign and broadcast through the configured wallet."
                .to_string()
        } else {
            "Do not sign or broadcast. Resolve the blocker and request a fresh plan.".to_string()
        },
        evidence_boundary: "This is an unsigned agent-native action plan. It is not a transaction, selection, verdict, settlement, or payment receipt.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_evidence() -> StandingMetaV4ReadinessEvidence {
        StandingMetaV4ReadinessEvidence {
            economics: StandingMetaV4EconomicsEvidence::default(),
            canonical_components_configured: true,
            valid_terms: true,
            gas_sponsorship_available: true,
            vrf_subscription_funded: true,
            vrf_consumers_authorized: true,
            official_vrf_configuration_revalidated: true,
            eligible_verifier_wallets: 8,
            eligible_child_solver_wallets_after_exclusions: 3,
            safe_timing: true,
            appeal_path_executable: true,
            r4_release_evidence_complete: true,
            monitoring_active: true,
        }
    }

    #[test]
    fn exact_micro_usdc_economics_have_one_usdc_margin() {
        let economics = StandingMetaV4EconomicsEvidence::default();
        assert!(economics.is_exact_and_profitable());
        assert_eq!(economics.successful_settlement_margin(), Some(1_000_000));
    }

    #[test]
    fn every_readiness_dependency_fails_closed() {
        let ready = ready_evidence();
        assert!(standing_meta_v4_readiness(&ready).ready_to_earn);

        for name in [
            "canonical_components",
            "valid_terms",
            "gas_sponsorship",
            "vrf_subscription_reserve",
            "vrf_consumer_authorization",
            "official_vrf_configuration",
            "verifier_pool",
            "child_solver_pool",
            "safe_timing",
            "appeal_path",
            "r4_release_evidence",
            "dependency_monitoring",
        ] {
            let mut evidence = ready.clone();
            match name {
                "canonical_components" => evidence.canonical_components_configured = false,
                "valid_terms" => evidence.valid_terms = false,
                "gas_sponsorship" => evidence.gas_sponsorship_available = false,
                "vrf_subscription_reserve" => evidence.vrf_subscription_funded = false,
                "vrf_consumer_authorization" => evidence.vrf_consumers_authorized = false,
                "official_vrf_configuration" => {
                    evidence.official_vrf_configuration_revalidated = false
                }
                "verifier_pool" => evidence.eligible_verifier_wallets = 7,
                "child_solver_pool" => evidence.eligible_child_solver_wallets_after_exclusions = 2,
                "safe_timing" => evidence.safe_timing = false,
                "appeal_path" => evidence.appeal_path_executable = false,
                "r4_release_evidence" => evidence.r4_release_evidence_complete = false,
                "dependency_monitoring" => evidence.monitoring_active = false,
                _ => unreachable!(),
            }
            let report = standing_meta_v4_readiness(&evidence);
            assert!(!report.ready_to_earn, "{name} did not fail closed");
            assert!(report.blockers.iter().any(|blocker| blocker == name));
        }
    }

    #[test]
    fn arithmetic_drift_and_underflow_fail_closed() {
        let mut evidence = ready_evidence();
        evidence.economics.child_funding_target = 2_000_001;
        let report = standing_meta_v4_readiness(&evidence);
        assert!(!report.ready_to_earn);
        assert_eq!(report.successful_settlement_margin_base_units, None);
        assert!(report
            .blockers
            .contains(&"profitable_integer_economics".to_string()));
    }

    #[test]
    fn direct_claim_plan_is_blocked_until_all_checks_pass() {
        let report = standing_meta_v4_readiness(&StandingMetaV4ReadinessEvidence::default());
        let plan = plan_standing_meta_v4_action(
            StandingMetaV4Operation::PrepareStandingMetaV4Claim,
            &report,
            Some("0x1111111111111111111111111111111111111111".to_string()),
            Some("claimAndCreateChild".to_string()),
            Value::Null,
        );
        assert!(!plan.allowed);
        assert!(plan.blocker.unwrap().contains("not ready to earn"));
    }
}
