use super::{
    address_from_word, base_network_descriptor, decode_words, deterministic_log_id,
    eip3009_typed_data, encode_address, encode_call, encode_uint256, event_topic, log_key,
    normalize_address, normalize_evm_address, normalize_hash, normalize_topic, parse_bytes32,
    parse_rpc_quantity, predict_minimal_proxy_address, rpc_result, selector, topic_u64, topic_word,
    word_hex, word_to_u128, word_to_u64, ChainBaseError, Eip3009AuthorizationTypedData,
    Eip712DomainData, Eip712TypeField, EvmLog, EvmTransactionIntent, JsonRpcTransport,
    ReqwestJsonRpcTransport,
};
use chrono::{DateTime, Utc};
use domain::Id;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha3::{Digest, Keccak256};
use std::collections::BTreeMap;

pub const OPEN_COMPETITION_READINESS_SCHEMA: &str =
    "agent-bounties/open-competition-v1-readiness-v1";
pub const OPEN_COMPETITION_ACTION_SCHEMA: &str = "agent-bounties/open-competition-v1-action-v1";
pub const OPEN_COMPETITION_COMMITMENT_SCHEMA: &str =
    "agent-bounties/open-competition-v1-commitment-v1";
pub const OPEN_COMPETITION_VERIFIER_CATALOG_SCHEMA: &str =
    "agent-bounties/open-competition-v1-verifier-catalog-v1";
pub const OPEN_COMPETITION_CREATION_SCHEMA: &str =
    "agent-bounties/open-competition-v1-creation-preparation-v1";
pub const OPEN_COMPETITION_STATE_SCHEMA: &str = "agent-bounties/open-competition-v1-state-v1";
pub const OPEN_COMPETITION_ENTRANT_ACTION_SCHEMA: &str =
    "agent-bounties/open-competition-entrant-wallet-action-v1";
pub const OPEN_COMPETITION_PROTOCOL_VERSION: &str = "agent-bounties/open-competition-v1";
pub const OPEN_COMPETITION_MAX_ENTRIES: u8 = 64;
pub const OPEN_COMPETITION_MAX_COMPETITION_WINDOW_SECONDS: u64 = 30 * 24 * 60 * 60;
pub const OPEN_COMPETITION_MAX_REVEAL_WINDOW_SECONDS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenCompetitionEventKind {
    CanonicalCompetitionCreated,
    CanonicalCompetitionTermsCommitted,
    CanonicalCompetitionEconomicsConfigured,
    CanonicalCompetitionVerificationConfigured,
    FundingAdded,
    CompetitionOpened,
    SolutionCommitted,
    SolutionRevealed,
    CompetitionSubmissionRejected,
    CommitmentExpired,
    BountySettled,
    EntryBondWithdrawn,
    BountyCancelled,
    RefundWithdrawn,
}

