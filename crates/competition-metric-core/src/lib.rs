#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::vec::Vec;
use alloc::string::String;
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
pub const MAXIMUM_ARTIFACT_BYTES: usize = 1024 * 1024;
pub const MAXIMUM_ARTIFACT_REQUIREMENTS: usize = 256;
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
pub const STRUCTURED_ARTIFACT_METRIC_PROGRAM_HASH: [u8; 32] =
    hex!("760b8c342a91b4c215b8f102c85b696e70073a98c62a87987d2930eadbeb22b9");
pub const STRUCTURED_ARTIFACT_JOURNAL_SCHEMA_HASH: [u8; 32] =
    hex!("63c02a04ca74b569649c9374b088b08d90fb1e85d2be0d1e0ca141307938fb0d");
const POLICY_DOMAIN: [u8; 32] =
    hex!("f6a226ca20aaca3b9c0b4a609939c334b6c2b03500a5df45188df8bcd7c2b369");
const SUBMISSION_DOMAIN: [u8; 32] =
    hex!("402204460b00978c26cee42ae0089d94fe8b0b17bd90c45a6cd78d466463a507");
const EVIDENCE_DOMAIN: [u8; 32] =
    hex!("16f60f26d350a38e6993a5454967d1efb0461d93785b7cdb38ba463284c5ab15");
const ARTIFACT_POLICY_DOMAIN: [u8; 32] =
    hex!("937c65ae27a5abe033da9ad95a1f023de32461fb6fba24e9090d5d879cdad770");
const ARTIFACT_SUBMISSION_DOMAIN: [u8; 32] =
    hex!("6c3e2c182e83869d996ddb7c5a78d3d43a611c656ef04d03c053e39fd2315659");
const ARTIFACT_EVIDENCE_DOMAIN: [u8; 32] =
    hex!("14446eea06e4c81f41f3fa83d219f5ae35ebbf777f45e84adf08ebbf3dbf2b48");

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArtifactRequirement {
    Utf8Contains {
        needle: String,
        minimum_occurrences: u32,
        weight: u32,
    },
    Utf8Excludes { needle: String, weight: u32 },
    MaximumBytes { maximum: u32, weight: u32 },
    JsonValid { weight: u32 },
    JsonPointerExists { pointer: String, weight: u32 },
    JsonPointerStringEquals {
        pointer: String,
        expected: String,
        weight: u32,
    },
    JsonArrayMinimumLength {
        pointer: String,
        minimum: u32,
        weight: u32,
    },
}

