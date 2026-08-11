//! Open Competition V2 contract bindings and invariant checks.
//!
//! Invariants enforced:
//! 1. `solver_reward + verifier_reward <= total_funded` — rewards cannot exceed
//!    the amount actually deposited into the contract.
//! 2. `deadline > created_at` — the competition window must be positive.
//! 3. `solver_reward > 0` — a zero solver reward is not a valid competition.
//! 4. `state` transitions are monotonic: Created → Funded → Active → Closed.
//! 5. A winner may only be recorded when `state == Active`.
//! 6. `winner` is `None` unless `state == Closed`.

use alloy_primitives::{Address, U256};
use serde::{Deserialize, Serialize};

/// On-chain state of an Open Competition V2 posting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompetitionState {
    Created = 0,
    Funded = 1,
    Active = 2,
    Closed = 3,
}

impl TryFrom<u8> for CompetitionState {
    type Error = InvariantError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Created),
            1 => Ok(Self::Funded),
            2 => Ok(Self::Active),
            3 => Ok(Self::Closed),
            other => Err(InvariantError::InvalidState(other)),
        }
    }
}

/// Errors produced when an invariant is violated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvariantError {
    /// solver_reward + verifier_reward > total_funded
    RewardsExceedFunding {
        solver_reward: U256,
        verifier_reward: U256,
        total_funded: U256,
    },
    /// deadline <= created_at
    NonPositiveWindow { created_at: u64, deadline: u64 },
    /// solver_reward == 0
    ZeroSolverReward,
    /// A winner was recorded but state != Closed
    WinnerWithoutClose { state: CompetitionState },
    /// state == Closed but no winner recorded
    ClosedWithoutWinner,
    /// Proposed next state would be a non-monotonic transition
    NonMonotonicTransition {
        from: CompetitionState,
        to: CompetitionState,
    },
    /// winner set while state != Active (pre-close transition)
    WinnerRecordedOutsideActiveState { state: CompetitionState },
    /// Raw state byte is not a known variant
    InvalidState(u8),
}

impl std::fmt::Display for InvariantError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RewardsExceedFunding {
                solver_reward,
                verifier_reward,
                total_funded,
            } => write!(
                f,
                "rewards ({solver_reward} + {verifier_reward}) exceed total funded ({total_funded})"
            ),
            Self::NonPositiveWindow {
                created_at,
                deadline,
            } => write!(
                f,
                "deadline ({deadline}) must be strictly after created_at ({created_at})"
            ),
            Self::ZeroSolverReward => write!(f, "solver_reward must be > 0"),
            Self::WinnerWithoutClose { state } => write!(
                f,
                "winner recorded but state is {state:?}, expected Closed"
            ),
            Self::ClosedWithoutWinner => {
                write!(f, "state is Closed but no winner address was recorded")
            }
            Self::NonMonotonicTransition { from, to } => write!(
                f,
                "state transition {from:?} → {to:?} is not monotonically increasing"
            ),
            Self::WinnerRecordedOutsideActiveState { state } => write!(
                f,
                "winner can only be recorded when state is Active, got {state:?}"
            ),
            Self::InvalidState(byte) => write!(f, "unknown state byte: {byte}"),
        }
    }
}

impl std::error::Error for InvariantError {}

/// Decoded snapshot of an Open Competition V2 contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionV2 {
    pub competition_id: U256,
    pub creator: Address,
    pub solver_reward: U256,
    pub verifier_reward: U256,
    pub total_funded: U256,
    pub created_at: u64,
    pub deadline: u64,
    pub state: CompetitionState,
    /// `Some` only when `state == Closed`.
    pub winner: Option<Address>,
}

impl OpenCompetitionV2 {
    /// Validate all structural invariants.
    ///
    /// Returns `Ok(())` when every invariant holds, otherwise the first
    /// violation encountered.
    pub fn check_invariants(&self) -> Result<(), InvariantError> {
        // Invariant 3: solver_reward > 0
        if self.solver_reward.is_zero() {
            return Err(InvariantError::ZeroSolverReward);
        }

        // Invariant 1: solver_reward + verifier_reward <= total_funded
        let total_rewards = self.solver_reward.saturating_add(self.verifier_reward);
        if total_rewards > self.total_funded {
            return Err(InvariantError::RewardsExceedFunding {
                solver_reward: self.solver_reward,
                verifier_reward: self.verifier_reward,
                total_funded: self.total_funded,
            });
        }

        // Invariant 2: deadline > created_at
        if self.deadline <= self.created_at {
            return Err(InvariantError::NonPositiveWindow {
                created_at: self.created_at,
                deadline: self.deadline,
            });
        }

        // Invariant 6: winner is None unless state == Closed
        if self.winner.is_some() && self.state != CompetitionState::Closed {
            return Err(InvariantError::WinnerWithoutClose { state: self.state });
        }

        // Invariant 6 (converse): state == Closed implies winner is Some
        if self.state == CompetitionState::Closed && self.winner.is_none() {
            return Err(InvariantError::ClosedWithoutWinner);
        }

        Ok(())
    }

