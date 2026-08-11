//! Open Competition contract bindings – V1 and V2 (SP1 proof backend).

/// Represents the on-chain state of an Open Competition bounty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCompetitionV1 {
    pub bounty_id: [u8; 32],
    pub reward_usdc: u64,
    pub solver: Option<[u8; 20]>,
    pub finalized: bool,
}

impl OpenCompetitionV1 {
    /// Enforce V1 invariants. Returns `Err` with a description if any invariant is violated.
    pub fn check_invariants(&self) -> Result<(), &'static str> {
        if self.reward_usdc == 0 {
            return Err("V1: reward_usdc must be non-zero");
        }
        if self.finalized && self.solver.is_none() {
            return Err("V1: finalized competition must have a solver");
        }
        Ok(())
    }
}

/// SP1 proof receipt attached to a V2 competition settlement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sp1ProofReceipt {
    /// The SP1 program verification key hash (32 bytes).
    pub vk_hash: [u8; 32],
    /// The public values committed by the guest program.
    pub public_values: Vec<u8>,
    /// The raw STARK/SNARK proof bytes as produced by SP1.
    pub proof_bytes: Vec<u8>,
}

impl Sp1ProofReceipt {
    /// Basic structural invariants – does not perform cryptographic verification.
    pub fn check_invariants(&self) -> Result<(), &'static str> {
        if self.vk_hash == [0u8; 32] {
            return Err("SP1: vk_hash must not be the zero hash");
        }
        if self.public_values.is_empty() {
            return Err("SP1: public_values must not be empty");
        }
        if self.proof_bytes.is_empty() {
            return Err("SP1: proof_bytes must not be empty");
        }
        Ok(())
    }
}

/// Represents the on-chain state of an Open Competition V2 bounty backed by an
/// SP1 proof of correct evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCompetitionV2 {
    pub bounty_id: [u8; 32],
    pub reward_usdc: u64,
    pub solver: Option<[u8; 20]>,
    pub finalized: bool,
    /// SP1 proof receipt that must be present when the competition is finalized.
    pub sp1_proof: Option<Sp1ProofReceipt>,
    /// The evaluation score committed inside the SP1 proof public values.
    pub committed_score: Option<u64>,
}

impl OpenCompetitionV2 {
    /// Enforce all V2 contract invariants.
    ///
    /// Invariants:
    /// 1. `reward_usdc` must be non-zero.
    /// 2. A finalized competition must have a solver address.
    /// 3. A finalized competition must have an attached SP1 proof receipt.
    /// 4. The SP1 proof receipt must itself satisfy its own structural invariants.
    /// 5. A finalized competition must have a committed score.
    /// 6. An unfinalized competition must not have a solver.
    pub fn check_invariants(&self) -> Result<(), &'static str> {
        if self.reward_usdc == 0 {
            return Err("V2: reward_usdc must be non-zero");
        }

        if self.finalized {
            if self.solver.is_none() {
                return Err("V2: finalized competition must have a solver");
            }
            match &self.sp1_proof {
                None => return Err("V2: finalized competition must include an SP1 proof receipt"),
                Some(receipt) => receipt.check_invariants()?,
            }
            if self.committed_score.is_none() {
                return Err("V2: finalized competition must have a committed_score");
            }
        } else {
            if self.solver.is_some() {
                return Err("V2: unfinalized competition must not have a solver");
            }
            if self.sp1_proof.is_some() {
                return Err("V2: unfinalized competition must not have an SP1 proof");
            }
        }