impl OpenCompetitionEventKind {
    pub fn is_payment_evidence(self) -> bool {
        matches!(self, Self::BountySettled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCompetitionEvent {
    pub id: Id,
    pub protocol_version: String,
    pub log_key: String,
    pub tx_hash: String,
    pub block_number: u64,
    pub log_index: u64,
    pub contract_address: String,
    pub bounty_id: String,
    pub kind: OpenCompetitionEventKind,
    pub data: Value,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenCompetitionEventSignature {
    CanonicalCompetitionCreated,
    CanonicalCompetitionTermsCommitted,
    CanonicalCompetitionEconomicsConfigured,
    CanonicalCompetitionVerificationConfigured,
    FundingAdded,
    CompetitionOpened,
    SolutionCommitted,
    SolutionRevealed,
    CompetitionSubmissionRejected,
    CommitmentExpired,
    BountySettled,
    EntryBondWithdrawn,
    BountyCancelled,
    RefundWithdrawn,
}

const OPEN_COMPETITION_EVENT_SIGNATURES: [(&str, OpenCompetitionEventSignature); 14] = [
    (
        "CanonicalCompetitionCreated(bytes32,address,address,bytes32,bytes32,bytes32)",
        OpenCompetitionEventSignature::CanonicalCompetitionCreated,
    ),
    (
        "CanonicalCompetitionTermsCommitted(bytes32,bytes32,bytes32,bytes32)",
        OpenCompetitionEventSignature::CanonicalCompetitionTermsCommitted,
    ),
    (
        "CanonicalCompetitionEconomicsConfigured(bytes32,uint256,uint256,uint256,uint256,uint256,uint64,uint64,uint64,uint8)",
        OpenCompetitionEventSignature::CanonicalCompetitionEconomicsConfigured,
    ),
    (
        "CanonicalCompetitionVerificationConfigured(bytes32,address,address)",
        OpenCompetitionEventSignature::CanonicalCompetitionVerificationConfigured,
    ),
    (
        "FundingAdded(bytes32,address,uint256,uint256,uint256)",
        OpenCompetitionEventSignature::FundingAdded,
    ),
    (
        "CompetitionOpened(bytes32,uint64,uint8)",
        OpenCompetitionEventSignature::CompetitionOpened,
    ),
    (
        "SolutionCommitted(bytes32,address,uint8,bytes32,uint64,uint64,uint256)",
        OpenCompetitionEventSignature::SolutionCommitted,
    ),
    (
        "SolutionRevealed(bytes32,uint64,address,bytes32,bytes32,bool,bytes32)",
        OpenCompetitionEventSignature::SolutionRevealed,
    ),
    (
        "CompetitionSubmissionRejected(bytes32,uint64,address,uint256,bytes32)",
        OpenCompetitionEventSignature::CompetitionSubmissionRejected,
    ),
    (
        "CommitmentExpired(bytes32,address,uint256,uint256)",
        OpenCompetitionEventSignature::CommitmentExpired,
    ),
    (
        "BountySettled(bytes32,uint64,address,uint256,uint256,uint256,uint256,bytes32,bytes32,bytes32,bytes32)",
        OpenCompetitionEventSignature::BountySettled,
    ),
    (
        "EntryBondWithdrawn(bytes32,address,uint256)",
        OpenCompetitionEventSignature::EntryBondWithdrawn,
    ),
    (
        "BountyCancelled(bytes32,uint256,uint256)",
        OpenCompetitionEventSignature::BountyCancelled,
    ),
    (
        "RefundWithdrawn(bytes32,address,uint256,uint256,uint256)",
        OpenCompetitionEventSignature::RefundWithdrawn,
    ),
];

pub fn open_competition_event_topics() -> Vec<String> {
    OPEN_COMPETITION_EVENT_SIGNATURES
        .iter()
        .map(|(signature, _)| event_topic(signature))
        .collect()
}

fn open_competition_event_signature(topic: &str) -> Option<OpenCompetitionEventSignature> {
    let normalized = normalize_topic(topic).ok()?;
    OPEN_COMPETITION_EVENT_SIGNATURES
        .iter()
        .find_map(|(signature, kind)| (normalized == event_topic(signature)).then_some(*kind))
}

pub fn decode_open_competition_logs(
    logs: impl IntoIterator<Item = EvmLog>,
) -> Result<Vec<OpenCompetitionEvent>, ChainBaseError> {
    let topics = open_competition_event_topics()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    logs.into_iter()
        .filter(|log| {
            log.topics
                .first()
                .and_then(|topic| normalize_topic(topic).ok())
                .is_some_and(|topic| topics.contains(&topic))
        })
        .map(decode_open_competition_log)
        .collect()
}

fn decode_open_competition_log(log: EvmLog) -> Result<OpenCompetitionEvent, ChainBaseError> {
    let topic0 = log
        .topics
        .first()
        .ok_or_else(|| ChainBaseError::InvalidLogTopics("missing topic0".to_string()))?;
    let signature = open_competition_event_signature(topic0)
        .ok_or_else(|| ChainBaseError::UnknownEventTopic(topic0.clone()))?;
    let (kind, bounty_id, data) = match signature {
        OpenCompetitionEventSignature::CanonicalCompetitionCreated => {
            require_open_competition_topics(&log, 4, "CanonicalCompetitionCreated")?;
            let words = decode_words(&log.data, 3, "CanonicalCompetitionCreated")?;
            (
                OpenCompetitionEventKind::CanonicalCompetitionCreated,
                word_hex(topic_word(&log, 1, "CanonicalCompetitionCreated")?),
                json!({
                    "bounty_contract": address_from_word(topic_word(&log, 2, "CanonicalCompetitionCreated")?),
                    "creator": address_from_word(topic_word(&log, 3, "CanonicalCompetitionCreated")?),
                    "terms_hash": word_hex(words[0]), "policy_hash": word_hex(words[1]),
                    "creation_nonce": word_hex(words[2]),
                }),
            )
        }
        OpenCompetitionEventSignature::CanonicalCompetitionTermsCommitted => {
            require_open_competition_topics(&log, 2, "CanonicalCompetitionTermsCommitted")?;
            let words = decode_words(&log.data, 3, "CanonicalCompetitionTermsCommitted")?;
            (
                OpenCompetitionEventKind::CanonicalCompetitionTermsCommitted,
                word_hex(topic_word(&log, 1, "CanonicalCompetitionTermsCommitted")?),
                json!({
                    "acceptance_criteria_hash": word_hex(words[0]),
                    "benchmark_hash": word_hex(words[1]),
                    "evidence_schema_hash": word_hex(words[2]),
                }),
            )
        }
        OpenCompetitionEventSignature::CanonicalCompetitionEconomicsConfigured => {
            let name = "CanonicalCompetitionEconomicsConfigured";
            require_open_competition_topics(&log, 2, name)?;
            let words = decode_words(&log.data, 9, name)?;
            (
                OpenCompetitionEventKind::CanonicalCompetitionEconomicsConfigured,
                word_hex(topic_word(&log, 1, name)?),
                json!({
                    "solver_reward": word_to_u128(words[0])?,
                    "verifier_reward": word_to_u128(words[1])?,
                    "entry_bond": word_to_u128(words[2])?,
                    "target_amount": word_to_u128(words[3])?,
                    "initial_funding": word_to_u128(words[4])?,
                    "funding_deadline": word_to_u64(words[5], name)?,
                    "competition_window_seconds": word_to_u64(words[6], name)?,
                    "reveal_window_seconds": word_to_u64(words[7], name)?,
                    "max_entries": word_to_u128(words[8])?,
                }),
            )
        }
        OpenCompetitionEventSignature::CanonicalCompetitionVerificationConfigured => {
            let name = "CanonicalCompetitionVerificationConfigured";
            require_open_competition_topics(&log, 2, name)?;
            let words = decode_words(&log.data, 2, name)?;
            (
                OpenCompetitionEventKind::CanonicalCompetitionVerificationConfigured,
                word_hex(topic_word(&log, 1, name)?),
                json!({
                    "verifier_module": address_from_word(words[0]),
                    "verifier_reward_recipient": address_from_word(words[1]),
                }),
            )
        }
        OpenCompetitionEventSignature::FundingAdded => {
            require_open_competition_topics(&log, 3, "FundingAdded")?;
            let words = decode_words(&log.data, 3, "FundingAdded")?;
            (
                OpenCompetitionEventKind::FundingAdded,
                word_hex(topic_word(&log, 1, "FundingAdded")?),
                json!({
                    "contributor": address_from_word(topic_word(&log, 2, "FundingAdded")?),
                    "amount": word_to_u128(words[0])?, "funded_amount": word_to_u128(words[1])?,
                    "target_amount": word_to_u128(words[2])?,
                }),
            )
        }
        OpenCompetitionEventSignature::CompetitionOpened => {
            require_open_competition_topics(&log, 2, "CompetitionOpened")?;
            let words = decode_words(&log.data, 2, "CompetitionOpened")?;
            (
                OpenCompetitionEventKind::CompetitionOpened,
                word_hex(topic_word(&log, 1, "CompetitionOpened")?),
                json!({
                    "competition_ends_at": word_to_u64(words[0], "CompetitionOpened")?,
                    "max_entries": word_to_u128(words[1])?,
                }),
            )
        }
        OpenCompetitionEventSignature::SolutionCommitted => {
            require_open_competition_topics(&log, 4, "SolutionCommitted")?;
            let words = decode_words(&log.data, 4, "SolutionCommitted")?;
            (
                OpenCompetitionEventKind::SolutionCommitted,
                word_hex(topic_word(&log, 1, "SolutionCommitted")?),
                json!({
                    "solver": address_from_word(topic_word(&log, 2, "SolutionCommitted")?),
                    "entry_number": word_to_u128(topic_word(&log, 3, "SolutionCommitted")?)?,
                    "commitment": word_hex(words[0]),
                    "committed_block": word_to_u64(words[1], "SolutionCommitted")?,
                    "reveal_deadline": word_to_u64(words[2], "SolutionCommitted")?,
                    "bond": word_to_u128(words[3])?,
                }),
            )
        }
        OpenCompetitionEventSignature::SolutionRevealed => {
            require_open_competition_topics(&log, 4, "SolutionRevealed")?;
            let words = decode_words(&log.data, 4, "SolutionRevealed")?;
            let passed = word_to_u128(words[2])?;
            if passed > 1 {
                return Err(ChainBaseError::InvalidLogData(
                    "SolutionRevealed passed is not an ABI bool".to_string(),
                ));
            }
            (
                OpenCompetitionEventKind::SolutionRevealed,
                word_hex(topic_word(&log, 1, "SolutionRevealed")?),
                json!({
                    "submission_sequence": topic_u64(&log, 2, "SolutionRevealed")?,
                    "solver": address_from_word(topic_word(&log, 3, "SolutionRevealed")?),
                    "submission_hash": word_hex(words[0]), "evidence_hash": word_hex(words[1]),
                    "passed": passed == 1, "verification_hash": word_hex(words[3]),
                }),
            )
        }
        OpenCompetitionEventSignature::CompetitionSubmissionRejected => {
            let name = "CompetitionSubmissionRejected";
            require_open_competition_topics(&log, 4, name)?;
            let words = decode_words(&log.data, 2, name)?;
            (
                OpenCompetitionEventKind::CompetitionSubmissionRejected,
                word_hex(topic_word(&log, 1, name)?),
                json!({
                    "submission_sequence": topic_u64(&log, 2, name)?,
                    "solver": address_from_word(topic_word(&log, 3, name)?),
                    "bond_paid_to_verifier": word_to_u128(words[0])?,
                    "verification_hash": word_hex(words[1]),
                }),
            )
        }
        OpenCompetitionEventSignature::CommitmentExpired => {
            require_open_competition_topics(&log, 3, "CommitmentExpired")?;
            let words = decode_words(&log.data, 2, "CommitmentExpired")?;
            (
                OpenCompetitionEventKind::CommitmentExpired,
                word_hex(topic_word(&log, 1, "CommitmentExpired")?),
                json!({
                    "solver": address_from_word(topic_word(&log, 2, "CommitmentExpired")?),
                    "bond_forfeited": word_to_u128(words[0])?,
                    "timeout_bond_pool": word_to_u128(words[1])?,
                }),
            )
        }
        OpenCompetitionEventSignature::BountySettled => {
            require_open_competition_topics(&log, 4, "BountySettled")?;
            let words = decode_words(&log.data, 8, "BountySettled")?;
            (
                OpenCompetitionEventKind::BountySettled,
                word_hex(topic_word(&log, 1, "BountySettled")?),
                json!({
                    "submission_sequence": topic_u64(&log, 2, "BountySettled")?,
                    "solver": address_from_word(topic_word(&log, 3, "BountySettled")?),
                    "solver_reward": word_to_u128(words[0])?,
                    "entry_bond_returned": word_to_u128(words[1])?,
                    "timeout_bond_bonus": word_to_u128(words[2])?,
                    "verifier_reward": word_to_u128(words[3])?,
                    "submission_hash": word_hex(words[4]), "evidence_hash": word_hex(words[5]),
                    "policy_hash": word_hex(words[6]), "verification_hash": word_hex(words[7]),
                    "canonical_payment_evidence": true,
                }),
            )
        }
        OpenCompetitionEventSignature::EntryBondWithdrawn => {
            require_open_competition_topics(&log, 3, "EntryBondWithdrawn")?;
            let words = decode_words(&log.data, 1, "EntryBondWithdrawn")?;
            (
                OpenCompetitionEventKind::EntryBondWithdrawn,
                word_hex(topic_word(&log, 1, "EntryBondWithdrawn")?),
                json!({
                    "solver": address_from_word(topic_word(&log, 2, "EntryBondWithdrawn")?),
                    "amount": word_to_u128(words[0])?,
                }),
            )
        }
        OpenCompetitionEventSignature::BountyCancelled => {
            require_open_competition_topics(&log, 2, "BountyCancelled")?;
            let words = decode_words(&log.data, 2, "BountyCancelled")?;
            (
                OpenCompetitionEventKind::BountyCancelled,
                word_hex(topic_word(&log, 1, "BountyCancelled")?),
                json!({
                    "principal": word_to_u128(words[0])?,
                    "expired_entry_bonus": word_to_u128(words[1])?,
                }),
            )
        }
        OpenCompetitionEventSignature::RefundWithdrawn => {
            require_open_competition_topics(&log, 3, "RefundWithdrawn")?;
            let words = decode_words(&log.data, 3, "RefundWithdrawn")?;
            (
                OpenCompetitionEventKind::RefundWithdrawn,
                word_hex(topic_word(&log, 1, "RefundWithdrawn")?),
                json!({
                    "contributor": address_from_word(topic_word(&log, 2, "RefundWithdrawn")?),
                    "principal": word_to_u128(words[0])?,
                    "expired_entry_bonus": word_to_u128(words[1])?, "amount": word_to_u128(words[2])?,
                }),
            )
        }
    };
    Ok(OpenCompetitionEvent {
        id: deterministic_log_id(&log),
        protocol_version: OPEN_COMPETITION_PROTOCOL_VERSION.to_string(),
        log_key: log_key(&log),
        tx_hash: log.tx_hash,
        block_number: log.block_number,
        log_index: log.log_index,
        contract_address: normalize_address(log.address)?,
        bounty_id,
        kind,
        data,
        occurred_at: log.occurred_at.unwrap_or_else(Utc::now),
    })
}

fn require_open_competition_topics(
    log: &EvmLog,
    expected: usize,
    event_name: &str,
) -> Result<(), ChainBaseError> {
    if log.topics.len() != expected {
        return Err(ChainBaseError::InvalidLogTopics(event_name.to_string()));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenCompetitionDeploymentState {
    #[default]
    SourceOnlyNotReadyToEarn,
    SepoliaRehearsedNotReadyToEarn,
    MainnetCanaryNotReadyToEarn,
    ActiveReadyToEarn,
}

impl OpenCompetitionDeploymentState {
    pub fn permits_public_inventory(self) -> bool {
        matches!(self, Self::ActiveReadyToEarn)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCompetitionVerifierProfile {
    pub profile_id: String,
    pub protocol_version: String,
    pub network: String,
    pub chain_id: u64,
    pub display_name: String,
    pub module_kind: String,
    pub verifier_address: String,
    pub runtime_code_hash: String,
    pub configuration: Value,
    pub benchmark_hash: String,
    pub evidence_schema_hash: String,
    pub evidence_schema: String,
    pub immutable_runtime_required: bool,
    pub approved_for_rehearsal: bool,
    pub public_inventory_eligible: bool,
    pub deployment_state: OpenCompetitionDeploymentState,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionVerifierCatalog {
    pub schema_version: String,
    pub protocol_version: String,
    pub network: String,
    pub profiles: Vec<OpenCompetitionVerifierProfile>,
    pub evidence_boundary: String,
}

pub fn built_in_open_competition_verifier_catalog(
    network: &str,
) -> Result<OpenCompetitionVerifierCatalog, ChainBaseError> {
    let descriptor = base_network_descriptor(network)?;
    let profiles = match network {
        "base-mainnet" => vec![OpenCompetitionVerifierProfile {
            profile_id: "leading-zero-work-v1-difficulty-16-mainnet-canary".to_string(),
            protocol_version: OPEN_COMPETITION_PROTOCOL_VERSION.to_string(),
            network: network.to_string(),
            chain_id: descriptor.chain_id,
            display_name: "Leading-zero work (16-bit protocol canary)".to_string(),
            module_kind: "leading-zero-work-v1".to_string(),
            verifier_address: "0xcc6059ceeda5bc4ba8a97ecfbffa7488c8fd579e".to_string(),
            runtime_code_hash:
                "0xbaa3a8305c4b65d0dc20131d0ef207fdaf4763f345393a831370cd04077df9b3"
                    .to_string(),
            configuration: json!({ "difficulty_bits": 16 }),
            benchmark_hash:
                "0x8f5dc601eaff77e6102aab44f16a9b176df7ce0a998078782fb5d4b9e0c0ebf2"
                    .to_string(),
            evidence_schema_hash:
                "0xea961c63fb67f86823003426b04a928406e44e9c8acc3dcb298189e9558083da"
                    .to_string(),
            evidence_schema: "agent-bounties/leading-zero-work-evidence-v1".to_string(),
            immutable_runtime_required: true,
            approved_for_rehearsal: true,
            public_inventory_eligible: false,
            deployment_state: OpenCompetitionDeploymentState::MainnetCanaryNotReadyToEarn,
            evidence_boundary: "This profile approves one exact immutable runtime and configuration for a protocol canary. It does not approve factory-origin modules generally or claim that leading-zero work represents ordinary digital work.".to_string(),
        }],
        "base-sepolia" => Vec::new(),
        _ => unreachable!("base_network_descriptor rejects unknown networks"),
    };
    Ok(OpenCompetitionVerifierCatalog {
        schema_version: OPEN_COMPETITION_VERIFIER_CATALOG_SCHEMA.to_string(),
        protocol_version: OPEN_COMPETITION_PROTOCOL_VERSION.to_string(),
        network: network.to_string(),
        profiles,
        evidence_boundary: "Hosted approval requires an exact network, address, runtime hash, immutable configuration, benchmark, and evidence schema match. Factory provenance alone is not verifier approval.".to_string(),
    })
}

pub fn validate_open_competition_verifier_profile(
    profile: &OpenCompetitionVerifierProfile,
    network: &str,
    verifier_address: &str,
    benchmark_hash: &str,
    evidence_schema_hash: &str,
) -> Result<OpenCompetitionVerifierProfile, ChainBaseError> {
    let descriptor = base_network_descriptor(network)?;
    let normalized_address = normalize_evm_address(verifier_address)?;
    let profile_address = normalize_evm_address(&profile.verifier_address)?;
    let profile_runtime =
        normalized_nonzero_bytes32(&profile.runtime_code_hash, "runtime code hash")?;
    let profile_benchmark = normalized_nonzero_bytes32(&profile.benchmark_hash, "benchmark hash")?;
    let profile_evidence =
        normalized_nonzero_bytes32(&profile.evidence_schema_hash, "evidence schema hash")?;
    let requested_benchmark = normalized_nonzero_bytes32(benchmark_hash, "benchmark hash")?;
    let requested_evidence =
        normalized_nonzero_bytes32(evidence_schema_hash, "evidence schema hash")?;
    if profile.protocol_version != OPEN_COMPETITION_PROTOCOL_VERSION
        || profile.network != network
        || profile.chain_id != descriptor.chain_id
        || !profile.immutable_runtime_required
        || !profile.approved_for_rehearsal
        || normalized_address != profile_address
        || requested_benchmark != profile_benchmark
        || requested_evidence != profile_evidence
        || profile_runtime == zero_hash()
    {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "open-competition verifier does not exactly match the approved catalog profile"
                .to_string(),
        ));
    }
    let mut normalized = profile.clone();
    normalized.verifier_address = profile_address;
    normalized.runtime_code_hash = profile_runtime;
    normalized.benchmark_hash = profile_benchmark;
    normalized.evidence_schema_hash = profile_evidence;
    normalized.public_inventory_eligible =
        profile.public_inventory_eligible && profile.deployment_state.permits_public_inventory();
    Ok(normalized)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCompetitionCommitmentInput {
    pub network: String,
    pub bounty: String,
    pub solver: String,
    pub submission_hash: String,
    pub evidence_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCompetitionCommitmentEnvelope {
    pub schema_version: String,
    pub network: String,
    pub chain_id: u64,
    pub bounty: String,
    pub solver: String,
    pub submission_hash: String,
    pub evidence_hash: String,
    pub salt: String,
    pub commitment: String,
    pub committed_block: Option<u64>,
    pub reveal_deadline: Option<u64>,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenCompetitionEntrantAction {
    Commit,
    Reveal,
    WithdrawBond,
}

impl OpenCompetitionEntrantAction {
    pub fn code(self) -> u8 {
        match self {
            Self::Commit => 0,
            Self::Reveal => 1,
            Self::WithdrawBond => 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenCompetitionEntrantActionMessage {
    pub wallet: String,
    pub action: u8,
    pub payload_hash: String,
    pub nonce: String,
    pub deadline: String,
    pub policy_version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenCompetitionEntrantActionTypedData {
    pub types: BTreeMap<String, Vec<Eip712TypeField>>,
    pub domain: Eip712DomainData,
    pub primary_type: String,
    pub message: OpenCompetitionEntrantActionMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCompetitionEntrantActionPlan {
    pub schema_version: String,
    pub network: String,
    pub chain_id: u64,
    pub wallet: String,
    pub delegate: String,
    pub policy_hash: String,
    pub policy_version: u64,
    pub action: OpenCompetitionEntrantAction,
    pub action_code: u8,
    pub nonce: u64,
    pub deadline: u64,
    pub payload: String,
    pub payload_hash: String,
    pub signing_payload: OpenCompetitionEntrantActionTypedData,
    pub relay_call: Value,
    pub evidence_boundary: String,
}

pub fn generate_open_competition_commitment_envelope(
    input: OpenCompetitionCommitmentInput,
) -> Result<OpenCompetitionCommitmentEnvelope, ChainBaseError> {
    let first = uuid::Uuid::new_v4();
    let second = uuid::Uuid::new_v4();
    let mut salt = [0u8; 32];
    salt[..16].copy_from_slice(first.as_bytes());
    salt[16..].copy_from_slice(second.as_bytes());
    build_open_competition_commitment_envelope(input, salt)
}

pub fn build_open_competition_commitment_envelope(
    input: OpenCompetitionCommitmentInput,
    salt: [u8; 32],
) -> Result<OpenCompetitionCommitmentEnvelope, ChainBaseError> {
    if salt.iter().all(|byte| *byte == 0) {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "open-competition commitment salt must be nonzero".to_string(),
        ));
    }
    let descriptor = base_network_descriptor(&input.network)?;
    let bounty = normalize_evm_address(input.bounty)?;
    let solver = normalize_evm_address(input.solver)?;
    let submission_hash = normalized_nonzero_bytes32(&input.submission_hash, "submission hash")?;
    let evidence_hash = normalized_nonzero_bytes32(&input.evidence_hash, "evidence hash")?;
    let salt = format!("0x{}", hex::encode(salt));
    let commitment = open_competition_solution_commitment(
        descriptor.chain_id,
        &bounty,
        &solver,
        &submission_hash,
        &evidence_hash,
        &salt,
    )?;
    Ok(OpenCompetitionCommitmentEnvelope {
        schema_version: OPEN_COMPETITION_COMMITMENT_SCHEMA.to_string(),
        network: input.network,
        chain_id: descriptor.chain_id,
        bounty,
        solver,
        submission_hash,
        evidence_hash,
        salt,
        commitment,
        committed_block: None,
        reveal_deadline: None,
        evidence_boundary: "This recovery envelope contains the secret salt. Store it locally and never send it during commitment preparation. It is not an entry, reveal, verdict, settlement, or payment receipt.".to_string(),
    })
}

pub fn validate_open_competition_commitment_envelope(
    envelope: &OpenCompetitionCommitmentEnvelope,
    expected_network: &str,
    expected_bounty: &str,
    expected_solver: &str,
) -> Result<OpenCompetitionCommitmentEnvelope, ChainBaseError> {
    let descriptor = base_network_descriptor(expected_network)?;
    let expected_bounty = normalize_evm_address(expected_bounty)?;
    let expected_solver = normalize_evm_address(expected_solver)?;
    let bounty = normalize_evm_address(&envelope.bounty)?;
    let solver = normalize_evm_address(&envelope.solver)?;
    if envelope.schema_version != OPEN_COMPETITION_COMMITMENT_SCHEMA
        || envelope.network != expected_network
        || envelope.chain_id != descriptor.chain_id
        || bounty != expected_bounty
        || solver != expected_solver
    {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "commitment envelope network, chain, bounty, or solver binding is invalid".to_string(),
        ));
    }
    let reconstructed = open_competition_solution_commitment(
        envelope.chain_id,
        &bounty,
        &solver,
        &envelope.submission_hash,
        &envelope.evidence_hash,
        &envelope.salt,
    )?;
    let commitment = normalized_nonzero_bytes32(&envelope.commitment, "commitment")?;
    if reconstructed != commitment {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "commitment envelope does not reconstruct the committed value".to_string(),
        ));
    }
    let mut normalized = envelope.clone();
    normalized.bounty = bounty;
    normalized.solver = solver;
    normalized.submission_hash =
        normalized_nonzero_bytes32(&envelope.submission_hash, "submission hash")?;
    normalized.evidence_hash =
        normalized_nonzero_bytes32(&envelope.evidence_hash, "evidence hash")?;
    normalized.salt = normalized_nonzero_bytes32(&envelope.salt, "salt")?;
    normalized.commitment = commitment;
    Ok(normalized)
}

pub fn open_competition_solution_commitment(
    chain_id: u64,
    bounty: &str,
    solver: &str,
    submission_hash: &str,
    evidence_hash: &str,
    salt: &str,
) -> Result<String, ChainBaseError> {
    let domain: [u8; 32] = Keccak256::digest(b"agent-bounties/open-competition-v1-solution").into();
    let words = [
        domain,
        encode_uint256(chain_id.into())?,
        encode_address(bounty)?,
        encode_address(solver)?,
        parse_bytes32(submission_hash)?,
        parse_bytes32(evidence_hash)?,
        parse_bytes32(salt)?,
    ];
    Ok(format!("0x{}", hex::encode(keccak_words(&words))))
}

pub fn encode_open_competition_entrant_commit_payload(
    bounty_contract: &str,
    commitment: &str,
) -> Result<String, ChainBaseError> {
    let words = [
        encode_address(&normalize_evm_address(bounty_contract)?)?,
        parse_bytes32(commitment)?,
    ];
    Ok(format!("0x{}", hex::encode(words.concat())))
}

pub fn encode_open_competition_entrant_reveal_payload(
    bounty_contract: &str,
    submission_hash: &str,
    evidence_hash: &str,
    salt: &str,
    proof: &str,
) -> Result<String, ChainBaseError> {
    let proof = decode_prefixed_hex(proof, "proof")?;
    let mut bytes = Vec::with_capacity(32 * 6 + proof.len().next_multiple_of(32));
    bytes.extend_from_slice(&encode_address(&normalize_evm_address(bounty_contract)?)?);
    bytes.extend_from_slice(&parse_bytes32(submission_hash)?);
    bytes.extend_from_slice(&parse_bytes32(evidence_hash)?);
    bytes.extend_from_slice(&parse_bytes32(salt)?);
    bytes.extend_from_slice(&encode_uint256(160)?);
    bytes.extend_from_slice(&encode_uint256(proof.len() as u128)?);
    bytes.extend_from_slice(&proof);
    let padding = (32 - proof.len() % 32) % 32;
    bytes.resize(bytes.len() + padding, 0);
    Ok(format!("0x{}", hex::encode(bytes)))
}

pub fn encode_open_competition_entrant_withdraw_payload(
    bounty_contract: &str,
) -> Result<String, ChainBaseError> {
    Ok(format!(
        "0x{}",
        hex::encode(encode_address(&normalize_evm_address(bounty_contract)?)?)
    ))
}

#[allow(clippy::too_many_arguments)]
pub fn plan_open_competition_entrant_action(
    network: &str,
    wallet: &str,
    delegate: &str,
    policy_hash: &str,
    policy_version: u64,
    action: OpenCompetitionEntrantAction,
    nonce: u64,
    deadline: u64,
    payload: &str,
) -> Result<OpenCompetitionEntrantActionPlan, ChainBaseError> {
    let descriptor = base_network_descriptor(network)?;
    let wallet = normalize_evm_address(wallet)?;
    let delegate = normalize_evm_address(delegate)?;
    let policy_hash = normalized_nonzero_bytes32(policy_hash, "entrant wallet policy hash")?;
    if deadline == 0 {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "entrant wallet action deadline must be positive".to_string(),
        ));
    }
    let payload_bytes = decode_prefixed_hex(payload, "entrant wallet payload")?;
    if payload_bytes.is_empty() {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "entrant wallet payload must not be empty".to_string(),
        ));
    }
    let payload = format!("0x{}", hex::encode(&payload_bytes));
    let payload_hash = format!("0x{}", hex::encode(Keccak256::digest(&payload_bytes)));
    let mut types = BTreeMap::new();
    types.insert(
        "EIP712Domain".to_string(),
        vec![
            eip712_type_field("name", "string"),
            eip712_type_field("version", "string"),
            eip712_type_field("chainId", "uint256"),
            eip712_type_field("verifyingContract", "address"),
        ],
    );
    types.insert(
        "OpenCompetitionEntrantAction".to_string(),
        vec![
            eip712_type_field("wallet", "address"),
            eip712_type_field("action", "uint8"),
            eip712_type_field("payloadHash", "bytes32"),
            eip712_type_field("nonce", "uint256"),
            eip712_type_field("deadline", "uint256"),
            eip712_type_field("policyVersion", "uint64"),
        ],
    );
    let action_code = action.code();
    Ok(OpenCompetitionEntrantActionPlan {
        schema_version: OPEN_COMPETITION_ENTRANT_ACTION_SCHEMA.to_string(),
        network: network.to_string(),
        chain_id: descriptor.chain_id,
        wallet: wallet.clone(),
        delegate,
        policy_hash,
        policy_version,
        action,
        action_code,
        nonce,
        deadline,
        payload: payload.clone(),
        payload_hash: payload_hash.clone(),
        signing_payload: OpenCompetitionEntrantActionTypedData {
            types,
            domain: Eip712DomainData {
                name: "Agent Bounties Open Competition Entrant Wallet".to_string(),
                version: "1".to_string(),
                chain_id: descriptor.chain_id,
                verifying_contract: wallet.clone(),
            },
            primary_type: "OpenCompetitionEntrantAction".to_string(),
            message: OpenCompetitionEntrantActionMessage {
                wallet: wallet.clone(),
                action: action_code,
                payload_hash,
                nonce: nonce.to_string(),
                deadline: deadline.to_string(),
                policy_version,
            },
        },
        relay_call: json!({
            "to": wallet,
            "function": "executeWithSignature(uint8,bytes,uint256,uint256,bytes)",
            "arguments_before_signature": [action_code, payload, nonce, deadline],
            "signature_tail": ["delegate_signature"]
        }),
        evidence_boundary: "This plan authorizes one exact policy-bound entrant-wallet action. It moves no value until a keeper simulates and broadcasts it. Only canonical competition events prove entry, reveal, bond recovery, or settlement; only BountySettled proves payout.".to_string(),
    })
}

pub fn attach_open_competition_entrant_relay_signature(
    plan: &OpenCompetitionEntrantActionPlan,
    relayer: &str,
    signature: &str,
) -> Result<EvmTransactionIntent, ChainBaseError> {
    let relayer = normalize_evm_address(relayer)?;
    let signature_bytes = decode_prefixed_hex(signature, "delegate signature")?;
    if signature_bytes.len() != 65 {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "entrant wallet delegate signature must contain 65 bytes".to_string(),
        ));
    }
    let payload = decode_prefixed_hex(&plan.payload, "entrant wallet payload")?;
    let data = encode_open_competition_entrant_execute_call(
        plan.action_code,
        &payload,
        plan.nonce,
        plan.deadline,
        &signature_bytes,
    )?;
    Ok(EvmTransactionIntent {
        from: Some(relayer),
        to: plan.wallet.clone(),
        value_wei: 0,
        data,
        function: "executeWithSignature(uint8,bytes,uint256,uint256,bytes)".to_string(),
    })
}

fn eip712_type_field(name: &str, field_type: &str) -> Eip712TypeField {
    Eip712TypeField {
        name: name.to_string(),
        field_type: field_type.to_string(),
    }
}

fn encode_open_competition_entrant_execute_call(
    action: u8,
    payload: &[u8],
    nonce: u64,
    deadline: u64,
    signature: &[u8],
) -> Result<String, ChainBaseError> {
    let payload_tail_length = 32 + payload.len().next_multiple_of(32);
    let mut bytes = selector("executeWithSignature(uint8,bytes,uint256,uint256,bytes)").to_vec();
    bytes.extend_from_slice(&encode_uint256(action.into())?);
    bytes.extend_from_slice(&encode_uint256(160)?);
    bytes.extend_from_slice(&encode_uint256(nonce.into())?);
    bytes.extend_from_slice(&encode_uint256(deadline.into())?);
    bytes.extend_from_slice(&encode_uint256((160 + payload_tail_length) as u128)?);
    append_dynamic_bytes(&mut bytes, payload)?;
    append_dynamic_bytes(&mut bytes, signature)?;
    Ok(format!("0x{}", hex::encode(bytes)))
}

fn append_dynamic_bytes(output: &mut Vec<u8>, value: &[u8]) -> Result<(), ChainBaseError> {
    output.extend_from_slice(&encode_uint256(value.len() as u128)?);
    output.extend_from_slice(value);
    let padding = (32 - value.len() % 32) % 32;
    output.resize(output.len() + padding, 0);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCompetitionCreateParams {
    pub solver_reward: u128,
    pub verifier_reward: u128,
    pub terms_hash: String,
    pub policy_hash: String,
    pub acceptance_criteria_hash: String,
    pub benchmark_hash: String,
    pub evidence_schema_hash: String,
    pub funding_deadline: u64,
    pub competition_window_seconds: u64,
    pub reveal_window_seconds: u64,
    pub max_entries: u8,
    pub verifier_module: String,
    pub verifier_reward_recipient: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCompetitionAuthorizationSignature {
    pub v: u8,
    pub r: String,
    pub s: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCompetitionFundingAuthorization {
    pub valid_after: u64,
    pub valid_before: u64,
    pub nonce: String,
    pub signature: Option<OpenCompetitionAuthorizationSignature>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCompetitionCreationRequest {
    pub network: String,
    pub factory_contract: String,
    pub implementation_contract: String,
    pub creator: String,
    pub creation_nonce: String,
    pub initial_funding: u128,
    pub verifier_profile: OpenCompetitionVerifierProfile,
    pub params: OpenCompetitionCreateParams,
    pub funding_authorization: Option<OpenCompetitionFundingAuthorization>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCompetitionCreationPlan {
    pub schema_version: String,
    pub protocol_version: String,
    pub network: String,
    pub chain_id: u64,
    pub funding_mode: String,
    pub factory_contract: String,
    pub implementation_contract: String,
    pub creator: String,
    pub bounty_id: String,
    pub predicted_bounty_contract: String,
    pub verifier_profile_id: String,
    pub approve: Option<EvmTransactionIntent>,
    pub create_competition: Option<EvmTransactionIntent>,
    pub wallet_calls: Vec<EvmTransactionIntent>,
    pub eip3009_authorization: Option<Eip3009AuthorizationTypedData>,
    pub ready_to_broadcast: bool,
    pub public_inventory_eligible: bool,
    pub next_action: String,
    pub evidence_boundary: String,
}

pub fn plan_open_competition_creation(
    request: OpenCompetitionCreationRequest,
) -> Result<OpenCompetitionCreationPlan, ChainBaseError> {
    let authorized = request.funding_authorization.is_some();
    let descriptor = base_network_descriptor(&request.network)?;
    let factory = normalize_evm_address(&request.factory_contract)?;
    let implementation = normalize_evm_address(&request.implementation_contract)?;
    let creator = normalize_evm_address(&request.creator)?;
    let creation_nonce = normalized_nonzero_bytes32(&request.creation_nonce, "creation nonce")?;
    let params = normalized_creation_params(&request.params)?;
    let profile = validate_open_competition_verifier_profile(
        &request.verifier_profile,
        &request.network,
        &params.verifier_module,
        &params.benchmark_hash,
        &params.evidence_schema_hash,
    )?;
    if request.initial_funding > params.solver_reward + params.verifier_reward {
        return Err(ChainBaseError::InitialFundingExceedsTarget);
    }
    let parameter_words = open_competition_parameter_words(&params)?;
    let mut bounty_words = Vec::with_capacity(17);
    bounty_words.push(encode_uint256(descriptor.chain_id.into())?);
    bounty_words.push(encode_address(&factory)?);
    bounty_words.push(encode_address(&creator)?);
    bounty_words.push(parse_bytes32(&creation_nonce)?);
    bounty_words.extend_from_slice(&parameter_words);
    let bounty_id = format!("0x{}", hex::encode(keccak_words(&bounty_words)));
    let predicted_bounty_contract =
        predict_minimal_proxy_address(&factory, &implementation, parse_bytes32(&bounty_id)?)?;

    let mut direct_words = parameter_words.clone();
    direct_words.push(encode_uint256(request.initial_funding)?);
    direct_words.push(parse_bytes32(&creation_nonce)?);
    let direct_data = encode_static_call(
        "createCompetition((uint256,uint256,bytes32,bytes32,bytes32,bytes32,bytes32,uint64,uint64,uint64,uint8,address,address),uint256,bytes32)",
        &direct_words,
    );
    let approve = (request.initial_funding > 0).then(|| EvmTransactionIntent {
        from: Some(creator.clone()),
        to: descriptor.native_usdc_token_address.clone(),
        value_wei: 0,
        data: encode_static_call(
            "approve(address,uint256)",
            &[
                encode_address(&factory).expect("normalized factory encodes"),
                encode_uint256(request.initial_funding).expect("u128 encodes"),
            ],
        ),
        function: "approve(address,uint256)".to_string(),
    });
    let direct_create = EvmTransactionIntent {
        from: Some(creator.clone()),
        to: factory.clone(),
        value_wei: 0,
        data: direct_data,
        function: "createCompetition((...),uint256,bytes32)".to_string(),
    };

    let (funding_mode, create_competition, eip3009_authorization, ready_to_broadcast, next_action) =
        if let Some(authorization) = request.funding_authorization {
            if request.initial_funding == 0
                || authorization.valid_before <= authorization.valid_after
            {
                return Err(ChainBaseError::InvalidVerificationConfiguration(
                    "authorized creation requires positive funding and a valid authorization window"
                        .to_string(),
                ));
            }
            let nonce = normalized_nonzero_bytes32(&authorization.nonce, "authorization nonce")?;
            let typed_data = eip3009_typed_data(
                &descriptor,
                &creator,
                &predicted_bounty_contract,
                request.initial_funding,
                authorization.valid_after,
                authorization.valid_before,
                &nonce,
            );
            let transaction = if let Some(signature) = authorization.signature {
                let v = normalize_signature_v(signature.v)?;
                let mut words = Vec::with_capacity(22);
                words.push(encode_address(&creator)?);
                words.extend_from_slice(&parameter_words);
                words.push(encode_uint256(request.initial_funding)?);
                words.push(parse_bytes32(&creation_nonce)?);
                words.push(encode_uint256(authorization.valid_after.into())?);
                words.push(encode_uint256(authorization.valid_before.into())?);
                words.push(parse_bytes32(&nonce)?);
                words.push(encode_uint256(v.into())?);
                words.push(parse_bytes32(&signature.r)?);
                words.push(parse_bytes32(&signature.s)?);
                Some(EvmTransactionIntent {
                    from: None,
                    to: factory.clone(),
                    value_wei: 0,
                    data: encode_static_call(
                        "createCompetitionWithAuthorization(address,(uint256,uint256,bytes32,bytes32,bytes32,bytes32,bytes32,uint64,uint64,uint64,uint8,address,address),uint256,bytes32,(uint256,uint256,bytes32,uint8,bytes32,bytes32))",
                        &words,
                    ),
                    function: "createCompetitionWithAuthorization(address,(...),uint256,bytes32,(...))".to_string(),
                })
            } else {
                None
            };
            let ready = transaction.is_some();
            (
                "eip3009_authorized".to_string(),
                transaction,
                Some(typed_data),
                ready,
                if ready {
                    "Re-check exact factory, implementation, verifier runtime, safe-block state, wallet policy, and signature before relaying the authorized creation.".to_string()
                } else {
                    "Sign the exact EIP-3009 typed data with the creator wallet, attach the signature, and request a fresh authorized creation plan.".to_string()
                },
            )
        } else {
            (
                "approval_then_create".to_string(),
                Some(direct_create.clone()),
                None,
                true,
                "Re-check exact factory, implementation, verifier runtime, safe-block state, wallet policy, approval amount, and creation calldata before signing the batch.".to_string(),
            )
        };
    let mut wallet_calls = Vec::new();
    if !authorized {
        if let Some(approval) = approve.clone() {
            wallet_calls.push(approval);
        }
        wallet_calls.push(direct_create);
    } else if let Some(transaction) = create_competition.clone() {
        wallet_calls.push(transaction);
    }
    Ok(OpenCompetitionCreationPlan {
        schema_version: OPEN_COMPETITION_CREATION_SCHEMA.to_string(),
        protocol_version: OPEN_COMPETITION_PROTOCOL_VERSION.to_string(),
        network: request.network,
        chain_id: descriptor.chain_id,
        funding_mode,
        factory_contract: factory,
        implementation_contract: implementation,
        creator,
        bounty_id,
        predicted_bounty_contract,
        verifier_profile_id: profile.profile_id,
        approve: if !authorized {
            approve
        } else {
            None
        },
        create_competition,
        wallet_calls,
        eip3009_authorization,
        ready_to_broadcast,
        public_inventory_eligible: profile.public_inventory_eligible,
        next_action,
        evidence_boundary: "This preparation is deterministic unsigned transaction evidence. It does not prove deployment, funding, entry, verification, settlement, or payment. Live signing must fail closed unless safe-block factory and verifier observations match the release manifest.".to_string(),
    })
}

fn normalized_creation_params(
    params: &OpenCompetitionCreateParams,
) -> Result<OpenCompetitionCreateParams, ChainBaseError> {
    let target = params
        .solver_reward
        .checked_add(params.verifier_reward)
        .ok_or(ChainBaseError::InvalidAmount)?;
    if params.solver_reward == 0
        || params.verifier_reward == 0
        || target > u64::MAX.into()
        || params.competition_window_seconds == 0
        || params.competition_window_seconds > OPEN_COMPETITION_MAX_COMPETITION_WINDOW_SECONDS
        || params.reveal_window_seconds == 0
        || params.reveal_window_seconds > OPEN_COMPETITION_MAX_REVEAL_WINDOW_SECONDS
        || params.reveal_window_seconds > params.competition_window_seconds
        || !(2..=OPEN_COMPETITION_MAX_ENTRIES).contains(&params.max_entries)
        || params.funding_deadline == 0
    {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "open-competition economics, deadlines, or entry capacity are out of bounds"
                .to_string(),
        ));
    }
    Ok(OpenCompetitionCreateParams {
        solver_reward: params.solver_reward,
        verifier_reward: params.verifier_reward,
        terms_hash: normalized_nonzero_bytes32(&params.terms_hash, "terms hash")?,
        policy_hash: normalized_nonzero_bytes32(&params.policy_hash, "policy hash")?,
        acceptance_criteria_hash: normalized_nonzero_bytes32(
            &params.acceptance_criteria_hash,
            "acceptance criteria hash",
        )?,
        benchmark_hash: normalized_nonzero_bytes32(&params.benchmark_hash, "benchmark hash")?,
        evidence_schema_hash: normalized_nonzero_bytes32(
            &params.evidence_schema_hash,
            "evidence schema hash",
        )?,
        funding_deadline: params.funding_deadline,
        competition_window_seconds: params.competition_window_seconds,
        reveal_window_seconds: params.reveal_window_seconds,
        max_entries: params.max_entries,
        verifier_module: normalize_evm_address(&params.verifier_module)?,
        verifier_reward_recipient: normalize_evm_address(&params.verifier_reward_recipient)?,
    })
}

fn open_competition_parameter_words(
    params: &OpenCompetitionCreateParams,
) -> Result<Vec<[u8; 32]>, ChainBaseError> {
    Ok(vec![
        encode_uint256(params.solver_reward)?,
        encode_uint256(params.verifier_reward)?,
        parse_bytes32(&params.terms_hash)?,
        parse_bytes32(&params.policy_hash)?,
        parse_bytes32(&params.acceptance_criteria_hash)?,
        parse_bytes32(&params.benchmark_hash)?,
        parse_bytes32(&params.evidence_schema_hash)?,
        encode_uint256(params.funding_deadline.into())?,
        encode_uint256(params.competition_window_seconds.into())?,
        encode_uint256(params.reveal_window_seconds.into())?,
        encode_uint256(params.max_entries.into())?,
        encode_address(&params.verifier_module)?,
        encode_address(&params.verifier_reward_recipient)?,
    ])
}

fn encode_static_call(signature: &str, words: &[[u8; 32]]) -> String {
    let mut bytes = selector(signature).to_vec();
    for word in words {
        bytes.extend_from_slice(word);
    }
    format!("0x{}", hex::encode(bytes))
}

fn normalized_nonzero_bytes32(value: &str, label: &str) -> Result<String, ChainBaseError> {
    let parsed = parse_bytes32(value)?;
    if parsed.iter().all(|byte| *byte == 0) {
        return Err(ChainBaseError::InvalidVerificationConfiguration(format!(
            "{label} must be nonzero"
        )));
    }
    Ok(format!("0x{}", hex::encode(parsed)))
}

fn zero_hash() -> String {
    format!("0x{}", "00".repeat(32))
}

fn keccak_words(words: &[[u8; 32]]) -> [u8; 32] {
    let mut bytes = Vec::with_capacity(words.len() * 32);
    for word in words {
        bytes.extend_from_slice(word);
    }
    Keccak256::digest(bytes).into()
}

fn normalize_signature_v(v: u8) -> Result<u8, ChainBaseError> {
    match v {
        0 | 1 => Ok(v + 27),
        27 | 28 => Ok(v),
        _ => Err(ChainBaseError::InvalidVerificationConfiguration(
            "authorization signature v must be 0, 1, 27, or 28".to_string(),
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCompetitionReleaseManifest {
    pub protocol_version: String,
    pub network: String,
    pub chain_id: u64,
    pub deployment_state: OpenCompetitionDeploymentState,
    pub factory_contract: String,
    pub implementation_contract: String,
    pub settlement_token: String,
    pub deployment_block: u64,
    pub factory_runtime_code_hash: String,
    pub implementation_runtime_code_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCompetitionStateQuery {
    pub release: OpenCompetitionReleaseManifest,
    pub bounty_contract: String,
    pub solver: Option<String>,
    pub verifier_profile: OpenCompetitionVerifierProfile,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCompetitionOffchainGates {
    pub gas_sponsorship_available: bool,
    pub relay_support_available: bool,
    pub r4_release_evidence_complete: bool,
    pub monitoring_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionSafeState {
    pub schema_version: String,
    pub protocol_version: String,
    pub network: String,
    pub chain_id: u64,
    pub deployment_state: OpenCompetitionDeploymentState,
    pub safe_block_number: u64,
    pub safe_block_hash: String,
    pub safe_block_timestamp: u64,
    pub factory_contract: String,
    pub factory_runtime_code_hash: String,
    pub factory_runtime_matches: bool,
    pub implementation_contract: String,
    pub implementation_runtime_code_hash: String,
    pub implementation_identity_matches: bool,
    pub factory_registered_bounty: bool,
    pub bounty_contract: String,
    pub bounty_runtime_code_hash: String,
    pub bounty_runtime_matches: bool,
    pub settlement_token: String,
    pub settlement_token_matches: bool,
    pub verifier_profile_id: String,
    pub verifier_module: String,
    pub verifier_runtime_code_hash: String,
    pub verifier_runtime_matches: bool,
    pub terms_hash: String,
    pub policy_hash: String,
    pub acceptance_criteria_hash: String,
    pub benchmark_hash: String,
    pub evidence_schema_hash: String,
    pub verifier_commitments_match: bool,
    pub solver_reward: u128,
    pub verifier_reward: u128,
    pub entry_bond: u128,
    pub target_amount: u128,
    pub funded_amount: u128,
    pub fully_funded: bool,
    pub status: u8,
    pub status_name: String,
    pub entry_count: u8,
    pub max_entries: u8,
    pub entry_capacity_available: bool,
    pub solver: Option<String>,
    pub solver_has_entered: Option<bool>,
    pub competition_ends_at: u64,
    pub reveal_window_seconds: u64,
    pub competition_open: bool,
    pub safe_commit_reveal_timing: bool,
    pub onchain_ready_to_enter: bool,
    pub public_inventory_eligible: bool,
    pub blockers: Vec<String>,
    pub evidence_boundary: String,
}

pub async fn observe_open_competition_safe_state(
    rpc_url: &str,
    query: &OpenCompetitionStateQuery,
) -> Result<OpenCompetitionSafeState, ChainBaseError> {
    observe_open_competition_safe_state_with_transport(
        rpc_url,
        query,
        &ReqwestJsonRpcTransport::default(),
    )
    .await
}

pub async fn observe_open_competition_safe_state_with_transport<T>(
    rpc_url: &str,
    query: &OpenCompetitionStateQuery,
    transport: &T,
) -> Result<OpenCompetitionSafeState, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let descriptor = base_network_descriptor(&query.release.network)?;
    if query.release.protocol_version != OPEN_COMPETITION_PROTOCOL_VERSION
        || query.release.chain_id != descriptor.chain_id
        || query.verifier_profile.network != query.release.network
        || query.verifier_profile.chain_id != descriptor.chain_id
    {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "open-competition release manifest has an unsupported protocol or chain".to_string(),
        ));
    }
    let factory = normalize_evm_address(&query.release.factory_contract)?;
    let implementation = normalize_evm_address(&query.release.implementation_contract)?;
    let settlement_token = normalize_evm_address(&query.release.settlement_token)?;
    if settlement_token != normalize_evm_address(&descriptor.native_usdc_token_address)? {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "open-competition release settlement token is not native Base USDC".to_string(),
        ));
    }
    let expected_factory_runtime =
        normalized_nonzero_bytes32(&query.release.factory_runtime_code_hash, "factory runtime")?;
    let expected_implementation_runtime = normalized_nonzero_bytes32(
        &query.release.implementation_runtime_code_hash,
        "implementation runtime",
    )?;
    let bounty = normalize_evm_address(&query.bounty_contract)?;
    let solver = query
        .solver
        .as_deref()
        .map(normalize_evm_address)
        .transpose()?;

    let safe_block = rpc_result(
        transport
            .post_json_value(
                rpc_url,
                &json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "eth_getBlockByNumber",
                    "params": ["safe", false]
                }),
            )
            .await?,
        1,
        "eth_getBlockByNumber",
    )?;
    let safe_block_number = parse_rpc_quantity(
        safe_block
            .get("number")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ChainBaseError::InvalidRpcResponse(
                    "safe block response is missing number".to_string(),
                )
            })?,
    )?;
    if safe_block_number < query.release.deployment_block {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "safe block predates the open-competition factory deployment".to_string(),
        ));
    }
    let safe_block_hash = normalize_hash(
        safe_block
            .get("hash")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ChainBaseError::InvalidRpcResponse(
                    "safe block response is missing hash".to_string(),
                )
            })?,
    )?;
    let safe_block_timestamp = parse_rpc_quantity(
        safe_block
            .get("timestamp")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ChainBaseError::InvalidRpcResponse(
                    "safe block response is missing timestamp".to_string(),
                )
            })?,
    )?;
    let exact_block = format!("0x{safe_block_number:x}");
    let mut request_id = 2_u64;

    let mut batch_requests = Vec::new();
    let profile_verifier = normalize_evm_address(&query.verifier_profile.verifier_address)?;
    let factory_runtime_id =
        push_batch_code(&mut batch_requests, &mut request_id, &factory, &exact_block);
    let implementation_runtime_id = push_batch_code(
        &mut batch_requests,
        &mut request_id,
        &implementation,
        &exact_block,
    );
    let bounty_runtime_id =
        push_batch_code(&mut batch_requests, &mut request_id, &bounty, &exact_block);
    let verifier_runtime_id = push_batch_code(
        &mut batch_requests,
        &mut request_id,
        &profile_verifier,
        &exact_block,
    );
    let factory_protocol_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &factory,
        encode_call("SUPPORTED_PROTOCOL_VERSION()", Vec::new()),
        &exact_block,
    );
    let factory_implementation_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &factory,
        encode_call("implementation()", Vec::new()),
        &exact_block,
    );
    let factory_token_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &factory,
        encode_call("settlementToken()", Vec::new()),
        &exact_block,
    );
    let registered_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &factory,
        encode_call(
            "isCanonicalCompetition(address)",
            vec![encode_address(&bounty)?],
        ),
        &exact_block,
    );
    let bounty_protocol_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &bounty,
        encode_call("protocolVersion()", Vec::new()),
        &exact_block,
    );
    let bounty_factory_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &bounty,
        encode_call("factory()", Vec::new()),
        &exact_block,
    );
    let bounty_token_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &bounty,
        encode_call("settlementToken()", Vec::new()),
        &exact_block,
    );
    let solver_reward_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &bounty,
        encode_call("solverReward()", Vec::new()),
        &exact_block,
    );
    let verifier_reward_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &bounty,
        encode_call("verifierReward()", Vec::new()),
        &exact_block,
    );
    let target_amount_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &bounty,
        encode_call("targetAmount()", Vec::new()),
        &exact_block,
    );
    let funded_amount_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &bounty,
        encode_call("fundedAmount()", Vec::new()),
        &exact_block,
    );
    let status_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &bounty,
        encode_call("status()", Vec::new()),
        &exact_block,
    );
    let entry_count_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &bounty,
        encode_call("entryCount()", Vec::new()),
        &exact_block,
    );
    let max_entries_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &bounty,
        encode_call("maxEntries()", Vec::new()),
        &exact_block,
    );
    let competition_ends_at_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &bounty,
        encode_call("competitionEndsAt()", Vec::new()),
        &exact_block,
    );
    let reveal_window_seconds_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &bounty,
        encode_call("revealWindowSeconds()", Vec::new()),
        &exact_block,
    );
    let verifier_module_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &bounty,
        encode_call("verifierModule()", Vec::new()),
        &exact_block,
    );
    let terms_hash_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &bounty,
        encode_call("termsHash()", Vec::new()),
        &exact_block,
    );
    let policy_hash_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &bounty,
        encode_call("policyHash()", Vec::new()),
        &exact_block,
    );
    let acceptance_criteria_hash_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &bounty,
        encode_call("acceptanceCriteriaHash()", Vec::new()),
        &exact_block,
    );
    let benchmark_hash_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &bounty,
        encode_call("benchmarkHash()", Vec::new()),
        &exact_block,
    );
    let evidence_schema_hash_id = push_batch_call(
        &mut batch_requests,
        &mut request_id,
        &bounty,
        encode_call("evidenceSchemaHash()", Vec::new()),
        &exact_block,
    );
    let solver_has_entered_id = solver
        .as_ref()
        .map(|solver| {
            Ok(push_batch_call(
                &mut batch_requests,
                &mut request_id,
                &bounty,
                encode_call("hasEntered(address)", vec![encode_address(solver)?]),
                &exact_block,
            ))
        })
        .transpose()?;

    let mut batch_results = fetch_batch_results(rpc_url, batch_requests, transport).await?;
    let factory_runtime = take_batch_code_hash(&mut batch_results, factory_runtime_id)?;
    let implementation_runtime =
        take_batch_code_hash(&mut batch_results, implementation_runtime_id)?;
    let bounty_runtime = take_batch_code_hash(&mut batch_results, bounty_runtime_id)?;
    let verifier_runtime = take_batch_code_hash(&mut batch_results, verifier_runtime_id)?;
    let factory_protocol = take_batch_word(&mut batch_results, factory_protocol_id)?;
    let expected_protocol: [u8; 32] =
        Keccak256::digest(OPEN_COMPETITION_PROTOCOL_VERSION.as_bytes()).into();
    let factory_implementation = address_from_word(take_batch_word(
        &mut batch_results,
        factory_implementation_id,
    )?);
    let factory_token = address_from_word(take_batch_word(&mut batch_results, factory_token_id)?);
    let registered = word_as_bool(
        take_batch_word(&mut batch_results, registered_id)?,
        "factory registration",
    )?;
    let expected_bounty_runtime = minimal_proxy_runtime_code_hash(&implementation)?;
    let bounty_protocol = take_batch_word(&mut batch_results, bounty_protocol_id)?;
    let bounty_factory = address_from_word(take_batch_word(&mut batch_results, bounty_factory_id)?);
    let bounty_token = address_from_word(take_batch_word(&mut batch_results, bounty_token_id)?);
    let solver_reward = word_as_u128(
        take_batch_word(&mut batch_results, solver_reward_id)?,
        "solver reward",
    )?;
    let verifier_reward = word_as_u128(
        take_batch_word(&mut batch_results, verifier_reward_id)?,
        "verifier reward",
    )?;
    let target_amount = word_as_u128(
        take_batch_word(&mut batch_results, target_amount_id)?,
        "target amount",
    )?;
    let funded_amount = word_as_u128(
        take_batch_word(&mut batch_results, funded_amount_id)?,
        "funded amount",
    )?;
    let status = word_as_u8(
        take_batch_word(&mut batch_results, status_id)?,
        "competition status",
    )?;
    if status > 3 {
        return Err(ChainBaseError::InvalidRpcResponse(
            "competition status is outside the V1 enum".to_string(),
        ));
    }
    let entry_count = word_as_u8(
        take_batch_word(&mut batch_results, entry_count_id)?,
        "entry count",
    )?;
    let max_entries = word_as_u8(
        take_batch_word(&mut batch_results, max_entries_id)?,
        "max entries",
    )?;
    let competition_ends_at = word_as_u64(
        take_batch_word(&mut batch_results, competition_ends_at_id)?,
        "competition deadline",
    )?;
    let reveal_window_seconds = word_as_u64(
        take_batch_word(&mut batch_results, reveal_window_seconds_id)?,
        "reveal window",
    )?;
    let verifier_module =
        address_from_word(take_batch_word(&mut batch_results, verifier_module_id)?);
    let terms_hash = word_as_hash(take_batch_word(&mut batch_results, terms_hash_id)?);
    let policy_hash = word_as_hash(take_batch_word(&mut batch_results, policy_hash_id)?);
    let acceptance_criteria_hash = word_as_hash(take_batch_word(
        &mut batch_results,
        acceptance_criteria_hash_id,
    )?);
    let benchmark_hash = word_as_hash(take_batch_word(&mut batch_results, benchmark_hash_id)?);
    let evidence_schema_hash = word_as_hash(take_batch_word(
        &mut batch_results,
        evidence_schema_hash_id,
    )?);
    let solver_has_entered = solver_has_entered_id
        .map(|id| word_as_bool(take_batch_word(&mut batch_results, id)?, "wallet entry"))
        .transpose()?;
    if !batch_results.is_empty() {
        return Err(ChainBaseError::InvalidRpcResponse(
            "JSON-RPC batch returned unconsumed results".to_string(),
        ));
    }

    let profile = validate_open_competition_verifier_profile(
        &query.verifier_profile,
        &query.release.network,
        &verifier_module,
        &benchmark_hash,
        &evidence_schema_hash,
    )?;
    let factory_runtime_matches = factory_runtime == expected_factory_runtime
        && factory_protocol == expected_protocol
        && factory_implementation == implementation
        && factory_token == settlement_token;
    let implementation_identity_matches = implementation_runtime == expected_implementation_runtime;
    let bounty_runtime_matches = bounty_runtime == expected_bounty_runtime
        && bounty_protocol == expected_protocol
        && bounty_factory == factory;
    let settlement_token_matches = bounty_token == settlement_token;
    let verifier_runtime_matches = verifier_runtime == profile.runtime_code_hash;
    let verifier_commitments_match = benchmark_hash == profile.benchmark_hash
        && evidence_schema_hash == profile.evidence_schema_hash
        && terms_hash != zero_hash()
        && policy_hash != zero_hash()
        && acceptance_criteria_hash != zero_hash();
    let fully_funded = solver_reward > 0
        && verifier_reward > 0
        && solver_reward.checked_add(verifier_reward) == Some(target_amount)
        && funded_amount == target_amount;
    let entry_capacity_available = entry_count < max_entries
        && max_entries <= OPEN_COMPETITION_MAX_ENTRIES
        && solver_has_entered != Some(true);
    let competition_open = status == 1 && safe_block_timestamp <= competition_ends_at;
    let safe_commit_reveal_timing = competition_open
        && reveal_window_seconds > 0
        && reveal_window_seconds <= OPEN_COMPETITION_MAX_REVEAL_WINDOW_SECONDS
        && safe_block_timestamp
            .checked_add(reveal_window_seconds)
            .is_some_and(|deadline| deadline <= competition_ends_at);
    let mut blockers = Vec::new();
    for (ready, name) in [
        (factory_runtime_matches, "factory_runtime"),
        (implementation_identity_matches, "implementation_identity"),
        (registered, "factory_registration"),
        (bounty_runtime_matches, "bounty_runtime"),
        (settlement_token_matches, "settlement_token"),
        (verifier_runtime_matches, "verifier_runtime"),
        (verifier_commitments_match, "verifier_commitments"),
        (fully_funded, "fully_funded"),
        (competition_open, "competition_open"),
        (entry_capacity_available, "entry_capacity"),
        (safe_commit_reveal_timing, "commit_reveal_timing"),
    ] {
        if !ready {
            blockers.push(name.to_string());
        }
    }
    let onchain_ready_to_enter = blockers.is_empty();
    Ok(OpenCompetitionSafeState {
        schema_version: OPEN_COMPETITION_STATE_SCHEMA.to_string(),
        protocol_version: OPEN_COMPETITION_PROTOCOL_VERSION.to_string(),
        network: query.release.network.clone(),
        chain_id: descriptor.chain_id,
        deployment_state: query.release.deployment_state,
        safe_block_number,
        safe_block_hash,
        safe_block_timestamp,
        factory_contract: factory,
        factory_runtime_code_hash: factory_runtime,
        factory_runtime_matches,
        implementation_contract: implementation,
        implementation_runtime_code_hash: implementation_runtime,
        implementation_identity_matches,
        factory_registered_bounty: registered,
        bounty_contract: bounty,
        bounty_runtime_code_hash: bounty_runtime,
        bounty_runtime_matches,
        settlement_token,
        settlement_token_matches,
        verifier_profile_id: profile.profile_id,
        verifier_module,
        verifier_runtime_code_hash: verifier_runtime,
        verifier_runtime_matches,
        terms_hash,
        policy_hash,
        acceptance_criteria_hash,
        benchmark_hash,
        evidence_schema_hash,
        verifier_commitments_match,
        solver_reward,
        verifier_reward,
        entry_bond: verifier_reward,
        target_amount,
        funded_amount,
        fully_funded,
        status,
        status_name: ["open", "competition", "settled", "cancelled"][status as usize]
            .to_string(),
        entry_count,
        max_entries,
        entry_capacity_available,
        solver,
        solver_has_entered,
        competition_ends_at,
        reveal_window_seconds,
        competition_open,
        safe_commit_reveal_timing,
        onchain_ready_to_enter,
        public_inventory_eligible: onchain_ready_to_enter
            && profile.public_inventory_eligible
            && query.release.deployment_state.permits_public_inventory(),
        blockers,
        evidence_boundary: "This is a safe-block observation of canonical identity, funding, verifier, capacity, wallet-entry, and timing facts. It is not a commitment, reveal, settlement, or payment receipt. Only a canonical BountySettled event proves payment.".to_string(),
    })
}

