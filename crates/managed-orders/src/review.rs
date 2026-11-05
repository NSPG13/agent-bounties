//! The paying customer's verdict on one canonical submission, the solver's appeal of a rejection
//! and the arbiter's ruling. Every document is public: it names the bounty, round, submission,
//! decision and reason, never the customer. The platform review key signs an attestation whose
//! response hash commits to exactly one of them: the verdict, or the ruling when there is one.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const REVIEW_SCHEMA: &str = "agent-bounties/customer-review-v1";
pub const APPEAL_SCHEMA: &str = "agent-bounties/customer-review-appeal-v1";
pub const RULING_SCHEMA: &str = "agent-bounties/customer-review-ruling-v1";
/// The customer accepts or rejects within this time after the delivery is published; silence
/// accepts.
pub const DECISION_SECONDS: u64 = 3 * 86_400;
/// A rejection waits this long for the solver's appeal before it becomes final.
pub const APPEAL_SECONDS: u64 = 2 * 86_400;
/// Time reserved for the arbiter's ruling on an appeal.
pub const ARBITRATION_SECONDS: u64 = 86_400;
/// Reserved before verification expiry for the final signature to reach the chain.
pub const SETTLEMENT_MARGIN_SECONDS: u64 = 3_600;
/// The shortest on-chain verification window that fits decision, appeal, ruling and settlement.
pub const MIN_VERIFICATION_WINDOW_SECONDS: u64 =
    DECISION_SECONDS + APPEAL_SECONDS + ARBITRATION_SECONDS + SETTLEMENT_MARGIN_SECONDS;
/// The public reason of a verdict the platform records when the customer stays silent.
pub const AUTO_ACCEPT_REASON: &str =
    "Accepted automatically: the customer did not decide within 3 days of the published delivery.";
/// The customer's refundable review deposit, in basis points of the solver reward.
pub const REVIEW_DEPOSIT_BPS: u64 = 1_000;

/// The deposit's USDC value: 10% of the solver reward, rounded up. A rejection the arbiter
/// reverses pays this to the solver.
pub fn review_deposit_micro_usdc(solver_reward_micro_usdc: u64) -> u64 {
    solver_reward_micro_usdc
        .saturating_mul(REVIEW_DEPOSIT_BPS)
        .div_ceil(10_000)
}
/// A rejection, appeal or ruling must say why, so everyone can see the reason.
pub const MIN_REJECTION_REASON_CHARS: usize = 20;
pub const MAX_REASON_CHARS: usize = 2_000;

/// When each review step closes for one canonical submission, in unix seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewSchedule {
    /// The customer accepts or rejects before this time. A rejection made then still leaves
    /// the full appeal and arbitration periods before `sign_by`.
    pub decide_by: u64,
    /// Silence until `decide_by` accepts only when the customer had the full decision period
    /// with the delivery published. Otherwise the submission is left to expire on chain, which
    /// returns the solver's bond and reopens the bounty.
    pub auto_accept: bool,
    /// The last moment the review key signs a verdict or ruling for this submission.
    pub sign_by: u64,
}

impl ReviewSchedule {
    /// `published_at` is when the evidence preimage became inspectable. The submission time is
    /// exact: the contract emits `block.timestamp + verificationWindowSeconds`.
    pub fn new(
        verification_expires_at: u64,
        window_seconds: u64,
        published_at: u64,
    ) -> Option<Self> {
        let submitted_at = verification_expires_at.checked_sub(window_seconds)?;
        let sign_by = verification_expires_at.checked_sub(SETTLEMENT_MARGIN_SECONDS)?;
        let closes = sign_by.checked_sub(APPEAL_SECONDS + ARBITRATION_SECONDS)?;
        let full = published_at
            .max(submitted_at)
            .checked_add(DECISION_SECONDS)?;
        Some(Self {
            decide_by: full.min(closes),
            auto_accept: full <= closes,
            sign_by,
        })
    }
}

