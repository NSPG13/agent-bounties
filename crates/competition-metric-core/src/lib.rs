#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::vec::Vec;
use serde::{Deserialize, Serialize};
use tiny_keccak::{Hasher, Keccak};

macro_rules! hex {
    ($value:literal) => {{
        const VALUE: &str = $value;
        const fn nibble(value: u8) -> u8 {
            match value {
                b'0'..=b'9' => value - b'0',
                b'a'..=b'f' => value - b'a' + 10,
                b'A'..=b'F' => value - b'A' + 10,
                _ => panic!("invalid hex literal"),
            }
        }
        const fn decode(value: &str) -> [u8; 32] {
            let bytes = value.as_bytes();
            if bytes.len() != 64 {
                panic!("bytes32 hex literal must contain 64 digits");
            }
            let mut out = [0_u8; 32];
            let mut index = 0;
            while index < 32 {
                out[index] = (nibble(bytes[index * 2]) << 4) | nibble(bytes[index * 2 + 1]);
                index += 1;
            }
            out
        }
        decode(VALUE)
    }};
}

pub const JOURNAL_ABI_LENGTH: usize = 20 * 32;
pub const MAXIMUM_VECTORS: usize = 10_000;
pub const JOURNAL_DOMAIN: [u8; 32] =
    hex!("110d7acc5c3397f452c974ba4f7296d7d2a2cede57290113d1fd256e1818804b");
pub const GROTH16_PROOF_SYSTEM: [u8; 32] =
    hex!("0fbfc39a4f588598b55fce747dc8dde3f1b661a9d538dc174b464d210d12a81d");
pub const PLONK_PROOF_SYSTEM: [u8; 32] =
    hex!("91e36d74d5d8703299314b82f85cab384a3df8064725b371f1f9f4ad49238f1b");
pub const METRIC_PROGRAM_HASH: [u8; 32] =
    hex!("1c27fc20ab65264c7db2997c8b76f78d7291cdb91243481bcae1e88f77beb88a");
pub const JOURNAL_SCHEMA_HASH: [u8; 32] =
    hex!("d9c492538aa0822e8a1d651886e79a2b8ddfc2c3428b3ed92e19d337eefe77d4");
const POLICY_DOMAIN: [u8; 32] =
    hex!("f6a226ca20aaca3b9c0b4a609939c334b6c2b03500a5df45188df8bcd7c2b369");
const SUBMISSION_DOMAIN: [u8; 32] =
    hex!("402204460b00978c26cee42ae0089d94fe8b0b17bd90c45a6cd78d466463a507");
const EVIDENCE_DOMAIN: [u8; 32] =
    hex!("16f60f26d350a38e6993a5454967d1efb0461d93785b7cdb38ba463284c5ab15");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicVectorMode {
    AllEqual,
    MaximizeExactMatches,
    MinimizeAbsoluteError,
}