pub fn open_competition_readiness_from_state(
    state: &OpenCompetitionSafeState,
    gates: &OpenCompetitionOffchainGates,
) -> OpenCompetitionReadinessReport {
    open_competition_readiness(&OpenCompetitionReadinessEvidence {
        canonical_factory_configured: state.factory_runtime_matches
            && state.implementation_identity_matches,
        canonical_bounty_runtime: state.factory_registered_bounty && state.bounty_runtime_matches,
        valid_terms: state.verifier_commitments_match,
        fully_funded: state.fully_funded,
        deterministic_verifier_ready: state.verifier_runtime_matches,
        competition_open: state.competition_open,
        entry_capacity_available: state.entry_capacity_available,
        safe_commit_reveal_timing: state.safe_commit_reveal_timing,
        gas_sponsorship_available: gates.gas_sponsorship_available,
        relay_support_available: gates.relay_support_available,
        r4_release_evidence_complete: gates.r4_release_evidence_complete,
        monitoring_active: gates.monitoring_active,
    })
}

fn take_request_id(request_id: &mut u64) -> u64 {
    let current = *request_id;
    *request_id += 1;
    current
}

fn word_as_bool(word: [u8; 32], label: &str) -> Result<bool, ChainBaseError> {
    if word[..31].iter().any(|byte| *byte != 0) || word[31] > 1 {
        return Err(ChainBaseError::InvalidRpcResponse(format!(
            "{label} is not an ABI bool"
        )));
    }
    Ok(word[31] == 1)
}