/// The solver may appeal a rejection recorded at `rejected_at` until this time.
pub fn appeal_ends_at(rejected_at: u64) -> u64 {
    rejected_at.saturating_add(APPEAL_SECONDS)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReviewError {
    #[error("the verdict does not name one exact canonical submission")]
    Scope,
    #[error("a rejection needs a reason of 20 to 2,000 characters; any reason is at most 2,000")]
    Reason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomerVerdict {
    pub bounty_contract: String,
    pub round: u64,
    pub submission_hash: String,
    pub evidence_hash: String,
    pub passed: bool,
    pub reason: String,
}

fn reason_ok(reason: &str, min_chars: usize) -> bool {
    let chars = reason.chars().count();
    reason == reason.trim()
        && reason.chars().all(|c| c == '\n' || !c.is_control())
        && (min_chars..=MAX_REASON_CHARS).contains(&chars)
}

fn lower_hex(value: &str, bytes: usize) -> bool {
    value.len() == 2 + bytes * 2
        && value.starts_with("0x")
        && value[2..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl CustomerVerdict {
    pub fn validate(&self) -> Result<(), ReviewError> {
        if !lower_hex(&self.bounty_contract, 20)
            || !lower_hex(&self.submission_hash, 32)
            || !lower_hex(&self.evidence_hash, 32)
            || self.round == 0
        {
            return Err(ReviewError::Scope);
        }
        let min = if self.passed {
            0
        } else {
            MIN_REJECTION_REASON_CHARS
        };
        if !reason_ok(&self.reason, min) {
            return Err(ReviewError::Reason);
        }
        Ok(())
    }

    pub fn document(&self) -> Value {
        json!({
            "schema": REVIEW_SCHEMA,
            "bounty_contract": self.bounty_contract,
            "round": self.round,
            "submission_hash": self.submission_hash,
            "evidence_hash": self.evidence_hash,
            "passed": self.passed,
            "reason": self.reason,
        })
    }

    /// The attestation `responseHash`: Keccak-256 of the canonical public document.
    pub fn response_hash(&self) -> Result<String, ReviewError> {
        self.validate()?;
        chain_base::keccak256_canonical_json(&self.document()).map_err(|_| ReviewError::Scope)
    }
}

/// The solver's appeal of one recorded rejection. The solver's wallet signs it as EIP-712
/// `ReviewAppeal`, which shows the exact reason in the wallet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewAppeal {
    pub bounty_contract: String,
    pub round: u64,
    pub submission_hash: String,
    /// The response hash of the rejection document being appealed.
    pub rejection_hash: String,
    pub solver: String,
    pub reason: String,
}

impl ReviewAppeal {
    pub fn validate(&self) -> Result<(), ReviewError> {
        if !lower_hex(&self.bounty_contract, 20)
            || !lower_hex(&self.submission_hash, 32)
            || !lower_hex(&self.rejection_hash, 32)
            || !lower_hex(&self.solver, 20)
            || self.round == 0
        {
            return Err(ReviewError::Scope);
        }
        if !reason_ok(&self.reason, MIN_REJECTION_REASON_CHARS) {
            return Err(ReviewError::Reason);
        }
        Ok(())
    }

    /// Whether this appeal names exactly the given rejection.
    pub fn appeals(&self, rejection: &CustomerVerdict, rejection_hash: &str) -> bool {
        !rejection.passed
            && self.bounty_contract == rejection.bounty_contract
            && self.round == rejection.round
            && self.submission_hash == rejection.submission_hash
            && self.rejection_hash == rejection_hash
    }

    pub fn document(&self) -> Value {
        json!({
            "schema": APPEAL_SCHEMA,
            "bounty_contract": self.bounty_contract,
            "round": self.round,
            "submission_hash": self.submission_hash,
            "rejection_hash": self.rejection_hash,
            "solver": self.solver,
            "reason": self.reason,
        })
    }

    pub fn appeal_hash(&self) -> Result<String, ReviewError> {
        self.validate()?;
        chain_base::keccak256_canonical_json(&self.document()).map_err(|_| ReviewError::Scope)
    }
}

/// The arbiter's ruling on an appeal. `passed` true reverses the rejection: the solver is paid
/// and the customer's review deposit goes to the solver. The signed attestation commits to this
/// document, which names the rejection and the appeal it decides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppealRuling {
    pub bounty_contract: String,
    pub round: u64,
    pub submission_hash: String,
    pub evidence_hash: String,
    pub rejection_hash: String,
    pub appeal_hash: String,
    pub passed: bool,
    pub reason: String,
}

impl AppealRuling {
    /// The ruling on `appeal` of `rejection`, with the arbiter's public reason.
    pub fn on(
        rejection: &CustomerVerdict,
        appeal: &ReviewAppeal,
        reversed: bool,
        reason: String,
    ) -> Result<Self, ReviewError> {
        let rejection_hash = rejection.response_hash()?;
        if !appeal.appeals(rejection, &rejection_hash) {
            return Err(ReviewError::Scope);
        }
        let ruling = Self {
            bounty_contract: rejection.bounty_contract.clone(),
            round: rejection.round,
            submission_hash: rejection.submission_hash.clone(),
            evidence_hash: rejection.evidence_hash.clone(),
            rejection_hash,
            appeal_hash: appeal.appeal_hash()?,
            passed: reversed,
            reason,
        };
        ruling.validate()?;
        Ok(ruling)
    }

    pub fn validate(&self) -> Result<(), ReviewError> {
        if !lower_hex(&self.bounty_contract, 20)
            || !lower_hex(&self.submission_hash, 32)
            || !lower_hex(&self.evidence_hash, 32)
            || !lower_hex(&self.rejection_hash, 32)
            || !lower_hex(&self.appeal_hash, 32)
            || self.round == 0
        {
            return Err(ReviewError::Scope);
        }
        if !reason_ok(&self.reason, MIN_REJECTION_REASON_CHARS) {
            return Err(ReviewError::Reason);
        }
        Ok(())
    }

    pub fn document(&self) -> Value {
        json!({
            "schema": RULING_SCHEMA,
            "bounty_contract": self.bounty_contract,
            "round": self.round,
            "submission_hash": self.submission_hash,
            "evidence_hash": self.evidence_hash,
            "rejection_hash": self.rejection_hash,
            "appeal_hash": self.appeal_hash,
            "passed": self.passed,
            "reason": self.reason,
        })
    }

    pub fn response_hash(&self) -> Result<String, ReviewError> {
        self.validate()?;
        chain_base::keccak256_canonical_json(&self.document()).map_err(|_| ReviewError::Scope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verdict(passed: bool, reason: &str) -> CustomerVerdict {
        CustomerVerdict {
            bounty_contract: format!("0x{}", "ab".repeat(20)),
            round: 1,
            submission_hash: format!("0x{}", "11".repeat(32)),
            evidence_hash: format!("0x{}", "22".repeat(32)),
            passed,
            reason: reason.into(),
        }
    }

    #[test]
    fn response_hash_commits_to_every_field() {
        let base = verdict(true, "");
        let hash = base.response_hash().unwrap();
        assert!(lower_hex(&hash, 32));
        let mut changes = vec![];
        for change in [
            |v: &mut CustomerVerdict| v.round = 2,
            |v: &mut CustomerVerdict| v.submission_hash = format!("0x{}", "33".repeat(32)),
            |v: &mut CustomerVerdict| v.evidence_hash = format!("0x{}", "44".repeat(32)),
            |v: &mut CustomerVerdict| v.bounty_contract = format!("0x{}", "cd".repeat(20)),
            |v: &mut CustomerVerdict| v.reason = "Looks good.".into(),
        ] {
            let mut changed = base.clone();
            change(&mut changed);
            changes.push(changed.response_hash().unwrap());
        }
        let rejected = verdict(false, "The report omits the requested benchmark.");
        changes.push(rejected.response_hash().unwrap());
        changes.push(hash);
        changes.sort();
        changes.dedup();
        assert_eq!(changes.len(), 7);
    }

    #[test]
    fn scope_and_reason_rules_fail_closed() {
        assert_eq!(
            verdict(false, "too short").validate(),
            Err(ReviewError::Reason)
        );
        assert_eq!(
            verdict(true, &"x".repeat(MAX_REASON_CHARS + 1)).validate(),
            Err(ReviewError::Reason)
        );
        assert_eq!(
            verdict(true, " padded").validate(),
            Err(ReviewError::Reason)
        );
        assert_eq!(
            verdict(false, "Contains a control \u{7} character here").validate(),
            Err(ReviewError::Reason)
        );
        verdict(false, "Line one of the reason.\nLine two.")
            .validate()
            .unwrap();
        for change in [
            |v: &mut CustomerVerdict| v.round = 0,
            |v: &mut CustomerVerdict| v.bounty_contract = v.bounty_contract.to_uppercase(),
            |v: &mut CustomerVerdict| v.submission_hash.truncate(10),
        ] {
            let mut changed = verdict(true, "");
            change(&mut changed);
            assert_eq!(changed.validate(), Err(ReviewError::Scope));
        }
    }

    fn appeal_of(rejection: &CustomerVerdict) -> ReviewAppeal {
        ReviewAppeal {
            bounty_contract: rejection.bounty_contract.clone(),
            round: rejection.round,
            submission_hash: rejection.submission_hash.clone(),
            rejection_hash: rejection.response_hash().unwrap(),
            solver: format!("0x{}", "5e".repeat(20)),
            reason: "The report includes the benchmark in section 3.".into(),
        }
    }

    #[test]
    fn schedule_leaves_appeal_and_ruling_time_and_never_auto_accepts_a_late_delivery() {
        let window = 7 * 86_400;
        let submitted = 1_000_000;
        let expires = submitted + window;
        let on_time = ReviewSchedule::new(expires, window, submitted - 50).unwrap();
        assert_eq!(on_time.decide_by, submitted + DECISION_SECONDS);
        assert!(on_time.auto_accept);
        assert_eq!(on_time.sign_by, expires - SETTLEMENT_MARGIN_SECONDS);
        // A rejection at the last moment still has the whole appeal and ruling periods.
        assert!(appeal_ends_at(on_time.decide_by) + ARBITRATION_SECONDS <= on_time.sign_by);

        // Publishing late cannot shorten the customer's decision into a forced acceptance.
        let late = ReviewSchedule::new(expires, window, submitted + 2 * 86_400).unwrap();
        assert!(!late.auto_accept);
        assert_eq!(
            late.decide_by,
            expires - SETTLEMENT_MARGIN_SECONDS - APPEAL_SECONDS - ARBITRATION_SECONDS
        );
        let minimum = ReviewSchedule::new(
            submitted + MIN_VERIFICATION_WINDOW_SECONDS,
            MIN_VERIFICATION_WINDOW_SECONDS,
            submitted,
        )
        .unwrap();
        assert!(minimum.auto_accept);
        assert!(ReviewSchedule::new(10, 20, 0).is_none());
    }

    #[test]
    fn appeal_and_ruling_bind_the_exact_rejection() {
        let rejection = verdict(false, "The report omits the requested benchmark.");
        let appeal = appeal_of(&rejection);
        let reversed = AppealRuling::on(
            &rejection,
            &appeal,
            true,
            "Section 3 contains the requested benchmark.".into(),
        )
        .unwrap();
        assert_eq!(reversed.document()["schema"], RULING_SCHEMA);
        let upheld = AppealRuling::on(
            &rejection,
            &appeal,
            false,
            "Section 3 benchmarks a different dataset.".into(),
        )
        .unwrap();
        assert_ne!(
            reversed.response_hash().unwrap(),
            upheld.response_hash().unwrap()
        );
        assert_ne!(
            appeal.appeal_hash().unwrap(),
            rejection.response_hash().unwrap()
        );

        let mut other = appeal.clone();
        other.rejection_hash = format!("0x{}", "77".repeat(32));
        assert_eq!(
            AppealRuling::on(&rejection, &other, true, "x".repeat(30)),
            Err(ReviewError::Scope)
        );
        assert_eq!(
            AppealRuling::on(&verdict(true, ""), &appeal, true, "x".repeat(30)),
            Err(ReviewError::Scope)
        );
        let mut short = appeal.clone();
        short.reason = "Disagree.".into();
        assert_eq!(short.validate(), Err(ReviewError::Reason));
        assert_eq!(
            AppealRuling::on(&rejection, &appeal, false, "Upheld.".into()),
            Err(ReviewError::Reason)
        );
        assert!(verdict(true, AUTO_ACCEPT_REASON).validate().is_ok());
    }
}
