use super::{
    address_from_word, decode_words, deterministic_log_id, event_topic, log_key, normalize_address,
    normalize_topic, topic_u64, topic_word, word_hex, word_to_u128, word_to_u64, ChainBaseError,
    EvmLog,
};
use alloy::primitives::{I256, U256};
use chrono::{DateTime, Utc};
use domain::Id;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub const OPEN_COMPETITION_V2_PROTOCOL_VERSION: &str = "agent-bounties/open-competition-v2-beta2";
pub const OPEN_COMPETITION_V2_JOURNAL_SCHEMA: &str =
    "agent-bounties/open-competition-v2-beta2/journal";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenCompetitionV2EventKind {
    CanonicalCompetitionCreated,
    CanonicalCompetitionEconomics,
    CanonicalCompetitionVerification,
    CanonicalCompetitionPolicies,
    FundingAdded,
    CompetitionActivated,
    EntryQualified,
    LeaderUpdated,
    CompetitionSettled,
    CompetitionCancelled,
    RefundWithdrawn,
}

impl OpenCompetitionV2EventKind {
    pub fn is_payment_evidence(self) -> bool {
        matches!(self, Self::CompetitionSettled)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenCompetitionV2Event {
    pub id: Id,
    pub protocol_version: String,
    pub log_key: String,
    pub tx_hash: String,
    pub block_number: u64,
    pub log_index: u64,
    pub contract_address: String,
    pub bounty_id: String,
    pub kind: OpenCompetitionV2EventKind,
    pub data: Value,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventSignature {
    CanonicalCompetitionCreated,
    CanonicalCompetitionEconomics,
    CanonicalCompetitionVerification,
    CanonicalCompetitionPolicies,
    FundingAdded,
    CompetitionActivated,
    EntryQualified,
    LeaderUpdated,
    CompetitionSettled,
    CompetitionCancelled,
    RefundWithdrawn,
}

const EVENT_SIGNATURES: [(&str, EventSignature); 11] = [
    (
        "CanonicalCompetitionCreatedV2(bytes32,address,address,bytes32,bytes32)",
        EventSignature::CanonicalCompetitionCreated,
    ),
    (
        "CanonicalCompetitionEconomicsV2(bytes32,uint256,uint256,uint64,uint64,uint8,uint8,int256)",
        EventSignature::CanonicalCompetitionEconomics,
    ),
    (
        "CanonicalCompetitionVerificationV2(bytes32,bytes32,address,bytes32,bytes32,bytes32,bytes32,bytes32)",
        EventSignature::CanonicalCompetitionVerification,
    ),
    (
        "CanonicalCompetitionPoliciesV2(bytes32,bytes32,bytes32,bytes32)",
        EventSignature::CanonicalCompetitionPolicies,
    ),
    (
        "FundingAddedV2(bytes32,address,uint256,uint256,uint256)",
        EventSignature::FundingAdded,
    ),
    (
        "CompetitionActivatedV2(bytes32,uint64)",
        EventSignature::CompetitionActivated,
    ),
    (
        "CompetitionEntryQualifiedV2(bytes32,uint256,address,uint256,bytes32,bytes32,int256,address)",
        EventSignature::EntryQualified,
    ),
    (
        "CompetitionLeaderUpdatedV2(bytes32,uint256,address,int256)",
        EventSignature::LeaderUpdated,
    ),
    (
        "CompetitionSettledV2(bytes32,uint256,address,uint256,address,uint256,bytes32,bytes32,int256,bytes32)",
        EventSignature::CompetitionSettled,
    ),
    (
        "CompetitionCancelledV2(bytes32,address,uint256,uint256,uint256,bytes32)",
        EventSignature::CompetitionCancelled,
    ),
    (
        "CompetitionRefundWithdrawnV2(bytes32,address,address,uint256)",
        EventSignature::RefundWithdrawn,
    ),
];

pub fn open_competition_v2_event_topics() -> Vec<String> {
    EVENT_SIGNATURES
        .iter()
        .map(|(signature, _)| event_topic(signature))
        .collect()
}

pub fn decode_open_competition_v2_logs(
    logs: impl IntoIterator<Item = EvmLog>,
) -> Result<Vec<OpenCompetitionV2Event>, ChainBaseError> {
    let topics = open_competition_v2_event_topics()
        .into_iter()
        .collect::<HashSet<_>>();
    logs.into_iter()
        .filter(|log| {
            log.topics
                .first()
                .and_then(|topic| normalize_topic(topic).ok())
                .is_some_and(|topic| topics.contains(&topic))
        })
        .map(decode_open_competition_v2_log)
        .collect()
}

pub fn decode_open_competition_v2_log(
    log: EvmLog,
) -> Result<OpenCompetitionV2Event, ChainBaseError> {
    let topic0 = log
        .topics
        .first()
        .ok_or_else(|| ChainBaseError::InvalidLogTopics("Open Competition V2".to_string()))?;
    let normalized = normalize_topic(topic0)?;
    let signature = EVENT_SIGNATURES
        .iter()
        .find_map(|(candidate, kind)| (normalized == event_topic(candidate)).then_some(*kind))
        .ok_or_else(|| ChainBaseError::UnknownEventTopic(topic0.clone()))?;
    let (kind, bounty_id, data) = decode_event_payload(&log, signature)?;
    Ok(OpenCompetitionV2Event {
        id: deterministic_log_id(&log),
        protocol_version: OPEN_COMPETITION_V2_PROTOCOL_VERSION.to_string(),
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

fn decode_event_payload(
    log: &EvmLog,
    signature: EventSignature,
) -> Result<(OpenCompetitionV2EventKind, String, Value), ChainBaseError> {
    let bounty_id = |name: &str| topic_word(log, 1, name).map(word_hex);
    match signature {
        EventSignature::CanonicalCompetitionCreated => {
            let name = "CanonicalCompetitionCreatedV2";
            require_topics(log, 4, name)?;
            let words = decode_words(&log.data, 2, name)?;
            Ok((
                OpenCompetitionV2EventKind::CanonicalCompetitionCreated,
                word_hex(topic_word(log, 1, name)?),
                json!({
                    "competition": address_from_word(topic_word(log, 2, name)?),
                    "creator": address_from_word(topic_word(log, 3, name)?),
                    "creation_nonce": word_hex(words[0]),
                    "beta_risk_hash": word_hex(words[1]),
                }),
            ))
        }
        EventSignature::CanonicalCompetitionEconomics => {
            let name = "CanonicalCompetitionEconomicsV2";
            require_topics(log, 2, name)?;
            let words = decode_words(&log.data, 7, name)?;
            Ok((
                OpenCompetitionV2EventKind::CanonicalCompetitionEconomics,
                bounty_id(name)?,
                json!({
                    "solver_reward": word_to_u128(words[0])?,
                    "keeper_reward": word_to_u128(words[1])?,
                    "funding_deadline": word_to_u64(words[2], name)?,
                    "proof_window_seconds": word_to_u64(words[3], name)?,
                    "winner_mode": word_to_u128(words[4])?,
                    "score_direction": word_to_u128(words[5])?,
                    "score_threshold": signed_word(words[6]),
                }),
            ))
        }
        EventSignature::CanonicalCompetitionVerification => {
            let name = "CanonicalCompetitionVerificationV2";
            require_topics(log, 2, name)?;
            let words = decode_words(&log.data, 7, name)?;
            Ok((
                OpenCompetitionV2EventKind::CanonicalCompetitionVerification,
                bounty_id(name)?,
                json!({
                    "proof_system": word_hex(words[0]),
                    "verifier_adapter": address_from_word(words[1]),
                    "program_vkey": word_hex(words[2]),
                    "source_hash": word_hex(words[3]),
                    "elf_hash": word_hex(words[4]),
                    "journal_schema_hash": word_hex(words[5]),
                    "metric_program_hash": word_hex(words[6]),
                }),
            ))
        }
        EventSignature::CanonicalCompetitionPolicies => {
            let name = "CanonicalCompetitionPoliciesV2";
            require_topics(log, 2, name)?;
            let words = decode_words(&log.data, 3, name)?;
            Ok((
                OpenCompetitionV2EventKind::CanonicalCompetitionPolicies,
                bounty_id(name)?,
                json!({
                    "execution_policy_hash": word_hex(words[0]),
                    "verification_policy_hash": word_hex(words[1]),
                    "settlement_policy_hash": word_hex(words[2]),
                }),
            ))
        }
        EventSignature::FundingAdded => {
            let name = "FundingAddedV2";
            require_topics(log, 3, name)?;
            let words = decode_words(&log.data, 3, name)?;
            Ok((
                OpenCompetitionV2EventKind::FundingAdded,
                bounty_id(name)?,
                json!({
                    "contributor": address_from_word(topic_word(log, 2, name)?),
                    "amount": word_to_u128(words[0])?,
                    "funded_amount": word_to_u128(words[1])?,
                    "target_amount": word_to_u128(words[2])?,
                }),
            ))
        }
        EventSignature::CompetitionActivated => {
            let name = "CompetitionActivatedV2";
            require_topics(log, 2, name)?;
            let words = decode_words(&log.data, 1, name)?;
            Ok((
                OpenCompetitionV2EventKind::CompetitionActivated,
                bounty_id(name)?,
                json!({ "proof_deadline": word_to_u64(words[0], name)? }),
            ))
        }
        EventSignature::EntryQualified => {
            let name = "CompetitionEntryQualifiedV2";
            require_topics(log, 4, name)?;
            let words = decode_words(&log.data, 5, name)?;
            Ok((
                OpenCompetitionV2EventKind::EntryQualified,
                bounty_id(name)?,
                json!({
                    "sequence": topic_u64(log, 2, name)?,
                    "solver": address_from_word(topic_word(log, 3, name)?),
                    "solver_nonce": word_to_u128(words[0])?,
                    "submission_hash": word_hex(words[1]),
                    "evidence_hash": word_hex(words[2]),
                    "score": signed_word(words[3]),
                    "proof_submitter": address_from_word(words[4]),
                }),
            ))
        }
        EventSignature::LeaderUpdated => {
            let name = "CompetitionLeaderUpdatedV2";
            require_topics(log, 4, name)?;
            let words = decode_words(&log.data, 1, name)?;
            Ok((
                OpenCompetitionV2EventKind::LeaderUpdated,
                bounty_id(name)?,
                json!({
                    "sequence": topic_u64(log, 2, name)?,
                    "solver": address_from_word(topic_word(log, 3, name)?),
                    "score": signed_word(words[0]),
                }),
            ))
        }
        EventSignature::CompetitionSettled => {
            let name = "CompetitionSettledV2";
            require_topics(log, 4, name)?;
            let words = decode_words(&log.data, 7, name)?;
            Ok((
                OpenCompetitionV2EventKind::CompetitionSettled,
                bounty_id(name)?,
                json!({
                    "winning_sequence": topic_u64(log, 2, name)?,
                    "solver": address_from_word(topic_word(log, 3, name)?),
                    "solver_reward": word_to_u128(words[0])?,
                    "keeper": address_from_word(words[1]),
                    "keeper_reward": word_to_u128(words[2])?,
                    "submission_hash": word_hex(words[3]),
                    "evidence_hash": word_hex(words[4]),
                    "score": signed_word(words[5]),
                    "settlement_policy_hash": word_hex(words[6]),
                }),
            ))
        }
        EventSignature::CompetitionCancelled => {
            let name = "CompetitionCancelledV2";
            require_topics(log, 3, name)?;
            let words = decode_words(&log.data, 4, name)?;
            Ok((
                OpenCompetitionV2EventKind::CompetitionCancelled,
                bounty_id(name)?,
                json!({
                    "transition_caller": address_from_word(topic_word(log, 2, name)?),
                    "refund_pool": word_to_u128(words[0])?,
                    "contribution_weight": word_to_u128(words[1])?,
                    "keeper_paid": word_to_u128(words[2])?,
                    "reason": word_hex(words[3]),
                }),
            ))
        }
        EventSignature::RefundWithdrawn => {
            let name = "CompetitionRefundWithdrawnV2";
            require_topics(log, 4, name)?;
            let words = decode_words(&log.data, 1, name)?;
            Ok((
                OpenCompetitionV2EventKind::RefundWithdrawn,
                bounty_id(name)?,
                json!({
                    "contributor": address_from_word(topic_word(log, 2, name)?),
                    "caller": address_from_word(topic_word(log, 3, name)?),
                    "amount": word_to_u128(words[0])?,
                }),
            ))
        }
    }
}

fn require_topics(log: &EvmLog, expected: usize, name: &str) -> Result<(), ChainBaseError> {
    if log.topics.len() != expected {
        return Err(ChainBaseError::InvalidLogTopics(name.to_string()));
    }
    Ok(())
}

fn signed_word(word: [u8; 32]) -> String {
    I256::from_raw(U256::from_be_bytes(word)).to_string()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenCompetitionV2ProjectedState {
    #[default]
    Announced,
    Funding,
    Active,
    Settled,
    Cancelled,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionV2Projection {
    pub bounty_id: String,
    pub competition: String,
    pub creator: String,
    pub creation_nonce: Option<String>,
    pub beta_risk_hash: Option<String>,
    pub state: OpenCompetitionV2ProjectedState,
    pub solver_reward: u128,
    pub keeper_reward: u128,
    pub funding_deadline: Option<u64>,
    pub proof_window_seconds: Option<u64>,
    pub winner_mode: Option<String>,
    pub score_direction: Option<String>,
    pub score_threshold: Option<String>,
    pub proof_system: Option<String>,
    pub verifier_adapter: Option<String>,
    pub program_vkey: Option<String>,
    pub source_hash: Option<String>,
    pub elf_hash: Option<String>,
    pub journal_schema_hash: Option<String>,
    pub metric_program_hash: Option<String>,
    pub execution_policy_hash: Option<String>,
    pub verification_policy_hash: Option<String>,
    pub settlement_policy_hash: Option<String>,
    pub funded_amount: u128,
    pub proof_deadline: Option<u64>,
    pub accepted_entries: u64,
    pub leader: Option<String>,
    pub winner: Option<String>,
    pub refund_pool_remaining: u128,
    pub last_block: u64,
    pub last_log_index: u64,
}

/// Applies safe-block events deterministically. Duplicate log identities are
/// ignored, allowing the same safe range to be replayed after a worker retry.
pub fn project_open_competition_v2(
    events: impl IntoIterator<Item = OpenCompetitionV2Event>,
) -> Result<HashMap<String, OpenCompetitionV2Projection>, ChainBaseError> {
    let mut events = events.into_iter().collect::<Vec<_>>();
    events.sort_by_key(|event| (event.block_number, event.log_index));
    let mut seen = HashSet::new();
    let mut projections = HashMap::<String, OpenCompetitionV2Projection>::new();
    for event in events {
        if !seen.insert(event.log_key.clone()) {
            continue;
        }
        let projection = projections
            .entry(event.bounty_id.clone())
            .or_insert_with(|| OpenCompetitionV2Projection {
                bounty_id: event.bounty_id.clone(),
                ..Default::default()
            });
        apply_event(projection, &event)?;
        projection.last_block = event.block_number;
        projection.last_log_index = event.log_index;
    }
    Ok(projections)
}

fn apply_event(
    projection: &mut OpenCompetitionV2Projection,
    event: &OpenCompetitionV2Event,
) -> Result<(), ChainBaseError> {
    let invalid = || ChainBaseError::InvalidLogData("Open Competition V2 projection".to_string());
    match event.kind {
        OpenCompetitionV2EventKind::CanonicalCompetitionCreated => {
            projection.competition = json_string(&event.data, "competition")?;
            projection.creator = json_string(&event.data, "creator")?;
            projection.creation_nonce = Some(json_string(&event.data, "creation_nonce")?);
            projection.beta_risk_hash = Some(json_string(&event.data, "beta_risk_hash")?);
            projection.state = OpenCompetitionV2ProjectedState::Funding;
        }
        OpenCompetitionV2EventKind::CanonicalCompetitionEconomics => {
            projection.solver_reward = json_u128(&event.data, "solver_reward")?;
            projection.keeper_reward = json_u128(&event.data, "keeper_reward")?;
            projection.funding_deadline = Some(json_u64(&event.data, "funding_deadline")?);
            projection.proof_window_seconds = Some(json_u64(&event.data, "proof_window_seconds")?);
            projection.winner_mode = Some(match json_u64(&event.data, "winner_mode")? {
                0 => "first_proven".to_string(),
                1 => "best_score".to_string(),
                _ => return Err(invalid()),
            });
            projection.score_direction = Some(match json_u64(&event.data, "score_direction")? {
                0 => "higher_is_better".to_string(),
                1 => "lower_is_better".to_string(),
                _ => return Err(invalid()),
            });
            projection.score_threshold = Some(json_string(&event.data, "score_threshold")?);
        }
        OpenCompetitionV2EventKind::CanonicalCompetitionVerification => {
            let proof_system = json_string(&event.data, "proof_system")?;
            projection.proof_system = Some(
                if proof_system == format!("{:#x}", alloy::primitives::keccak256("sp1-groth16")) {
                    "groth16".to_string()
                } else if proof_system
                    == format!("{:#x}", alloy::primitives::keccak256("sp1-plonk"))
                {
                    "plonk".to_string()
                } else {
                    return Err(invalid());
                },
            );
            projection.verifier_adapter = Some(json_string(&event.data, "verifier_adapter")?);
            projection.program_vkey = Some(json_string(&event.data, "program_vkey")?);
            projection.source_hash = Some(json_string(&event.data, "source_hash")?);
            projection.elf_hash = Some(json_string(&event.data, "elf_hash")?);
            projection.journal_schema_hash = Some(json_string(&event.data, "journal_schema_hash")?);
            projection.metric_program_hash = Some(json_string(&event.data, "metric_program_hash")?);
        }
        OpenCompetitionV2EventKind::FundingAdded => {
            if projection.state != OpenCompetitionV2ProjectedState::Funding {
                return Err(invalid());
            }
            projection.funded_amount = json_u128(&event.data, "funded_amount")?;
        }
        OpenCompetitionV2EventKind::CompetitionActivated => {
            if projection.state != OpenCompetitionV2ProjectedState::Funding {
                return Err(invalid());
            }
            let target = projection
                .solver_reward
                .checked_add(projection.keeper_reward)
                .ok_or_else(invalid)?;
            if projection.funded_amount != target {
                return Err(invalid());
            }
            projection.state = OpenCompetitionV2ProjectedState::Active;
            projection.proof_deadline = Some(json_u64(&event.data, "proof_deadline")?);
        }
        OpenCompetitionV2EventKind::EntryQualified => {
            if projection.state != OpenCompetitionV2ProjectedState::Active {
                return Err(invalid());
            }
            projection.accepted_entries = json_u64(&event.data, "sequence")?;
        }
        OpenCompetitionV2EventKind::LeaderUpdated => {
            if projection.state != OpenCompetitionV2ProjectedState::Active {
                return Err(invalid());
            }
            projection.leader = Some(json_string(&event.data, "solver")?);
        }
        OpenCompetitionV2EventKind::CompetitionSettled => {
            if projection.state != OpenCompetitionV2ProjectedState::Active {
                return Err(invalid());
            }
            projection.state = OpenCompetitionV2ProjectedState::Settled;
            projection.funded_amount = 0;
            projection.winner = Some(json_string(&event.data, "solver")?);
        }
        OpenCompetitionV2EventKind::CompetitionCancelled => {
            if !matches!(
                projection.state,
                OpenCompetitionV2ProjectedState::Funding | OpenCompetitionV2ProjectedState::Active
            ) {
                return Err(invalid());
            }
            projection.state = OpenCompetitionV2ProjectedState::Cancelled;
            projection.funded_amount = 0;
            projection.refund_pool_remaining = json_u128(&event.data, "refund_pool")?;
        }
        OpenCompetitionV2EventKind::RefundWithdrawn => {
            if projection.state != OpenCompetitionV2ProjectedState::Cancelled {
                return Err(invalid());
            }
            projection.refund_pool_remaining = projection
                .refund_pool_remaining
                .checked_sub(json_u128(&event.data, "amount")?)
                .ok_or_else(invalid)?;
        }
        OpenCompetitionV2EventKind::CanonicalCompetitionPolicies => {
            projection.execution_policy_hash =
                Some(json_string(&event.data, "execution_policy_hash")?);
            projection.verification_policy_hash =
                Some(json_string(&event.data, "verification_policy_hash")?);
            projection.settlement_policy_hash =
                Some(json_string(&event.data, "settlement_policy_hash")?);
        }
    }
    Ok(())
}

fn json_string(value: &Value, field: &str) -> Result<String, ChainBaseError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ChainBaseError::InvalidLogData(format!("Open Competition V2 {field}")))
}

fn json_u128(value: &Value, field: &str) -> Result<u128, ChainBaseError> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .map(u128::from)
        .or_else(|| {
            value
                .get(field)
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<u128>().ok())
        })
        .ok_or_else(|| ChainBaseError::InvalidLogData(format!("Open Competition V2 {field}")))
}

fn json_u64(value: &Value, field: &str) -> Result<u64, ChainBaseError> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| ChainBaseError::InvalidLogData(format!("Open Competition V2 {field}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        primitives::{Address, B256},
        sol_types::SolValue,
    };

    fn topic_address(byte: u8) -> String {
        format!("0x{}{}", "00".repeat(12), hex::encode([byte; 20]))
    }

    fn topic_u256(value: u64) -> String {
        format!("0x{value:064x}")
    }

    fn log(signature: &str, topics: Vec<String>, data: Vec<u8>, index: u64) -> EvmLog {
        let mut all_topics = vec![event_topic(signature)];
        all_topics.extend(topics);
        EvmLog {
            address: "0x9999999999999999999999999999999999999999".to_string(),
            topics: all_topics,
            data: format!("0x{}", hex::encode(data)),
            tx_hash: format!("0x{index:064x}"),
            block_number: 100 + index,
            log_index: index,
            occurred_at: Some(Utc::now()),
        }
    }

    #[test]
    fn settled_is_the_only_payment_evidence() {
        for kind in [
            OpenCompetitionV2EventKind::FundingAdded,
            OpenCompetitionV2EventKind::EntryQualified,
            OpenCompetitionV2EventKind::LeaderUpdated,
            OpenCompetitionV2EventKind::CompetitionCancelled,
        ] {
            assert!(!kind.is_payment_evidence());
        }
        assert!(OpenCompetitionV2EventKind::CompetitionSettled.is_payment_evidence());
    }

    #[test]
    fn decodes_settlement_with_signed_score() {
        let bounty = format!("0x{}", "11".repeat(32));
        let data = (
            U256::from(1_000_000_u64),
            Address::from([0x33; 20]),
            U256::from(50_000_u64),
            B256::from([0x44; 32]),
            B256::from([0x55; 32]),
            I256::try_from(-7_i64).unwrap(),
            B256::from([0x66; 32]),
        )
            .abi_encode();
        let decoded = decode_open_competition_v2_log(log(
            "CompetitionSettledV2(bytes32,uint256,address,uint256,address,uint256,bytes32,bytes32,int256,bytes32)",
            vec![bounty, topic_u256(2), topic_address(0x22)],
            data,
            4,
        ))
        .unwrap();
        assert_eq!(decoded.kind, OpenCompetitionV2EventKind::CompetitionSettled);
        assert_eq!(decoded.data["score"], "-7");
        assert_eq!(decoded.data["solver_reward"], 1_000_000_u64);
    }

    #[test]
    fn projection_is_replay_safe_and_rejects_underfunded_activation() {
        let bounty = format!("0x{}", "11".repeat(32));
        let created = decode_open_competition_v2_log(log(
            "CanonicalCompetitionCreatedV2(bytes32,address,address,bytes32,bytes32)",
            vec![bounty.clone(), topic_address(0x22), topic_address(0x33)],
            (B256::from([0x44; 32]), B256::from([0x55; 32])).abi_encode(),
            1,
        ))
        .unwrap();
        let economics = decode_open_competition_v2_log(log(
            "CanonicalCompetitionEconomicsV2(bytes32,uint256,uint256,uint64,uint64,uint8,uint8,int256)",
            vec![bounty.clone()],
            (
                U256::from(1_000_u64),
                U256::from(50_u64),
                1_000_u64,
                500_u64,
                U256::ZERO,
                U256::ZERO,
                I256::ZERO,
            )
                .abi_encode(),
            2,
        ))
        .unwrap();
        let funding = decode_open_competition_v2_log(log(
            "FundingAddedV2(bytes32,address,uint256,uint256,uint256)",
            vec![bounty.clone(), topic_address(0x33)],
            (
                U256::from(1_050_u64),
                U256::from(1_050_u64),
                U256::from(1_050_u64),
            )
                .abi_encode(),
            3,
        ))
        .unwrap();
        let activated = decode_open_competition_v2_log(log(
            "CompetitionActivatedV2(bytes32,uint64)",
            vec![bounty.clone()],
            2_000_u64.abi_encode(),
            4,
        ))
        .unwrap();

        let projected = project_open_competition_v2(vec![
            created.clone(),
            economics,
            funding,
            activated,
            created,
        ])
        .unwrap();
        let state = projected.get(&bounty).unwrap();
        assert_eq!(state.state, OpenCompetitionV2ProjectedState::Active);
        assert_eq!(state.funded_amount, 1_050);
        assert_eq!(state.proof_deadline, Some(2_000));
    }
}