fn word_as_u8(word: [u8; 32], label: &str) -> Result<u8, ChainBaseError> {
    if word[..31].iter().any(|byte| *byte != 0) {
        return Err(ChainBaseError::InvalidRpcResponse(format!(
            "{label} exceeds uint8"
        )));
    }
    Ok(word[31])
}

fn word_as_u64(word: [u8; 32], label: &str) -> Result<u64, ChainBaseError> {
    if word[..24].iter().any(|byte| *byte != 0) {
        return Err(ChainBaseError::InvalidRpcResponse(format!(
            "{label} exceeds uint64"
        )));
    }
    Ok(u64::from_be_bytes(
        word[24..].try_into().expect("eight bytes"),
    ))
}

fn word_as_u128(word: [u8; 32], label: &str) -> Result<u128, ChainBaseError> {
    if word[..16].iter().any(|byte| *byte != 0) {
        return Err(ChainBaseError::InvalidRpcResponse(format!(
            "{label} exceeds uint128"
        )));
    }
    Ok(u128::from_be_bytes(
        word[16..].try_into().expect("sixteen bytes"),
    ))
}

fn word_as_hash(word: [u8; 32]) -> String {
    format!("0x{}", hex::encode(word))
}

fn minimal_proxy_runtime_code_hash(implementation: &str) -> Result<String, ChainBaseError> {
    runtime_code_hash(&minimal_proxy_runtime_code(implementation)?)
}