impl PublicVectorMode {
    fn tag(self) -> u8 {
        match self {
            Self::AllEqual => 0,
            Self::MaximizeExactMatches => 1,
            Self::MinimizeAbsoluteError => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicVectorCase {
    pub expected: i64,
    pub observed: i64,
    pub weight: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalScopeV2 {
    pub chain_id: u64,
    pub competition: [u8; 20],
    pub bounty_id: [u8; 32],
    pub solver: [u8; 20],
    pub solver_nonce: u128,
    pub proof_system: [u8; 32],
    pub program_vkey: [u8; 32],
    pub source_hash: [u8; 32],
    pub elf_hash: [u8; 32],
    pub execution_policy_hash: [u8; 32],
    pub settlement_policy_hash: [u8; 32],
    pub beta_risk_hash: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicVectorProgramInput {
    pub scope: JournalScopeV2,
    pub mode: PublicVectorMode,
    pub threshold: i128,
    pub vectors: Vec<PublicVectorCase>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicVectorProgramOutput {
    pub passed: bool,
    pub score: i128,
    pub verification_policy_hash: [u8; 32],
    pub submission_hash: [u8; 32],
    pub evidence_hash: [u8; 32],
    pub journal: [u8; JOURNAL_ABI_LENGTH],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricError {
    EmptyVectors,
    TooManyVectors,
    ZeroWeight,
    NegativeThreshold,
    ArithmeticOverflow,
    UnsupportedProofSystem,
}

pub fn execute_public_vector_program(
    input: &PublicVectorProgramInput,
) -> Result<PublicVectorProgramOutput, MetricError> {
    if input.vectors.is_empty() {
        return Err(MetricError::EmptyVectors);
    }
    if input.vectors.len() > MAXIMUM_VECTORS {
        return Err(MetricError::TooManyVectors);
    }
    if input.vectors.iter().any(|vector| vector.weight == 0) {
        return Err(MetricError::ZeroWeight);
    }
    if !matches!(
        input.scope.proof_system,
        GROTH16_PROOF_SYSTEM | PLONK_PROOF_SYSTEM
    ) {
        return Err(MetricError::UnsupportedProofSystem);
    }

    let (passed, score) = evaluate(input)?;
    let verification_policy_hash = verification_policy_hash(input);
    let submission_hash = submission_hash(input);
    let evidence_hash = evidence_hash(verification_policy_hash, submission_hash);
    let journal = encode_journal(
        &input.scope,
        submission_hash,
        evidence_hash,
        verification_policy_hash,
        passed,
        score,
    );
    Ok(PublicVectorProgramOutput {
        passed,
        score,
        verification_policy_hash,
        submission_hash,
        evidence_hash,
        journal,
    })
}

fn evaluate(input: &PublicVectorProgramInput) -> Result<(bool, i128), MetricError> {
    if input.mode != PublicVectorMode::AllEqual && input.threshold < 0 {
        return Err(MetricError::NegativeThreshold);
    }
    let mut score = 0_i128;
    let mut total_weight = 0_i128;
    for vector in &input.vectors {
        let weight = i128::from(vector.weight);
        total_weight = total_weight
            .checked_add(weight)
            .ok_or(MetricError::ArithmeticOverflow)?;
        match input.mode {
            PublicVectorMode::AllEqual | PublicVectorMode::MaximizeExactMatches => {
                if vector.expected == vector.observed {
                    score = score
                        .checked_add(weight)
                        .ok_or(MetricError::ArithmeticOverflow)?;
                }
            }
            PublicVectorMode::MinimizeAbsoluteError => {
                let difference = i128::from(vector.expected)
                    .checked_sub(i128::from(vector.observed))
                    .ok_or(MetricError::ArithmeticOverflow)?
                    .checked_abs()
                    .ok_or(MetricError::ArithmeticOverflow)?;
                score = score
                    .checked_add(
                        difference
                            .checked_mul(weight)
                            .ok_or(MetricError::ArithmeticOverflow)?,
                    )
                    .ok_or(MetricError::ArithmeticOverflow)?;
            }
        }
    }
    let passed = match input.mode {
        PublicVectorMode::AllEqual => score == total_weight,
        PublicVectorMode::MaximizeExactMatches => score >= input.threshold,
        PublicVectorMode::MinimizeAbsoluteError => score <= input.threshold,
    };
    Ok((passed, score))
}

pub fn verification_policy_hash(input: &PublicVectorProgramInput) -> [u8; 32] {
    let mut bytes = Vec::with_capacity(32 + 1 + 16 + 4 + input.vectors.len() * 12);
    bytes.extend_from_slice(&POLICY_DOMAIN);
    bytes.push(input.mode.tag());
    bytes.extend_from_slice(&input.threshold.to_be_bytes());
    bytes.extend_from_slice(&(input.vectors.len() as u32).to_be_bytes());
    for vector in &input.vectors {
        bytes.extend_from_slice(&vector.expected.to_be_bytes());
        bytes.extend_from_slice(&vector.weight.to_be_bytes());
    }
    keccak256(&bytes)
}

pub fn submission_hash(input: &PublicVectorProgramInput) -> [u8; 32] {
    let mut bytes = Vec::with_capacity(32 + 4 + input.vectors.len() * 8);
    bytes.extend_from_slice(&SUBMISSION_DOMAIN);
    bytes.extend_from_slice(&(input.vectors.len() as u32).to_be_bytes());
    for vector in &input.vectors {
        bytes.extend_from_slice(&vector.observed.to_be_bytes());
    }
    keccak256(&bytes)
}

pub fn evidence_hash(policy_hash: [u8; 32], submission_hash: [u8; 32]) -> [u8; 32] {
    let mut bytes = [0_u8; 96];
    bytes[..32].copy_from_slice(&EVIDENCE_DOMAIN);
    bytes[32..64].copy_from_slice(&policy_hash);
    bytes[64..].copy_from_slice(&submission_hash);
    keccak256(&bytes)
}

fn encode_journal(
    scope: &JournalScopeV2,
    submission_hash: [u8; 32],
    evidence_hash: [u8; 32],
    verification_policy_hash: [u8; 32],
    passed: bool,
    score: i128,
) -> [u8; JOURNAL_ABI_LENGTH] {
    let mut journal = [0_u8; JOURNAL_ABI_LENGTH];
    let mut word = 0_usize;
    let mut push = |value: [u8; 32]| {
        journal[word * 32..(word + 1) * 32].copy_from_slice(&value);
        word += 1;
    };
    push(JOURNAL_DOMAIN);
    push(uint_word(u128::from(scope.chain_id)));
    push(address_word(scope.competition));
    push(scope.bounty_id);
    push(address_word(scope.solver));
    push(uint_word(scope.solver_nonce));
    push(submission_hash);
    push(evidence_hash);
    push(scope.proof_system);
    push(scope.program_vkey);
    push(scope.source_hash);
    push(scope.elf_hash);
    push(JOURNAL_SCHEMA_HASH);
    push(METRIC_PROGRAM_HASH);
    push(scope.execution_policy_hash);
    push(verification_policy_hash);
    push(scope.settlement_policy_hash);
    push(scope.beta_risk_hash);
    push(uint_word(u128::from(passed)));
    push(int_word(score));
    debug_assert_eq!(word, 20);
    journal
}

fn uint_word(value: u128) -> [u8; 32] {
    let mut word = [0_u8; 32];
    word[16..].copy_from_slice(&value.to_be_bytes());
    word
}

fn int_word(value: i128) -> [u8; 32] {
    let mut word = if value < 0 { [0xff_u8; 32] } else { [0_u8; 32] };
    word[16..].copy_from_slice(&value.to_be_bytes());
    word
}

fn address_word(value: [u8; 20]) -> [u8; 32] {
    let mut word = [0_u8; 32];
    word[12..].copy_from_slice(&value);
    word
}

fn keccak256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    hasher.update(bytes);
    let mut output = [0_u8; 32];
    hasher.finalize(&mut output);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PublicVectorProgramInput {
        PublicVectorProgramInput {
            scope: JournalScopeV2 {
                chain_id: 84_532,
                competition: [0x11; 20],
                bounty_id: [0x22; 32],
                solver: [0x33; 20],
                solver_nonce: 7,
                proof_system: GROTH16_PROOF_SYSTEM,
                program_vkey: [0x66; 32],
                source_hash: [0x77; 32],
                elf_hash: [0x88; 32],
                execution_policy_hash: [0xbb; 32],
                settlement_policy_hash: [0xdd; 32],
                beta_risk_hash: [0xee; 32],
            },
            mode: PublicVectorMode::MinimizeAbsoluteError,
            threshold: 4,
            vectors: alloc::vec![
                PublicVectorCase {
                    expected: 2,
                    observed: 2,
                    weight: 3
                },
                PublicVectorCase {
                    expected: 5,
                    observed: 3,
                    weight: 2
                },
            ],
        }
    }

    #[test]
    fn program_binds_policy_submission_and_exact_journal() {
        let output = execute_public_vector_program(&fixture()).unwrap();
        assert!(output.passed);
        assert_eq!(output.score, 4);
        assert_eq!(output.journal.len(), JOURNAL_ABI_LENGTH);
        assert_eq!(&output.journal[..32], &JOURNAL_DOMAIN);
        assert_eq!(&output.journal[6 * 32..7 * 32], &output.submission_hash);
        assert_eq!(&output.journal[7 * 32..8 * 32], &output.evidence_hash);
        assert_eq!(
            &output.journal[15 * 32..16 * 32],
            &output.verification_policy_hash
        );
    }

    #[test]
    fn shared_release_vector_matches_exactly() {
        let raw = include_str!("../../../programs/public-vector-metric-v1/fixtures/golden-v1.json");
        let input: PublicVectorProgramInput = serde_json::from_str(raw).unwrap();
        let fixture: serde_json::Value = serde_json::from_str(raw).unwrap();
        let expected = fixture.get("expected").unwrap();
        let output = execute_public_vector_program(&input).unwrap();

        assert_eq!(output.passed, expected["passed"].as_bool().unwrap());
        assert_eq!(
            output.score.to_string(),
            expected["score"].as_str().unwrap()
        );
        assert_eq!(
            format!("0x{}", hex::encode(output.verification_policy_hash)),
            expected["verification_policy_hash"].as_str().unwrap()
        );
        assert_eq!(
            format!("0x{}", hex::encode(output.submission_hash)),
            expected["submission_hash"].as_str().unwrap()
        );
        assert_eq!(
            format!("0x{}", hex::encode(output.evidence_hash)),
            expected["evidence_hash"].as_str().unwrap()
        );
        assert_eq!(
            format!("0x{}", hex::encode(output.journal)),
            expected["journal_hex"].as_str().unwrap()
        );
    }

    #[test]
    fn zero_weight_and_unapproved_proof_system_fail_closed() {
        let mut input = fixture();
        input.vectors[0].weight = 0;
        assert_eq!(
            execute_public_vector_program(&input),
            Err(MetricError::ZeroWeight)
        );
        input = fixture();
        input.scope.proof_system = [0x99; 32];
        assert_eq!(
            execute_public_vector_program(&input),
            Err(MetricError::UnsupportedProofSystem)
        );
    }

    fn oracle(input: &PublicVectorProgramInput) -> (bool, i128) {
        let exact = input
            .vectors
            .iter()
            .filter(|vector| vector.expected == vector.observed)
            .map(|vector| i128::from(vector.weight))
            .sum::<i128>();
        let total = input
            .vectors
            .iter()
            .map(|vector| i128::from(vector.weight))
            .sum::<i128>();
        let error = input
            .vectors
            .iter()
            .map(|vector| {
                (i128::from(vector.expected) - i128::from(vector.observed)).abs()
                    * i128::from(vector.weight)
            })
            .sum::<i128>();
        match input.mode {
            PublicVectorMode::AllEqual => (exact == total, exact),
            PublicVectorMode::MaximizeExactMatches => (exact >= input.threshold, exact),
            PublicVectorMode::MinimizeAbsoluteError => (error <= input.threshold, error),
        }
    }

    #[test]
    fn adversarial_fixture_corpus_has_zero_classification_errors() {
        let mut evaluated = 0_usize;
        for seed in 0_i64..128 {
            let vectors = alloc::vec![
                PublicVectorCase {
                    expected: seed - 64,
                    observed: seed - 64,
                    weight: 1 + (seed as u32 % 17),
                },
                PublicVectorCase {
                    expected: i64::MIN + seed,
                    observed: i64::MAX - seed,
                    weight: 1 + (seed as u32 % 31),
                },
                PublicVectorCase {
                    expected: seed.saturating_mul(seed),
                    observed: seed.saturating_mul(seed).saturating_add(seed % 5),
                    weight: 1 + (seed as u32 % 43),
                },
            ];
            for (mode, threshold) in [
                (PublicVectorMode::AllEqual, -1),
                (
                    PublicVectorMode::MaximizeExactMatches,
                    i128::from(seed % 50),
                ),
                (
                    PublicVectorMode::MinimizeAbsoluteError,
                    i128::from(seed).saturating_mul(i128::from(seed % 11)),
                ),
            ] {
                let mut input = fixture();
                input.mode = mode;
                input.threshold = threshold;
                input.vectors = vectors.clone();
                let expected = oracle(&input);
                let output = execute_public_vector_program(&input).unwrap();
                assert_eq!(
                    (output.passed, output.score),
                    expected,
                    "seed={seed} mode={mode:?}"
                );
                evaluated += 1;
            }
        }
        assert_eq!(evaluated, 384);
    }

    #[test]
    fn every_replay_boundary_changes_the_journal() {
        let original = fixture();
        let baseline = execute_public_vector_program(&original).unwrap().journal;
        let mut mutations = alloc::vec![original.clone(); 9];
        mutations[0].scope.chain_id += 1;
        mutations[1].scope.competition[0] ^= 1;
        mutations[2].scope.bounty_id[0] ^= 1;
        mutations[3].scope.solver[0] ^= 1;
        mutations[4].scope.solver_nonce += 1;
        mutations[5].scope.execution_policy_hash[0] ^= 1;
        mutations[6].scope.settlement_policy_hash[0] ^= 1;
        mutations[7].vectors[0].expected += 1;
        mutations[8].vectors[0].observed += 1;
        for mutation in mutations {
            assert_ne!(
                execute_public_vector_program(&mutation).unwrap().journal,
                baseline
            );
        }
    }

    #[test]
    fn malformed_metric_inputs_fail_closed() {
        let mut input = fixture();
        input.vectors.clear();
        assert_eq!(
            execute_public_vector_program(&input),
            Err(MetricError::EmptyVectors)
        );

        input = fixture();
        input.vectors = alloc::vec![
            PublicVectorCase { expected: 0, observed: 0, weight: 1 };
            MAXIMUM_VECTORS + 1
        ];
        assert_eq!(
            execute_public_vector_program(&input),
            Err(MetricError::TooManyVectors)
        );

        input = fixture();
        input.mode = PublicVectorMode::MaximizeExactMatches;
        input.threshold = -1;
        assert_eq!(
            execute_public_vector_program(&input),
            Err(MetricError::NegativeThreshold)
        );
    }
}