    /// Validate a proposed state transition.
    ///
    /// Transitions must be strictly monotonically increasing (Created → Funded
    /// → Active → Closed).  The caller is responsible for supplying the
    /// optional winner address when transitioning to `Closed`; this method
    /// only validates the state machine itself.
    pub fn validate_transition(
        &self,
        next_state: CompetitionState,
    ) -> Result<(), InvariantError> {
        if next_state <= self.state {
            return Err(InvariantError::NonMonotonicTransition {
                from: self.state,
                to: next_state,
            });
        }
        Ok(())
    }

    /// Record a winner and transition to `Closed`.
    ///
    /// Enforces invariant 5: a winner may only be recorded when the current
    /// state is `Active`.
    pub fn record_winner(&mut self, winner: Address) -> Result<(), InvariantError> {
        if self.state != CompetitionState::Active {
            return Err(InvariantError::WinnerRecordedOutsideActiveState {
                state: self.state,
            });
        }
        self.winner = Some(winner);
        self.state = CompetitionState::Closed;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;

    fn base() -> OpenCompetitionV2 {
        OpenCompetitionV2 {
            competition_id: U256::from(1u64),
            creator: address!("0000000000000000000000000000000000000001"),
            solver_reward: U256::from(100u64),
            verifier_reward: U256::from(20u64),
            total_funded: U256::from(120u64),
            created_at: 1_000_000,
            deadline: 1_086_400,
            state: CompetitionState::Active,
            winner: None,
        }
    }

    #[test]
    fn valid_active_competition_passes() {
        assert!(base().check_invariants().is_ok());
    }

    #[test]
    fn valid_closed_competition_passes() {
        let mut c = base();
        c.state = CompetitionState::Closed;
        c.winner = Some(address!("0000000000000000000000000000000000000002"));
        assert!(c.check_invariants().is_ok());
    }

    #[test]
    fn rewards_exceed_funding_fails() {
        let mut c = base();
        c.total_funded = U256::from(50u64);
        assert!(matches!(
            c.check_invariants(),
            Err(InvariantError::RewardsExceedFunding { .. })
        ));
    }

    #[test]
    fn zero_solver_reward_fails() {
        let mut c = base();
        c.solver_reward = U256::ZERO;
        assert!(matches!(
            c.check_invariants(),
            Err(InvariantError::ZeroSolverReward)
        ));
    }

    #[test]
    fn non_positive_window_fails() {
        let mut c = base();
        c.deadline = c.created_at;
        assert!(matches!(
            c.check_invariants(),
            Err(InvariantError::NonPositiveWindow { .. })
        ));
    }

    #[test]
    fn winner_without_close_fails() {
        let mut c = base(); // state == Active
        c.winner = Some(address!("0000000000000000000000000000000000000002"));
        assert!(matches!(
            c.check_invariants(),
            Err(InvariantError::WinnerWithoutClose { .. })
        ));
    }

    #[test]
    fn closed_without_winner_fails() {
        let mut c = base();
        c.state = CompetitionState::Closed;
        // winner remains None
        assert!(matches!(
            c.check_invariants(),
            Err(InvariantError::ClosedWithoutWinner)
        ));
    }

    #[test]
    fn non_monotonic_transition_fails() {
        let mut c = base();
        c.state = CompetitionState::Closed;
        c.winner = Some(address!("0000000000000000000000000000000000000002"));
        assert!(matches!(
            c.validate_transition(CompetitionState::Active),
            Err(InvariantError::NonMonotonicTransition { .. })
        ));
    }

    #[test]
    fn same_state_transition_fails() {
        let c = base(); // state == Active
        assert!(matches!(
            c.validate_transition(CompetitionState::Active),
            Err(InvariantError::NonMonotonicTransition { .. })
        ));
    }

    #[test]
    fn valid_transition_passes() {
        let c = base(); // state == Active
        assert!(c.validate_transition(CompetitionState::Closed).is_ok());
    }

    #[test]
    fn record_winner_from_active_succeeds() {
        let mut c = base();
        let winner = address!("0000000000000000000000000000000000000002");
        assert!(c.record_winner(winner).is_ok());
        assert_eq!(c.state, CompetitionState::Closed);
        assert_eq!(c.winner, Some(winner));
        assert!(c.check_invariants().is_ok());
    }

    #[test]
    fn record_winner_from_funded_fails() {
        let mut c = base();
        c.state = CompetitionState::Funded;
        let winner = address!("0000000000000000000000000000000000000002");
        assert!(matches!(
            c.record_winner(winner),
            Err(InvariantError::WinnerRecordedOutsideActiveState { .. })
        ));
    }

    #[test]
    fn rewards_exactly_equal_funding_passes() {
        let mut c = base();
        c.total_funded = U256::from(120u64); // exactly solver(100) + verifier(20)
        assert!(c.check_invariants().is_ok());
    }

    #[test]
    fn verifier_reward_zero_is_allowed() {
        let mut c = base();
        c.verifier_reward = U256::ZERO;
        c.total_funded = U256::from(100u64);
        assert!(c.check_invariants().is_ok());
    }
}