fn minimal_proxy_runtime_code(implementation: &str) -> Result<String, ChainBaseError> {
    let implementation = normalize_evm_address(implementation)?;
    let mut runtime = hex::decode("363d3d373d3d3d363d73").expect("proxy prefix is valid hex");
    runtime.extend_from_slice(
        &hex::decode(&implementation[2..]).expect("normalized implementation is valid hex"),
    );
    runtime.extend_from_slice(
        &hex::decode("5af43d82803e903d91602b57fd5bf3").expect("proxy suffix is valid hex"),
    );
    Ok(format!("0x{}", hex::encode(runtime)))
}

fn runtime_code_hash(code: &str) -> Result<String, ChainBaseError> {
    let bytes = code
        .strip_prefix("0x")
        .filter(|hex| hex.len() % 2 == 0)
        .ok_or_else(|| {
            ChainBaseError::InvalidRpcResponse("eth_getCode result is malformed".to_string())
        })?;
    let bytes = hex::decode(bytes).map_err(|_| {
        ChainBaseError::InvalidRpcResponse("eth_getCode result is malformed".to_string())
    })?;
    Ok(format!("0x{}", hex::encode(Keccak256::digest(bytes))))
}

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
    pub wallet_calls: Vec<EvmTransactionIntent>,
    pub supports_single_wallet_batch: bool,
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
        wallet_calls: Vec::new(),
        supports_single_wallet_batch: false,
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
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use alloy::{
        primitives::{keccak256, Address, Bytes, B256, U256},
        sol_types::SolValue,
    };
    use std::sync::Mutex;

    struct SafeStateFixtureTransport {
        query: OpenCompetitionStateQuery,
        verifier_runtime_drift: bool,
        seen: Mutex<Vec<Value>>,
    }

    #[async_trait::async_trait]
    impl JsonRpcTransport for SafeStateFixtureTransport {
        async fn post_json_value(
            &self,
            _rpc_url: &str,
            request: &Value,
        ) -> Result<Value, ChainBaseError> {
            self.seen.lock().unwrap().push(request.clone());
            let id = request["id"].as_u64().unwrap();
            match request["method"].as_str().unwrap() {
                "eth_getBlockByNumber" => Ok(json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": { "number": "0x64", "hash": format!("0x{}", "99".repeat(32)), "timestamp": "0x3e8" }
                })),
                "eth_getCode" => {
                    let address = request["params"][0].as_str().unwrap().to_ascii_lowercase();
                    let code = if address == self.query.release.factory_contract {
                        "0x6001".to_string()
                    } else if address == self.query.release.implementation_contract {
                        "0x6002".to_string()
                    } else if address == self.query.bounty_contract {
                        minimal_proxy_runtime_code(&self.query.release.implementation_contract)?
                    } else if address == self.query.verifier_profile.verifier_address {
                        if self.verifier_runtime_drift {
                            "0x6077".to_string()
                        } else {
                            "0x6003".to_string()
                        }
                    } else {
                        return Err(ChainBaseError::RpcTransport(format!(
                            "unexpected eth_getCode address {address}"
                        )));
                    };
                    Ok(json!({ "jsonrpc": "2.0", "id": id, "result": code }))
                }
                "eth_call" => {
                    let to = request["params"][0]["to"]
                        .as_str()
                        .unwrap()
                        .to_ascii_lowercase();
                    let data = request["params"][0]["data"].as_str().unwrap();
                    let function_selector = &data[..10];
                    let selector_for =
                        |function: &str| format!("0x{}", hex::encode(selector(function)));
                    let protocol: [u8; 32] =
                        Keccak256::digest(OPEN_COMPETITION_PROTOCOL_VERSION.as_bytes()).into();
                    let word = if to == self.query.release.factory_contract {
                        if function_selector == selector_for("SUPPORTED_PROTOCOL_VERSION()") {
                            protocol
                        } else if function_selector == selector_for("implementation()") {
                            encode_address(&self.query.release.implementation_contract)?
                        } else if function_selector == selector_for("settlementToken()") {
                            encode_address(&self.query.release.settlement_token)?
                        } else if function_selector
                            == selector_for("isCanonicalCompetition(address)")
                        {
                            encode_uint256(1_u128)?
                        } else {
                            return Err(ChainBaseError::RpcTransport(format!(
                                "unexpected factory call {data}"
                            )));
                        }
                    } else if to == self.query.bounty_contract {
                        if function_selector == selector_for("protocolVersion()") {
                            protocol
                        } else if function_selector == selector_for("factory()") {
                            encode_address(&self.query.release.factory_contract)?
                        } else if function_selector == selector_for("settlementToken()") {
                            encode_address(&self.query.release.settlement_token)?
                        } else if function_selector == selector_for("solverReward()") {
                            encode_uint256(900_000_u128)?
                        } else if function_selector == selector_for("verifierReward()") {
                            encode_uint256(100_000_u128)?
                        } else if function_selector == selector_for("targetAmount()")
                            || function_selector == selector_for("fundedAmount()")
                        {
                            encode_uint256(1_000_000_u128)?
                        } else if function_selector == selector_for("status()")
                            || function_selector == selector_for("entryCount()")
                        {
                            encode_uint256(1_u128)?
                        } else if function_selector == selector_for("maxEntries()") {
                            encode_uint256(4_u128)?
                        } else if function_selector == selector_for("competitionEndsAt()") {
                            encode_uint256(5_000_u128)?
                        } else if function_selector == selector_for("revealWindowSeconds()") {
                            encode_uint256(300_u128)?
                        } else if function_selector == selector_for("verifierModule()") {
                            encode_address(&self.query.verifier_profile.verifier_address)?
                        } else if function_selector == selector_for("termsHash()") {
                            [0x31; 32]
                        } else if function_selector == selector_for("policyHash()") {
                            [0x32; 32]
                        } else if function_selector == selector_for("acceptanceCriteriaHash()") {
                            [0x33; 32]
                        } else if function_selector == selector_for("benchmarkHash()") {
                            parse_bytes32(&self.query.verifier_profile.benchmark_hash)?
                        } else if function_selector == selector_for("evidenceSchemaHash()") {
                            parse_bytes32(&self.query.verifier_profile.evidence_schema_hash)?
                        } else if function_selector == selector_for("hasEntered(address)") {
                            encode_uint256(0_u128)?
                        } else {
                            return Err(ChainBaseError::RpcTransport(format!(
                                "unexpected bounty call {data}"
                            )));
                        }
                    } else {
                        return Err(ChainBaseError::RpcTransport(format!(
                            "unexpected eth_call target {to}"
                        )));
                    };
                    Ok(json!({ "jsonrpc": "2.0", "id": id, "result": word_hex(word) }))
                }
                method => Err(ChainBaseError::RpcTransport(format!(
                    "unexpected method {method}"
                ))),
            }
        }
    }

    fn safe_state_query() -> OpenCompetitionStateQuery {
        let descriptor = base_network_descriptor("base-mainnet").unwrap();
        let mut profile = built_in_open_competition_verifier_catalog("base-mainnet")
            .unwrap()
            .profiles
            .remove(0);
        profile.deployment_state = OpenCompetitionDeploymentState::ActiveReadyToEarn;
        profile.public_inventory_eligible = true;
        profile.runtime_code_hash = runtime_code_hash("0x6003").unwrap();
        OpenCompetitionStateQuery {
            release: OpenCompetitionReleaseManifest {
                protocol_version: OPEN_COMPETITION_PROTOCOL_VERSION.to_string(),
                network: "base-mainnet".to_string(),
                chain_id: descriptor.chain_id,
                deployment_state: OpenCompetitionDeploymentState::ActiveReadyToEarn,
                factory_contract: "0x1111111111111111111111111111111111111111".to_string(),
                implementation_contract: "0x2222222222222222222222222222222222222222".to_string(),
                settlement_token: descriptor.native_usdc_token_address,
                deployment_block: 50,
                factory_runtime_code_hash: runtime_code_hash("0x6001").unwrap(),
                implementation_runtime_code_hash: runtime_code_hash("0x6002").unwrap(),
            },
            bounty_contract: "0x3333333333333333333333333333333333333333".to_string(),
            solver: Some("0x4444444444444444444444444444444444444444".to_string()),
            verifier_profile: profile,
        }
    }

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

    #[tokio::test]
    async fn safe_state_reads_every_canonical_fact_at_one_exact_block() {
        let query = safe_state_query();
        let transport = SafeStateFixtureTransport {
            query: query.clone(),
            verifier_runtime_drift: false,
            seen: Mutex::new(Vec::new()),
        };
        let state = observe_open_competition_safe_state_with_transport(
            "https://mainnet.example",
            &query,
            &transport,
        )
        .await
        .unwrap();
        assert!(state.onchain_ready_to_enter);
        assert!(state.public_inventory_eligible);
        assert_eq!(state.entry_bond, 100_000);
        assert_eq!(state.entry_count, 1);
        assert_eq!(state.max_entries, 4);
        assert_eq!(state.solver_has_entered, Some(false));
        let seen = transport.seen.lock().unwrap();
        assert_eq!(seen[0]["params"], json!(["safe", false]));
        assert!(seen[1..].iter().all(|request| {
            request["params"]
                .as_array()
                .is_some_and(|params| params.last().is_some_and(|block| block == "0x64"))
        }));
    }

    #[tokio::test]
    async fn safe_state_runtime_drift_removes_inventory_and_entry_readiness() {
        let query = safe_state_query();
        let transport = SafeStateFixtureTransport {
            query: query.clone(),
            verifier_runtime_drift: true,
            seen: Mutex::new(Vec::new()),
        };
        let state = observe_open_competition_safe_state_with_transport(
            "https://mainnet.example",
            &query,
            &transport,
        )
        .await
        .unwrap();
        assert!(!state.verifier_runtime_matches);
        assert!(!state.onchain_ready_to_enter);
        assert!(!state.public_inventory_eligible);
        assert!(state.blockers.contains(&"verifier_runtime".to_string()));
    }

    #[test]
    fn recovery_actions_remain_plannable_after_new_entries_close() {
        let mut evidence = OpenCompetitionReadinessEvidence {
            canonical_factory_configured: true,
            canonical_bounty_runtime: true,
            valid_terms: true,
            deterministic_verifier_ready: true,
            competition_open: true,
            safe_commit_reveal_timing: true,
            ..OpenCompetitionReadinessEvidence::default()
        };
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

    #[test]
    fn executable_entry_plan_approves_exact_bond_and_commits_only_public_value() {
        let readiness = open_competition_readiness(&ready_evidence());
        let mut plan = plan_open_competition_action(
            OpenCompetitionOperation::PrepareOpenCompetitionCommit,
            &readiness,
            Some("0x1111111111111111111111111111111111111111".to_string()),
            Some("commitSolution".to_string()),
            json!({ "commitment": format!("0x{}", "aa".repeat(32)) }),
        );
        attach_open_competition_commit_calls(
            &mut plan,
            "0x2222222222222222222222222222222222222222",
            "0x1111111111111111111111111111111111111111",
            "0x3333333333333333333333333333333333333333",
            &format!("0x{}", "aa".repeat(32)),
            100_000,
        )
        .unwrap();
        assert_eq!(plan.wallet_calls.len(), 2);
        assert!(plan.supports_single_wallet_batch);
        assert_eq!(
            &plan.wallet_calls[0].data[2..10],
            hex::encode(selector("approve(address,uint256)"))
        );
        assert_eq!(
            &plan.wallet_calls[1].data[2..10],
            hex::encode(selector("commitSolution(bytes32)"))
        );
        assert!(!plan.wallet_calls[1].data.contains(&"cc".repeat(32)));
    }

    #[test]
    fn executable_reveal_plan_encodes_dynamic_proof() {
        let readiness = open_competition_readiness(&ready_evidence());
        let mut plan = plan_open_competition_action(
            OpenCompetitionOperation::PrepareOpenCompetitionReveal,
            &readiness,
            Some("0x1111111111111111111111111111111111111111".to_string()),
            Some("revealSolution".to_string()),
            Value::Null,
        );
        attach_open_competition_reveal_call(
            &mut plan,
            "0x1111111111111111111111111111111111111111",
            "0x3333333333333333333333333333333333333333",
            &format!("0x{}", "aa".repeat(32)),
            &format!("0x{}", "bb".repeat(32)),
            &format!("0x{}", "cc".repeat(32)),
            "0x010203",
        )
        .unwrap();
        assert_eq!(plan.wallet_calls.len(), 1);
        let calldata = hex::decode(&plan.wallet_calls[0].data[2..]).unwrap();
        assert_eq!(
            &calldata[..4],
            &selector("revealSolution(bytes32,bytes32,bytes32,bytes)")
        );
        assert_eq!(calldata.len(), 4 + 6 * 32);
        assert_eq!(&calldata[4 + 5 * 32..4 + 5 * 32 + 3], &[1, 2, 3]);
    }

    #[test]
    fn blocked_entry_plan_never_contains_wallet_calls() {
        let readiness = open_competition_readiness(&OpenCompetitionReadinessEvidence::default());
        let mut plan = plan_open_competition_action(
            OpenCompetitionOperation::PrepareOpenCompetitionCommit,
            &readiness,
            Some("0x1111111111111111111111111111111111111111".to_string()),
            Some("commitSolution".to_string()),
            Value::Null,
        );
        attach_open_competition_commit_calls(
            &mut plan,
            "0x2222222222222222222222222222222222222222",
            "0x1111111111111111111111111111111111111111",
            "0x3333333333333333333333333333333333333333",
            &format!("0x{}", "aa".repeat(32)),
            100_000,
        )
        .unwrap();
        assert!(plan.wallet_calls.is_empty());
    }

    #[test]
    fn versioned_decoder_treats_only_settlement_as_payment_evidence() {
        let bounty_id = format!("0x{}", "11".repeat(32));
        let solver = "0x3333333333333333333333333333333333333333";
        let words = [
            encode_uint256(900_000).unwrap(),
            encode_uint256(100_000).unwrap(),
            encode_uint256(0).unwrap(),
            encode_uint256(100_000).unwrap(),
            [0xaa; 32],
            [0xbb; 32],
            [0xcc; 32],
            [0xdd; 32],
        ];
        let mut data = String::from("0x");
        for word in words {
            data.push_str(&hex::encode(word));
        }
        let events = decode_open_competition_logs([EvmLog {
            address: "0x1111111111111111111111111111111111111111".to_string(),
            topics: vec![
                event_topic("BountySettled(bytes32,uint64,address,uint256,uint256,uint256,uint256,bytes32,bytes32,bytes32,bytes32)"),
                bounty_id.clone(),
                word_hex(encode_uint256(1).unwrap()),
                word_hex(encode_address(solver).unwrap()),
            ],
            data,
            tx_hash: format!("0x{}", "22".repeat(32)),
            block_number: 123,
            log_index: 4,
            occurred_at: Some(Utc::now()),
        }])
        .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].protocol_version,
            OPEN_COMPETITION_PROTOCOL_VERSION
        );
        assert!(events[0].kind.is_payment_evidence());
        assert_eq!(events[0].data["canonical_payment_evidence"], true);
        assert_eq!(events[0].data["solver_reward"], 900_000_u64);
    }

    #[test]
    fn open_competition_topics_are_version_specific_and_complete() {
        let topics = open_competition_event_topics();
        assert_eq!(topics.len(), 14);
        assert_eq!(
            topics
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            topics.len()
        );
        assert!(!topics.contains(&event_topic(
            "BountyClaimed(bytes32,uint64,address,bytes32,bytes32,uint256,uint64)"
        )));
    }

    #[test]
    fn commitment_envelope_matches_solidity_abi_encoding_and_rejects_substitution() {
        let salt = [0xcc; 32];
        let input = OpenCompetitionCommitmentInput {
            network: "base-sepolia".to_string(),
            bounty: "0x1111111111111111111111111111111111111111".to_string(),
            solver: "0x2222222222222222222222222222222222222222".to_string(),
            submission_hash: format!("0x{}", "aa".repeat(32)),
            evidence_hash: format!("0x{}", "bb".repeat(32)),
        };
        let envelope = build_open_competition_commitment_envelope(input, salt).unwrap();
        let domain = keccak256(b"agent-bounties/open-competition-v1-solution");
        let encoded = (
            B256::from(domain),
            U256::from(84_532_u64),
            "0x1111111111111111111111111111111111111111"
                .parse::<Address>()
                .unwrap(),
            "0x2222222222222222222222222222222222222222"
                .parse::<Address>()
                .unwrap(),
            B256::from([0xaa; 32]),
            B256::from([0xbb; 32]),
            B256::from([0xcc; 32]),
        )
            .abi_encode();
        assert_eq!(envelope.commitment, format!("{:#x}", keccak256(encoded)));
        assert!(validate_open_competition_commitment_envelope(
            &envelope,
            "base-sepolia",
            "0x1111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
        )
        .is_ok());
        assert!(validate_open_competition_commitment_envelope(
            &envelope,
            "base-sepolia",
            "0x1111111111111111111111111111111111111111",
            "0x3333333333333333333333333333333333333333",
        )
        .is_err());

        let mut tampered = envelope;
        tampered.evidence_hash = format!("0x{}", "dd".repeat(32));
        assert!(validate_open_competition_commitment_envelope(
            &tampered,
            "base-sepolia",
            "0x1111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
        )
        .is_err());
    }

    fn creation_request(
        funding_authorization: Option<OpenCompetitionFundingAuthorization>,
    ) -> OpenCompetitionCreationRequest {
        let profile = built_in_open_competition_verifier_catalog("base-mainnet")
            .unwrap()
            .profiles
            .remove(0);
        OpenCompetitionCreationRequest {
            network: "base-mainnet".to_string(),
            factory_contract: "0x1111111111111111111111111111111111111111".to_string(),
            implementation_contract: "0x2222222222222222222222222222222222222222".to_string(),
            creator: "0x884834e884d6e93462655a2820140ad03e6747bc".to_string(),
            creation_nonce: format!("0x{}", "12".repeat(32)),
            initial_funding: 1_000_000,
            verifier_profile: profile.clone(),
            params: OpenCompetitionCreateParams {
                solver_reward: 900_000,
                verifier_reward: 100_000,
                terms_hash: format!("0x{}", "31".repeat(32)),
                policy_hash: format!("0x{}", "32".repeat(32)),
                acceptance_criteria_hash: format!("0x{}", "33".repeat(32)),
                benchmark_hash: profile.benchmark_hash,
                evidence_schema_hash: profile.evidence_schema_hash,
                funding_deadline: 2_000_000_000,
                competition_window_seconds: 86_400,
                reveal_window_seconds: 3_600,
                max_entries: 4,
                verifier_module: profile.verifier_address,
                verifier_reward_recipient: "0x3333333333333333333333333333333333333333".to_string(),
            },
            funding_authorization,
        }
    }

    #[test]
    fn creation_plan_is_exact_and_not_public_during_hidden_canary() {
        let plan = plan_open_competition_creation(creation_request(None)).unwrap();
        assert!(plan.ready_to_broadcast);
        assert_eq!(plan.funding_mode, "approval_then_create");
        assert_eq!(plan.wallet_calls.len(), 2);
        assert!(plan.approve.is_some());
        assert!(plan.create_competition.is_some());
        assert!(!plan.public_inventory_eligible);
        assert!(plan.wallet_calls[0].data.starts_with("0x095ea7b3"));
        assert_eq!(plan.bounty_id.len(), 66);
        assert_eq!(plan.predicted_bounty_contract.len(), 42);
    }

    #[test]
    fn authorized_creation_binds_usdc_to_predicted_bounty_before_signature() {
        let authorization = OpenCompetitionFundingAuthorization {
            valid_after: 1_900_000_000,
            valid_before: 2_000_000_000,
            nonce: format!("0x{}", "44".repeat(32)),
            signature: None,
        };
        let plan = plan_open_competition_creation(creation_request(Some(authorization))).unwrap();
        assert_eq!(plan.funding_mode, "eip3009_authorized");
        assert!(!plan.ready_to_broadcast);
        assert!(plan.wallet_calls.is_empty());
        assert!(plan.create_competition.is_none());
        let typed_data = plan.eip3009_authorization.unwrap();
        assert_eq!(typed_data.message.to, plan.predicted_bounty_contract);
        assert_eq!(typed_data.message.from, plan.creator);
        assert_eq!(typed_data.message.value, "1000000");
    }

    #[test]
    fn catalog_rejects_factory_origin_as_verifier_approval() {
        let profile = built_in_open_competition_verifier_catalog("base-mainnet")
            .unwrap()
            .profiles
            .remove(0);
        let error = validate_open_competition_verifier_profile(
            &profile,
            "base-mainnet",
            "0x4444444444444444444444444444444444444444",
            &profile.benchmark_hash,
            &profile.evidence_schema_hash,
        )
        .unwrap_err();
        assert!(error.to_string().contains("approved catalog profile"));
    }

    #[test]
    fn mainnet_catalog_pins_the_settled_hidden_canary_profile() {
        let profile = built_in_open_competition_verifier_catalog("base-mainnet")
            .unwrap()
            .profiles
            .remove(0);
        assert_eq!(
            profile.deployment_state,
            OpenCompetitionDeploymentState::MainnetCanaryNotReadyToEarn
        );
        assert!(!profile.public_inventory_eligible);
        assert_eq!(
            profile.benchmark_hash,
            "0x8f5dc601eaff77e6102aab44f16a9b176df7ce0a998078782fb5d4b9e0c0ebf2"
        );
        assert_eq!(
            profile.evidence_schema_hash,
            "0xea961c63fb67f86823003426b04a928406e44e9c8acc3dcb298189e9558083da"
        );
        assert_eq!(
            profile.evidence_schema,
            "agent-bounties/leading-zero-work-evidence-v1"
        );
    }

    #[test]
    fn entrant_wallet_payloads_match_solidity_abi_encoding() {
        let bounty = "0x1111111111111111111111111111111111111111";
        let commitment = format!("0x{}", "22".repeat(32));
        let commit = encode_open_competition_entrant_commit_payload(bounty, &commitment).unwrap();
        let expected_commit =
            (bounty.parse::<Address>().unwrap(), B256::from([0x22; 32])).abi_encode();
        assert_eq!(commit, format!("0x{}", hex::encode(expected_commit)));

        let reveal = encode_open_competition_entrant_reveal_payload(
            bounty,
            &format!("0x{}", "33".repeat(32)),
            &format!("0x{}", "44".repeat(32)),
            &format!("0x{}", "55".repeat(32)),
            "0xaabbcc",
        )
        .unwrap();
        let expected_reveal = (
            bounty.parse::<Address>().unwrap(),
            B256::from([0x33; 32]),
            B256::from([0x44; 32]),
            B256::from([0x55; 32]),
            Bytes::from(vec![0xaa, 0xbb, 0xcc]),
        )
            .abi_encode_params();
        assert_eq!(reveal, format!("0x{}", hex::encode(expected_reveal)));
    }

    #[test]
    fn entrant_wallet_typed_action_and_relay_call_are_exact() {
        let wallet = "0x1111111111111111111111111111111111111111";
        let relayer = "0x2222222222222222222222222222222222222222";
        let payload = encode_open_competition_entrant_commit_payload(
            "0x3333333333333333333333333333333333333333",
            &format!("0x{}", "44".repeat(32)),
        )
        .unwrap();
        let plan = plan_open_competition_entrant_action(
            "base-sepolia",
            wallet,
            "0x5555555555555555555555555555555555555555",
            &format!("0x{}", "66".repeat(32)),
            7,
            OpenCompetitionEntrantAction::Commit,
            9,
            2_000_000_000,
            &payload,
        )
        .unwrap();
        assert_eq!(plan.schema_version, OPEN_COMPETITION_ENTRANT_ACTION_SCHEMA);
        assert_eq!(plan.chain_id, 84_532);
        assert_eq!(plan.action_code, 0);
        assert_eq!(
            plan.signing_payload.primary_type,
            "OpenCompetitionEntrantAction"
        );
        assert_eq!(
            plan.signing_payload.domain.name,
            "Agent Bounties Open Competition Entrant Wallet"
        );
        assert_eq!(plan.signing_payload.message.nonce, "9");
        assert_eq!(
            plan.payload_hash,
            format!(
                "0x{}",
                hex::encode(keccak256(
                    hex::decode(payload.trim_start_matches("0x")).unwrap()
                ))
            )
        );

        let signature = format!("0x{}", "77".repeat(65));
        let intent =
            attach_open_competition_entrant_relay_signature(&plan, relayer, &signature).unwrap();
        let mut expected =
            selector("executeWithSignature(uint8,bytes,uint256,uint256,bytes)").to_vec();
        expected.extend_from_slice(
            &(
                U256::ZERO,
                Bytes::from(hex::decode(payload.trim_start_matches("0x")).unwrap()),
                U256::from(9_u64),
                U256::from(2_000_000_000_u64),
                Bytes::from(vec![0x77_u8; 65]),
            )
                .abi_encode_params(),
        );
        assert_eq!(intent.from.as_deref(), Some(relayer));
        assert_eq!(intent.to, wallet);
        assert_eq!(intent.value_wei, 0);
        assert_eq!(intent.data, format!("0x{}", hex::encode(expected)));
    }
}

