use alloy::{
    primitives::{keccak256, Address, I256, U256},
    sol,
    sol_types::SolValue,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROTOCOL_VERSION: &str = "agent-bounties/open-competition-v2-beta3";
pub const JOURNAL_DOMAIN: &str = "agent-bounties/open-competition-v2-beta3/journal";
pub const PUBLIC_VECTOR_METRIC_V1: &str = "agent-bounties/public-vector-metric-v1";
pub const JOURNAL_ABI_LENGTH: usize = 20 * 32;
pub const MAXIMUM_VECTORS: usize = 10_000;

sol! {
    struct CompetitionJournalV2Abi {
        bytes32 domain;
        uint256 chainId;
        address competition;
        bytes32 bountyId;
        address solver;
        uint256 solverNonce;
        bytes32 submissionHash;
        bytes32 evidenceHash;
        bytes32 proofSystem;
        bytes32 programVKey;
        bytes32 sourceHash;
        bytes32 elfHash;
        bytes32 journalSchemaHash;
        bytes32 metricProgramHash;
        bytes32 executionPolicyHash;
        bytes32 verificationPolicyHash;
        bytes32 settlementPolicyHash;
        bytes32 betaRiskHash;
        bool passed;
        int256 score;
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MetricSdkError {
    #[error("invalid EVM address: {0}")]
    InvalidAddress(String),
    #[error("invalid bytes32 value: {0}")]
    InvalidHash(String),
    #[error("journal has invalid ABI length {0}")]
    InvalidJournalLength(usize),
    #[error("journal ABI decode failed")]
    JournalDecode,
    #[error("journal domain is not Open Competition V2 Beta3")]
    JournalDomain,
    #[error("metric input has no vectors")]
    EmptyVectors,
    #[error("metric input exceeds {MAXIMUM_VECTORS} vectors")]
    TooManyVectors,
    #[error("metric vector weights must be positive")]
    ZeroWeight,
    #[error("metric arithmetic overflow")]
    ArithmeticOverflow,
    #[error("metric threshold is negative")]
    NegativeThreshold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofSystem {
    Groth16,
    Plonk,
}

impl ProofSystem {
    pub fn protocol_id(self) -> [u8; 32] {
        let value = match self {
            Self::Groth16 => "sp1-groth16",
            Self::Plonk => "sp1-plonk",
        };
        keccak256(value).0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramClassification {
    Reviewed,
    CustomUnreviewed,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricManifest {
    pub schema: String,
    pub name: String,
    pub version: String,
    pub classification: ProgramClassification,
    pub source_hash: String,
    pub elf_hash: String,
    pub program_vkey: String,
    pub journal_schema_hash: String,
    pub fixtures_hash: String,
    pub maximum_vectors: u32,
    pub maximum_weight: u32,
    pub measured_cycle_limit: Option<u64>,
    pub measured_memory_bytes: Option<u64>,
}

impl MetricManifest {
    pub fn is_usable_in_hosted_discovery(&self) -> bool {
        self.classification != ProgramClassification::Disabled
    }

    pub fn review_evidence_complete(&self) -> bool {
        self.classification == ProgramClassification::Reviewed
            && !is_zero_hash(&self.source_hash)
            && !is_zero_hash(&self.elf_hash)
            && !is_zero_hash(&self.program_vkey)
            && !is_zero_hash(&self.journal_schema_hash)
            && !is_zero_hash(&self.fixtures_hash)
            && self.maximum_vectors > 0
            && self.maximum_weight > 0
            && self.measured_cycle_limit.is_some()
            && self.measured_memory_bytes.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompetitionJournalV2 {
    pub chain_id: u64,
    pub competition: String,
    pub bounty_id: String,
    pub solver: String,
    pub solver_nonce: u128,
    pub submission_hash: String,
    pub evidence_hash: String,
    pub proof_system: ProofSystem,
    pub program_vkey: String,
    pub source_hash: String,
    pub elf_hash: String,
    pub journal_schema_hash: String,
    pub metric_program_hash: String,
    pub execution_policy_hash: String,
    pub verification_policy_hash: String,
    pub settlement_policy_hash: String,
    pub beta_risk_hash: String,
    pub passed: bool,
    pub score: i128,
}

impl CompetitionJournalV2 {
    pub fn abi_encode(&self) -> Result<Vec<u8>, MetricSdkError> {
        let journal = CompetitionJournalV2Abi {
            domain: keccak256(JOURNAL_DOMAIN),
            chainId: U256::from(self.chain_id),
            competition: parse_address(&self.competition)?,
            bountyId: parse_hash(&self.bounty_id)?.into(),
            solver: parse_address(&self.solver)?,
            solverNonce: U256::from(self.solver_nonce),
            submissionHash: parse_hash(&self.submission_hash)?.into(),
            evidenceHash: parse_hash(&self.evidence_hash)?.into(),
            proofSystem: self.proof_system.protocol_id().into(),
            programVKey: parse_hash(&self.program_vkey)?.into(),
            sourceHash: parse_hash(&self.source_hash)?.into(),
            elfHash: parse_hash(&self.elf_hash)?.into(),
            journalSchemaHash: parse_hash(&self.journal_schema_hash)?.into(),
            metricProgramHash: parse_hash(&self.metric_program_hash)?.into(),
            executionPolicyHash: parse_hash(&self.execution_policy_hash)?.into(),
            verificationPolicyHash: parse_hash(&self.verification_policy_hash)?.into(),
            settlementPolicyHash: parse_hash(&self.settlement_policy_hash)?.into(),
            betaRiskHash: parse_hash(&self.beta_risk_hash)?.into(),
            passed: self.passed,
            score: I256::try_from(self.score).map_err(|_| MetricSdkError::ArithmeticOverflow)?,
        };
        let encoded = journal.abi_encode();
        debug_assert_eq!(encoded.len(), JOURNAL_ABI_LENGTH);
        Ok(encoded)
    }

    pub fn abi_hash(&self) -> Result<String, MetricSdkError> {
        Ok(format!("{:#x}", keccak256(self.abi_encode()?)))
    }
}

pub fn validate_journal_abi(bytes: &[u8]) -> Result<(), MetricSdkError> {
    if bytes.len() != JOURNAL_ABI_LENGTH {
        return Err(MetricSdkError::InvalidJournalLength(bytes.len()));
    }
    let decoded =
        CompetitionJournalV2Abi::abi_decode(bytes).map_err(|_| MetricSdkError::JournalDecode)?;
    if decoded.domain != keccak256(JOURNAL_DOMAIN) {
        return Err(MetricSdkError::JournalDomain);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicVectorMode {
    AllEqual,
    MaximizeExactMatches,
    MinimizeAbsoluteError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicVectorCase {
    pub expected: i64,
    pub observed: i64,
    pub weight: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicVectorMetricInput {
    pub mode: PublicVectorMode,
    pub threshold: i128,
    pub vectors: Vec<PublicVectorCase>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicVectorMetricResult {
    pub passed: bool,
    pub score: i128,
}

pub fn evaluate_public_vectors(
    input: &PublicVectorMetricInput,
) -> Result<PublicVectorMetricResult, MetricSdkError> {
    if input.vectors.is_empty() {
        return Err(MetricSdkError::EmptyVectors);
    }
    if input.vectors.len() > MAXIMUM_VECTORS {
        return Err(MetricSdkError::TooManyVectors);
    }
    if input.vectors.iter().any(|vector| vector.weight == 0) {
        return Err(MetricSdkError::ZeroWeight);
    }
    match input.mode {
        PublicVectorMode::AllEqual => {
            let mut exact_weight = 0_i128;
            let mut total_weight = 0_i128;
            for vector in &input.vectors {
                let weight = i128::from(vector.weight);
                total_weight = total_weight
                    .checked_add(weight)
                    .ok_or(MetricSdkError::ArithmeticOverflow)?;
                if vector.expected == vector.observed {
                    exact_weight = exact_weight
                        .checked_add(weight)
                        .ok_or(MetricSdkError::ArithmeticOverflow)?;
                }
            }
            Ok(PublicVectorMetricResult {
                passed: exact_weight == total_weight,
                score: exact_weight,
            })
        }
        PublicVectorMode::MaximizeExactMatches => {
            if input.threshold < 0 {
                return Err(MetricSdkError::NegativeThreshold);
            }
            let mut score = 0_i128;
            for vector in &input.vectors {
                if vector.expected == vector.observed {
                    score = score
                        .checked_add(i128::from(vector.weight))
                        .ok_or(MetricSdkError::ArithmeticOverflow)?;
                }
            }
            Ok(PublicVectorMetricResult {
                passed: score >= input.threshold,
                score,
            })
        }
        PublicVectorMode::MinimizeAbsoluteError => {
            if input.threshold < 0 {
                return Err(MetricSdkError::NegativeThreshold);
            }
            let mut score = 0_i128;
            for vector in &input.vectors {
                let difference = i128::from(vector.expected)
                    .checked_sub(i128::from(vector.observed))
                    .ok_or(MetricSdkError::ArithmeticOverflow)?
                    .abs();
                let weighted = difference
                    .checked_mul(i128::from(vector.weight))
                    .ok_or(MetricSdkError::ArithmeticOverflow)?;
                score = score
                    .checked_add(weighted)
                    .ok_or(MetricSdkError::ArithmeticOverflow)?;
            }
            Ok(PublicVectorMetricResult {
                passed: score <= input.threshold,
                score,
            })
        }
    }
}

fn parse_address(value: &str) -> Result<Address, MetricSdkError> {
    value
        .parse::<Address>()
        .map_err(|_| MetricSdkError::InvalidAddress(value.to_string()))
}

fn parse_hash(value: &str) -> Result<[u8; 32], MetricSdkError> {
    let stripped = value.strip_prefix("0x").unwrap_or(value);
    let bytes =
        hex::decode(stripped).map_err(|_| MetricSdkError::InvalidHash(value.to_string()))?;
    bytes
        .try_into()
        .map_err(|_| MetricSdkError::InvalidHash(value.to_string()))
}

fn is_zero_hash(value: &str) -> bool {
    match parse_hash(value) {
        Ok(hash) => hash == [0_u8; 32],
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn hash(byte: u8) -> String {
        format!("0x{}", hex::encode([byte; 32]))
    }

    fn fixture() -> CompetitionJournalV2 {
        CompetitionJournalV2 {
            chain_id: 84_532,
            competition: "0x1111111111111111111111111111111111111111".to_string(),
            bounty_id: hash(0x22),
            solver: "0x3333333333333333333333333333333333333333".to_string(),
            solver_nonce: 7,
            submission_hash: hash(0x44),
            evidence_hash: hash(0x55),
            proof_system: ProofSystem::Groth16,
            program_vkey: hash(0x66),
            source_hash: hash(0x77),
            elf_hash: hash(0x88),
            journal_schema_hash: hash(0x99),
            metric_program_hash: hash(0xaa),
            execution_policy_hash: hash(0xbb),
            verification_policy_hash: hash(0xcc),
            settlement_policy_hash: hash(0xdd),
            beta_risk_hash: hash(0xee),
            passed: true,
            score: -5,
        }
    }

    #[test]
    fn journal_is_exact_fixed_solidity_abi() {
        let encoded = fixture().abi_encode().unwrap();
        assert_eq!(encoded.len(), JOURNAL_ABI_LENGTH);
        validate_journal_abi(&encoded).unwrap();
        assert_eq!(
            fixture().abi_hash().unwrap(),
            "0x4e5101adf422615af036e756448e0c016e7bd04051f4cb33c0248f2b072d41e7"
        );
    }

    #[test]
    fn public_vector_modes_have_explicit_semantics() {
        let vectors = vec![
            PublicVectorCase {
                expected: 2,
                observed: 2,
                weight: 3,
            },
            PublicVectorCase {
                expected: 5,
                observed: 3,
                weight: 2,
            },
        ];
        assert_eq!(
            evaluate_public_vectors(&PublicVectorMetricInput {
                mode: PublicVectorMode::AllEqual,
                threshold: 0,
                vectors: vectors.clone(),
            })
            .unwrap(),
            PublicVectorMetricResult {
                passed: false,
                score: 3
            }
        );
        assert_eq!(
            evaluate_public_vectors(&PublicVectorMetricInput {
                mode: PublicVectorMode::MaximizeExactMatches,
                threshold: 3,
                vectors: vectors.clone(),
            })
            .unwrap(),
            PublicVectorMetricResult {
                passed: true,
                score: 3
            }
        );
        assert_eq!(
            evaluate_public_vectors(&PublicVectorMetricInput {
                mode: PublicVectorMode::MinimizeAbsoluteError,
                threshold: 4,
                vectors,
            })
            .unwrap(),
            PublicVectorMetricResult {
                passed: true,
                score: 4
            }
        );
    }

    #[test]
    fn public_vector_limits_match_the_guest() {
        let input = PublicVectorMetricInput {
            mode: PublicVectorMode::AllEqual,
            threshold: 0,
            vectors: vec![PublicVectorCase {
                expected: 1,
                observed: 1,
                weight: 0,
            }],
        };
        assert_eq!(
            evaluate_public_vectors(&input),
            Err(MetricSdkError::ZeroWeight)
        );

        let mut oversized = input;
        oversized.vectors = vec![
            PublicVectorCase {
                expected: 1,
                observed: 1,
                weight: 1,
            };
            MAXIMUM_VECTORS + 1
        ];
        assert_eq!(
            evaluate_public_vectors(&oversized),
            Err(MetricSdkError::TooManyVectors)
        );
    }

    proptest! {
        #[test]
        fn exact_match_score_never_exceeds_total_weight(
            pairs in prop::collection::vec((any::<i16>(), any::<i16>(), 1_u16..1000), 1..100)
        ) {
            let vectors = pairs.into_iter().map(|(expected, observed, weight)| PublicVectorCase {
                expected: i64::from(expected),
                observed: i64::from(observed),
                weight: u32::from(weight),
            }).collect::<Vec<_>>();
            let total = vectors.iter().map(|value| i128::from(value.weight)).sum::<i128>();
            let result = evaluate_public_vectors(&PublicVectorMetricInput {
                mode: PublicVectorMode::MaximizeExactMatches,
                threshold: 0,
                vectors,
            }).unwrap();
            prop_assert!(result.score >= 0 && result.score <= total);
            prop_assert!(result.passed);
        }
    }
}
