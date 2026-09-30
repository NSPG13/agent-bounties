use crate::{invalid, Invalid, Plan};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const DAY: u64 = 86_400;
// Exact dates in the same range browsers can display without rounding.
const MAX_DATE: u64 = 253_402_300_799;

/// Unix seconds. Equal cutoffs mean the unused stage is omitted.
/// These rules describe new contests only; they never replace a legacy clock.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TimeWindows {
    pub funding_cutoff: u64,
    pub submission_cutoff: u64,
    pub checks_cutoff: u64,
    pub review_cutoff: u64,
    pub final_cutoff: u64,
}

impl TimeWindows {
    pub fn recommended(
        plan: &Plan,
        funding_cutoff: u64,
        submission_cutoff: u64,
    ) -> Result<Self, Invalid> {
        if funding_cutoff == 0
            || funding_cutoff >= submission_cutoff
            || submission_cutoff > MAX_DATE - 30 * DAY
        {
            return Err(invalid("funding_must_close_before_submissions"));
        }
        let has_checks = !plan.checks.is_empty();
        let has_review = plan.criteria.iter().any(|c| c.review.is_some());
        let checks_cutoff = submission_cutoff + if has_checks { DAY } else { 0 };
        let review_cutoff = checks_cutoff + if has_review { 2 * DAY } else { 0 };
        let final_cutoff = review_cutoff + if has_review { DAY } else { 0 };
        Ok(Self {
            funding_cutoff,
            submission_cutoff,
            checks_cutoff,
            review_cutoff,
            final_cutoff,
        })
    }

    pub fn validate(&self, plan: &Plan) -> Result<(), Invalid> {
        let recommended = Self::recommended(plan, self.funding_cutoff, self.submission_cutoff)?;
        if self.final_cutoff > self.submission_cutoff + 30 * DAY {
            return Err(invalid("verification_windows_exceed_30_days"));
        }
        if plan.checks.is_empty() {
            if self.checks_cutoff != self.submission_cutoff {
                return Err(invalid("omit_unused_check_window"));
            }
        } else if self.checks_cutoff < recommended.checks_cutoff {
            return Err(invalid("allow_at_least_24_hours_for_checks"));
        }
        if !plan.criteria.iter().any(|c| c.review.is_some()) {
            if self.review_cutoff != self.checks_cutoff || self.final_cutoff != self.review_cutoff {
                return Err(invalid("omit_unused_review_and_recovery_windows"));
            }
        } else if self
            .checks_cutoff
            .checked_add(2 * DAY)
            .is_none_or(|end| self.review_cutoff < end)
        {
            return Err(invalid("allow_at_least_48_hours_for_review"));
        } else if self.review_cutoff.checked_add(DAY) != Some(self.final_cutoff) {
            return Err(invalid("recovery_window_must_be_24_hours"));
        }
        if self.final_cutoff < self.checks_cutoff || self.final_cutoff < self.review_cutoff {
            return Err(invalid("verification_deadlines_out_of_order"));
        }
        Ok(())
    }

    /// Funding admission must also reserve capacity and confirm reviewer readiness.
    pub fn validate_before_funding(&self, plan: &Plan, now: u64) -> Result<(), Invalid> {
        self.validate(plan)?;
        if now >= self.funding_cutoff || self.submission_cutoff > now.saturating_add(366 * DAY) {
            return Err(invalid("funding_closed_or_submission_cutoff_too_far"));
        }
        Ok(())
    }
}