fn push_batch_request(
    requests: &mut Vec<(u64, String, Value)>,
    request_id: &mut u64,
    method: &str,
    params: Value,
) -> u64 {
    let id = take_request_id(request_id);
    requests.push((
        id,
        method.to_string(),
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }),
    ));
    id
}

fn push_batch_code(
    requests: &mut Vec<(u64, String, Value)>,
    request_id: &mut u64,
    address: &str,
    block: &str,
) -> u64 {
    push_batch_request(requests, request_id, "eth_getCode", json!([address, block]))
}

fn push_batch_call(
    requests: &mut Vec<(u64, String, Value)>,
    request_id: &mut u64,
    contract: &str,
    data: String,
    block: &str,
) -> u64 {
    push_batch_request(
        requests,
        request_id,
        "eth_call",
        json!([{ "to": contract, "data": data }, block]),
    )
}

async fn fetch_batch_results<T>(
    rpc_url: &str,
    requests: Vec<(u64, String, Value)>,
    transport: &T,
) -> Result<BTreeMap<u64, Value>, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let mut results = BTreeMap::new();
    let mut chunks = Vec::new();
    let mut current_chunk = Vec::new();
    let mut current_weight = 0_u8;
    for request in requests {
        let weight = if request.1 == "eth_getCode" { 2 } else { 1 };
        if current_weight + weight > 2 {
            chunks.push(std::mem::take(&mut current_chunk));
            current_weight = 0;
        }
        current_weight += weight;
        current_chunk.push(request);
    }
    if !current_chunk.is_empty() {
        chunks.push(current_chunk);
    }

    for chunk in &chunks {
        let payloads = chunk
            .iter()
            .map(|(_, _, payload)| payload.clone())
            .collect::<Vec<_>>();
        let mut attempt = 0_u64;
        let chunk_results = loop {
            attempt += 1;
            let responses = match transport.post_json_values(rpc_url, &payloads).await {
                Ok(responses) => responses,
                Err(error) if rpc_rate_limited(&error) && attempt < 4 => {
                    tokio::time::sleep(std::time::Duration::from_millis(500 * attempt)).await;
                    continue;
                }
                Err(error) => return Err(error),
            };
            if responses.len() != chunk.len() {
                return Err(ChainBaseError::InvalidRpcResponse(
                    "JSON-RPC batch response count does not match the request count".to_string(),
                ));
            }
            let mut responses_by_id = BTreeMap::new();
            for response in responses {
                let id = response.get("id").and_then(Value::as_u64).ok_or_else(|| {
                    ChainBaseError::InvalidRpcResponse(
                        "JSON-RPC batch response is missing a numeric id".to_string(),
                    )
                })?;
                if responses_by_id.insert(id, response).is_some() {
                    return Err(ChainBaseError::InvalidRpcResponse(
                        "JSON-RPC batch response contains a duplicate id".to_string(),
                    ));
                }
            }
            let mut chunk_results = BTreeMap::new();
            let mut rate_limit = None;
            for (id, method, _) in chunk {
                let response = responses_by_id.remove(id).ok_or_else(|| {
                    ChainBaseError::InvalidRpcResponse(format!(
                        "JSON-RPC batch response is missing id {id}"
                    ))
                })?;
                match rpc_result(response, *id, method) {
                    Ok(result) => {
                        chunk_results.insert(*id, result);
                    }
                    Err(error) if rpc_rate_limited(&error) => {
                        rate_limit = Some(error);
                        break;
                    }
                    Err(error) => return Err(error),
                }
            }
            if let Some(error) = rate_limit {
                if attempt < 4 {
                    tokio::time::sleep(std::time::Duration::from_millis(500 * attempt)).await;
                    continue;
                }
                let methods = chunk
                    .iter()
                    .map(|(id, method, _)| format!("{id}:{method}"))
                    .collect::<Vec<_>>()
                    .join(",");
                return Err(match error {
                    ChainBaseError::RpcProviderError { code, message } => {
                        ChainBaseError::RpcProviderError {
                            code,
                            message: format!("{message}; requests={methods}"),
                        }
                    }
                    other => other,
                });
            }
            if !responses_by_id.is_empty() {
                return Err(ChainBaseError::InvalidRpcResponse(
                    "JSON-RPC batch response contains an unknown id".to_string(),
                ));
            }
            break chunk_results;
        };
        results.extend(chunk_results);
    }
    Ok(results)
}