        Ok(())
    }

    /// Decode the committed score from the SP1 public values (first 8 bytes, little-endian).
    /// Returns `None` if no proof is attached or the public values are too short.
    pub fn decode_committed_score(&self) -> Option<u64> {
        let receipt = self.sp1_proof.as_ref()?;
        let bytes: [u8; 8] = receipt.public_values.get(..8)?.try_into().ok()?;
        Some(u64::from_le_bytes(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_vk() -> [u8; 32] {
        let mut v = [0u8; 32];
        v[0] = 0xab;
        v
    }

    fn valid_receipt() -> Sp1ProofReceipt {
        let mut public_values = vec![0u8; 8];
        // encode score = 42
        public_values[..8].copy_from_slice(&42u64.to_le_bytes());
        Sp1ProofReceipt {
            vk_hash: dummy_vk(),
            public_values,
            proof_bytes: vec![0xde, 0xad, 0xbe, 0xef],
        }
    }

    fn valid_finalized_v2() -> OpenCompetitionV2 {
        OpenCompetitionV2 {
            bounty_id: [1u8; 32],
            reward_usdc: 500,
            solver: Some([0xaau8; 20]),
            finalized: true,
            sp1_proof: Some(valid_receipt()),
            committed_score: Some(42),
        }
    }

    // ── V1 tests ──────────────────────────────────────────────────────────────

    #[test]
    fn v1_valid_open() {
        let c = OpenCompetitionV1 {
            bounty_id: [0u8; 32],
            reward_usdc: 100,
            solver: None,
            finalized: false,
        };
        assert!(c.check_invariants().is_ok());
    }

    #[test]
    fn v1_valid_finalized() {
        let c = OpenCompetitionV1 {
            bounty_id: [0u8; 32],
            reward_usdc: 100,
            solver: Some([0u8; 20]),
            finalized: true,
        };
        assert!(c.check_invariants().is_ok());
    }

    #[test]
    fn v1_zero_reward_rejected() {
        let c = OpenCompetitionV1 {
            bounty_id: [0u8; 32],
            reward_usdc: 0,
            solver: None,
            finalized: false,
        };
        assert_eq!(c.check_invariants(), Err("V1: reward_usdc must be non-zero"));
    }

    #[test]
    fn v1_finalized_without_solver_rejected() {
        let c = OpenCompetitionV1 {
            bounty_id: [0u8; 32],
            reward_usdc: 100,
            solver: None,
            finalized: true,
        };
        assert_eq!(
            c.check_invariants(),
            Err("V1: finalized competition must have a solver")
        );
    }

    // ── SP1 receipt tests ─────────────────────────────────────────────────────

    #[test]
    fn sp1_receipt_valid() {
        assert!(valid_receipt().check_invariants().is_ok());
    }

    #[test]
    fn sp1_receipt_zero_vk_rejected() {
        let mut r = valid_receipt();
        r.vk_hash = [0u8; 32];
        assert_eq!(r.check_invariants(), Err("SP1: vk_hash must not be the zero hash"));
    }

    #[test]
    fn sp1_receipt_empty_public_values_rejected() {
        let mut r = valid_receipt();
        r.public_values = vec![];
        assert_eq!(r.check_invariants(), Err("SP1: public_values must not be empty"));
    }

    #[test]
    fn sp1_receipt_empty_proof_bytes_rejected() {
        let mut r = valid_receipt();
        r.proof_bytes = vec![];
        assert_eq!(r.check_invariants(), Err("SP1: proof_bytes must not be empty"));
    }

    // ── V2 tests ──────────────────────────────────────────────────────────────

    #[test]
    fn v2_valid_finalized() {
        assert!(valid_finalized_v2().check_invariants().is_ok());
    }

    #[test]
    fn v2_valid_open() {
        let c = OpenCompetitionV2 {
            bounty_id: [1u8; 32],
            reward_usdc: 200,
            solver: None,
            finalized: false,
            sp1_proof: None,
            committed_score: None,
        };
        assert!(c.check_invariants().is_ok());
    }

    #[test]
    fn v2_zero_reward_rejected() {
        let mut c = valid_finalized_v2();
        c.reward_usdc = 0;
        assert_eq!(c.check_invariants(), Err("V2: reward_usdc must be non-zero"));
    }

    #[test]
    fn v2_finalized_without_solver_rejected() {
        let mut c = valid_finalized_v2();
        c.solver = None;
        assert_eq!(
            c.check_invariants(),
            Err("V2: finalized competition must have a solver")
        );
    }

    #[test]
    fn v2_finalized_without_proof_rejected() {
        let mut c = valid_finalized_v2();
        c.sp1_proof = None;
        assert_eq!(
            c.check_invariants(),
            Err("V2: finalized competition must include an SP1 proof receipt")
        );
    }

    #[test]
    fn v2_finalized_without_score_rejected() {
        let mut c = valid_finalized_v2();
        c.committed_score = None;
        assert_eq!(
            c.check_invariants(),
            Err("V2: finalized competition must have a committed_score")
        );
    }

    #[test]
    fn v2_unfinalized_with_solver_rejected() {
        let c = OpenCompetitionV2 {
            bounty_id: [1u8; 32],
            reward_usdc: 200,
            solver: Some([0u8; 20]),
            finalized: false,
            sp1_proof: None,
            committed_score: None,
        };
        assert_eq!(
            c.check_invariants(),
            Err("V2: unfinalized competition must not have a solver")
        );
    }

    #[test]
    fn v2_unfinalized_with_proof_rejected() {
        let c = OpenCompetitionV2 {
            bounty_id: [1u8; 32],
            reward_usdc: 200,
            solver: None,
            finalized: false,
            sp1_proof: Some(valid_receipt()),
            committed_score: None,
        };
        assert_eq!(
            c.check_invariants(),
            Err("V2: unfinalized competition must not have an SP1 proof")
        );
    }

    #[test]
    fn v2_decode_committed_score() {
        let c = valid_finalized_v2();
        assert_eq!(c.decode_committed_score(), Some(42));
    }

    #[test]
    fn v2_decode_committed_score_no_proof() {
        let c = OpenCompetitionV2 {
            bounty_id: [1u8; 32],
            reward_usdc: 200,
            solver: None,
            finalized: false,
            sp1_proof: None,
            committed_score: None,
        };
        assert_eq!(c.decode_committed_score(), None);
    }
}