impl ArtifactRequirement {
    fn weight(&self) -> u32 {
        match self {
            Self::Utf8Contains { weight, .. }
            | Self::Utf8Excludes { weight, .. }
            | Self::MaximumBytes { weight, .. }
            | Self::JsonValid { weight }
            | Self::JsonPointerExists { weight, .. }
            | Self::JsonPointerStringEquals { weight, .. }
            | Self::JsonArrayMinimumLength { weight, .. } => *weight,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredArtifactProgramInput {
    pub scope: JournalScopeV2,
    pub threshold: u128,
    pub artifact: Vec<u8>,
    pub requirements: Vec<ArtifactRequirement>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StructuredArtifactProgramOutput {
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
    EmptyArtifact,
    ArtifactTooLarge,
    InvalidArtifactRequirements,
    InvalidUtf8,
    InvalidJson,
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
        JOURNAL_SCHEMA_HASH,
        METRIC_PROGRAM_HASH,
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

pub fn execute_structured_artifact_program(
    input: &StructuredArtifactProgramInput,
) -> Result<StructuredArtifactProgramOutput, MetricError> {
    validate_scope(&input.scope)?;
    if input.artifact.is_empty() {
        return Err(MetricError::EmptyArtifact);
    }
    if input.artifact.len() > MAXIMUM_ARTIFACT_BYTES {
        return Err(MetricError::ArtifactTooLarge);
    }
    if input.requirements.is_empty()
        || input.requirements.len() > MAXIMUM_ARTIFACT_REQUIREMENTS
        || input.requirements.iter().any(|requirement| requirement.weight() == 0)
    {
        return Err(MetricError::InvalidArtifactRequirements);
    }
    let total_weight = input.requirements.iter().try_fold(0_u128, |total, requirement| {
        total
            .checked_add(u128::from(requirement.weight()))
            .ok_or(MetricError::ArithmeticOverflow)
    })?;
    if input.threshold == 0 || input.threshold > total_weight {
        return Err(MetricError::InvalidArtifactRequirements);
    }
    validate_requirements(&input.requirements)?;

    let needs_text = input.requirements.iter().any(|requirement| {
        matches!(requirement, ArtifactRequirement::Utf8Contains { .. } | ArtifactRequirement::Utf8Excludes { .. })
    });
    let needs_json = input.requirements.iter().any(|requirement| {
        matches!(requirement, ArtifactRequirement::JsonValid { .. }
            | ArtifactRequirement::JsonPointerExists { .. }
            | ArtifactRequirement::JsonPointerStringEquals { .. }
            | ArtifactRequirement::JsonArrayMinimumLength { .. })
    });
    let text = if needs_text || needs_json {
        Some(core::str::from_utf8(&input.artifact).map_err(|_| MetricError::InvalidUtf8)?)
    } else {
        None
    };
    let json = if needs_json {
        Some(serde_json::from_str::<serde_json::Value>(text.expect("UTF-8 was validated"))
            .map_err(|_| MetricError::InvalidJson)?)
    } else {
        None
    };

    let mut score = 0_u128;
    for requirement in &input.requirements {
        let satisfied = artifact_requirement_satisfied(
            requirement,
            &input.artifact,
            text,
            json.as_ref(),
        );
        if satisfied {
            score = score
                .checked_add(u128::from(requirement.weight()))
                .ok_or(MetricError::ArithmeticOverflow)?;
        }
    }
    let passed = score >= input.threshold;
    let score = i128::try_from(score).map_err(|_| MetricError::ArithmeticOverflow)?;
    let verification_policy_hash = structured_artifact_policy_hash(input);
    let submission_hash = structured_artifact_submission_hash(&input.artifact);
    let evidence_hash = structured_artifact_evidence_hash(
        verification_policy_hash,
        submission_hash,
    );
    let journal = encode_journal(
        &input.scope,
        submission_hash,
        evidence_hash,
        verification_policy_hash,
        passed,
        score,
        STRUCTURED_ARTIFACT_JOURNAL_SCHEMA_HASH,
        STRUCTURED_ARTIFACT_METRIC_PROGRAM_HASH,
    );
    Ok(StructuredArtifactProgramOutput {
        passed,
        score,
        verification_policy_hash,
        submission_hash,
        evidence_hash,
        journal,
    })
}

fn validate_scope(scope: &JournalScopeV2) -> Result<(), MetricError> {
    if matches!(scope.proof_system, GROTH16_PROOF_SYSTEM | PLONK_PROOF_SYSTEM) {
        Ok(())
    } else {
        Err(MetricError::UnsupportedProofSystem)
    }
}

fn validate_requirements(requirements: &[ArtifactRequirement]) -> Result<(), MetricError> {
    for requirement in requirements {
        match requirement {
            ArtifactRequirement::Utf8Contains { needle, minimum_occurrences, .. } => {
                if needle.is_empty() || needle.len() > 4096 || *minimum_occurrences == 0 {
                    return Err(MetricError::InvalidArtifactRequirements);
                }
            }
            ArtifactRequirement::Utf8Excludes { needle, .. } => {
                if needle.is_empty() || needle.len() > 4096 {
                    return Err(MetricError::InvalidArtifactRequirements);
                }
            }
            ArtifactRequirement::MaximumBytes { maximum, .. } => {
                if *maximum == 0 || usize::try_from(*maximum).ok().is_none_or(|value| value > MAXIMUM_ARTIFACT_BYTES) {
                    return Err(MetricError::InvalidArtifactRequirements);
                }
            }
            ArtifactRequirement::JsonValid { .. } => {}
            ArtifactRequirement::JsonPointerExists { pointer, .. }
            | ArtifactRequirement::JsonArrayMinimumLength { pointer, .. } => {
                if !valid_json_pointer(pointer) {
                    return Err(MetricError::InvalidArtifactRequirements);
                }
            }
            ArtifactRequirement::JsonPointerStringEquals { pointer, expected, .. } => {
                if !valid_json_pointer(pointer) || expected.len() > 4096 {
                    return Err(MetricError::InvalidArtifactRequirements);
                }
            }
        }
    }
    Ok(())
}

fn valid_json_pointer(pointer: &str) -> bool {
    pointer.len() <= 4096 && (pointer.is_empty() || pointer.starts_with('/'))
}

fn artifact_requirement_satisfied(
    requirement: &ArtifactRequirement,
    artifact: &[u8],
    text: Option<&str>,
    json: Option<&serde_json::Value>,
) -> bool {
    match requirement {
        ArtifactRequirement::Utf8Contains { needle, minimum_occurrences, .. } => text
            .is_some_and(|text| text.match_indices(needle).count() >= *minimum_occurrences as usize),
        ArtifactRequirement::Utf8Excludes { needle, .. } => {
            text.is_some_and(|text| !text.contains(needle))
        }
        ArtifactRequirement::MaximumBytes { maximum, .. } => {
            artifact.len() <= *maximum as usize
        }
        ArtifactRequirement::JsonValid { .. } => json.is_some(),
        ArtifactRequirement::JsonPointerExists { pointer, .. } => {
            json.is_some_and(|value| value.pointer(pointer).is_some())
        }
        ArtifactRequirement::JsonPointerStringEquals { pointer, expected, .. } => json
            .and_then(|value| value.pointer(pointer))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| value == expected),
        ArtifactRequirement::JsonArrayMinimumLength { pointer, minimum, .. } => json
            .and_then(|value| value.pointer(pointer))
            .and_then(serde_json::Value::as_array)
            .is_some_and(|value| value.len() >= *minimum as usize),
    }
}

pub fn structured_artifact_policy_hash(input: &StructuredArtifactProgramInput) -> [u8; 32] {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&ARTIFACT_POLICY_DOMAIN);
    bytes.extend_from_slice(&input.threshold.to_be_bytes());
    bytes.extend_from_slice(&(input.requirements.len() as u32).to_be_bytes());
    for requirement in &input.requirements {
        encode_artifact_requirement(&mut bytes, requirement);
    }
    keccak256(&bytes)
}

fn encode_artifact_requirement(bytes: &mut Vec<u8>, requirement: &ArtifactRequirement) {
    fn push_string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&(value.len() as u32).to_be_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
    match requirement {
        ArtifactRequirement::Utf8Contains { needle, minimum_occurrences, weight } => {
            bytes.push(0);
            bytes.extend_from_slice(&weight.to_be_bytes());
            bytes.extend_from_slice(&minimum_occurrences.to_be_bytes());
            push_string(bytes, needle);
        }
        ArtifactRequirement::Utf8Excludes { needle, weight } => {
            bytes.push(1);
            bytes.extend_from_slice(&weight.to_be_bytes());
            push_string(bytes, needle);
        }
        ArtifactRequirement::MaximumBytes { maximum, weight } => {
            bytes.push(2);
            bytes.extend_from_slice(&weight.to_be_bytes());
            bytes.extend_from_slice(&maximum.to_be_bytes());
        }
        ArtifactRequirement::JsonValid { weight } => {
            bytes.push(3);
            bytes.extend_from_slice(&weight.to_be_bytes());
        }
        ArtifactRequirement::JsonPointerExists { pointer, weight } => {
            bytes.push(4);
            bytes.extend_from_slice(&weight.to_be_bytes());
            push_string(bytes, pointer);
        }
        ArtifactRequirement::JsonPointerStringEquals { pointer, expected, weight } => {
            bytes.push(5);
            bytes.extend_from_slice(&weight.to_be_bytes());
            push_string(bytes, pointer);
            push_string(bytes, expected);
        }
        ArtifactRequirement::JsonArrayMinimumLength { pointer, minimum, weight } => {
            bytes.push(6);
            bytes.extend_from_slice(&weight.to_be_bytes());
            bytes.extend_from_slice(&minimum.to_be_bytes());
            push_string(bytes, pointer);
        }
    }
}

pub fn structured_artifact_submission_hash(artifact: &[u8]) -> [u8; 32] {
    let mut bytes = Vec::with_capacity(36 + artifact.len());
    bytes.extend_from_slice(&ARTIFACT_SUBMISSION_DOMAIN);
    bytes.extend_from_slice(&(artifact.len() as u32).to_be_bytes());
    bytes.extend_from_slice(artifact);
    keccak256(&bytes)
}

fn structured_artifact_evidence_hash(
    policy_hash: [u8; 32],
    submission_hash: [u8; 32],
) -> [u8; 32] {
    let mut bytes = [0_u8; 96];
    bytes[..32].copy_from_slice(&ARTIFACT_EVIDENCE_DOMAIN);
    bytes[32..64].copy_from_slice(&policy_hash);
    bytes[64..].copy_from_slice(&submission_hash);
    keccak256(&bytes)
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
    journal_schema_hash: [u8; 32],
    metric_program_hash: [u8; 32],
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
    push(journal_schema_hash);
    push(metric_program_hash);
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

    fn artifact_fixture() -> StructuredArtifactProgramInput {
        StructuredArtifactProgramInput {
            scope: fixture().scope,
            threshold: 7,
            artifact: br#"{"schema":"agent-bounties/agent-guide-v1","canonical_url":"https://agentbounties.app/tasks/","steps":["discover","inspect","solve","prove","get_paid"]}"#.to_vec(),
            requirements: alloc::vec![
                ArtifactRequirement::JsonValid { weight: 1 },
                ArtifactRequirement::JsonPointerStringEquals {
                    pointer: "/schema".into(),
                    expected: "agent-bounties/agent-guide-v1".into(),
                    weight: 2,
                },
                ArtifactRequirement::JsonPointerStringEquals {
                    pointer: "/canonical_url".into(),
                    expected: "https://agentbounties.app/tasks/".into(),
                    weight: 2,
                },
                ArtifactRequirement::JsonArrayMinimumLength {
                    pointer: "/steps".into(),
                    minimum: 5,
                    weight: 1,
                },
                ArtifactRequirement::Utf8Contains {
                    needle: "get_paid".into(),
                    minimum_occurrences: 1,
                    weight: 1,
                },
                ArtifactRequirement::Utf8Excludes {
                    needle: "localhost".into(),
                    weight: 1,
                },
                ArtifactRequirement::MaximumBytes {
                    maximum: 512,
                    weight: 1,
                },
            ],
        }
    }

    #[test]
    fn structured_artifact_derives_score_from_bound_bytes() {
        let input = artifact_fixture();
        let output = execute_structured_artifact_program(&input).unwrap();
        assert!(output.passed);
        assert_eq!(output.score, 9);
        assert_eq!(output.submission_hash, structured_artifact_submission_hash(&input.artifact));
        assert_eq!(
            &output.journal[12 * 32..13 * 32],
            &STRUCTURED_ARTIFACT_JOURNAL_SCHEMA_HASH
        );
        assert_eq!(
            &output.journal[13 * 32..14 * 32],
            &STRUCTURED_ARTIFACT_METRIC_PROGRAM_HASH
        );
    }

    #[test]
    fn structured_artifact_cannot_claim_unobserved_success() {
        let mut input = artifact_fixture();
        input.artifact = br#"{"schema":"wrong","canonical_url":"http://localhost","steps":[]}"#.to_vec();
        let output = execute_structured_artifact_program(&input).unwrap();
        assert!(!output.passed);
        assert_eq!(output.score, 2);
    }

    #[test]
    fn structured_artifact_policy_and_replay_boundaries_are_immutable() {
        let input = artifact_fixture();
        let output = execute_structured_artifact_program(&input).unwrap();

        let mut changed_policy = input.clone();
        if let ArtifactRequirement::MaximumBytes { maximum, .. } =
            &mut changed_policy.requirements[6]
        {
            *maximum += 1;
        }
        let changed_policy = execute_structured_artifact_program(&changed_policy).unwrap();
        assert_ne!(output.verification_policy_hash, changed_policy.verification_policy_hash);
        assert_ne!(output.journal, changed_policy.journal);

        let mut changed_solver = input;
        changed_solver.scope.solver[0] ^= 1;
        assert_ne!(
            output.journal,
            execute_structured_artifact_program(&changed_solver).unwrap().journal
        );
    }

    #[test]
    fn malformed_structured_artifacts_fail_closed() {
        let mut input = artifact_fixture();
        input.artifact.clear();
        assert_eq!(
            execute_structured_artifact_program(&input),
            Err(MetricError::EmptyArtifact)
        );

        input = artifact_fixture();
        input.threshold = 100;
        assert_eq!(
            execute_structured_artifact_program(&input),
            Err(MetricError::InvalidArtifactRequirements)
        );

        input = artifact_fixture();
        input.artifact = b"not json".to_vec();
        assert_eq!(
            execute_structured_artifact_program(&input),
            Err(MetricError::InvalidJson)
        );

        input = artifact_fixture();
        input.requirements[0] = ArtifactRequirement::Utf8Contains {
            needle: String::new(),
            minimum_occurrences: 1,
            weight: 1,
        };
        assert_eq!(
            execute_structured_artifact_program(&input),
            Err(MetricError::InvalidArtifactRequirements)
        );
    }
}