fn rpc_rate_limited(error: &ChainBaseError) -> bool {
    match error {
        ChainBaseError::RpcHttpStatus(429) => true,
        ChainBaseError::RpcProviderError { code, message } => {
            *code == -32016 || message.to_ascii_lowercase().contains("rate limit")
        }
        _ => false,
    }
}

fn take_batch_word(
    results: &mut BTreeMap<u64, Value>,
    request_id: u64,
) -> Result<[u8; 32], ChainBaseError> {
    let result = results.remove(&request_id).ok_or_else(|| {
        ChainBaseError::InvalidRpcResponse(format!(
            "JSON-RPC batch result is missing id {request_id}"
        ))
    })?;
    parse_bytes32(result.as_str().ok_or_else(|| {
        ChainBaseError::InvalidRpcResponse("eth_call result is not one ABI word".to_string())
    })?)
}

fn take_batch_code_hash(
    results: &mut BTreeMap<u64, Value>,
    request_id: u64,
) -> Result<String, ChainBaseError> {
    let result = results.remove(&request_id).ok_or_else(|| {
        ChainBaseError::InvalidRpcResponse(format!(
            "JSON-RPC batch result is missing id {request_id}"
        ))
    })?;
    let code = result.as_str().ok_or_else(|| {
        ChainBaseError::InvalidRpcResponse("eth_getCode result is not hex bytecode".to_string())
    })?;
    runtime_code_hash(code)
}

pub fn attach_open_competition_commit_calls(
    plan: &mut OpenCompetitionActionPlan,
    settlement_token: &str,
    bounty_contract: &str,
    solver: &str,
    commitment: &str,
    entry_bond: u128,
) -> Result<(), ChainBaseError> {
    if !plan.allowed {
        return Ok(());
    }
    if entry_bond == 0 {
        return Err(ChainBaseError::InvalidAmount);
    }
    let settlement_token = normalize_evm_address(settlement_token)?;
    let bounty_contract = normalize_evm_address(bounty_contract)?;
    let solver = normalize_evm_address(solver)?;
    let commitment = normalized_nonzero_bytes32(commitment, "commitment")?;
    plan.function = Some("commitSolution(bytes32)".to_string());
    plan.wallet_calls = vec![
        EvmTransactionIntent {
            from: Some(solver.clone()),
            to: settlement_token,
            value_wei: 0,
            data: encode_static_call(
                "approve(address,uint256)",
                &[
                    encode_address(&bounty_contract)?,
                    encode_uint256(entry_bond)?,
                ],
            ),
            function: "approve(address,uint256)".to_string(),
        },
        EvmTransactionIntent {
            from: Some(solver),
            to: bounty_contract,
            value_wei: 0,
            data: encode_static_call("commitSolution(bytes32)", &[parse_bytes32(&commitment)?]),
            function: "commitSolution(bytes32)".to_string(),
        },
    ];
    plan.supports_single_wallet_batch = true;
    Ok(())
}

pub fn attach_open_competition_reveal_call(
    plan: &mut OpenCompetitionActionPlan,
    bounty_contract: &str,
    solver: &str,
    submission_hash: &str,
    evidence_hash: &str,
    salt: &str,
    proof: &str,
) -> Result<(), ChainBaseError> {
    if !plan.allowed {
        return Ok(());
    }
    let bounty_contract = normalize_evm_address(bounty_contract)?;
    let solver = normalize_evm_address(solver)?;
    let submission_hash = normalized_nonzero_bytes32(submission_hash, "submission hash")?;
    let evidence_hash = normalized_nonzero_bytes32(evidence_hash, "evidence hash")?;
    let salt = normalized_nonzero_bytes32(salt, "salt")?;
    let proof = decode_prefixed_hex(proof, "proof")?;
    let mut bytes = selector("revealSolution(bytes32,bytes32,bytes32,bytes)").to_vec();
    bytes.extend_from_slice(&parse_bytes32(&submission_hash)?);
    bytes.extend_from_slice(&parse_bytes32(&evidence_hash)?);
    bytes.extend_from_slice(&parse_bytes32(&salt)?);
    bytes.extend_from_slice(&encode_uint256(4_u128 * 32)?);
    bytes.extend_from_slice(&encode_uint256(proof.len() as u128)?);
    bytes.extend_from_slice(&proof);
    let padding = (32 - proof.len() % 32) % 32;
    bytes.resize(bytes.len() + padding, 0);
    plan.function = Some("revealSolution(bytes32,bytes32,bytes32,bytes)".to_string());
    plan.wallet_calls = vec![EvmTransactionIntent {
        from: Some(solver),
        to: bounty_contract,
        value_wei: 0,
        data: format!("0x{}", hex::encode(bytes)),
        function: "revealSolution(bytes32,bytes32,bytes32,bytes)".to_string(),
    }];
    Ok(())
}

pub fn attach_open_competition_withdrawal_call(
    plan: &mut OpenCompetitionActionPlan,
    bounty_contract: &str,
    solver: &str,
) -> Result<(), ChainBaseError> {
    if !plan.allowed {
        return Ok(());
    }
    let bounty_contract = normalize_evm_address(bounty_contract)?;
    let solver = normalize_evm_address(solver)?;
    plan.function = Some("withdrawEntryBond()".to_string());
    plan.wallet_calls = vec![EvmTransactionIntent {
        from: Some(solver),
        to: bounty_contract,
        value_wei: 0,
        data: format!("0x{}", hex::encode(selector("withdrawEntryBond()"))),
        function: "withdrawEntryBond()".to_string(),
    }];
    Ok(())
}

fn decode_prefixed_hex(value: &str, label: &str) -> Result<Vec<u8>, ChainBaseError> {
    let encoded = value.strip_prefix("0x").ok_or_else(|| {
        ChainBaseError::InvalidVerificationConfiguration(format!(
            "open-competition {label} must be 0x-prefixed hex"
        ))
    })?;
    if encoded.len() % 2 != 0 {
        return Err(ChainBaseError::InvalidVerificationConfiguration(format!(
            "open-competition {label} must contain complete bytes"
        )));
    }
    hex::decode(encoded).map_err(|_| {
        ChainBaseError::InvalidVerificationConfiguration(format!(
            "open-competition {label} is not valid hex"
        ))
    })
}
