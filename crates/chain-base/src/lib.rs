use alloy::{
    network::TransactionBuilder,
    primitives::{Address, Bytes, B256, U256},
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
    signers::{local::PrivateKeySigner, SignerSync},
};
use chrono::{DateTime, Utc};
use domain::{
    AutonomousBountyTermsDocument, AutonomousBountyTermsRecord, AutonomousSubmissionEvidenceRecord,
    Id, Money,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;
use sha3::{Digest, Keccak256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    env,
};
use thiserror::Error;
use uuid::Uuid;
use verifier_sdk::RegressionSandboxPolicy;

mod agent_wallet_readiness;

pub use agent_wallet_readiness::*;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChainBaseError {
    #[error("duplicate chain log")]
    DuplicateLog,
    #[error("invalid escrow release split")]
    InvalidReleaseSplit,
    #[error("invalid EVM address: {0}")]
    InvalidAddress(String),
    #[error("invalid bytes32 hex value: {0}")]
    InvalidBytes32(String),
    #[error("invalid on-chain escrow id")]
    InvalidEscrowId,
    #[error("invalid on-chain amount")]
    InvalidAmount,
    #[error("autonomous bounties settle in USDC")]
    InvalidSettlementCurrency,
    #[error("initial bounty funding exceeds the solver and verifier reward target")]
    InitialFundingExceedsTarget,
    #[error("invalid autonomous bounty verification configuration: {0}")]
    InvalidVerificationConfiguration(String),
    #[error("invalid canonical JSON commitment: {0}")]
    InvalidCanonicalJson(String),
    #[error("invalid autonomous bounty terms document: {0}")]
    InvalidTermsDocument(String),
    #[error("invalid autonomous verification attestation scope: {0}")]
    InvalidAttestationScope(String),
    #[error("invalid autonomous submission evidence: {0}")]
    InvalidSubmissionEvidence(String),
    #[error("invalid autonomous submission preparation: {0}")]
    InvalidSubmissionPreparation(String),
    #[error("autonomous bounty terms document exceeds 256 KiB")]
    TermsDocumentTooLarge,
    #[error("release recipients must be non-empty")]
    EmptyRecipients,
    #[error("release recipients must use a single currency")]
    MixedRecipientCurrencies,
    #[error("unknown escrow event topic: {0}")]
    UnknownEventTopic(String),
    #[error("invalid EVM log topics for {0}")]
    InvalidLogTopics(String),
    #[error("invalid EVM log data for {0}")]
    InvalidLogData(String),
    #[error("terminal escrow log arrived before created log")]
    UnknownEscrowForTerminalLog,
    #[error("invalid block range: from {from_block} is greater than to {to_block}")]
    InvalidBlockRange { from_block: u64, to_block: u64 },
    #[error("invalid EVM RPC quantity: {0}")]
    InvalidRpcQuantity(String),
    #[error("invalid signed EVM transaction: {0}")]
    InvalidSignedTransaction(String),
    #[error("invalid hex bytes: {0}")]
    InvalidHexBytes(String),
    #[error("invalid EVM transaction hash: {0}")]
    InvalidTransactionHash(String),
    #[error("unknown Base network: {0}")]
    UnknownNetwork(String),
    #[error("missing RPC URL for {network}; set {env_var}")]
    MissingRpcUrl { network: String, env_var: String },
    #[error("Base RPC transport error: {0}")]
    RpcTransport(String),
    #[error("Base RPC returned HTTP status {0}")]
    RpcHttpStatus(u16),
    #[error("Base RPC provider error {code}: {message}")]
    RpcProviderError { code: i64, message: String },
    #[error("invalid Base RPC response: {0}")]
    InvalidRpcResponse(String),
    #[error("invalid Base relayer private key")]
    InvalidRelayerPrivateKey,
    #[error("invalid bounded Base relay intent: {0}")]
    InvalidRelayIntent(String),
    #[error("Base relayer connected to chain {observed}; expected {expected}")]
    RelayerChainMismatch { expected: u64, observed: u64 },
    #[error("Base relay gas estimate {estimated} exceeds cap {maximum}")]
    RelayerGasLimitExceeded { estimated: u64, maximum: u64 },
    #[error("Base relay max fee per gas {estimated} exceeds cap {maximum}")]
    RelayerFeeCapExceeded { estimated: u128, maximum: u128 },
    #[error("Base relayer balance {balance} is below bounded transaction cost {required}")]
    RelayerInsufficientBalance { balance: u128, required: u128 },
    #[error("Base relayer provider error: {0}")]
    RelayerProvider(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmTransactionIntent {
    pub from: Option<String>,
    pub to: String,
    pub value_wei: u128,
    pub data: String,
    pub function: String,
}

#[derive(Clone)]
pub struct BaseTransactionRelayer {
    signer: PrivateKeySigner,
}

impl std::fmt::Debug for BaseTransactionRelayer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BaseTransactionRelayer")
            .field("address", &self.address())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaseRelayedTransaction {
    pub relayer: String,
    pub tx_hash: String,
    pub estimated_gas: u64,
    pub gas_limit: u64,
    pub max_fee_per_gas_wei: u128,
    pub max_priority_fee_per_gas_wei: u128,
    pub estimated_max_cost_wei: u128,
}

impl BaseTransactionRelayer {
    pub fn from_private_key(private_key: &str) -> Result<Self, ChainBaseError> {
        let signer = private_key
            .trim()
            .parse::<PrivateKeySigner>()
            .map_err(|_| ChainBaseError::InvalidRelayerPrivateKey)?;
        Ok(Self { signer })
    }

    pub fn address(&self) -> String {
        format!("{:#x}", self.signer.address())
    }

    pub fn sign_digest(&self, digest: &str) -> Result<String, ChainBaseError> {
        let digest = B256::from(parse_bytes32(digest)?);
        let signature = self
            .signer
            .sign_hash_sync(&digest)
            .map_err(|_| ChainBaseError::InvalidRelayIntent("digest signing failed".to_string()))?;
        Ok(format!("0x{}", hex::encode(signature.as_bytes())))
    }

    pub async fn simulate_and_broadcast(
        &self,
        rpc_url: &str,
        expected_chain_id: u64,
        intent: &EvmTransactionIntent,
        max_gas: u64,
        max_fee_per_gas_wei: u128,
    ) -> Result<BaseRelayedTransaction, ChainBaseError> {
        if max_gas == 0 || max_fee_per_gas_wei == 0 {
            return Err(ChainBaseError::InvalidRelayIntent(
                "gas and fee caps must be positive".to_string(),
            ));
        }
        if intent.value_wei != 0 {
            return Err(ChainBaseError::InvalidRelayIntent(
                "hosted relays cannot transfer ETH value".to_string(),
            ));
        }
        let relayer = self.signer.address();
        if let Some(from) = intent.from.as_deref() {
            let expected = parse_alloy_address(from)?;
            if expected != relayer {
                return Err(ChainBaseError::InvalidRelayIntent(
                    "transaction sender does not match the configured relayer".to_string(),
                ));
            }
        }
        let to = parse_alloy_address(&intent.to)?;
        let data = parse_alloy_bytes(&intent.data)?;
        if data.len() < 4 {
            return Err(ChainBaseError::InvalidRelayIntent(
                "transaction calldata is missing a function selector".to_string(),
            ));
        }
        let rpc_url = rpc_url.parse().map_err(|_| {
            ChainBaseError::RelayerProvider("configured RPC URL is invalid".to_string())
        })?;
        let provider = ProviderBuilder::new()
            .wallet(self.signer.clone())
            .connect_http(rpc_url);
        let observed_chain_id = provider
            .get_chain_id()
            .await
            .map_err(sanitize_relayer_provider_error)?;
        if observed_chain_id != expected_chain_id {
            return Err(ChainBaseError::RelayerChainMismatch {
                expected: expected_chain_id,
                observed: observed_chain_id,
            });
        }

        let transaction = TransactionRequest::default()
            .with_from(relayer)
            .with_to(to)
            .with_value(U256::ZERO)
            .with_input(data);
        provider
            .call(transaction.clone())
            .await
            .map_err(sanitize_relayer_provider_error)?;
        let estimated_gas = provider
            .estimate_gas(transaction.clone())
            .await
            .map_err(sanitize_relayer_provider_error)?;
        let gas_limit = estimated_gas
            .checked_mul(120)
            .and_then(|value| value.checked_add(99))
            .map(|value| value / 100)
            .ok_or_else(|| {
                ChainBaseError::InvalidRelayIntent("gas estimate overflow".to_string())
            })?;
        if gas_limit > max_gas {
            return Err(ChainBaseError::RelayerGasLimitExceeded {
                estimated: gas_limit,
                maximum: max_gas,
            });
        }
        let fees = provider
            .estimate_eip1559_fees()
            .await
            .map_err(sanitize_relayer_provider_error)?;
        if fees.max_fee_per_gas > max_fee_per_gas_wei {
            return Err(ChainBaseError::RelayerFeeCapExceeded {
                estimated: fees.max_fee_per_gas,
                maximum: max_fee_per_gas_wei,
            });
        }
        let estimated_max_cost_wei = u128::from(gas_limit)
            .checked_mul(fees.max_fee_per_gas)
            .ok_or_else(|| {
                ChainBaseError::InvalidRelayIntent("maximum gas cost overflow".to_string())
            })?;
        let balance = provider
            .get_balance(relayer)
            .await
            .map_err(sanitize_relayer_provider_error)?;
        let balance = u128::try_from(balance).map_err(|_| {
            ChainBaseError::InvalidRelayIntent("relayer balance exceeds u128".to_string())
        })?;
        if balance < estimated_max_cost_wei {
            return Err(ChainBaseError::RelayerInsufficientBalance {
                balance,
                required: estimated_max_cost_wei,
            });
        }

        let transaction = transaction
            .with_gas_limit(gas_limit)
            .with_max_fee_per_gas(fees.max_fee_per_gas)
            .with_max_priority_fee_per_gas(fees.max_priority_fee_per_gas);
        let pending = provider
            .send_transaction(transaction)
            .await
            .map_err(sanitize_relayer_provider_error)?;
        Ok(BaseRelayedTransaction {
            relayer: format!("{relayer:#x}"),
            tx_hash: format!("{:#x}", pending.tx_hash()),
            estimated_gas,
            gas_limit,
            max_fee_per_gas_wei: fees.max_fee_per_gas,
            max_priority_fee_per_gas_wei: fees.max_priority_fee_per_gas,
            estimated_max_cost_wei,
        })
    }
}

fn parse_alloy_address(value: &str) -> Result<Address, ChainBaseError> {
    value
        .parse::<Address>()
        .map_err(|_| ChainBaseError::InvalidAddress(value.to_string()))
}

fn parse_alloy_bytes(value: &str) -> Result<Bytes, ChainBaseError> {
    let raw = value.strip_prefix("0x").ok_or_else(|| {
        ChainBaseError::InvalidRelayIntent("calldata must be 0x-prefixed".to_string())
    })?;
    let decoded = hex::decode(raw).map_err(|_| {
        ChainBaseError::InvalidRelayIntent("calldata must be valid hex".to_string())
    })?;
    Ok(Bytes::from(decoded))
}

fn sanitize_relayer_provider_error(error: impl std::fmt::Display) -> ChainBaseError {
    let message = redact_relayer_provider_urls(&error.to_string());
    let first_line = message.lines().next().unwrap_or("provider request failed");
    let bounded = first_line.chars().take(300).collect::<String>();
    ChainBaseError::RelayerProvider(bounded)
}

fn redact_relayer_provider_urls(message: &str) -> String {
    let mut redacted = String::with_capacity(message.len());
    let mut index = 0;
    while index < message.len() {
        let remaining = &message[index..];
        let scheme_len = if remaining.starts_with("https://") {
            Some(8)
        } else if remaining.starts_with("http://") {
            Some(7)
        } else if remaining.starts_with("wss://") {
            Some(6)
        } else if remaining.starts_with("ws://") {
            Some(5)
        } else {
            None
        };
        if let Some(scheme_len) = scheme_len {
            redacted.push_str("[redacted-url]");
            index += scheme_len;
            while index < message.len() {
                let character = message[index..]
                    .chars()
                    .next()
                    .expect("index remains on a character boundary");
                if character.is_whitespace()
                    || matches!(character, '"' | '\'' | ')' | ']' | '}' | ',' | ';')
                {
                    break;
                }
                index += character.len_utf8();
            }
            continue;
        }
        let character = remaining
            .chars()
            .next()
            .expect("index remains below message length");
        redacted.push(character);
        index += character.len_utf8();
    }
    redacted
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutonomousVerificationMode {
    DeterministicModule,
    SignedQuorum,
    AiJudgeQuorum,
}

impl AutonomousVerificationMode {
    fn word(self) -> Result<[u8; 32], ChainBaseError> {
        encode_uint256(match self {
            Self::DeterministicModule => 0,
            Self::SignedQuorum => 1,
            Self::AiJudgeQuorum => 2,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousBountyCreate {
    pub creator: String,
    pub solver_reward: Money,
    pub verifier_reward: Money,
    pub terms_hash: String,
    pub policy_hash: String,
    pub acceptance_criteria_hash: String,
    pub benchmark_hash: String,
    pub evidence_schema_hash: String,
    pub funding_deadline: u64,
    pub claim_window_seconds: u64,
    pub verification_window_seconds: u64,
    pub verification_mode: AutonomousVerificationMode,
    pub verifier_module: Option<String>,
    pub verifier_reward_recipient: Option<String>,
    #[serde(default)]
    pub verifiers: Vec<String>,
    pub threshold: u8,
    pub initial_funding: Money,
    pub creation_nonce: String,
}

pub const CANONICAL_CHILD_PROTOCOL_VERSION: &str = "agent-bounties/canonical-child-v1";
pub const STANDING_META_V2_PROTOCOL_VERSION: &str = "agent-bounties/independent-child-v2";
pub const STANDING_META_V2_REGRESSION_ENGINE: &str = "sandboxed_regression_v1";
pub const BASE_MAINNET_STANDING_META_V2_VERIFIER: &str =
    "0xe573cb4f471d38b5bf10ce82237251ac902c9867";
pub const BASE_MAINNET_AUTONOMOUS_BOUNTY_FACTORY: &str =
    "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9";
pub const BASE_MAINNET_AUTONOMOUS_BOUNTY_IMPLEMENTATION: &str =
    "0x2fa36d2b2327642db3a6cc8cdd91544ad7484eb9";
pub const BASE_MAINNET_STANDING_META_V2_TERMS_REGISTRY: &str =
    "0x35e5d49c12b75c119d33951c2c4f054c5732208c";
pub const BASE_MAINNET_STANDING_META_V2_PARTICIPANT_REGISTRY: &str =
    "0x9875dcaf570bde8ff1aa62275d3c8985f4fd1294";
pub const BASE_MAINNET_STANDING_META_V2_ACCEPTANCE_CRITERIA_HASH: &str =
    "0x25c41d7d51e2c807754b901733de17cdb1778dbd353f86347ff33e10289fcb54";
pub const BASE_MAINNET_STANDING_META_V2_VERIFIER_SET_HASH: &str =
    "0x2c5a10915ca1fb99d4a11e2222b4f32b986b4e0f5599f55d70e9c8f9725a28cd";
pub const BASE_MAINNET_STANDING_META_V2_VERIFIERS: [&str; 2] = [
    "0xbe6292b9e465f549e2363b918d6dd9187038431e",
    "0xb7c2ce6430b66fb986e27b6140b29309550d487a",
];
pub const STANDING_META_V2_DEFAULT_VERIFIER_REWARD: i64 = 100_000;
pub const STANDING_META_V2_DEFAULT_WORK_WINDOW_SECONDS: u64 = 3 * 24 * 60 * 60;
pub const STANDING_META_V2_MAX_ONCHAIN_TERMS_BYTES: usize = 32_768;
pub const AUTONOMOUS_FUND_WITH_AUTHORIZATION_FUNCTION: &str =
    "fundWithAuthorization(address,uint256,uint256,uint256,bytes32,uint8,bytes32,bytes32)";
pub const AUTONOMOUS_FUND_WITH_AUTHORIZATION_SELECTOR: &str = "e1c9e96f";
pub const CANONICAL_CHILD_ACCEPTANCE_CRITERIA: [&str; 4] = [
    "Post a canonical autonomous-v1 child bounty whose creator is the active solver.",
    "Fully fund the child to at least the parent solver reward; pooled contributors are allowed.",
    "Bind the child benchmark to the parent bounty ID and round and use an explicit deterministic verifier.",
    "Have a different wallet complete the child and receive canonical settlement before the parent verification deadline.",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalChildBountyTermsRequest {
    pub parent_bounty_id: String,
    pub parent_round: u64,
    pub parent_solver: String,
    pub parent_solver_reward: Money,
    pub child_acceptance_criteria: Vec<String>,
    pub verifier_module: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalChildBountyTermsPlan {
    pub protocol_version: String,
    pub parent_bounty_id: String,
    pub parent_round: u64,
    pub required_creator: String,
    pub minimum_child_target: Money,
    pub acceptance_criteria: Vec<String>,
    pub acceptance_criteria_hash: String,
    pub benchmark: Value,
    pub benchmark_hash: String,
    pub verification_mode: AutonomousVerificationMode,
    pub verifier_module: String,
    pub threshold: u8,
    pub required_child_status: String,
    pub proof_encoding: String,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StandingMetaV2BenchmarkSource {
    pub kind: String,
    pub repository: String,
    pub commit: String,
    pub subdirectory: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StandingMetaV2ChildPreparationRequest {
    pub network: Option<String>,
    pub parent_bounty_contract: String,
    pub parent_solver: String,
    pub intended_child_solver: String,
    pub title: String,
    pub goal: String,
    pub acceptance_criteria: Vec<String>,
    pub benchmark_source: StandingMetaV2BenchmarkSource,
    pub runner_manifest: RegressionSandboxPolicy,
    pub evidence_schema: Option<Value>,
    pub verifier_reward: Option<Money>,
    pub funding_deadline: Option<u64>,
    pub claim_window_seconds: Option<u64>,
    pub verification_window_seconds: Option<u64>,
    pub creation_nonce: Option<String>,
    pub nonce_salt: Option<String>,
    pub source_url: Option<String>,
    pub discovery_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StandingMetaV2ParentContext {
    pub bounty_contract: String,
    pub bounty_id: String,
    pub creator: String,
    pub round: u64,
    pub solver_reward: Money,
    pub funding_deadline: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StandingMetaV2ParticipantPreconditions {
    pub registry: String,
    pub parent_solver: String,
    pub intended_child_solver: String,
    pub required_before_parent_claim: bool,
    pub distinct_participant_ids_required: bool,
    pub evidence_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StandingMetaV2ParentClaimTiming {
    pub terms_must_predate_parent_claim: bool,
    pub participant_registrations_must_predate_parent_claim: bool,
    pub strict_timestamp_ordering: bool,
    pub same_block_claim_allowed: bool,
    pub evidence_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StandingMetaV2ChildPreparationPlan {
    pub protocol_version: String,
    pub network: BaseNetworkDescriptor,
    pub parent_bounty_contract: String,
    pub parent_bounty_id: String,
    pub parent_round: u64,
    pub parent_solver: String,
    pub intended_child_solver: String,
    pub participant_preconditions: StandingMetaV2ParticipantPreconditions,
    pub parent_claim_timing: StandingMetaV2ParentClaimTiming,
    pub terms_registry: String,
    pub task_verifiers: Vec<String>,
    pub task_verifier_set_hash: String,
    pub task_verifier_threshold: u8,
    pub terms: AutonomousBountyTermsRecord,
    pub canonical_terms_json: String,
    pub canonical_terms_hex: String,
    pub hosted_terms_published: bool,
    pub publish_terms: EvmTransactionIntent,
    pub child_create: AutonomousBountyCreate,
    pub child_creation: AutonomousBountyCreationPlan,
    pub pre_claim_wallet_calls: Vec<EvmTransactionIntent>,
    pub supports_single_wallet_batch: bool,
    pub current_state: String,
    pub next_action: String,
    pub required_canonical_events: Vec<String>,
    pub evidence_boundary: String,
}

pub fn plan_canonical_child_bounty_terms(
    request: &CanonicalChildBountyTermsRequest,
) -> Result<CanonicalChildBountyTermsPlan, ChainBaseError> {
    let parent_id = parse_bytes32(&request.parent_bounty_id)?;
    if request.parent_round == 0 {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "canonical child parent round must be positive".to_string(),
        ));
    }
    let parent_solver = normalize_address(&request.parent_solver)?;
    let verifier_module = normalize_address(&request.verifier_module)?;
    if parent_solver == "0x0000000000000000000000000000000000000000"
        || verifier_module == "0x0000000000000000000000000000000000000000"
    {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "canonical child solver and verifier module must be nonzero".to_string(),
        ));
    }
    if verifier_module.eq_ignore_ascii_case(BASE_MAINNET_CANONICAL_CHILD_VERIFIER) {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "the parent canonical-child verifier cannot verify its own child task; choose the child's task-specific deterministic verifier"
                .to_string(),
        ));
    }
    if verifier_module.eq_ignore_ascii_case(BASE_MAINNET_LEADING_ZERO_WORK_VERIFIER) {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "the leading-zero work canary cannot verify a canonical child task: its exact proof-of-work benchmark conflicts with the required parent-bound child benchmark; deploy or choose a task-specific deterministic verifier"
                .to_string(),
        ));
    }
    autonomous_money_to_uint256(&request.parent_solver_reward, false)?;
    if request.child_acceptance_criteria.is_empty()
        || request.child_acceptance_criteria.len() > 20
        || request
            .child_acceptance_criteria
            .iter()
            .any(|criterion| criterion.trim().is_empty() || criterion.len() > 500)
    {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "canonical child acceptance criteria must contain 1-20 nonempty items of at most 500 bytes"
                .to_string(),
        ));
    }

    let parent_bounty_id = format!("0x{}", hex::encode(parent_id));
    let acceptance_criteria = request.child_acceptance_criteria.clone();
    let benchmark = json!({
        "parent_bounty_id": parent_bounty_id,
        "parent_round_hex": format!("0x{:016x}", request.parent_round),
        "protocol": CANONICAL_CHILD_PROTOCOL_VERSION,
    });

    Ok(CanonicalChildBountyTermsPlan {
        protocol_version: CANONICAL_CHILD_PROTOCOL_VERSION.to_string(),
        parent_bounty_id,
        parent_round: request.parent_round,
        required_creator: parent_solver,
        minimum_child_target: Money {
            amount: request.parent_solver_reward.amount,
            currency: "usdc".to_string(),
        },
        acceptance_criteria_hash: keccak256_canonical_json(&json!(acceptance_criteria))?,
        acceptance_criteria,
        benchmark_hash: keccak256_canonical_json(&benchmark)?,
        benchmark,
        verification_mode: AutonomousVerificationMode::DeterministicModule,
        verifier_module,
        threshold: 1,
        required_child_status: "settled".to_string(),
        proof_encoding: "abi.encode(address childBounty)".to_string(),
        evidence_boundary: "This plan is not completion or payout evidence. The parent passes only after the configured verifier reads a parent-bound canonical child in Settled state, created by the parent solver and completed by a different wallet through its own explicit deterministic verifier. The child's confirmed canonical BountySettled event proves the child solver was paid; the parent's confirmed canonical BountySettled event proves the parent solver was paid.".to_string(),
    })
}

fn standing_meta_v2_benchmark_source(
    source: &StandingMetaV2BenchmarkSource,
) -> Result<Value, ChainBaseError> {
    let repository_parts = source.repository.split('/').collect::<Vec<_>>();
    let valid_repository_part = |value: &&str| {
        !value.is_empty()
            && value.len() <= 100
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    };
    let commit = source.commit.to_ascii_lowercase();
    let subdirectory_parts = source.subdirectory.split('/').collect::<Vec<_>>();
    if source.kind != "github_commit"
        || repository_parts.len() != 2
        || !repository_parts.iter().all(valid_repository_part)
        || commit.len() != 40
        || !commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        || source.subdirectory.starts_with('/')
        || source.subdirectory.ends_with('/')
        || source.subdirectory.contains('\\')
        || subdirectory_parts
            .iter()
            .any(|part| part.is_empty() || matches!(*part, "." | ".."))
    {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "benchmark source must be an exact github_commit with owner/repository, a full Git SHA, and a normalized non-root subdirectory"
                .to_string(),
        ));
    }
    Ok(json!({
        "kind": "github_commit",
        "repository": source.repository,
        "commit": commit,
        "subdirectory": source.subdirectory,
    }))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Eip3009AuthorizationMessage {
    pub from: String,
    pub to: String,
    pub value: String,
    pub valid_after: String,
    pub valid_before: String,
    pub nonce: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Eip712DomainData {
    pub name: String,
    pub version: String,
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    #[serde(rename = "verifyingContract")]
    pub verifying_contract: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Eip712TypeField {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Eip3009AuthorizationTypedData {
    pub types: BTreeMap<String, Vec<Eip712TypeField>>,
    pub domain: Eip712DomainData,
    pub primary_type: String,
    pub message: Eip3009AuthorizationMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousBountyCreationPlan {
    pub protocol_version: String,
    pub network: BaseNetworkDescriptor,
    pub factory_contract: String,
    pub implementation_contract: String,
    pub bounty_id: String,
    pub predicted_bounty_contract: String,
    pub approve: Option<EvmTransactionIntent>,
    pub create_bounty: EvmTransactionIntent,
    pub wallet_calls: Vec<EvmTransactionIntent>,
    pub supports_single_wallet_batch: bool,
    pub eip3009_authorization: Option<Eip3009AuthorizationTypedData>,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousBountyCreationBatchPlan {
    pub protocol_version: String,
    pub network: BaseNetworkDescriptor,
    pub creator: String,
    pub total_initial_funding: String,
    pub approve: Option<EvmTransactionIntent>,
    pub creations: Vec<AutonomousBountyCreationPlan>,
    pub wallet_calls: Vec<EvmTransactionIntent>,
    pub supports_single_wallet_batch: bool,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousBountyAuthorizationSignature {
    pub v: u8,
    pub r: String,
    pub s: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousBountyAuthorizedCreationPlan {
    pub protocol_version: String,
    pub network: BaseNetworkDescriptor,
    pub bounty_id: String,
    pub predicted_bounty_contract: String,
    pub relay_transaction: EvmTransactionIntent,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousBountyContribution {
    pub bounty_contract: String,
    pub contributor: String,
    pub amount: Money,
    pub authorization_nonce: Option<String>,
    pub authorization_valid_before: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousBountyContributionPlan {
    pub protocol_version: String,
    pub network: BaseNetworkDescriptor,
    pub approve: EvmTransactionIntent,
    pub fund: EvmTransactionIntent,
    pub wallet_calls: Vec<EvmTransactionIntent>,
    pub supports_single_wallet_batch: bool,
    pub eip3009_authorization: Option<Eip3009AuthorizationTypedData>,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousBountyAuthorizedContributionPlan {
    pub protocol_version: String,
    pub network: BaseNetworkDescriptor,
    pub bounty_contract: String,
    pub relay_transaction: EvmTransactionIntent,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousBountyClaimPlan {
    pub protocol_version: String,
    pub network: BaseNetworkDescriptor,
    pub bounty_contract: String,
    pub solver: String,
    pub claim_bond: String,
    pub approve: Option<EvmTransactionIntent>,
    pub claim: EvmTransactionIntent,
    pub wallet_calls: Vec<EvmTransactionIntent>,
    pub supports_single_wallet_batch: bool,
    pub eip3009_authorization: Option<Eip3009AuthorizationTypedData>,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousBountyAuthorizedClaimPlan {
    pub protocol_version: String,
    pub network: BaseNetworkDescriptor,
    pub bounty_contract: String,
    pub solver: String,
    pub claim_bond: String,
    pub relay_transaction: EvmTransactionIntent,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AtomicClaimSponsorGrant {
    pub sponsor_contract: String,
    pub bounty_contract: String,
    pub solver: String,
    pub round: u64,
    pub bond: u128,
    pub terms_hash: String,
    pub policy_hash: String,
    pub authorization_nonce: String,
    pub valid_after: u64,
    pub valid_before: u64,
    pub grant_nonce: String,
    pub deadline: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtomicSponsoredClaimPlan {
    pub protocol_version: String,
    pub network: BaseNetworkDescriptor,
    pub sponsor_contract: String,
    pub factory_contract: String,
    pub bounty_contract: String,
    pub solver: String,
    pub grant_digest: String,
    pub grant_signature: String,
    pub relay_transaction: EvmTransactionIntent,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousBountySubmissionAuthorizationRequest {
    pub bounty_contract: String,
    pub bounty_id: String,
    pub round: u64,
    pub solver: String,
    pub submission_hash: String,
    pub evidence_hash: String,
    pub policy_hash: String,
    pub deadline: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutonomousBountySubmissionAuthorizationMessage {
    pub bounty: String,
    pub bounty_id: String,
    pub solver: String,
    pub round: String,
    pub submission_hash: String,
    pub evidence_hash: String,
    pub policy_hash: String,
    pub deadline: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutonomousBountySubmissionAuthorizationTypedData {
    pub types: BTreeMap<String, Vec<Eip712TypeField>>,
    pub domain: Eip712DomainData,
    pub primary_type: String,
    pub message: AutonomousBountySubmissionAuthorizationMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousBountySubmissionPreparation {
    pub protocol_version: String,
    pub network: BaseNetworkDescriptor,
    pub bounty_contract: String,
    pub bounty_id: String,
    pub current_bounty_state: String,
    pub expected_bounty_state: String,
    pub expected_canonical_event: String,
    pub solver: String,
    pub round: u64,
    pub claim_expires_at: u64,
    pub authorization_deadline: u64,
    pub artifact_reference: String,
    pub submission_hash: String,
    pub evidence_hash: String,
    pub policy_hash: String,
    pub signing_payload: AutonomousBountySubmissionAuthorizationTypedData,
    pub unsigned_relay_envelope: Value,
    pub evidence_publication: Value,
    pub relay_issue_url: Option<String>,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousVerificationAttestationRequest {
    pub bounty_contract: String,
    pub bounty_id: String,
    pub round: u64,
    pub verifier: String,
    pub submission_hash: String,
    pub evidence_hash: String,
    pub policy_hash: String,
    pub passed: bool,
    pub response_hash: String,
    pub deadline: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutonomousVerificationAttestationMessage {
    pub bounty: String,
    pub bounty_id: String,
    pub round: String,
    pub verifier: String,
    pub submission_hash: String,
    pub evidence_hash: String,
    pub policy_hash: String,
    pub passed: bool,
    pub response_hash: String,
    pub deadline: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutonomousVerificationAttestationTypedData {
    pub types: BTreeMap<String, Vec<Eip712TypeField>>,
    pub domain: Eip712DomainData,
    pub primary_type: String,
    pub message: AutonomousVerificationAttestationMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousSignedAttestation {
    pub verifier: String,
    pub passed: bool,
    pub response_hash: String,
    pub deadline: u64,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousBountyTxPlanner {
    pub factory_contract: String,
    pub implementation_contract: String,
}

impl AutonomousBountyTxPlanner {
    pub fn new(
        factory_contract: impl Into<String>,
        implementation_contract: impl Into<String>,
    ) -> Result<Self, ChainBaseError> {
        Ok(Self {
            factory_contract: normalize_address(factory_contract.into())?,
            implementation_contract: normalize_address(implementation_contract.into())?,
        })
    }

    pub fn plan_creation(
        &self,
        network: &str,
        create: &AutonomousBountyCreate,
    ) -> Result<AutonomousBountyCreationPlan, ChainBaseError> {
        let network = base_network_descriptor(network)?;
        let creator = normalize_address(&create.creator)?;
        let params = autonomous_create_param_words(create)?;
        let verifiers = normalized_verifiers(create)?;
        validate_autonomous_creation(create, &verifiers)?;
        let creation_nonce = parse_bytes32(&create.creation_nonce)?;
        let bounty_id = autonomous_bounty_id(
            network.chain_id,
            &self.factory_contract,
            &creator,
            creation_nonce,
            &params,
            &verifiers,
        )?;
        let predicted_bounty_contract = predict_minimal_proxy_address(
            &self.factory_contract,
            &self.implementation_contract,
            bounty_id,
        )?;
        let initial_funding = autonomous_money_to_uint256(&create.initial_funding, true)?;

        let create_bounty = EvmTransactionIntent {
            from: Some(creator.clone()),
            to: self.factory_contract.clone(),
            value_wei: 0,
            data: encode_autonomous_create_call(&params, &verifiers, initial_funding, creation_nonce)?,
            function: "createBounty((uint256,uint256,bytes32,bytes32,bytes32,bytes32,bytes32,uint64,uint64,uint64,uint8,address,address,uint8),address[],uint256,bytes32)".to_string(),
        };
        let approve = if initial_funding == 0 {
            None
        } else {
            Some(EvmTransactionIntent {
                from: Some(creator.clone()),
                to: network.native_usdc_token_address.clone(),
                value_wei: 0,
                data: encode_call(
                    "approve(address,uint256)",
                    vec![
                        encode_address(&self.factory_contract)?,
                        encode_uint256(initial_funding)?,
                    ],
                ),
                function: "approve(address,uint256)".to_string(),
            })
        };
        let mut wallet_calls = Vec::with_capacity(if approve.is_some() { 2 } else { 1 });
        if let Some(approve) = approve.clone() {
            wallet_calls.push(approve);
        }
        wallet_calls.push(create_bounty.clone());
        let eip3009_authorization = (initial_funding > 0).then(|| {
            eip3009_typed_data(
                &network,
                &creator,
                &predicted_bounty_contract,
                initial_funding,
                0,
                create.funding_deadline,
                &create.creation_nonce,
            )
        });

        Ok(AutonomousBountyCreationPlan {
            protocol_version: "agent-bounties/autonomous-v1".to_string(),
            network,
            factory_contract: self.factory_contract.clone(),
            implementation_contract: self.implementation_contract.clone(),
            bounty_id: format!("0x{}", hex::encode(bounty_id)),
            predicted_bounty_contract,
            approve,
            create_bounty,
            wallet_calls,
            supports_single_wallet_batch: true,
            eip3009_authorization,
            evidence_boundary: "A transaction plan or signature is not funding. Funding is applied only after a confirmed canonical factory event and matching FundingAdded event from the predicted bounty contract.".to_string(),
        })
    }

    pub fn plan_standing_meta_v2_child(
        &self,
        request: &StandingMetaV2ChildPreparationRequest,
        parent: &StandingMetaV2ParentContext,
        created_at: DateTime<Utc>,
    ) -> Result<StandingMetaV2ChildPreparationPlan, ChainBaseError> {
        let network_name = request.network.as_deref().unwrap_or("base-mainnet");
        let network = base_network_descriptor(network_name)?;
        if network.chain_id != 8_453
            || !self
                .factory_contract
                .eq_ignore_ascii_case(BASE_MAINNET_AUTONOMOUS_BOUNTY_FACTORY)
            || !self
                .implementation_contract
                .eq_ignore_ascii_case(BASE_MAINNET_AUTONOMOUS_BOUNTY_IMPLEMENTATION)
        {
            return Err(ChainBaseError::InvalidVerificationConfiguration(
                "standing-meta-v2 child preparation requires the canonical Base-mainnet factory"
                    .to_string(),
            ));
        }
        request.runner_manifest.validate().map_err(|error| {
            ChainBaseError::InvalidVerificationConfiguration(format!(
                "invalid sandboxed-regression runner manifest: {error}"
            ))
        })?;
        let benchmark_source = standing_meta_v2_benchmark_source(&request.benchmark_source)?;

        let parent_bounty_contract = normalize_address(&request.parent_bounty_contract)?;
        if parent_bounty_contract != normalize_address(&parent.bounty_contract)? {
            return Err(ChainBaseError::InvalidVerificationConfiguration(
                "standing-meta-v2 parent context does not match the requested contract".to_string(),
            ));
        }
        let parent_bounty_id = format!("0x{}", hex::encode(parse_bytes32(&parent.bounty_id)?));
        if parent.round == 0 {
            return Err(ChainBaseError::InvalidVerificationConfiguration(
                "standing-meta-v2 parent round must be positive".to_string(),
            ));
        }
        let parent_solver = normalize_address(&request.parent_solver)?;
        if parent_solver == normalize_address(&parent.creator)? {
            return Err(ChainBaseError::InvalidVerificationConfiguration(
                "standing-meta-v2 parent creator cannot claim as its solver".to_string(),
            ));
        }
        let intended_child_solver = normalize_address(&request.intended_child_solver)?;
        if parent_solver == intended_child_solver {
            return Err(ChainBaseError::InvalidVerificationConfiguration(
                "parent and intended child solvers must use different wallets and participant IDs"
                    .to_string(),
            ));
        }

        let target = autonomous_money_to_uint256(&parent.solver_reward, false)?;
        let default_verifier_reward = Money::new(STANDING_META_V2_DEFAULT_VERIFIER_REWARD, "usdc")
            .map_err(|_| ChainBaseError::InvalidAmount)?;
        let verifier_reward = request
            .verifier_reward
            .as_ref()
            .unwrap_or(&default_verifier_reward);
        let verifier_amount = autonomous_money_to_uint256(verifier_reward, false)?;
        let threshold = u8::try_from(BASE_MAINNET_STANDING_META_V2_VERIFIERS.len())
            .expect("canonical verifier set fits uint8");
        if verifier_amount >= target || verifier_amount % u128::from(threshold) != 0 {
            return Err(ChainBaseError::InvalidVerificationConfiguration(
                "child verifier reward must be below the parent solver reward and divide evenly across the canonical quorum"
                    .to_string(),
            ));
        }
        let solver_amount = target - verifier_amount;
        let child_solver_reward = Money::new(
            i64::try_from(solver_amount).map_err(|_| ChainBaseError::InvalidAmount)?,
            "usdc",
        )
        .map_err(|_| ChainBaseError::InvalidAmount)?;
        let child_verifier_reward = Money::new(
            i64::try_from(verifier_amount).map_err(|_| ChainBaseError::InvalidAmount)?,
            "usdc",
        )
        .map_err(|_| ChainBaseError::InvalidAmount)?;
        let initial_funding = Money::new(
            i64::try_from(target).map_err(|_| ChainBaseError::InvalidAmount)?,
            "usdc",
        )
        .map_err(|_| ChainBaseError::InvalidAmount)?;
        let created_at_unix =
            u64::try_from(created_at.timestamp()).map_err(|_| ChainBaseError::InvalidAmount)?;
        let funding_deadline = request.funding_deadline.unwrap_or(parent.funding_deadline);
        if funding_deadline <= created_at_unix {
            return Err(ChainBaseError::InvalidVerificationConfiguration(
                "child funding deadline must remain in the future".to_string(),
            ));
        }

        let task_verifiers = BASE_MAINNET_STANDING_META_V2_VERIFIERS
            .into_iter()
            .map(normalize_address)
            .collect::<Result<Vec<_>, _>>()?;
        let verification_policy = json!({
            "mechanism": "signed_quorum",
            "engine": STANDING_META_V2_REGRESSION_ENGINE,
            "verifier_module": Value::Null,
            "verifier_reward_recipient": Value::Null,
            "verifiers": task_verifiers,
            "threshold": threshold,
            "rubric": "Run the immutable sandboxed regression manifest against the submitted source snapshot. Sign the exact pass or fail result; infrastructure failures produce no verdict."
        });
        let verifier_set_hash = verifier_set_hash_from_policy(&verification_policy)?;
        if !verifier_set_hash.eq_ignore_ascii_case(BASE_MAINNET_STANDING_META_V2_VERIFIER_SET_HASH)
        {
            return Err(ChainBaseError::InvalidVerificationConfiguration(
                "canonical standing-meta-v2 verifier set hash drifted".to_string(),
            ));
        }

        let creation_nonce = match request.creation_nonce.as_deref() {
            Some(value) => format!("0x{}", hex::encode(parse_bytes32(value)?)),
            None => keccak256_canonical_json(&json!({
                "protocol": STANDING_META_V2_PROTOCOL_VERSION,
                "parent_bounty_contract": parent_bounty_contract,
                "parent_bounty_id": parent_bounty_id,
                "parent_round": parent.round,
                "parent_solver": parent_solver,
                "intended_child_solver": intended_child_solver,
                "title": request.title,
                "goal": request.goal,
                "acceptance_criteria": request.acceptance_criteria,
                "benchmark_source": benchmark_source.clone(),
                "runner_manifest": request.runner_manifest,
                "nonce_salt": request.nonce_salt,
            }))?,
        };
        let benchmark = json!({
            "engine": STANDING_META_V2_REGRESSION_ENGINE,
            "parent_binding": {
                "protocol": STANDING_META_V2_PROTOCOL_VERSION,
                "parent_bounty_contract": parent_bounty_contract,
                "parent_bounty_id": parent_bounty_id,
                "parent_round": parent.round,
            },
            "source": benchmark_source,
            "runner_manifest": request.runner_manifest,
        });
        let evidence_schema = request.evidence_schema.clone().unwrap_or_else(|| {
            json!({
                "type": "object",
                "required": ["source_snapshot_digest"],
                "properties": {
                    "source_snapshot_digest": {
                        "type": "string",
                        "pattern": "^sha256:[0-9a-f]{64}$"
                    }
                },
                "additionalProperties": true
            })
        });
        let contract_terms = json!({
            "protocol_version": "agent-bounties/autonomous-v1",
            "creator_wallet": parent_solver,
            "network": network.name,
            "settlement_token": normalize_address(&network.native_usdc_token_address)?,
            "solver_reward": child_solver_reward,
            "verifier_reward": child_verifier_reward,
            "claim_bond": child_verifier_reward,
            "initial_funding": initial_funding,
            "funding_deadline": funding_deadline,
            "claim_window_seconds": request
                .claim_window_seconds
                .unwrap_or(STANDING_META_V2_DEFAULT_WORK_WINDOW_SECONDS),
            "verification_window_seconds": request
                .verification_window_seconds
                .unwrap_or(STANDING_META_V2_DEFAULT_WORK_WINDOW_SECONDS),
            "creation_nonce": creation_nonce,
        });
        let document = AutonomousBountyTermsDocument {
            schema_version: "agent-bounties/terms-v1".to_string(),
            contract_terms,
            title: request.title.clone(),
            goal: request.goal.clone(),
            acceptance_criteria: request.acceptance_criteria.clone(),
            benchmark,
            evidence_schema,
            verification_policy,
            source_url: request.source_url.clone(),
            discovery_source: request
                .discovery_source
                .clone()
                .or_else(|| Some("standing-meta-v2 child preparation".to_string())),
            agent_eligibility: None,
            claim_coordination: None,
        };
        let terms = build_autonomous_bounty_terms_record(&parent_solver, document, created_at)?;
        let canonical_terms = canonical_json_bytes(
            &serde_json::to_value(&terms.document)
                .map_err(|error| ChainBaseError::InvalidCanonicalJson(error.to_string()))?,
        )?;
        if canonical_terms.len() > STANDING_META_V2_MAX_ONCHAIN_TERMS_BYTES {
            return Err(ChainBaseError::InvalidTermsDocument(format!(
                "standing-meta-v2 canonical terms contain {} bytes; the on-chain limit is {}",
                canonical_terms.len(),
                STANDING_META_V2_MAX_ONCHAIN_TERMS_BYTES
            )));
        }
        let canonical_terms_json = String::from_utf8(canonical_terms.clone())
            .map_err(|error| ChainBaseError::InvalidCanonicalJson(error.to_string()))?;
        let publish_terms = standing_meta_v2_publish_terms_intent(
            &parent_solver,
            &canonical_terms,
            parent,
            &terms,
            &verifier_set_hash,
            threshold,
        )?;
        let child_create = autonomous_bounty_create_from_terms(&terms)?;
        let child_creation = self.plan_creation(&network.name, &child_create)?;
        let mut pre_claim_wallet_calls = Vec::with_capacity(child_creation.wallet_calls.len() + 1);
        pre_claim_wallet_calls.push(publish_terms.clone());
        pre_claim_wallet_calls.extend(child_creation.wallet_calls.clone());

        Ok(StandingMetaV2ChildPreparationPlan {
            protocol_version: STANDING_META_V2_PROTOCOL_VERSION.to_string(),
            network,
            parent_bounty_contract,
            parent_bounty_id,
            parent_round: parent.round,
            parent_solver: parent_solver.clone(),
            intended_child_solver: intended_child_solver.clone(),
            participant_preconditions: StandingMetaV2ParticipantPreconditions {
                registry: BASE_MAINNET_STANDING_META_V2_PARTICIPANT_REGISTRY.to_string(),
                parent_solver,
                intended_child_solver,
                required_before_parent_claim: true,
                distinct_participant_ids_required: true,
                evidence_status: "not_checked_by_pure_planner".to_string(),
            },
            parent_claim_timing: StandingMetaV2ParentClaimTiming {
                terms_must_predate_parent_claim: true,
                participant_registrations_must_predate_parent_claim: true,
                strict_timestamp_ordering: true,
                same_block_claim_allowed: false,
                evidence_status: "confirm_registrations_and_terms_then_wait_for_a_strictly_later_base_timestamp"
                    .to_string(),
            },
            terms_registry: BASE_MAINNET_STANDING_META_V2_TERMS_REGISTRY.to_string(),
            task_verifiers: child_create.verifiers.clone(),
            task_verifier_set_hash: verifier_set_hash,
            task_verifier_threshold: threshold,
            terms,
            canonical_terms_hex: format!("0x{}", hex::encode(&canonical_terms)),
            canonical_terms_json,
            hosted_terms_published: false,
            publish_terms,
            child_create,
            child_creation,
            pre_claim_wallet_calls,
            supports_single_wallet_batch: true,
            current_state: "child_terms_prepared_parent_unclaimed".to_string(),
            next_action: "Confirm both distinct participant IDs were registered, then send pre_claim_wallet_calls in order from the parent solver wallet. After TermsPublished, CanonicalBountyCreated, FundingAdded, and BountyBecameClaimable are confirmed, wait for a Base block with a strictly later timestamp before claiming the parent; a same-timestamp claim cannot satisfy standing-meta-v2.".to_string(),
            required_canonical_events: vec![
                "TermsPublished".to_string(),
                "CanonicalBountyCreated".to_string(),
                "FundingAdded".to_string(),
                "BountyBecameClaimable".to_string(),
                "parent:BountyClaimed".to_string(),
                "child:BountySettled".to_string(),
                "parent:BountySettled".to_string(),
            ],
            evidence_boundary: "Hosted storage and transaction plans are not on-chain terms, funding, claims, completion, or payment. The parent solver must publish the exact returned bytes before the parent claim; canonical contract events alone prove the later state transitions, and BountySettled alone proves each payout.".to_string(),
        })
    }

    pub fn plan_creation_batch(
        &self,
        network: &str,
        creates: &[AutonomousBountyCreate],
    ) -> Result<AutonomousBountyCreationBatchPlan, ChainBaseError> {
        if creates.is_empty() {
            return Err(ChainBaseError::InvalidVerificationConfiguration(
                "creation batch must contain at least one bounty".to_string(),
            ));
        }
        let network = base_network_descriptor(network)?;
        let creator = normalize_address(&creates[0].creator)?;
        let mut bounty_ids = HashSet::new();
        let mut total_initial_funding = 0u128;
        let mut creations = Vec::with_capacity(creates.len());

        for create in creates {
            if normalize_address(&create.creator)? != creator {
                return Err(ChainBaseError::InvalidVerificationConfiguration(
                    "every creation in a wallet batch must use the same creator".to_string(),
                ));
            }
            let plan = self.plan_creation(&network.name, create)?;
            if !bounty_ids.insert(plan.bounty_id.clone()) {
                return Err(ChainBaseError::InvalidVerificationConfiguration(
                    "creation batch contains a duplicate bounty id".to_string(),
                ));
            }
            total_initial_funding = total_initial_funding
                .checked_add(autonomous_money_to_uint256(&create.initial_funding, true)?)
                .ok_or(ChainBaseError::InvalidAmount)?;
            creations.push(plan);
        }

        let approve = if total_initial_funding == 0 {
            None
        } else {
            Some(EvmTransactionIntent {
                from: Some(creator.clone()),
                to: network.native_usdc_token_address.clone(),
                value_wei: 0,
                data: encode_call(
                    "approve(address,uint256)",
                    vec![
                        encode_address(&self.factory_contract)?,
                        encode_uint256(total_initial_funding)?,
                    ],
                ),
                function: "approve(address,uint256)".to_string(),
            })
        };
        let mut wallet_calls = Vec::with_capacity(creations.len() + usize::from(approve.is_some()));
        if let Some(approve) = approve.clone() {
            wallet_calls.push(approve);
        }
        wallet_calls.extend(creations.iter().map(|plan| plan.create_bounty.clone()));

        Ok(AutonomousBountyCreationBatchPlan {
            protocol_version: "agent-bounties/autonomous-v1".to_string(),
            network,
            creator,
            total_initial_funding: total_initial_funding.to_string(),
            approve,
            creations,
            wallet_calls,
            supports_single_wallet_batch: true,
            evidence_boundary: "This unsigned batch is not deployment or funding evidence. Each bounty is live only after confirmed canonical creation, FundingAdded, and BountyBecameClaimable events from the configured factory and predicted bounty contract.".to_string(),
        })
    }

    pub fn plan_contribution(
        &self,
        network: &str,
        contribution: &AutonomousBountyContribution,
    ) -> Result<AutonomousBountyContributionPlan, ChainBaseError> {
        let network = base_network_descriptor(network)?;
        let bounty_contract = normalize_address(&contribution.bounty_contract)?;
        let contributor = normalize_address(&contribution.contributor)?;
        let amount = autonomous_money_to_uint256(&contribution.amount, false)?;
        let approve = EvmTransactionIntent {
            from: Some(contributor.clone()),
            to: network.native_usdc_token_address.clone(),
            value_wei: 0,
            data: encode_call(
                "approve(address,uint256)",
                vec![encode_address(&bounty_contract)?, encode_uint256(amount)?],
            ),
            function: "approve(address,uint256)".to_string(),
        };
        let fund = EvmTransactionIntent {
            from: Some(contributor.clone()),
            to: bounty_contract.clone(),
            value_wei: 0,
            data: encode_call("fund(uint256)", vec![encode_uint256(amount)?]),
            function: "fund(uint256)".to_string(),
        };
        let eip3009_authorization = match (
            contribution.authorization_nonce.as_deref(),
            contribution.authorization_valid_before,
        ) {
            (None, None) => None,
            (Some(nonce), Some(valid_before)) if valid_before > 0 => Some(eip3009_typed_data(
                &network,
                &contributor,
                &bounty_contract,
                amount,
                0,
                valid_before,
                &word_hex(parse_bytes32(nonce)?),
            )),
            _ => {
                return Err(ChainBaseError::InvalidVerificationConfiguration(
                    "contribution authorization requires both a bytes32 nonce and positive valid-before timestamp"
                        .to_string(),
                ))
            }
        };
        Ok(AutonomousBountyContributionPlan {
            protocol_version: "agent-bounties/autonomous-v1".to_string(),
            network,
            approve: approve.clone(),
            fund: fund.clone(),
            wallet_calls: vec![approve, fund],
            supports_single_wallet_batch: true,
            eip3009_authorization,
            evidence_boundary: "Approval and transaction submission are not funding evidence. Use confirmed FundingAdded and BountyBecameClaimable logs from the canonical bounty contract.".to_string(),
        })
    }

    pub fn plan_authorized_contribution(
        &self,
        network: &str,
        contribution: &AutonomousBountyContribution,
        signature: &AutonomousBountyAuthorizationSignature,
        relayer: Option<&str>,
    ) -> Result<AutonomousBountyAuthorizedContributionPlan, ChainBaseError> {
        let plan = self.plan_contribution(network, contribution)?;
        let nonce = contribution.authorization_nonce.as_deref().ok_or_else(|| {
            ChainBaseError::InvalidVerificationConfiguration(
                "authorization nonce is required".to_string(),
            )
        })?;
        let valid_before = contribution.authorization_valid_before.ok_or_else(|| {
            ChainBaseError::InvalidVerificationConfiguration(
                "authorization validity is required".to_string(),
            )
        })?;
        let amount = autonomous_money_to_uint256(&contribution.amount, false)?;
        let v = normalized_signature_v(signature.v)?;
        let bounty_contract = normalize_address(&contribution.bounty_contract)?;
        let relay_transaction = EvmTransactionIntent {
            from: relayer.map(normalize_address).transpose()?,
            to: bounty_contract.clone(),
            value_wei: 0,
            data: encode_call(
                AUTONOMOUS_FUND_WITH_AUTHORIZATION_FUNCTION,
                vec![
                    encode_address(&contribution.contributor)?,
                    encode_uint256(amount)?,
                    encode_uint256(0)?,
                    encode_uint256(valid_before.into())?,
                    parse_bytes32(nonce)?,
                    encode_uint256(v.into())?,
                    parse_bytes32(&signature.r)?,
                    parse_bytes32(&signature.s)?,
                ],
            ),
            function: AUTONOMOUS_FUND_WITH_AUTHORIZATION_FUNCTION.to_string(),
        };
        Ok(AutonomousBountyAuthorizedContributionPlan {
            protocol_version: plan.protocol_version,
            network: plan.network,
            bounty_contract,
            relay_transaction,
            evidence_boundary: "A signed authorization or relay transaction hash is not funding evidence. Wait for a confirmed FundingAdded event from this canonical bounty contract.".to_string(),
        })
    }

    pub fn plan_authorized_creation(
        &self,
        network: &str,
        create: &AutonomousBountyCreate,
        signature: &AutonomousBountyAuthorizationSignature,
        relayer: Option<&str>,
    ) -> Result<AutonomousBountyAuthorizedCreationPlan, ChainBaseError> {
        let creation_plan = self.plan_creation(network, create)?;
        let initial_funding = autonomous_money_to_uint256(&create.initial_funding, false)?;
        let params = autonomous_create_param_words(create)?;
        let verifiers = normalized_verifiers(create)?;
        let creation_nonce = parse_bytes32(&create.creation_nonce)?;
        let v = normalized_signature_v(signature.v)?;
        let relay_transaction = EvmTransactionIntent {
            from: relayer.map(normalize_address).transpose()?,
            to: self.factory_contract.clone(),
            value_wei: 0,
            data: encode_autonomous_authorized_create_call(
                &create.creator,
                &params,
                &verifiers,
                initial_funding,
                creation_nonce,
                0,
                create.funding_deadline,
                v,
                parse_bytes32(&signature.r)?,
                parse_bytes32(&signature.s)?,
            )?,
            function: "createBountyWithAuthorization(address,(uint256,uint256,bytes32,bytes32,bytes32,bytes32,bytes32,uint64,uint64,uint64,uint8,address,address,uint8),address[],uint256,bytes32,(uint256,uint256,bytes32,uint8,bytes32,bytes32))".to_string(),
        };
        Ok(AutonomousBountyAuthorizedCreationPlan {
            protocol_version: creation_plan.protocol_version,
            network: creation_plan.network,
            bounty_id: creation_plan.bounty_id,
            predicted_bounty_contract: creation_plan.predicted_bounty_contract,
            relay_transaction,
            evidence_boundary: "A valid authorization and relayed transaction hash are not funding evidence. Recognize funding only after the canonical factory creation event and matching FundingAdded log are confirmed.".to_string(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn plan_claim(
        &self,
        network: &str,
        bounty_contract: &str,
        solver: &str,
        claim_bond: u128,
        authorization_nonce: Option<&str>,
        authorization_valid_before: Option<u64>,
    ) -> Result<AutonomousBountyClaimPlan, ChainBaseError> {
        let network = base_network_descriptor(network)?;
        let solver = normalize_address(solver)?;
        let bounty_contract = normalize_address(bounty_contract)?;
        let claim = EvmTransactionIntent {
            from: Some(solver.clone()),
            to: bounty_contract.clone(),
            value_wei: 0,
            data: encode_call("claim()", vec![]),
            function: "claim()".to_string(),
        };
        let approve = if claim_bond > 0 {
            Some(EvmTransactionIntent {
                from: Some(solver.clone()),
                to: network.native_usdc_token_address.clone(),
                value_wei: 0,
                data: encode_call(
                    "approve(address,uint256)",
                    vec![
                        encode_address(&bounty_contract)?,
                        encode_uint256(claim_bond)?,
                    ],
                ),
                function: "approve(address,uint256)".to_string(),
            })
        } else {
            None
        };
        let eip3009_authorization = match (authorization_nonce, authorization_valid_before) {
            (None, None) => None,
            (Some(nonce), Some(valid_before)) if claim_bond > 0 && valid_before > 0 => {
                Some(eip3009_typed_data(
                    &network,
                    &solver,
                    &bounty_contract,
                    claim_bond,
                    0,
                    valid_before,
                    &word_hex(parse_bytes32(nonce)?),
                ))
            }
            _ => {
                return Err(ChainBaseError::InvalidVerificationConfiguration(
                    "claim authorization requires a positive bond, bytes32 nonce, and positive valid-before timestamp"
                        .to_string(),
                ))
            }
        };
        let mut wallet_calls = Vec::with_capacity(if approve.is_some() { 2 } else { 1 });
        if let Some(approve) = approve.clone() {
            wallet_calls.push(approve);
        }
        wallet_calls.push(claim.clone());
        Ok(AutonomousBountyClaimPlan {
            protocol_version: "agent-bounties/autonomous-v1".to_string(),
            network,
            bounty_contract,
            solver,
            claim_bond: claim_bond.to_string(),
            approve,
            claim,
            wallet_calls,
            supports_single_wallet_batch: true,
            eip3009_authorization,
            evidence_boundary: "A claim transaction is active only after confirmed BountyClaimed evidence. The solver bond equals one verifier reward: acceptance or verifier timeout returns it, rejection replaces the paid verifier reserve, and a no-submission claim timeout forfeits it into the completion bonus pool.".to_string(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn plan_authorized_claim(
        &self,
        network: &str,
        bounty_contract: &str,
        solver: &str,
        claim_bond: u128,
        authorization_nonce: &str,
        authorization_valid_before: u64,
        signature: &AutonomousBountyAuthorizationSignature,
        relayer: Option<&str>,
    ) -> Result<AutonomousBountyAuthorizedClaimPlan, ChainBaseError> {
        if claim_bond == 0 || authorization_valid_before == 0 {
            return Err(ChainBaseError::InvalidVerificationConfiguration(
                "authorized claim requires a positive claim bond and validity deadline".to_string(),
            ));
        }
        let network = base_network_descriptor(network)?;
        let bounty_contract = normalize_address(bounty_contract)?;
        let solver = normalize_address(solver)?;
        let nonce = parse_bytes32(authorization_nonce)?;
        let v = normalized_signature_v(signature.v)?;
        let relay_transaction = EvmTransactionIntent {
            from: relayer.map(normalize_address).transpose()?,
            to: bounty_contract.clone(),
            value_wei: 0,
            data: encode_call(
                "claimWithAuthorization(address,uint256,uint256,bytes32,uint8,bytes32,bytes32)",
                vec![
                    encode_address(&solver)?,
                    encode_uint256(0)?,
                    encode_uint256(authorization_valid_before.into())?,
                    nonce,
                    encode_uint256(v.into())?,
                    parse_bytes32(&signature.r)?,
                    parse_bytes32(&signature.s)?,
                ],
            ),
            function:
                "claimWithAuthorization(address,uint256,uint256,bytes32,uint8,bytes32,bytes32)"
                    .to_string(),
        };
        Ok(AutonomousBountyAuthorizedClaimPlan {
            protocol_version: "agent-bounties/autonomous-v1".to_string(),
            network,
            bounty_contract,
            solver,
            claim_bond: claim_bond.to_string(),
            relay_transaction,
            evidence_boundary: "A signed bond authorization or relay hash is not an active claim. Wait for confirmed BountyClaimed evidence with the exact solver and claim bond.".to_string(),
        })
    }

    pub fn atomic_sponsor_grant_digest(
        &self,
        network: &str,
        grant: &AtomicClaimSponsorGrant,
    ) -> Result<String, ChainBaseError> {
        let network = base_network_descriptor(network)?;
        atomic_sponsor_grant_digest(network.chain_id, &self.factory_contract, grant)
    }

    pub fn plan_atomic_sponsored_claim(
        &self,
        network: &str,
        grant: &AtomicClaimSponsorGrant,
        grant_signature: &str,
        solver_signature: &AutonomousBountyAuthorizationSignature,
        relayer: &str,
    ) -> Result<AtomicSponsoredClaimPlan, ChainBaseError> {
        validate_atomic_sponsor_grant(grant)?;
        let network = base_network_descriptor(network)?;
        let sponsor_contract = normalize_address(&grant.sponsor_contract)?;
        let bounty_contract = normalize_address(&grant.bounty_contract)?;
        let solver = normalize_address(&grant.solver)?;
        let grant_digest =
            atomic_sponsor_grant_digest(network.chain_id, &self.factory_contract, grant)?;
        let grant_signature_bytes = parse_hex_bytes(grant_signature)?;
        if grant_signature_bytes.len() != 65 || !matches!(grant_signature_bytes[64], 27 | 28) {
            return Err(ChainBaseError::InvalidVerificationConfiguration(
                "atomic sponsorship grant signature must be 65 bytes with v 27 or 28".to_string(),
            ));
        }
        let relay_transaction = EvmTransactionIntent {
            from: Some(normalize_address(relayer)?),
            to: sponsor_contract.clone(),
            value_wei: 0,
            data: encode_atomic_sponsored_claim_call(
                grant,
                &grant_signature_bytes,
                solver_signature,
            )?,
            function: "sponsorAndClaim((address,address,uint64,uint256,bytes32,bytes32,bytes32,uint256,uint256,bytes32,uint256),bytes,uint8,bytes32,bytes32)".to_string(),
        };
        Ok(AtomicSponsoredClaimPlan {
            protocol_version: "agent-bounties/atomic-claim-sponsor-v1".to_string(),
            network,
            sponsor_contract,
            factory_contract: self.factory_contract.clone(),
            bounty_contract,
            solver,
            grant_digest,
            grant_signature: grant_signature.to_ascii_lowercase(),
            relay_transaction,
            evidence_boundary: "A valid sponsorship grant, solver authorization, or transaction hash is not a claim or payout. Only the canonical bounty's confirmed BountyClaimed event activates the round; only confirmed BountySettled proves payout.".to_string(),
        })
    }

    pub fn plan_submission(
        &self,
        bounty_contract: &str,
        solver: &str,
        submission_hash: &str,
        evidence_hash: &str,
    ) -> Result<EvmTransactionIntent, ChainBaseError> {
        Ok(EvmTransactionIntent {
            from: Some(normalize_address(solver)?),
            to: normalize_address(bounty_contract)?,
            value_wei: 0,
            data: encode_call(
                "submit(bytes32,bytes32)",
                vec![
                    parse_bytes32(submission_hash)?,
                    parse_bytes32(evidence_hash)?,
                ],
            ),
            function: "submit(bytes32,bytes32)".to_string(),
        })
    }

    pub fn plan_submission_authorization(
        &self,
        network: &str,
        request: &AutonomousBountySubmissionAuthorizationRequest,
    ) -> Result<AutonomousBountySubmissionAuthorizationTypedData, ChainBaseError> {
        if request.round == 0 || request.deadline == 0 {
            return Err(ChainBaseError::InvalidVerificationConfiguration(
                "submission authorization round and deadline must be positive".to_string(),
            ));
        }
        let network = base_network_descriptor(network)?;
        let bounty = normalize_address(&request.bounty_contract)?;
        let solver = normalize_address(&request.solver)?;
        let mut types = BTreeMap::new();
        types.insert(
            "EIP712Domain".to_string(),
            vec![
                eip712_field("name", "string"),
                eip712_field("version", "string"),
                eip712_field("chainId", "uint256"),
                eip712_field("verifyingContract", "address"),
            ],
        );
        types.insert(
            "Submit".to_string(),
            vec![
                eip712_field("bounty", "address"),
                eip712_field("bountyId", "bytes32"),
                eip712_field("solver", "address"),
                eip712_field("round", "uint64"),
                eip712_field("submissionHash", "bytes32"),
                eip712_field("evidenceHash", "bytes32"),
                eip712_field("policyHash", "bytes32"),
                eip712_field("deadline", "uint256"),
            ],
        );
        Ok(AutonomousBountySubmissionAuthorizationTypedData {
            types,
            domain: Eip712DomainData {
                name: "Agent Bounties".to_string(),
                version: "1".to_string(),
                chain_id: network.chain_id,
                verifying_contract: bounty.clone(),
            },
            primary_type: "Submit".to_string(),
            message: AutonomousBountySubmissionAuthorizationMessage {
                bounty,
                bounty_id: word_hex(parse_bytes32(&request.bounty_id)?),
                solver,
                round: request.round.to_string(),
                submission_hash: word_hex(parse_bytes32(&request.submission_hash)?),
                evidence_hash: word_hex(parse_bytes32(&request.evidence_hash)?),
                policy_hash: word_hex(parse_bytes32(&request.policy_hash)?),
                deadline: request.deadline.to_string(),
            },
        })
    }

    pub fn plan_verification_attestation(
        &self,
        network: &str,
        request: &AutonomousVerificationAttestationRequest,
    ) -> Result<AutonomousVerificationAttestationTypedData, ChainBaseError> {
        let network = base_network_descriptor(network)?;
        let bounty = normalize_address(&request.bounty_contract)?;
        let verifier = normalize_address(&request.verifier)?;
        if request.round == 0 || request.deadline == 0 {
            return Err(ChainBaseError::InvalidVerificationConfiguration(
                "attestation round and deadline must be positive".to_string(),
            ));
        }
        let mut types = BTreeMap::new();
        types.insert(
            "EIP712Domain".to_string(),
            vec![
                eip712_field("name", "string"),
                eip712_field("version", "string"),
                eip712_field("chainId", "uint256"),
                eip712_field("verifyingContract", "address"),
            ],
        );
        types.insert(
            "VerificationAttestation".to_string(),
            vec![
                eip712_field("bounty", "address"),
                eip712_field("bountyId", "bytes32"),
                eip712_field("round", "uint64"),
                eip712_field("verifier", "address"),
                eip712_field("submissionHash", "bytes32"),
                eip712_field("evidenceHash", "bytes32"),
                eip712_field("policyHash", "bytes32"),
                eip712_field("passed", "bool"),
                eip712_field("responseHash", "bytes32"),
                eip712_field("deadline", "uint256"),
            ],
        );
        Ok(AutonomousVerificationAttestationTypedData {
            types,
            domain: Eip712DomainData {
                name: "Agent Bounties".to_string(),
                version: "1".to_string(),
                chain_id: network.chain_id,
                verifying_contract: bounty.clone(),
            },
            primary_type: "VerificationAttestation".to_string(),
            message: AutonomousVerificationAttestationMessage {
                bounty,
                bounty_id: word_hex(parse_bytes32(&request.bounty_id)?),
                round: request.round.to_string(),
                verifier,
                submission_hash: word_hex(parse_bytes32(&request.submission_hash)?),
                evidence_hash: word_hex(parse_bytes32(&request.evidence_hash)?),
                policy_hash: word_hex(parse_bytes32(&request.policy_hash)?),
                passed: request.passed,
                response_hash: word_hex(parse_bytes32(&request.response_hash)?),
                deadline: request.deadline.to_string(),
            },
        })
    }

    pub fn plan_module_settlement(
        &self,
        bounty_contract: &str,
        caller: Option<&str>,
        proof: &str,
    ) -> Result<EvmTransactionIntent, ChainBaseError> {
        Ok(EvmTransactionIntent {
            from: caller.map(normalize_address).transpose()?,
            to: normalize_address(bounty_contract)?,
            value_wei: 0,
            data: encode_single_dynamic_bytes_call(
                "verifyAndSettle(bytes)",
                &parse_hex_bytes(proof)?,
            ),
            function: "verifyAndSettle(bytes)".to_string(),
        })
    }

    pub fn plan_attestation_settlement(
        &self,
        bounty_contract: &str,
        caller: Option<&str>,
        attestations: &[AutonomousSignedAttestation],
    ) -> Result<EvmTransactionIntent, ChainBaseError> {
        if attestations.is_empty() || attestations.len() > 8 {
            return Err(ChainBaseError::InvalidVerificationConfiguration(
                "settlement requires one to eight attestations".to_string(),
            ));
        }
        let expected = attestations[0].passed;
        let mut seen = HashSet::new();
        let encoded = attestations
            .iter()
            .map(|attestation| {
                let verifier = normalize_address(&attestation.verifier)?;
                if !seen.insert(verifier.clone()) {
                    return Err(ChainBaseError::InvalidVerificationConfiguration(
                        "duplicate attestation verifier".to_string(),
                    ));
                }
                if attestation.passed != expected || attestation.deadline == 0 {
                    return Err(ChainBaseError::InvalidVerificationConfiguration(
                        "attestations must have one decision and positive deadlines".to_string(),
                    ));
                }
                Ok(EncodedAutonomousAttestation {
                    verifier: encode_address(&verifier)?,
                    passed: encode_bool(attestation.passed),
                    response_hash: parse_bytes32(&attestation.response_hash)?,
                    deadline: encode_uint256(attestation.deadline.into())?,
                    signature: parse_hex_bytes(&attestation.signature)?,
                })
            })
            .collect::<Result<Vec<_>, ChainBaseError>>()?;
        Ok(EvmTransactionIntent {
            from: caller.map(normalize_address).transpose()?,
            to: normalize_address(bounty_contract)?,
            value_wei: 0,
            data: encode_attestation_array_call(&encoded),
            function: "settleWithAttestations((address,bool,bytes32,uint256,bytes)[])".to_string(),
        })
    }

    pub fn plan_expire_claim(
        &self,
        bounty_contract: &str,
        caller: Option<&str>,
    ) -> Result<EvmTransactionIntent, ChainBaseError> {
        self.plan_permissionless_call(bounty_contract, caller, "expireClaim()")
    }

    pub fn plan_expire_submission(
        &self,
        bounty_contract: &str,
        caller: Option<&str>,
    ) -> Result<EvmTransactionIntent, ChainBaseError> {
        self.plan_permissionless_call(bounty_contract, caller, "expireSubmission()")
    }

    pub fn plan_cancel(
        &self,
        bounty_contract: &str,
        caller: Option<&str>,
    ) -> Result<EvmTransactionIntent, ChainBaseError> {
        self.plan_permissionless_call(bounty_contract, caller, "cancel()")
    }

    pub fn plan_refund_withdrawal(
        &self,
        bounty_contract: &str,
        contributor: &str,
    ) -> Result<EvmTransactionIntent, ChainBaseError> {
        self.plan_permissionless_call(bounty_contract, Some(contributor), "withdrawRefund()")
    }

    fn plan_permissionless_call(
        &self,
        bounty_contract: &str,
        caller: Option<&str>,
        function: &str,
    ) -> Result<EvmTransactionIntent, ChainBaseError> {
        Ok(EvmTransactionIntent {
            from: caller.map(normalize_address).transpose()?,
            to: normalize_address(bounty_contract)?,
            value_wei: 0,
            data: encode_call(function, vec![]),
            function: function.to_string(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmLog {
    pub address: String,
    pub topics: Vec<String>,
    pub data: String,
    pub tx_hash: String,
    pub block_number: u64,
    pub log_index: u64,
    pub occurred_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseContractLogQuery {
    pub contract: String,
    pub from_block: u64,
    pub to_block: Option<u64>,
    pub topic0: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseMultiContractLogQuery {
    pub contracts: Vec<String>,
    pub from_block: u64,
    pub to_block: Option<u64>,
    pub topic0: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseNetworkDescriptor {
    pub name: String,
    pub chain_id: u64,
    pub rpc_url_env: String,
    pub native_usdc_token_address: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Erc20BalanceSafeObservation {
    pub token: String,
    pub account: String,
    pub balance: u128,
    pub safe_block_number: u64,
    pub safe_block_hash: String,
    pub safe_block_timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolverLeaderboardAwardSafeObservation {
    pub contract: String,
    pub award_id: String,
    pub paid_winner: Option<String>,
    pub safe_block_number: u64,
    pub safe_block_hash: String,
    pub safe_block_timestamp: u64,
}

struct ContractWordSafeObservation {
    word: [u8; 32],
    safe_block_number: u64,
    safe_block_hash: String,
    safe_block_timestamp: u64,
}

pub const BASE_MAINNET_USDC_TOKEN_ADDRESS: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
pub const BASE_SEPOLIA_USDC_TOKEN_ADDRESS: &str = "0x036CbD53842c5426634e7929541eC2318f3dCF7e";
pub const BASE_MAINNET_LEADING_ZERO_WORK_VERIFIER: &str =
    "0xcc6059ceeda5bc4ba8a97ecfbffa7488c8fd579e";
pub const BASE_MAINNET_CANONICAL_CHILD_VERIFIER: &str =
    "0x40adac5a1d00a725f77682f8940b893eaed31ecf";
pub const AUTONOMOUS_BOUNTY_PROTOCOL_HASH: &str =
    "0x0afcbf01041498cc301207aa5cd21a838c522d8c057d9b29c2dd83d7d94053e7";
pub const AUTONOMOUS_SUBMISSION_AUTHORIZATION_TTL_SECONDS: u64 = 1_800;
pub const AUTONOMOUS_SUBMISSION_MIN_SIGNING_WINDOW_SECONDS: u64 = 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousFactoryExpectedState {
    pub protocol_version: String,
    pub network: String,
    pub chain_id: u64,
    pub factory_contract: String,
    pub implementation_contract: String,
    pub native_usdc_token_address: String,
    pub protocol_hash: String,
    pub factory_runtime_code_hash: String,
    pub implementation_runtime_code_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousFactorySafeObservation {
    pub protocol_version: String,
    pub network: String,
    pub chain_id: u64,
    pub safe_block_number: u64,
    pub safe_block_hash: String,
    pub safe_block_timestamp: u64,
    pub block_tag: String,
    pub factory_contract: String,
    pub implementation_contract: String,
    pub native_usdc_token_address: String,
    pub protocol_hash: String,
    pub factory_runtime_code_hash: String,
    pub implementation_runtime_code_hash: String,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BaseRpcUrlConfig {
    pub base_sepolia: Option<String>,
    pub base_mainnet: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthGetLogsRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    pub params: Vec<EthGetLogsFilter>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthSendRawTransactionRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    pub params: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthSendRawTransactionResponse {
    pub jsonrpc: String,
    pub id: u64,
    pub result: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthGetTransactionReceiptRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    pub params: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthGetTransactionReceiptResponse {
    pub jsonrpc: String,
    pub id: u64,
    pub result: Option<RpcTransactionReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthBlockNumberRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    pub params: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthBlockNumberResponse {
    pub jsonrpc: String,
    pub id: u64,
    pub result: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthGetLogsFilter {
    #[serde(rename = "fromBlock")]
    pub from_block: String,
    #[serde(rename = "toBlock")]
    pub to_block: String,
    pub address: EthGetLogsAddressFilter,
    pub topics: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EthGetLogsAddressFilter {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthGetLogsResponse {
    pub jsonrpc: String,
    pub id: u64,
    pub result: Vec<RpcEvmLog>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthJsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthGetLogsEnvelope {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(default)]
    pub result: Option<Vec<RpcEvmLog>>,
    #[serde(default)]
    pub error: Option<EthJsonRpcError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthSendRawTransactionEnvelope {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub error: Option<EthJsonRpcError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthGetTransactionReceiptEnvelope {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(default)]
    pub result: Option<RpcTransactionReceipt>,
    #[serde(default)]
    pub error: Option<EthJsonRpcError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthBlockNumberEnvelope {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub error: Option<EthJsonRpcError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcLogSubmission {
    Logs(Vec<RpcEvmLog>),
    Response(EthGetLogsResponse),
}

impl RpcLogSubmission {
    pub fn into_logs(self) -> Vec<RpcEvmLog> {
        match self {
            Self::Logs(logs) => logs,
            Self::Response(response) => response.result,
        }
    }
}

impl TryFrom<EthGetLogsEnvelope> for EthGetLogsResponse {
    type Error = ChainBaseError;

    fn try_from(envelope: EthGetLogsEnvelope) -> Result<Self, Self::Error> {
        if let Some(error) = envelope.error {
            return Err(ChainBaseError::RpcProviderError {
                code: error.code,
                message: error.message,
            });
        }
        let result = envelope.result.ok_or_else(|| {
            ChainBaseError::InvalidRpcResponse("missing result array".to_string())
        })?;
        Ok(Self {
            jsonrpc: envelope.jsonrpc,
            id: envelope.id,
            result,
        })
    }
}

impl TryFrom<EthSendRawTransactionEnvelope> for EthSendRawTransactionResponse {
    type Error = ChainBaseError;

    fn try_from(envelope: EthSendRawTransactionEnvelope) -> Result<Self, Self::Error> {
        if let Some(error) = envelope.error {
            return Err(ChainBaseError::RpcProviderError {
                code: error.code,
                message: error.message,
            });
        }
        let result = normalize_hash(&envelope.result.ok_or_else(|| {
            ChainBaseError::InvalidRpcResponse("missing transaction hash result".to_string())
        })?)
        .map_err(|_| ChainBaseError::InvalidTransactionHash("result".to_string()))?;
        Ok(Self {
            jsonrpc: envelope.jsonrpc,
            id: envelope.id,
            result,
        })
    }
}

impl TryFrom<EthGetTransactionReceiptEnvelope> for EthGetTransactionReceiptResponse {
    type Error = ChainBaseError;

    fn try_from(envelope: EthGetTransactionReceiptEnvelope) -> Result<Self, Self::Error> {
        if let Some(error) = envelope.error {
            return Err(ChainBaseError::RpcProviderError {
                code: error.code,
                message: error.message,
            });
        }
        Ok(Self {
            jsonrpc: envelope.jsonrpc,
            id: envelope.id,
            result: envelope.result,
        })
    }
}

impl TryFrom<EthBlockNumberEnvelope> for EthBlockNumberResponse {
    type Error = ChainBaseError;

    fn try_from(envelope: EthBlockNumberEnvelope) -> Result<Self, Self::Error> {
        if let Some(error) = envelope.error {
            return Err(ChainBaseError::RpcProviderError {
                code: error.code,
                message: error.message,
            });
        }
        let result = envelope.result.ok_or_else(|| {
            ChainBaseError::InvalidRpcResponse("missing block number result".to_string())
        })?;
        parse_rpc_quantity(&result)?;
        Ok(Self {
            jsonrpc: envelope.jsonrpc,
            id: envelope.id,
            result,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcEvmLog {
    pub address: String,
    pub topics: Vec<String>,
    pub data: String,
    #[serde(rename = "transactionHash")]
    pub transaction_hash: String,
    #[serde(rename = "blockNumber")]
    pub block_number: String,
    #[serde(rename = "logIndex")]
    pub log_index: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcTransactionReceipt {
    #[serde(rename = "transactionHash")]
    pub transaction_hash: String,
    #[serde(rename = "blockNumber")]
    pub block_number: Option<String>,
    pub status: Option<String>,
    #[serde(default)]
    pub logs: Vec<RpcEvmLog>,
}

pub fn base_network_descriptor(network: &str) -> Result<BaseNetworkDescriptor, ChainBaseError> {
    match network.to_ascii_lowercase().as_str() {
        "base-sepolia" | "sepolia" | "84532" => Ok(BaseNetworkDescriptor {
            name: "Base Sepolia".to_string(),
            chain_id: 84_532,
            rpc_url_env: "BASE_SEPOLIA_RPC_URL".to_string(),
            native_usdc_token_address: BASE_SEPOLIA_USDC_TOKEN_ADDRESS.to_string(),
        }),
        "base" | "base-mainnet" | "mainnet" | "8453" => Ok(BaseNetworkDescriptor {
            name: "Base".to_string(),
            chain_id: 8_453,
            rpc_url_env: "BASE_MAINNET_RPC_URL".to_string(),
            native_usdc_token_address: BASE_MAINNET_USDC_TOKEN_ADDRESS.to_string(),
        }),
        other => Err(ChainBaseError::UnknownNetwork(other.to_string())),
    }
}

impl BaseRpcUrlConfig {
    pub fn from_env() -> Self {
        Self {
            base_sepolia: non_empty_env("BASE_SEPOLIA_RPC_URL"),
            base_mainnet: non_empty_env("BASE_MAINNET_RPC_URL"),
        }
    }

    pub fn resolve(
        &self,
        network: &str,
    ) -> Result<(BaseNetworkDescriptor, String), ChainBaseError> {
        let descriptor = base_network_descriptor(network)?;
        let url = match descriptor.rpc_url_env.as_str() {
            "BASE_SEPOLIA_RPC_URL" => self.base_sepolia.clone(),
            "BASE_MAINNET_RPC_URL" => self.base_mainnet.clone(),
            _ => None,
        }
        .ok_or_else(|| ChainBaseError::MissingRpcUrl {
            network: descriptor.name.clone(),
            env_var: descriptor.rpc_url_env.clone(),
        })?;
        Ok((descriptor, url))
    }
}

pub fn parse_eth_get_logs_response(value: Value) -> Result<EthGetLogsResponse, ChainBaseError> {
    let envelope: EthGetLogsEnvelope = serde_json::from_value(value)
        .map_err(|error| ChainBaseError::InvalidRpcResponse(error.to_string()))?;
    envelope.try_into()
}

pub fn parse_eth_send_raw_transaction_response(
    value: Value,
) -> Result<EthSendRawTransactionResponse, ChainBaseError> {
    let envelope: EthSendRawTransactionEnvelope = serde_json::from_value(value)
        .map_err(|error| ChainBaseError::InvalidRpcResponse(error.to_string()))?;
    envelope.try_into()
}

pub fn parse_eth_get_transaction_receipt_response(
    value: Value,
) -> Result<EthGetTransactionReceiptResponse, ChainBaseError> {
    let envelope: EthGetTransactionReceiptEnvelope = serde_json::from_value(value)
        .map_err(|error| ChainBaseError::InvalidRpcResponse(error.to_string()))?;
    envelope.try_into()
}

pub fn parse_eth_block_number_response(
    value: Value,
) -> Result<EthBlockNumberResponse, ChainBaseError> {
    let envelope: EthBlockNumberEnvelope = serde_json::from_value(value)
        .map_err(|error| ChainBaseError::InvalidRpcResponse(error.to_string()))?;
    envelope.try_into()
}

pub fn eth_send_raw_transaction_request(
    signed_transaction: &str,
    request_id: u64,
) -> Result<EthSendRawTransactionRequest, ChainBaseError> {
    Ok(EthSendRawTransactionRequest {
        jsonrpc: "2.0".to_string(),
        id: request_id,
        method: "eth_sendRawTransaction".to_string(),
        params: vec![normalize_signed_transaction(signed_transaction)?],
    })
}

pub fn eth_get_transaction_receipt_request(
    tx_hash: &str,
    request_id: u64,
) -> Result<EthGetTransactionReceiptRequest, ChainBaseError> {
    Ok(EthGetTransactionReceiptRequest {
        jsonrpc: "2.0".to_string(),
        id: request_id,
        method: "eth_getTransactionReceipt".to_string(),
        params: vec![normalize_hash(tx_hash)
            .map_err(|_| ChainBaseError::InvalidTransactionHash(tx_hash.to_string()))?],
    })
}

pub fn eth_block_number_request(request_id: u64) -> EthBlockNumberRequest {
    EthBlockNumberRequest {
        jsonrpc: "2.0".to_string(),
        id: request_id,
        method: "eth_blockNumber".to_string(),
        params: Vec::new(),
    }
}

#[async_trait::async_trait]
pub trait JsonRpcTransport: Send + Sync {
    async fn post_json_value(
        &self,
        rpc_url: &str,
        request: &Value,
    ) -> Result<Value, ChainBaseError>;
}

#[derive(Debug, Clone)]
pub struct ReqwestJsonRpcTransport {
    client: reqwest::Client,
}

impl Default for ReqwestJsonRpcTransport {
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl JsonRpcTransport for ReqwestJsonRpcTransport {
    async fn post_json_value(
        &self,
        rpc_url: &str,
        request: &Value,
    ) -> Result<Value, ChainBaseError> {
        let response = self
            .client
            .post(rpc_url)
            .json(request)
            .send()
            .await
            .map_err(|error| {
                let message = if error.is_timeout() {
                    "request timed out"
                } else if error.is_connect() {
                    "connection failed"
                } else if error.is_request() {
                    "request could not be constructed"
                } else {
                    "request failed"
                };
                ChainBaseError::RpcTransport(message.to_string())
            })?;
        let status = response.status();
        if !status.is_success() {
            return Err(ChainBaseError::RpcHttpStatus(status.as_u16()));
        }
        response
            .json::<Value>()
            .await
            .map_err(|error| ChainBaseError::InvalidRpcResponse(error.to_string()))
    }
}

pub async fn verify_autonomous_factory_safe_state(
    rpc_url: &str,
    expected: &AutonomousFactoryExpectedState,
) -> Result<AutonomousFactorySafeObservation, ChainBaseError> {
    verify_autonomous_factory_safe_state_with_transport(
        rpc_url,
        expected,
        &ReqwestJsonRpcTransport::default(),
    )
    .await
}

pub async fn verify_autonomous_factory_safe_state_with_transport<T>(
    rpc_url: &str,
    expected: &AutonomousFactoryExpectedState,
    transport: &T,
) -> Result<AutonomousFactorySafeObservation, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let network = base_network_descriptor(&expected.network)?;
    if expected.protocol_version != "agent-bounties/autonomous-v1"
        || network.chain_id != expected.chain_id
    {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "canonical factory manifest has an unsupported protocol or chain".to_string(),
        ));
    }

    let factory_contract = normalize_address(&expected.factory_contract)?;
    let implementation_contract = normalize_address(&expected.implementation_contract)?;
    let native_usdc_token_address = normalize_address(&expected.native_usdc_token_address)?;
    if native_usdc_token_address != normalize_address(&network.native_usdc_token_address)? {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "canonical factory manifest settlement token does not match the Base network"
                .to_string(),
        ));
    }
    let protocol_hash = normalize_hash(&expected.protocol_hash)?;
    if protocol_hash != AUTONOMOUS_BOUNTY_PROTOCOL_HASH {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "canonical factory manifest protocol hash is not autonomous-v1".to_string(),
        ));
    }
    let factory_runtime_code_hash = normalize_hash(&expected.factory_runtime_code_hash)?;
    let implementation_runtime_code_hash =
        normalize_hash(&expected.implementation_runtime_code_hash)?;

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
    let safe_block_number_hex = safe_block
        .get("number")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ChainBaseError::InvalidRpcResponse("safe block response is missing number".to_string())
        })?;
    let safe_block_number = parse_rpc_quantity(safe_block_number_hex)?;
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
    let exact_block = hex_quantity(safe_block_number);

    let factory_proof =
        fetch_account_code_hash(rpc_url, &factory_contract, &exact_block, 2, transport).await?;
    require_canonical_match(
        "factory runtime code hash",
        &factory_proof,
        &factory_runtime_code_hash,
    )?;

    let implementation_proof = fetch_account_code_hash(
        rpc_url,
        &implementation_contract,
        &exact_block,
        3,
        transport,
    )
    .await?;
    require_canonical_match(
        "implementation runtime code hash",
        &implementation_proof,
        &implementation_runtime_code_hash,
    )?;

    let observed_protocol_hash = fetch_contract_word(
        rpc_url,
        &factory_contract,
        &encode_call("SUPPORTED_PROTOCOL_VERSION()", Vec::new()),
        &exact_block,
        4,
        transport,
    )
    .await?;
    require_canonical_match(
        "factory protocol hash",
        &observed_protocol_hash,
        &protocol_hash,
    )?;

    let observed_implementation_word = fetch_contract_word(
        rpc_url,
        &factory_contract,
        &encode_call("implementation()", Vec::new()),
        &exact_block,
        5,
        transport,
    )
    .await?;
    let observed_implementation = address_from_word(parse_bytes32(&observed_implementation_word)?);
    require_canonical_match(
        "factory implementation",
        &observed_implementation,
        &implementation_contract,
    )?;

    let observed_token_word = fetch_contract_word(
        rpc_url,
        &factory_contract,
        &encode_call("settlementToken()", Vec::new()),
        &exact_block,
        6,
        transport,
    )
    .await?;
    let observed_token = address_from_word(parse_bytes32(&observed_token_word)?);
    require_canonical_match(
        "factory settlement token",
        &observed_token,
        &native_usdc_token_address,
    )?;

    Ok(AutonomousFactorySafeObservation {
        protocol_version: expected.protocol_version.clone(),
        network: expected.network.clone(),
        chain_id: expected.chain_id,
        safe_block_number,
        safe_block_hash,
        safe_block_timestamp,
        block_tag: "safe".to_string(),
        factory_contract,
        implementation_contract,
        native_usdc_token_address,
        protocol_hash,
        factory_runtime_code_hash,
        implementation_runtime_code_hash,
        evidence_boundary: "This observation proves exact factory code and immutable configuration at one Base safe block. It does not prove that any wallet call was authorized, broadcast, funded, claimable, accepted, paid, or settled.".to_string(),
    })
}

fn rpc_result(value: Value, request_id: u64, method: &str) -> Result<Value, ChainBaseError> {
    let object = value.as_object().ok_or_else(|| {
        ChainBaseError::InvalidRpcResponse(format!("{method} response is not an object"))
    })?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || object.get("id").and_then(Value::as_u64) != Some(request_id)
    {
        return Err(ChainBaseError::InvalidRpcResponse(format!(
            "{method} response has the wrong JSON-RPC version or id"
        )));
    }
    if let Some(error) = object.get("error") {
        let code = error.get("code").and_then(Value::as_i64).ok_or_else(|| {
            ChainBaseError::InvalidRpcResponse(format!("{method} provider error is missing code"))
        })?;
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("provider error")
            .to_string();
        return Err(ChainBaseError::RpcProviderError { code, message });
    }
    object.get("result").cloned().ok_or_else(|| {
        ChainBaseError::InvalidRpcResponse(format!("{method} response is missing result"))
    })
}

async fn fetch_account_code_hash<T>(
    rpc_url: &str,
    address: &str,
    block: &str,
    request_id: u64,
    transport: &T,
) -> Result<String, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let result = rpc_result(
        transport
            .post_json_value(
                rpc_url,
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "method": "eth_getProof",
                    "params": [address, [], block]
                }),
            )
            .await?,
        request_id,
        "eth_getProof",
    )?;
    normalize_hash(
        result
            .get("codeHash")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ChainBaseError::InvalidRpcResponse(
                    "eth_getProof response is missing codeHash".to_string(),
                )
            })?,
    )
}

async fn fetch_contract_word<T>(
    rpc_url: &str,
    contract: &str,
    data: &str,
    block: &str,
    request_id: u64,
    transport: &T,
) -> Result<String, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let result = rpc_result(
        transport
            .post_json_value(
                rpc_url,
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "method": "eth_call",
                    "params": [{ "to": contract, "data": data }, block]
                }),
            )
            .await?,
        request_id,
        "eth_call",
    )?;
    normalize_hash(result.as_str().ok_or_else(|| {
        ChainBaseError::InvalidRpcResponse("eth_call result is not one ABI word".to_string())
    })?)
}

fn require_canonical_match(
    field: &str,
    observed: &str,
    expected: &str,
) -> Result<(), ChainBaseError> {
    if observed != expected {
        return Err(ChainBaseError::InvalidVerificationConfiguration(format!(
            "canonical {field} mismatch: expected {expected}, observed {observed}"
        )));
    }
    Ok(())
}

pub async fn fetch_base_contract_logs(
    rpc_url: &str,
    query: &BaseContractLogQuery,
    request_id: u64,
) -> Result<EthGetLogsResponse, ChainBaseError> {
    fetch_base_contract_logs_with_transport(
        rpc_url,
        query,
        request_id,
        &ReqwestJsonRpcTransport::default(),
    )
    .await
}

pub async fn fetch_base_contract_logs_with_transport<T>(
    rpc_url: &str,
    query: &BaseContractLogQuery,
    request_id: u64,
    transport: &T,
) -> Result<EthGetLogsResponse, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let request = query.rpc_request(request_id);
    parse_eth_get_logs_response(transport.post_json_value(rpc_url, &json!(request)).await?)
}

pub async fn fetch_base_multi_contract_logs(
    rpc_url: &str,
    query: &BaseMultiContractLogQuery,
    request_id: u64,
) -> Result<EthGetLogsResponse, ChainBaseError> {
    fetch_base_multi_contract_logs_with_transport(
        rpc_url,
        query,
        request_id,
        &ReqwestJsonRpcTransport::default(),
    )
    .await
}

pub async fn fetch_base_multi_contract_logs_with_transport<T>(
    rpc_url: &str,
    query: &BaseMultiContractLogQuery,
    request_id: u64,
    transport: &T,
) -> Result<EthGetLogsResponse, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let request = query.rpc_request(request_id);
    parse_eth_get_logs_response(transport.post_json_value(rpc_url, &json!(request)).await?)
}

pub async fn fetch_block_number(rpc_url: &str, request_id: u64) -> Result<u64, ChainBaseError> {
    fetch_block_number_with_transport(rpc_url, request_id, &ReqwestJsonRpcTransport::default())
        .await
}

pub async fn fetch_block_number_with_transport<T>(
    rpc_url: &str,
    request_id: u64,
    transport: &T,
) -> Result<u64, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let request = eth_block_number_request(request_id);
    let response = parse_eth_block_number_response(
        transport.post_json_value(rpc_url, &json!(request)).await?,
    )?;
    parse_rpc_quantity(&response.result)
}

pub async fn fetch_block_timestamp(
    rpc_url: &str,
    block_number: u64,
    request_id: u64,
) -> Result<DateTime<Utc>, ChainBaseError> {
    fetch_block_timestamp_with_transport(
        rpc_url,
        block_number,
        request_id,
        &ReqwestJsonRpcTransport::default(),
    )
    .await
}

pub async fn fetch_block_timestamp_with_transport<T>(
    rpc_url: &str,
    block_number: u64,
    request_id: u64,
    transport: &T,
) -> Result<DateTime<Utc>, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let block = rpc_result(
        transport
            .post_json_value(
                rpc_url,
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "method": "eth_getBlockByNumber",
                    "params": [hex_quantity(block_number), false]
                }),
            )
            .await?,
        request_id,
        "eth_getBlockByNumber",
    )?;
    let timestamp = parse_rpc_quantity(
        block
            .get("timestamp")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ChainBaseError::InvalidRpcResponse(
                    "exact block response is missing timestamp".to_string(),
                )
            })?,
    )?;
    let timestamp = i64::try_from(timestamp).map_err(|_| {
        ChainBaseError::InvalidRpcResponse("block timestamp exceeds i64".to_string())
    })?;
    DateTime::from_timestamp(timestamp, 0).ok_or_else(|| {
        ChainBaseError::InvalidRpcResponse("block timestamp is outside UTC range".to_string())
    })
}

pub async fn observe_erc20_balance_safe(
    rpc_url: &str,
    token: &str,
    account: &str,
    request_id: u64,
) -> Result<Erc20BalanceSafeObservation, ChainBaseError> {
    observe_erc20_balance_safe_with_transport(
        rpc_url,
        token,
        account,
        request_id,
        &ReqwestJsonRpcTransport::default(),
    )
    .await
}

pub async fn observe_erc20_balance_safe_with_transport<T>(
    rpc_url: &str,
    token: &str,
    account: &str,
    request_id: u64,
    transport: &T,
) -> Result<Erc20BalanceSafeObservation, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let token = normalize_address(token)?;
    let account = normalize_address(account)?;
    let observation = fetch_contract_word_safe_with_transport(
        rpc_url,
        &token,
        &encode_call("balanceOf(address)", vec![encode_address(&account)?]),
        request_id,
        transport,
    )
    .await?;
    let balance = word_to_u128(observation.word)?;

    Ok(Erc20BalanceSafeObservation {
        token,
        account,
        balance,
        safe_block_number: observation.safe_block_number,
        safe_block_hash: observation.safe_block_hash,
        safe_block_timestamp: observation.safe_block_timestamp,
    })
}

pub fn solver_leaderboard_award_id(
    period_kind: u8,
    starts_at: u64,
) -> Result<String, ChainBaseError> {
    if period_kind > 1 {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "leaderboard period kind must be 0 or 1".to_string(),
        ));
    }
    let words = [
        encode_uint256(period_kind.into())?,
        encode_uint256(starts_at.into())?,
    ];
    Ok(format!("0x{}", hex::encode(keccak_words(&words))))
}

pub async fn observe_solver_leaderboard_paid_winner_safe(
    rpc_url: &str,
    contract: &str,
    award_id: &str,
    request_id: u64,
) -> Result<SolverLeaderboardAwardSafeObservation, ChainBaseError> {
    observe_solver_leaderboard_paid_winner_safe_with_transport(
        rpc_url,
        contract,
        award_id,
        request_id,
        &ReqwestJsonRpcTransport::default(),
    )
    .await
}

pub async fn observe_solver_leaderboard_paid_winner_safe_with_transport<T>(
    rpc_url: &str,
    contract: &str,
    award_id: &str,
    request_id: u64,
    transport: &T,
) -> Result<SolverLeaderboardAwardSafeObservation, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let contract = normalize_address(contract)?;
    let award_id = normalize_hash(award_id)?;
    let observation = fetch_contract_word_safe_with_transport(
        rpc_url,
        &contract,
        &encode_call("paidAwardWinner(bytes32)", vec![parse_bytes32(&award_id)?]),
        request_id,
        transport,
    )
    .await?;
    if observation.word[..12].iter().any(|byte| *byte != 0) {
        return Err(ChainBaseError::InvalidRpcResponse(
            "paid award winner is not an ABI address".to_string(),
        ));
    }
    let paid_winner = if observation.word[12..].iter().all(|byte| *byte == 0) {
        None
    } else {
        Some(format!("0x{}", hex::encode(&observation.word[12..])))
    };
    Ok(SolverLeaderboardAwardSafeObservation {
        contract,
        award_id,
        paid_winner,
        safe_block_number: observation.safe_block_number,
        safe_block_hash: observation.safe_block_hash,
        safe_block_timestamp: observation.safe_block_timestamp,
    })
}

async fn fetch_contract_word_safe_with_transport<T>(
    rpc_url: &str,
    contract: &str,
    data: &str,
    request_id: u64,
    transport: &T,
) -> Result<ContractWordSafeObservation, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let safe_block = rpc_result(
        transport
            .post_json_value(
                rpc_url,
                &json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "method": "eth_getBlockByNumber",
                    "params": ["safe", false]
                }),
            )
            .await?,
        request_id,
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
    let call_request_id = request_id.checked_add(1).ok_or_else(|| {
        ChainBaseError::InvalidVerificationConfiguration(
            "safe contract call request id overflow".to_string(),
        )
    })?;
    let word = fetch_contract_word(
        rpc_url,
        contract,
        data,
        &hex_quantity(safe_block_number),
        call_request_id,
        transport,
    )
    .await?;
    Ok(ContractWordSafeObservation {
        word: parse_bytes32(&word)?,
        safe_block_number,
        safe_block_hash,
        safe_block_timestamp,
    })
}

pub async fn broadcast_signed_transaction(
    rpc_url: &str,
    signed_transaction: &str,
    request_id: u64,
) -> Result<EthSendRawTransactionResponse, ChainBaseError> {
    broadcast_signed_transaction_with_transport(
        rpc_url,
        signed_transaction,
        request_id,
        &ReqwestJsonRpcTransport::default(),
    )
    .await
}

pub async fn broadcast_signed_transaction_with_transport<T>(
    rpc_url: &str,
    signed_transaction: &str,
    request_id: u64,
    transport: &T,
) -> Result<EthSendRawTransactionResponse, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let request = eth_send_raw_transaction_request(signed_transaction, request_id)?;
    parse_eth_send_raw_transaction_response(
        transport.post_json_value(rpc_url, &json!(request)).await?,
    )
}

pub async fn fetch_transaction_receipt(
    rpc_url: &str,
    tx_hash: &str,
    request_id: u64,
) -> Result<EthGetTransactionReceiptResponse, ChainBaseError> {
    fetch_transaction_receipt_with_transport(
        rpc_url,
        tx_hash,
        request_id,
        &ReqwestJsonRpcTransport::default(),
    )
    .await
}

pub async fn fetch_transaction_receipt_with_transport<T>(
    rpc_url: &str,
    tx_hash: &str,
    request_id: u64,
    transport: &T,
) -> Result<EthGetTransactionReceiptResponse, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let request = eth_get_transaction_receipt_request(tx_hash, request_id)?;
    parse_eth_get_transaction_receipt_response(
        transport.post_json_value(rpc_url, &json!(request)).await?,
    )
}

fn non_empty_env(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

impl BaseContractLogQuery {
    pub fn new(
        contract: impl Into<String>,
        from_block: u64,
        to_block: Option<u64>,
        topic0: Vec<String>,
    ) -> Result<Self, ChainBaseError> {
        if let Some(to_block) = to_block {
            if from_block > to_block {
                return Err(ChainBaseError::InvalidBlockRange {
                    from_block,
                    to_block,
                });
            }
        }
        if topic0.is_empty() {
            return Err(ChainBaseError::InvalidLogTopics(
                "at least one topic0 is required".to_string(),
            ));
        }
        Ok(Self {
            contract: normalize_address(contract.into())?,
            from_block,
            to_block,
            topic0: topic0
                .into_iter()
                .map(|topic| normalize_topic(&topic))
                .collect::<Result<Vec<_>, _>>()?,
        })
    }

    pub fn rpc_request(&self, id: u64) -> EthGetLogsRequest {
        EthGetLogsRequest {
            jsonrpc: "2.0".to_string(),
            id,
            method: "eth_getLogs".to_string(),
            params: vec![EthGetLogsFilter {
                from_block: hex_quantity(self.from_block),
                to_block: self
                    .to_block
                    .map(hex_quantity)
                    .unwrap_or_else(|| "latest".to_string()),
                address: EthGetLogsAddressFilter::One(self.contract.clone()),
                topics: vec![self.topic0.clone()],
            }],
        }
    }
}

impl BaseMultiContractLogQuery {
    pub fn new<I, S>(
        contracts: I,
        from_block: u64,
        to_block: Option<u64>,
        topic0: Vec<String>,
    ) -> Result<Self, ChainBaseError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        if let Some(to_block) = to_block {
            if from_block > to_block {
                return Err(ChainBaseError::InvalidBlockRange {
                    from_block,
                    to_block,
                });
            }
        }
        if topic0.is_empty() {
            return Err(ChainBaseError::InvalidLogTopics(
                "at least one topic0 is required".to_string(),
            ));
        }
        let mut contracts = contracts
            .into_iter()
            .map(|contract| normalize_address(contract.into()))
            .collect::<Result<Vec<_>, _>>()?;
        contracts.sort();
        contracts.dedup();
        if contracts.is_empty() {
            return Err(ChainBaseError::InvalidLogTopics(
                "at least one contract address is required".to_string(),
            ));
        }
        Ok(Self {
            contracts,
            from_block,
            to_block,
            topic0: topic0
                .into_iter()
                .map(|topic| normalize_topic(&topic))
                .collect::<Result<Vec<_>, _>>()?,
        })
    }

    pub fn rpc_request(&self, id: u64) -> EthGetLogsRequest {
        EthGetLogsRequest {
            jsonrpc: "2.0".to_string(),
            id,
            method: "eth_getLogs".to_string(),
            params: vec![EthGetLogsFilter {
                from_block: hex_quantity(self.from_block),
                to_block: self
                    .to_block
                    .map(hex_quantity)
                    .unwrap_or_else(|| "latest".to_string()),
                address: EthGetLogsAddressFilter::Many(self.contracts.clone()),
                topics: vec![self.topic0.clone()],
            }],
        }
    }
}

impl RpcEvmLog {
    pub fn to_evm_log(&self) -> Result<EvmLog, ChainBaseError> {
        Ok(EvmLog {
            address: normalize_address(&self.address)?,
            topics: self
                .topics
                .iter()
                .map(|topic| normalize_topic(topic))
                .collect::<Result<Vec<_>, _>>()?,
            data: normalize_data(&self.data)?,
            tx_hash: normalize_hash(&self.transaction_hash)?,
            block_number: parse_rpc_quantity(&self.block_number)?,
            log_index: parse_rpc_quantity(&self.log_index)?,
            occurred_at: None,
        })
    }
}

impl RpcTransactionReceipt {
    pub fn normalized_tx_hash(&self) -> Result<String, ChainBaseError> {
        normalize_hash(&self.transaction_hash)
            .map_err(|_| ChainBaseError::InvalidTransactionHash(self.transaction_hash.clone()))
    }

    pub fn block_number(&self) -> Result<Option<u64>, ChainBaseError> {
        self.block_number
            .as_deref()
            .map(parse_rpc_quantity)
            .transpose()
    }

    pub fn succeeded(&self) -> Result<Option<bool>, ChainBaseError> {
        self.status
            .as_deref()
            .map(parse_rpc_quantity)
            .transpose()
            .map(|status| status.map(|status| status == 1))
    }

    pub fn logs_to_evm_logs(&self) -> Result<Vec<EvmLog>, ChainBaseError> {
        rpc_logs_to_evm_logs(self.logs.clone())
    }
}

pub fn rpc_logs_to_evm_logs(
    logs: impl IntoIterator<Item = RpcEvmLog>,
) -> Result<Vec<EvmLog>, ChainBaseError> {
    logs.into_iter().map(|log| log.to_evm_log()).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutonomousBountyEventKind {
    CanonicalBountyCreated,
    CanonicalBountyTermsCommitted,
    CanonicalBountyEconomicsConfigured,
    CanonicalBountyVerificationConfigured,
    ExternalBountySubmitted,
    FundingAdded,
    BountyBecameClaimable,
    BountyClaimed,
    SubmissionAdded,
    SubmissionRejected,
    BountySettled,
    ClaimExpired,
    SubmissionExpired,
    BountyCancelled,
    RefundWithdrawn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousBountyEvent {
    pub id: Id,
    pub log_key: String,
    pub tx_hash: String,
    pub block_number: u64,
    pub log_index: u64,
    pub contract_address: String,
    pub bounty_id: String,
    pub kind: AutonomousBountyEventKind,
    pub data: Value,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousBountyFeedItem {
    pub bounty_id: String,
    pub bounty_contract: String,
    pub creator: String,
    pub status: String,
    pub solver_reward: String,
    pub verifier_reward: String,
    pub claim_bond: String,
    pub timeout_bond_pool: String,
    pub target_amount: String,
    pub funded_amount: String,
    pub terms_hash: String,
    pub terms: Option<AutonomousBountyTermsRecord>,
    pub terms_valid: bool,
    pub verification_mode: String,
    pub verifier_module: Option<String>,
    pub verification_ready: bool,
    pub verification_readiness_reason: String,
    pub validation_errors: Vec<String>,
    pub events: Vec<AutonomousBountyEvent>,
}

pub fn standing_meta_v2_parent_context(
    item: &AutonomousBountyFeedItem,
) -> Result<StandingMetaV2ParentContext, ChainBaseError> {
    let terms = item.terms.as_ref().ok_or_else(|| {
        ChainBaseError::InvalidVerificationConfiguration(
            "standing-meta-v2 parent terms are unavailable".to_string(),
        )
    })?;
    let benchmark = terms.document.benchmark.as_object().ok_or_else(|| {
        ChainBaseError::InvalidVerificationConfiguration(
            "standing-meta-v2 parent benchmark is unavailable".to_string(),
        )
    })?;
    let exact_parent = item.status == "claimable"
        && item.terms_valid
        && item.verification_ready
        && item.validation_errors.is_empty()
        && item.verifier_module.as_deref().is_some_and(|module| {
            module.eq_ignore_ascii_case(BASE_MAINNET_STANDING_META_V2_VERIFIER)
        })
        && terms
            .acceptance_criteria_hash
            .eq_ignore_ascii_case(BASE_MAINNET_STANDING_META_V2_ACCEPTANCE_CRITERIA_HASH)
        && benchmark.get("engine").and_then(Value::as_str) == Some("standing_meta_v2_parent")
        && benchmark
            .get("required_child_engine")
            .and_then(Value::as_str)
            == Some(STANDING_META_V2_REGRESSION_ENGINE)
        && benchmark
            .get("required_child_verifier_set_hash")
            .and_then(Value::as_str)
            .is_some_and(|hash| {
                hash.eq_ignore_ascii_case(BASE_MAINNET_STANDING_META_V2_VERIFIER_SET_HASH)
            })
        && benchmark
            .get("required_child_verifier_threshold")
            .and_then(Value::as_u64)
            == Some(2)
        && benchmark
            .get("participant_registry")
            .and_then(Value::as_str)
            .is_some_and(|address| {
                address.eq_ignore_ascii_case(BASE_MAINNET_STANDING_META_V2_PARTICIPANT_REGISTRY)
            })
        && benchmark
            .get("terms_registry")
            .and_then(Value::as_str)
            .is_some_and(|address| {
                address.eq_ignore_ascii_case(BASE_MAINNET_STANDING_META_V2_TERMS_REGISTRY)
            });
    if !exact_parent {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "bounty is not an exact, valid, claimable standing-meta-v2 parent".to_string(),
        ));
    }
    let round = item
        .events
        .iter()
        .filter_map(|event| event.data.get("round").and_then(Value::as_u64))
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| {
            ChainBaseError::InvalidVerificationConfiguration(
                "standing-meta-v2 parent round overflow".to_string(),
            )
        })?;
    let reward = item
        .solver_reward
        .parse::<i64>()
        .ok()
        .and_then(|amount| Money::new(amount, "usdc").ok())
        .ok_or(ChainBaseError::InvalidAmount)?;
    let funding_deadline = terms.document.contract_terms["funding_deadline"]
        .as_u64()
        .ok_or_else(|| {
            ChainBaseError::InvalidVerificationConfiguration(
                "standing-meta-v2 parent funding deadline is unavailable".to_string(),
            )
        })?;
    Ok(StandingMetaV2ParentContext {
        bounty_contract: normalize_address(&item.bounty_contract)?,
        bounty_id: item.bounty_id.clone(),
        creator: normalize_address(&item.creator)?,
        round,
        solver_reward: reward,
        funding_deadline,
    })
}

pub const RECOVERY_RESERVED_VERIFICATION_REASON: &str =
    "incident recovery reservation is active; do not claim, sign, or post a bond";

pub const BUILTIN_RECOVERY_RESERVED_BOUNTY_CONTRACTS: [&str; 3] = [
    "0x680030abf3ffffbc8d0a550b6355a8713c54d3c8",
    "0x3137e6c0f44b940580ea7efc5f8cc6c6c0bda3f1",
    "0xb35b94e1225b66e50644a331feccdab0439e63d7",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutonomousBountyRecoveryReservations {
    contracts: HashSet<String>,
}

impl Default for AutonomousBountyRecoveryReservations {
    fn default() -> Self {
        Self {
            contracts: BUILTIN_RECOVERY_RESERVED_BOUNTY_CONTRACTS
                .into_iter()
                .map(str::to_string)
                .collect(),
        }
    }
}

impl AutonomousBountyRecoveryReservations {
    pub fn parse_csv(value: Option<&str>) -> Result<Self, ChainBaseError> {
        let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(Self::default());
        };
        let mut contracts = Self::default().contracts;
        for candidate in value.split(',') {
            let candidate = candidate.trim();
            if candidate.is_empty() {
                return Err(ChainBaseError::InvalidVerificationConfiguration(
                    "recovery-reserved bounty contract list contains an empty entry".to_string(),
                ));
            }
            let contract = normalize_address(candidate).map_err(|_| {
                ChainBaseError::InvalidVerificationConfiguration(format!(
                    "recovery-reserved bounty contract is not a valid EVM address: {candidate}"
                ))
            })?;
            contracts.insert(contract);
        }
        Ok(Self { contracts })
    }

    pub fn contains(&self, bounty_contract: &str) -> bool {
        self.contracts
            .contains(&bounty_contract.to_ascii_lowercase())
    }

    pub fn apply(&self, feed: &mut Vec<AutonomousBountyFeedItem>, claimable_only: bool) {
        for item in feed.iter_mut() {
            if self.contains(&item.bounty_contract) {
                item.verification_ready = false;
                item.verification_readiness_reason =
                    RECOVERY_RESERVED_VERIFICATION_REASON.to_string();
            }
        }
        if claimable_only {
            feed.retain(autonomous_bounty_is_earning_ready);
        }
    }

    pub fn exclude_from_verification_jobs(&self, feed: &mut Vec<AutonomousBountyFeedItem>) {
        feed.retain(|item| !self.contains(&item.bounty_contract));
    }
}

pub fn autonomous_bounty_is_earning_ready(item: &AutonomousBountyFeedItem) -> bool {
    item.status == "claimable" && item.terms_valid && item.verification_ready
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousVerificationJob {
    pub job_id: String,
    pub network: String,
    pub bounty_id: String,
    pub bounty_contract: String,
    pub round: u64,
    pub solver_wallet: String,
    pub verification_mode: String,
    pub verifier_module: Option<String>,
    pub eligible_verifiers: Vec<String>,
    pub threshold: u8,
    pub verifier_reward: String,
    pub current_solver_payout: String,
    pub verification_expires_at: u64,
    pub terms: AutonomousBountyTermsRecord,
    pub submission_evidence: AutonomousSubmissionEvidenceRecord,
    pub required_action: String,
    pub payout_boundary: String,
}

fn validate_autonomous_terms_against_creation(
    creation_data: &Value,
    terms: Option<&AutonomousBountyTermsRecord>,
) -> Vec<String> {
    let Some(terms) = terms else {
        return vec!["content-addressed terms are unavailable".to_string()];
    };
    let mut errors = Vec::new();
    let check_hash = |field: &str, expected: &str, errors: &mut Vec<String>| {
        if !creation_data[field]
            .as_str()
            .is_some_and(|value| value.eq_ignore_ascii_case(expected))
        {
            errors.push(format!("{field} does not match published terms"));
        }
    };
    check_hash("terms_hash", &terms.terms_hash, &mut errors);
    check_hash("policy_hash", &terms.policy_hash, &mut errors);
    check_hash(
        "acceptance_criteria_hash",
        &terms.acceptance_criteria_hash,
        &mut errors,
    );
    check_hash("benchmark_hash", &terms.benchmark_hash, &mut errors);
    check_hash(
        "evidence_schema_hash",
        &terms.evidence_schema_hash,
        &mut errors,
    );

    if let Some(contract_terms) = terms.document.contract_terms.as_object() {
        let compare_amount = |terms_field: &str, event_field: &str, errors: &mut Vec<String>| {
            match contract_terms_money(contract_terms, terms_field, terms_field != "solver_reward")
            {
                Ok(expected) if creation_data[event_field].as_u64() == Some(expected) => {}
                Ok(_) => errors.push(format!(
                    "contract_terms {terms_field} does not match the contract"
                )),
                Err(error) => errors.push(error.to_string()),
            }
        };
        compare_amount("solver_reward", "solver_reward", &mut errors);
        compare_amount("verifier_reward", "verifier_reward", &mut errors);
        compare_amount("claim_bond", "claim_bond", &mut errors);
        compare_amount("initial_funding", "initial_funding", &mut errors);
        for field in [
            "funding_deadline",
            "claim_window_seconds",
            "verification_window_seconds",
        ] {
            match contract_terms_u64(contract_terms, field) {
                Ok(expected) if creation_data[field].as_u64() == Some(expected) => {}
                Ok(_) => errors.push(format!(
                    "contract_terms {field} does not match the contract"
                )),
                Err(error) => errors.push(error.to_string()),
            }
        }
        match contract_terms_string(contract_terms, "creation_nonce") {
            Ok(expected)
                if creation_data["creation_nonce"]
                    .as_str()
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(expected)) => {}
            Ok(_) => {
                errors.push("contract_terms creation_nonce does not match the contract".to_string())
            }
            Err(error) => errors.push(error.to_string()),
        }
        match contract_terms_string(contract_terms, "creator_wallet") {
            Ok(expected)
                if creation_data["creator"]
                    .as_str()
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(expected)) => {}
            Ok(_) => {
                errors.push("contract_terms creator_wallet does not match the contract".to_string())
            }
            Err(error) => errors.push(error.to_string()),
        }
    } else {
        errors.push("published contract_terms are unavailable".to_string());
    }

    let policy = &terms.document.verification_policy;
    let expected_mode = match policy.get("mechanism").and_then(Value::as_str) {
        Some("deterministic_module") => Some(0),
        Some("signed_quorum") => Some(1),
        Some("ai_judge_quorum") => Some(2),
        _ => None,
    };
    if expected_mode != creation_data["verification_mode"].as_u64() {
        errors.push("verification mechanism does not match the contract".to_string());
    }
    if policy.get("threshold").and_then(Value::as_u64) != creation_data["threshold"].as_u64() {
        errors.push("verification threshold does not match the contract".to_string());
    }

    let zero = "0x0000000000000000000000000000000000000000";
    for (policy_field, event_field) in [
        ("verifier_module", "verifier_module"),
        ("verifier_reward_recipient", "verifier_reward_recipient"),
    ] {
        let expected = policy
            .get(policy_field)
            .and_then(Value::as_str)
            .unwrap_or(zero);
        if !creation_data[event_field]
            .as_str()
            .is_some_and(|value| value.eq_ignore_ascii_case(expected))
        {
            errors.push(format!("{policy_field} does not match the contract"));
        }
    }

    let expected_verifier_set_hash = if expected_mode == Some(0) {
        Ok(format!("0x{}", "00".repeat(32)))
    } else {
        verifier_set_hash_from_policy(policy)
    };
    match expected_verifier_set_hash {
        Ok(expected) => {
            if !creation_data["verifier_set_hash"]
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case(&expected))
            {
                errors.push("verifier set does not match the contract".to_string());
            }
        }
        Err(error) => errors.push(error.to_string()),
    }
    errors
}

fn verifier_set_hash_from_policy(policy: &Value) -> Result<String, ChainBaseError> {
    let verifiers = policy
        .get("verifiers")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let words = verifiers
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| {
                    ChainBaseError::InvalidTermsDocument(
                        "verification-policy verifier is not an address".to_string(),
                    )
                })
                .and_then(encode_address)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut encoded = Vec::with_capacity((2 + words.len()) * 32);
    encoded.extend_from_slice(&encode_uint256(32)?);
    encoded.extend_from_slice(&encode_uint256(words.len() as u128)?);
    for word in words {
        encoded.extend_from_slice(&word);
    }
    Ok(format!("0x{}", hex::encode(Keccak256::digest(encoded))))
}

pub fn validate_attestation_request_against_feed(
    item: &AutonomousBountyFeedItem,
    request: &AutonomousVerificationAttestationRequest,
    observed_at_unix: u64,
) -> Result<(), ChainBaseError> {
    if !item.terms_valid
        || item.status != "submitted"
        || !item
            .bounty_contract
            .eq_ignore_ascii_case(&request.bounty_contract)
        || !item.bounty_id.eq_ignore_ascii_case(&request.bounty_id)
    {
        return Err(ChainBaseError::InvalidAttestationScope(
            "bounty is not the indexed submitted canonical instance".to_string(),
        ));
    }
    let terms = item.terms.as_ref().ok_or_else(|| {
        ChainBaseError::InvalidAttestationScope(
            "content-addressed bounty terms are unavailable".to_string(),
        )
    })?;
    if !terms.policy_hash.eq_ignore_ascii_case(&request.policy_hash) {
        return Err(ChainBaseError::InvalidAttestationScope(
            "policy hash does not match the published terms".to_string(),
        ));
    }
    let policy = &terms.document.verification_policy;
    let mechanism = policy
        .get("mechanism")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if mechanism == "deterministic_module" {
        return Err(ChainBaseError::InvalidAttestationScope(
            "deterministic module bounties do not accept signed attestations".to_string(),
        ));
    }
    let verifier_allowed = policy
        .get("verifiers")
        .and_then(Value::as_array)
        .is_some_and(|verifiers| {
            verifiers.iter().any(|value| {
                value
                    .as_str()
                    .is_some_and(|verifier| verifier.eq_ignore_ascii_case(&request.verifier))
            })
        });
    if !verifier_allowed {
        return Err(ChainBaseError::InvalidAttestationScope(
            "verifier is outside the immutable verifier set".to_string(),
        ));
    }
    let submission = item
        .events
        .iter()
        .rev()
        .find(|event| event.kind == AutonomousBountyEventKind::SubmissionAdded)
        .ok_or_else(|| {
            ChainBaseError::InvalidAttestationScope(
                "indexed SubmissionAdded evidence is missing".to_string(),
            )
        })?;
    let matches_round = submission.data.get("round").and_then(Value::as_u64) == Some(request.round);
    let matches_submission = submission
        .data
        .get("submission_hash")
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case(&request.submission_hash));
    let matches_evidence = submission
        .data
        .get("evidence_hash")
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case(&request.evidence_hash));
    let verification_expires_at = submission
        .data
        .get("verification_expires_at")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            ChainBaseError::InvalidAttestationScope(
                "submission verification deadline is missing".to_string(),
            )
        })?;
    if !matches_round || !matches_submission || !matches_evidence {
        return Err(ChainBaseError::InvalidAttestationScope(
            "attestation does not match the current submission round and hashes".to_string(),
        ));
    }
    if request.deadline <= observed_at_unix || request.deadline > verification_expires_at {
        return Err(ChainBaseError::InvalidAttestationScope(
            "attestation deadline must be live and no later than verification expiry".to_string(),
        ));
    }
    parse_bytes32(&request.response_hash)?;
    Ok(())
}

pub fn build_autonomous_submission_preparation(
    planner: &AutonomousBountyTxPlanner,
    network: &str,
    item: &AutonomousBountyFeedItem,
    solver_wallet: &str,
    artifact_reference: &str,
    evidence: Value,
    observed_at_unix: u64,
) -> Result<AutonomousBountySubmissionPreparation, ChainBaseError> {
    if item.status != "claimed" || !item.terms_valid || !item.verification_ready {
        return Err(ChainBaseError::InvalidSubmissionPreparation(format!(
            "canonical bounty must be claimed with valid terms and executable verification; state={}, terms_valid={}, verification_ready={}",
            item.status, item.terms_valid, item.verification_ready
        )));
    }
    let terms = item.terms.as_ref().ok_or_else(|| {
        ChainBaseError::InvalidSubmissionPreparation(
            "content-addressed bounty terms are unavailable".to_string(),
        )
    })?;
    let claim = item
        .events
        .iter()
        .rev()
        .find(|event| event.kind == AutonomousBountyEventKind::BountyClaimed)
        .ok_or_else(|| {
            ChainBaseError::InvalidSubmissionPreparation(
                "indexed BountyClaimed evidence is missing".to_string(),
            )
        })?;
    let solver = normalize_evm_address(solver_wallet)?;
    let indexed_solver = claim
        .data
        .get("solver")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ChainBaseError::InvalidSubmissionPreparation(
                "BountyClaimed solver is missing".to_string(),
            )
        })?;
    if !indexed_solver.eq_ignore_ascii_case(&solver) {
        return Err(ChainBaseError::InvalidSubmissionPreparation(
            "requested solver does not own the active claim".to_string(),
        ));
    }
    let round = claim
        .data
        .get("round")
        .and_then(Value::as_u64)
        .filter(|round| *round > 0)
        .ok_or_else(|| {
            ChainBaseError::InvalidSubmissionPreparation(
                "BountyClaimed round is missing or zero".to_string(),
            )
        })?;
    let claim_expires_at = claim
        .data
        .get("claim_expires_at")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            ChainBaseError::InvalidSubmissionPreparation(
                "BountyClaimed claim expiry is missing".to_string(),
            )
        })?;
    let minimum_deadline = observed_at_unix
        .checked_add(AUTONOMOUS_SUBMISSION_MIN_SIGNING_WINDOW_SECONDS)
        .ok_or_else(|| {
            ChainBaseError::InvalidSubmissionPreparation(
                "submission signing window overflowed".to_string(),
            )
        })?;
    if claim_expires_at <= minimum_deadline {
        return Err(ChainBaseError::InvalidSubmissionPreparation(
            "active claim expires too soon to prepare and relay a submission".to_string(),
        ));
    }
    let authorization_deadline = claim_expires_at.min(
        observed_at_unix
            .checked_add(AUTONOMOUS_SUBMISSION_AUTHORIZATION_TTL_SECONDS)
            .ok_or_else(|| {
                ChainBaseError::InvalidSubmissionPreparation(
                    "submission authorization deadline overflowed".to_string(),
                )
            })?,
    );
    let (artifact_reference, submission_hash, evidence_hash) =
        validate_submission_preimages(artifact_reference, &evidence)?;
    let bounty_contract = normalize_evm_address(&item.bounty_contract)?;
    let authorization_request = AutonomousBountySubmissionAuthorizationRequest {
        bounty_contract: bounty_contract.clone(),
        bounty_id: item.bounty_id.clone(),
        round,
        solver: solver.clone(),
        submission_hash: submission_hash.clone(),
        evidence_hash: evidence_hash.clone(),
        policy_hash: terms.policy_hash.clone(),
        deadline: authorization_deadline,
    };
    let signing_payload = planner.plan_submission_authorization(network, &authorization_request)?;
    let network_descriptor = base_network_descriptor(network)?;
    let unsigned_relay_envelope = json!({
        "schema": "agent-bounties/autonomous-gas-relay-v1",
        "action": "submit",
        "network": network_descriptor.name,
        "bounty_contract": bounty_contract,
        "solver": solver,
        "round": round,
        "submission_hash": submission_hash,
        "evidence_hash": evidence_hash,
        "deadline": authorization_deadline,
        "signature": Value::Null,
    });
    let evidence_publication = json!({
        "network": network_descriptor.name,
        "bounty_contract": bounty_contract,
        "bounty_id": item.bounty_id,
        "round": round,
        "solver_wallet": solver,
        "artifact_reference": artifact_reference,
        "evidence": evidence,
    });
    let relay_issue_url = terms
        .document
        .source_url
        .clone()
        .filter(|url| url.starts_with("https://github.com/") && url.contains("/issues/"));
    Ok(AutonomousBountySubmissionPreparation {
        protocol_version: "agent-bounties/autonomous-v1".to_string(),
        network: network_descriptor,
        bounty_contract,
        bounty_id: item.bounty_id.clone(),
        current_bounty_state: "claimed".to_string(),
        expected_bounty_state: "submitted".to_string(),
        expected_canonical_event: "SubmissionAdded".to_string(),
        solver,
        round,
        claim_expires_at,
        authorization_deadline,
        artifact_reference,
        submission_hash,
        evidence_hash,
        policy_hash: terms.policy_hash.clone(),
        signing_payload,
        unsigned_relay_envelope,
        evidence_publication,
        relay_issue_url,
        evidence_boundary: "This preparation validates one indexed active claim and computes deterministic public commitments. It does not sign, broadcast, submit, publish evidence, verify, settle, or prove payment. Add the solver's EIP-712 signature to the relay envelope, wait for confirmed canonical SubmissionAdded, then publish the returned evidence preimages. Only BountySettled proves payout.".to_string(),
    })
}

fn validate_submission_preimages(
    artifact_reference: &str,
    evidence: &Value,
) -> Result<(String, String, String), ChainBaseError> {
    let artifact_reference = artifact_reference.trim();
    if artifact_reference.is_empty() || artifact_reference.len() > 16 * 1024 {
        return Err(ChainBaseError::InvalidSubmissionEvidence(
            "artifact reference must be non-empty and no larger than 16 KiB".to_string(),
        ));
    }
    let evidence_size = serde_json::to_vec(evidence)
        .map_err(|error| ChainBaseError::InvalidCanonicalJson(error.to_string()))?
        .len();
    if evidence_size > 256 * 1024 || !evidence.is_object() {
        return Err(ChainBaseError::InvalidSubmissionEvidence(
            "evidence must be an object no larger than 256 KiB".to_string(),
        ));
    }
    Ok((
        artifact_reference.to_string(),
        sha256_utf8(artifact_reference),
        sha256_canonical_json(evidence)?,
    ))
}

#[allow(clippy::too_many_arguments)]
pub fn build_autonomous_submission_evidence_record(
    network: &str,
    item: &AutonomousBountyFeedItem,
    bounty_contract: &str,
    bounty_id: &str,
    round: u64,
    solver_wallet: &str,
    artifact_reference: &str,
    evidence: Value,
    created_at: DateTime<Utc>,
) -> Result<domain::AutonomousSubmissionEvidenceRecord, ChainBaseError> {
    if item.status != "submitted"
        || round == 0
        || !item.bounty_contract.eq_ignore_ascii_case(bounty_contract)
        || !item.bounty_id.eq_ignore_ascii_case(bounty_id)
    {
        return Err(ChainBaseError::InvalidSubmissionEvidence(
            "bounty identity, state, round, or artifact reference is invalid".to_string(),
        ));
    }
    let (artifact_reference, artifact_hash, evidence_hash) =
        validate_submission_preimages(artifact_reference, &evidence)?;
    let solver_wallet = normalize_evm_address(solver_wallet)?;
    let bounty_contract = normalize_evm_address(bounty_contract)?;
    let submission = item
        .events
        .iter()
        .rev()
        .find(|event| event.kind == AutonomousBountyEventKind::SubmissionAdded)
        .ok_or_else(|| {
            ChainBaseError::InvalidSubmissionEvidence(
                "indexed SubmissionAdded evidence is missing".to_string(),
            )
        })?;
    let matches = submission.data.get("round").and_then(Value::as_u64) == Some(round)
        && submission
            .data
            .get("solver")
            .and_then(Value::as_str)
            .is_some_and(|value| value.eq_ignore_ascii_case(&solver_wallet))
        && submission
            .data
            .get("submission_hash")
            .and_then(Value::as_str)
            .is_some_and(|value| value.eq_ignore_ascii_case(&artifact_hash))
        && submission
            .data
            .get("evidence_hash")
            .and_then(Value::as_str)
            .is_some_and(|value| value.eq_ignore_ascii_case(&evidence_hash));
    if !matches {
        return Err(ChainBaseError::InvalidSubmissionEvidence(
            "published preimages do not match the current SubmissionAdded commitments".to_string(),
        ));
    }
    Ok(domain::AutonomousSubmissionEvidenceRecord {
        network: network.to_string(),
        bounty_contract,
        bounty_id: bounty_id.to_ascii_lowercase(),
        round,
        solver_wallet,
        artifact_reference,
        artifact_hash,
        evidence,
        evidence_hash,
        created_at,
    })
}

pub fn build_autonomous_verification_jobs(
    network: &str,
    feed: impl IntoIterator<Item = AutonomousBountyFeedItem>,
    evidence_records: impl IntoIterator<Item = AutonomousSubmissionEvidenceRecord>,
    observed_at_unix: u64,
) -> Result<Vec<AutonomousVerificationJob>, ChainBaseError> {
    let evidence_records = evidence_records
        .into_iter()
        .map(|record| {
            (
                (record.bounty_contract.to_ascii_lowercase(), record.round),
                record,
            )
        })
        .collect::<HashMap<_, _>>();
    let mut jobs = Vec::new();
    for item in feed {
        if item.status != "submitted" || !item.terms_valid {
            continue;
        }
        let terms = item.terms.clone().ok_or_else(|| {
            ChainBaseError::InvalidSubmissionEvidence(
                "submitted canonical bounty is missing terms".to_string(),
            )
        })?;
        let submission = item
            .events
            .iter()
            .rev()
            .find(|event| event.kind == AutonomousBountyEventKind::SubmissionAdded)
            .ok_or_else(|| {
                ChainBaseError::InvalidSubmissionEvidence(
                    "submitted canonical bounty is missing SubmissionAdded".to_string(),
                )
            })?;
        let round = submission.data["round"].as_u64().ok_or_else(|| {
            ChainBaseError::InvalidSubmissionEvidence(
                "SubmissionAdded round is unavailable".to_string(),
            )
        })?;
        let solver_wallet = submission.data["solver"]
            .as_str()
            .ok_or_else(|| {
                ChainBaseError::InvalidSubmissionEvidence(
                    "SubmissionAdded solver is unavailable".to_string(),
                )
            })?
            .to_string();
        let verification_expires_at = submission.data["verification_expires_at"]
            .as_u64()
            .ok_or_else(|| {
                ChainBaseError::InvalidSubmissionEvidence(
                    "SubmissionAdded verification deadline is unavailable".to_string(),
                )
            })?;
        if verification_expires_at <= observed_at_unix {
            continue;
        }
        let Some(evidence) = evidence_records
            .get(&(item.bounty_contract.to_ascii_lowercase(), round))
            .cloned()
        else {
            continue;
        };
        let evidence_matches = evidence.bounty_id.eq_ignore_ascii_case(&item.bounty_id)
            && evidence.solver_wallet.eq_ignore_ascii_case(&solver_wallet)
            && submission.data["submission_hash"]
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case(&evidence.artifact_hash))
            && submission.data["evidence_hash"]
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case(&evidence.evidence_hash));
        if !evidence_matches {
            return Err(ChainBaseError::InvalidSubmissionEvidence(
                "stored evidence no longer matches the current indexed submission".to_string(),
            ));
        }
        let policy = terms
            .document
            .verification_policy
            .as_object()
            .ok_or_else(|| {
                ChainBaseError::InvalidTermsDocument(
                    "verification policy is not an object".to_string(),
                )
            })?;
        let verification_mode = policy
            .get("mechanism")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ChainBaseError::InvalidTermsDocument(
                    "verification mechanism is unavailable".to_string(),
                )
            })?
            .to_string();
        let threshold = policy
            .get("threshold")
            .and_then(Value::as_u64)
            .and_then(|value| u8::try_from(value).ok())
            .ok_or_else(|| {
                ChainBaseError::InvalidTermsDocument(
                    "verification threshold is unavailable".to_string(),
                )
            })?;
        let eligible_verifiers = policy
            .get("verifiers")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| {
                        ChainBaseError::InvalidTermsDocument(
                            "eligible verifier is not an address".to_string(),
                        )
                    })
                    .and_then(normalize_address)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let verifier_module = policy
            .get("verifier_module")
            .and_then(Value::as_str)
            .filter(|value| {
                !value.eq_ignore_ascii_case("0x0000000000000000000000000000000000000000")
            })
            .map(normalize_address)
            .transpose()?;
        let solver_reward = item.solver_reward.parse::<u128>().map_err(|_| {
            ChainBaseError::InvalidLogData("feed solver reward is invalid".to_string())
        })?;
        let timeout_bonus = item.timeout_bond_pool.parse::<u128>().map_err(|_| {
            ChainBaseError::InvalidLogData("feed timeout bond pool is invalid".to_string())
        })?;
        let required_action = if verification_mode == "deterministic_module" {
            "Evaluate the committed module proof format, then relay verifyAndSettle. A valid pass settles and a valid fail pays the verifier and reopens the bounty."
        } else {
            "Evaluate only the immutable policy and exact evidence preimages, request the scoped EIP-712 attestation payload, sign one verdict, and relay a matching threshold quorum."
        };
        jobs.push(AutonomousVerificationJob {
            job_id: format!(
                "{}:{}:{}",
                network,
                item.bounty_contract.to_ascii_lowercase(),
                round
            ),
            network: network.to_string(),
            bounty_id: item.bounty_id,
            bounty_contract: item.bounty_contract,
            round,
            solver_wallet,
            verification_mode,
            verifier_module,
            eligible_verifiers,
            threshold,
            verifier_reward: item.verifier_reward,
            current_solver_payout: solver_reward
                .checked_add(timeout_bonus)
                .ok_or(ChainBaseError::InvalidAmount)?
                .to_string(),
            verification_expires_at,
            terms,
            submission_evidence: evidence,
            required_action: required_action.to_string(),
            payout_boundary:
                "A verdict, signature, relay hash, or AI output is not payout evidence. Only confirmed canonical BountySettled proves payment."
                    .to_string(),
        });
    }
    jobs.sort_by_key(|job| (job.verification_expires_at, job.job_id.clone()));
    Ok(jobs)
}

pub fn autonomous_bounty_event_topics() -> Vec<String> {
    vec![
        event_topic("CanonicalBountyCreated(bytes32,address,address,bytes32,bytes32,bytes32)"),
        event_topic("CanonicalBountyTermsCommitted(bytes32,bytes32,bytes32,bytes32)"),
        event_topic("CanonicalBountyEconomicsConfigured(bytes32,uint256,uint256,uint256,uint256,uint64,uint64,uint64)"),
        event_topic("CanonicalBountyVerificationConfigured(bytes32,uint8,address,address,uint8,bytes32)"),
        event_topic("ExternalBountySubmitted(address,address,bytes32,bytes32,bytes32)"),
        event_topic("FundingAdded(bytes32,address,uint256,uint256,uint256)"),
        event_topic("BountyBecameClaimable(bytes32,uint256)"),
        event_topic("BountyClaimed(bytes32,uint64,address,bytes32,bytes32,uint256,uint64)"),
        event_topic("SubmissionAdded(bytes32,uint64,address,bytes32,bytes32,uint64)"),
        event_topic("SubmissionRejected(bytes32,uint64,address,uint256,uint256,bytes32)"),
        event_topic("BountySettled(bytes32,uint64,address,uint256,uint256,uint256,uint256,bytes32,bytes32,bytes32,bytes32)"),
        event_topic("ClaimExpired(bytes32,uint64,address,uint256,uint256)"),
        event_topic("SubmissionExpired(bytes32,uint64,address,uint256)"),
        event_topic("BountyCancelled(bytes32,uint256)"),
        event_topic("RefundWithdrawn(bytes32,address,uint256,uint256,uint256)"),
    ]
}

#[derive(Debug, Clone, Default)]
pub struct AutonomousBountyLogDecoder;

impl AutonomousBountyLogDecoder {
    pub fn decode(&self, log: EvmLog) -> Result<AutonomousBountyEvent, ChainBaseError> {
        let topic0 = log
            .topics
            .first()
            .ok_or_else(|| ChainBaseError::InvalidLogTopics("missing topic0".to_string()))?;
        let signature = autonomous_event_signature(topic0)
            .ok_or_else(|| ChainBaseError::UnknownEventTopic(topic0.clone()))?;
        let (kind, bounty_id, data) = match signature {
            AutonomousEventSignature::CanonicalBountyCreated => {
                require_topic_count(&log, 4, "CanonicalBountyCreated")?;
                let words = decode_words(&log.data, 3, "CanonicalBountyCreated")?;
                (
                    AutonomousBountyEventKind::CanonicalBountyCreated,
                    word_hex(topic_word(&log, 1, "CanonicalBountyCreated")?),
                    json!({
                        "bounty_contract": address_from_word(topic_word(&log, 2, "CanonicalBountyCreated")?),
                        "creator": address_from_word(topic_word(&log, 3, "CanonicalBountyCreated")?),
                        "terms_hash": word_hex(words[0]),
                        "policy_hash": word_hex(words[1]),
                        "creation_nonce": word_hex(words[2]),
                    }),
                )
            }
            AutonomousEventSignature::CanonicalBountyTermsCommitted => {
                require_topic_count(&log, 2, "CanonicalBountyTermsCommitted")?;
                let words = decode_words(&log.data, 3, "CanonicalBountyTermsCommitted")?;
                (
                    AutonomousBountyEventKind::CanonicalBountyTermsCommitted,
                    word_hex(topic_word(&log, 1, "CanonicalBountyTermsCommitted")?),
                    json!({
                        "acceptance_criteria_hash": word_hex(words[0]),
                        "benchmark_hash": word_hex(words[1]),
                        "evidence_schema_hash": word_hex(words[2]),
                    }),
                )
            }
            AutonomousEventSignature::CanonicalBountyEconomicsConfigured => {
                require_topic_count(&log, 2, "CanonicalBountyEconomicsConfigured")?;
                let words = decode_words(&log.data, 7, "CanonicalBountyEconomicsConfigured")?;
                (
                    AutonomousBountyEventKind::CanonicalBountyEconomicsConfigured,
                    word_hex(topic_word(&log, 1, "CanonicalBountyEconomicsConfigured")?),
                    json!({
                        "solver_reward": word_to_u128(words[0])?,
                        "verifier_reward": word_to_u128(words[1])?,
                        "claim_bond": word_to_u128(words[1])?,
                        "target_amount": word_to_u128(words[2])?,
                        "initial_funding": word_to_u128(words[3])?,
                        "funding_deadline": word_to_u64(words[4], "CanonicalBountyEconomicsConfigured")?,
                        "claim_window_seconds": word_to_u64(words[5], "CanonicalBountyEconomicsConfigured")?,
                        "verification_window_seconds": word_to_u64(words[6], "CanonicalBountyEconomicsConfigured")?,
                    }),
                )
            }
            AutonomousEventSignature::CanonicalBountyVerificationConfigured => {
                require_topic_count(&log, 2, "CanonicalBountyVerificationConfigured")?;
                let words = decode_words(&log.data, 5, "CanonicalBountyVerificationConfigured")?;
                (
                    AutonomousBountyEventKind::CanonicalBountyVerificationConfigured,
                    word_hex(topic_word(
                        &log,
                        1,
                        "CanonicalBountyVerificationConfigured",
                    )?),
                    json!({
                        "verification_mode": word_to_u8(words[0], "CanonicalBountyVerificationConfigured")?,
                        "verifier_module": address_from_word(words[1]),
                        "verifier_reward_recipient": address_from_word(words[2]),
                        "threshold": word_to_u8(words[3], "CanonicalBountyVerificationConfigured")?,
                        "verifier_set_hash": word_hex(words[4]),
                    }),
                )
            }
            AutonomousEventSignature::ExternalBountySubmitted => {
                require_topic_count(&log, 4, "ExternalBountySubmitted")?;
                let words = decode_words(&log.data, 2, "ExternalBountySubmitted")?;
                (
                    AutonomousBountyEventKind::ExternalBountySubmitted,
                    word_hex(topic_word(&log, 3, "ExternalBountySubmitted")?),
                    json!({
                        "bounty_contract": address_from_word(topic_word(&log, 1, "ExternalBountySubmitted")?),
                        "submitter": address_from_word(topic_word(&log, 2, "ExternalBountySubmitted")?),
                        "terms_hash": word_hex(words[0]),
                        "policy_hash": word_hex(words[1]),
                        "canonical": false,
                    }),
                )
            }
            AutonomousEventSignature::FundingAdded => {
                require_topic_count(&log, 3, "FundingAdded")?;
                let words = decode_words(&log.data, 3, "FundingAdded")?;
                (
                    AutonomousBountyEventKind::FundingAdded,
                    word_hex(topic_word(&log, 1, "FundingAdded")?),
                    json!({
                        "contributor": address_from_word(topic_word(&log, 2, "FundingAdded")?),
                        "amount": word_to_u128(words[0])?,
                        "funded_amount": word_to_u128(words[1])?,
                        "target_amount": word_to_u128(words[2])?,
                    }),
                )
            }
            AutonomousEventSignature::BountyBecameClaimable => {
                require_topic_count(&log, 2, "BountyBecameClaimable")?;
                let words = decode_words(&log.data, 1, "BountyBecameClaimable")?;
                (
                    AutonomousBountyEventKind::BountyBecameClaimable,
                    word_hex(topic_word(&log, 1, "BountyBecameClaimable")?),
                    json!({ "funded_amount": word_to_u128(words[0])? }),
                )
            }
            AutonomousEventSignature::BountyClaimed => {
                require_topic_count(&log, 4, "BountyClaimed")?;
                let words = decode_words(&log.data, 4, "BountyClaimed")?;
                (
                    AutonomousBountyEventKind::BountyClaimed,
                    word_hex(topic_word(&log, 1, "BountyClaimed")?),
                    json!({
                        "round": topic_u64(&log, 2, "BountyClaimed")?,
                        "solver": address_from_word(topic_word(&log, 3, "BountyClaimed")?),
                        "terms_hash": word_hex(words[0]),
                        "policy_hash": word_hex(words[1]),
                        "claim_bond": word_to_u128(words[2])?,
                        "claim_expires_at": word_to_u64(words[3], "BountyClaimed")?,
                    }),
                )
            }
            AutonomousEventSignature::SubmissionAdded => {
                require_topic_count(&log, 4, "SubmissionAdded")?;
                let words = decode_words(&log.data, 3, "SubmissionAdded")?;
                (
                    AutonomousBountyEventKind::SubmissionAdded,
                    word_hex(topic_word(&log, 1, "SubmissionAdded")?),
                    json!({
                        "round": topic_u64(&log, 2, "SubmissionAdded")?,
                        "solver": address_from_word(topic_word(&log, 3, "SubmissionAdded")?),
                        "submission_hash": word_hex(words[0]),
                        "evidence_hash": word_hex(words[1]),
                        "verification_expires_at": word_to_u64(words[2], "SubmissionAdded")?,
                    }),
                )
            }
            AutonomousEventSignature::SubmissionRejected => {
                require_topic_count(&log, 4, "SubmissionRejected")?;
                let words = decode_words(&log.data, 3, "SubmissionRejected")?;
                (
                    AutonomousBountyEventKind::SubmissionRejected,
                    word_hex(topic_word(&log, 1, "SubmissionRejected")?),
                    json!({
                        "round": topic_u64(&log, 2, "SubmissionRejected")?,
                        "solver": address_from_word(topic_word(&log, 3, "SubmissionRejected")?),
                        "verifier_reward": word_to_u128(words[0])?,
                        "claim_bond_forfeited": word_to_u128(words[1])?,
                        "verification_hash": word_hex(words[2]),
                    }),
                )
            }
            AutonomousEventSignature::BountySettled => {
                require_topic_count(&log, 4, "BountySettled")?;
                let words = decode_words(&log.data, 8, "BountySettled")?;
                (
                    AutonomousBountyEventKind::BountySettled,
                    word_hex(topic_word(&log, 1, "BountySettled")?),
                    json!({
                        "round": topic_u64(&log, 2, "BountySettled")?,
                        "solver": address_from_word(topic_word(&log, 3, "BountySettled")?),
                        "solver_reward": word_to_u128(words[0])?,
                        "claim_bond_returned": word_to_u128(words[1])?,
                        "timeout_bond_bonus": word_to_u128(words[2])?,
                        "solver_payout": word_to_u128(words[0])? + word_to_u128(words[1])? + word_to_u128(words[2])?,
                        "verifier_reward": word_to_u128(words[3])?,
                        "submission_hash": word_hex(words[4]),
                        "evidence_hash": word_hex(words[5]),
                        "policy_hash": word_hex(words[6]),
                        "verification_hash": word_hex(words[7]),
                    }),
                )
            }
            AutonomousEventSignature::ClaimExpired => {
                require_topic_count(&log, 4, "ClaimExpired")?;
                let words = decode_words(&log.data, 2, "ClaimExpired")?;
                (
                    AutonomousBountyEventKind::ClaimExpired,
                    word_hex(topic_word(&log, 1, "ClaimExpired")?),
                    json!({
                        "round": topic_u64(&log, 2, "ClaimExpired")?,
                        "solver": address_from_word(topic_word(&log, 3, "ClaimExpired")?),
                        "claim_bond_forfeited": word_to_u128(words[0])?,
                        "timeout_bond_pool": word_to_u128(words[1])?,
                    }),
                )
            }
            AutonomousEventSignature::SubmissionExpired => decode_expiry_event(
                &log,
                "SubmissionExpired",
                AutonomousBountyEventKind::SubmissionExpired,
            )?,
            AutonomousEventSignature::BountyCancelled => {
                require_topic_count(&log, 2, "BountyCancelled")?;
                let words = decode_words(&log.data, 1, "BountyCancelled")?;
                (
                    AutonomousBountyEventKind::BountyCancelled,
                    word_hex(topic_word(&log, 1, "BountyCancelled")?),
                    json!({ "timeout_bond_refund_pool": word_to_u128(words[0])? }),
                )
            }
            AutonomousEventSignature::RefundWithdrawn => {
                require_topic_count(&log, 3, "RefundWithdrawn")?;
                let words = decode_words(&log.data, 3, "RefundWithdrawn")?;
                (
                    AutonomousBountyEventKind::RefundWithdrawn,
                    word_hex(topic_word(&log, 1, "RefundWithdrawn")?),
                    json!({
                        "contributor": address_from_word(topic_word(&log, 2, "RefundWithdrawn")?),
                        "principal": word_to_u128(words[0])?,
                        "timeout_bond_bonus": word_to_u128(words[1])?,
                        "amount": word_to_u128(words[2])?,
                    }),
                )
            }
        };

        Ok(AutonomousBountyEvent {
            id: deterministic_log_id(&log),
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
}

pub fn decode_autonomous_bounty_logs(
    logs: impl IntoIterator<Item = EvmLog>,
) -> Result<Vec<AutonomousBountyEvent>, ChainBaseError> {
    let topics = autonomous_bounty_event_topics()
        .into_iter()
        .collect::<HashSet<_>>();
    let decoder = AutonomousBountyLogDecoder;
    logs.into_iter()
        .filter(|log| {
            log.topics
                .first()
                .and_then(|topic| normalize_topic(topic).ok())
                .is_some_and(|topic| topics.contains(&topic))
        })
        .map(|log| decoder.decode(log))
        .collect()
}

pub fn build_autonomous_bounty_feed(
    events: impl IntoIterator<Item = AutonomousBountyEvent>,
    terms: impl IntoIterator<Item = AutonomousBountyTermsRecord>,
    claimable_only: bool,
) -> Result<Vec<AutonomousBountyFeedItem>, ChainBaseError> {
    let terms = terms
        .into_iter()
        .map(|record| (record.terms_hash.to_ascii_lowercase(), record))
        .collect::<BTreeMap<_, _>>();
    let mut grouped = BTreeMap::<String, Vec<AutonomousBountyEvent>>::new();
    for event in events {
        grouped
            .entry(event.bounty_id.to_ascii_lowercase())
            .or_default()
            .push(event);
    }
    let mut feed = Vec::new();
    for (bounty_id, mut events) in grouped {
        events.sort_by_key(|event| (event.block_number, event.log_index));
        let created = events
            .iter()
            .filter(|event| event.kind == AutonomousBountyEventKind::CanonicalBountyCreated)
            .collect::<Vec<_>>();
        if created.is_empty() {
            continue;
        }
        if created.len() != 1 {
            return Err(ChainBaseError::InvalidLogData(
                "duplicate CanonicalBountyCreated".to_string(),
            ));
        }
        let created = created[0];
        let configuration_event = |kind: AutonomousBountyEventKind, name: &str| {
            let matches = events
                .iter()
                .filter(|event| event.kind == kind)
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err(ChainBaseError::InvalidLogData(format!(
                    "expected one {name}, found {}",
                    matches.len()
                )));
            }
            Ok(matches[0])
        };
        let terms_committed = configuration_event(
            AutonomousBountyEventKind::CanonicalBountyTermsCommitted,
            "CanonicalBountyTermsCommitted",
        )?;
        let economics = configuration_event(
            AutonomousBountyEventKind::CanonicalBountyEconomicsConfigured,
            "CanonicalBountyEconomicsConfigured",
        )?;
        let verification = configuration_event(
            AutonomousBountyEventKind::CanonicalBountyVerificationConfigured,
            "CanonicalBountyVerificationConfigured",
        )?;
        let mut creation_data = created.data.clone();
        let creation_object = creation_data.as_object_mut().ok_or_else(|| {
            ChainBaseError::InvalidLogData(
                "CanonicalBountyCreated data is not an object".to_string(),
            )
        })?;
        for configuration in [terms_committed, economics, verification] {
            let object = configuration.data.as_object().ok_or_else(|| {
                ChainBaseError::InvalidLogData(format!(
                    "{:?} data is not an object",
                    configuration.kind
                ))
            })?;
            creation_object.extend(object.clone());
        }
        let required_string = |field: &str| {
            creation_data[field]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| {
                    ChainBaseError::InvalidLogData(format!(
                        "CanonicalBountyCreated missing {field}"
                    ))
                })
        };
        let bounty_contract = required_string("bounty_contract")?;
        let creator = required_string("creator")?;
        let terms_hash = required_string("terms_hash")?;
        let solver_reward = creation_data["solver_reward"].as_u64().ok_or_else(|| {
            ChainBaseError::InvalidLogData(
                "CanonicalBountyEconomicsConfigured missing solver_reward".to_string(),
            )
        })?;
        let verifier_reward = creation_data["verifier_reward"].as_u64().ok_or_else(|| {
            ChainBaseError::InvalidLogData(
                "CanonicalBountyEconomicsConfigured missing verifier_reward".to_string(),
            )
        })?;
        let claim_bond = creation_data["claim_bond"].as_u64().ok_or_else(|| {
            ChainBaseError::InvalidLogData(
                "CanonicalBountyEconomicsConfigured missing claim_bond".to_string(),
            )
        })?;
        let target_amount = creation_data["target_amount"].as_u64().ok_or_else(|| {
            ChainBaseError::InvalidLogData(
                "CanonicalBountyCreated missing target_amount".to_string(),
            )
        })?;
        if solver_reward.checked_add(verifier_reward) != Some(target_amount)
            || claim_bond != verifier_reward
        {
            return Err(ChainBaseError::InvalidLogData(
                "canonical bounty economics are inconsistent".to_string(),
            ));
        }
        let mut funded_amount = creation_data["initial_funding"].as_u64().ok_or_else(|| {
            ChainBaseError::InvalidLogData(
                "CanonicalBountyCreated missing initial_funding".to_string(),
            )
        })?;
        let mut status = if target_amount > 0 && funded_amount == target_amount {
            "claimable"
        } else {
            "open"
        };
        let mut timeout_bond_pool = 0u64;
        for event in &events {
            match event.kind {
                AutonomousBountyEventKind::FundingAdded => {
                    funded_amount = event.data["funded_amount"].as_u64().ok_or_else(|| {
                        ChainBaseError::InvalidLogData(
                            "FundingAdded missing funded_amount".to_string(),
                        )
                    })?;
                }
                AutonomousBountyEventKind::ClaimExpired => {
                    status = "claimable";
                    timeout_bond_pool =
                        event.data["timeout_bond_pool"].as_u64().ok_or_else(|| {
                            ChainBaseError::InvalidLogData(
                                "ClaimExpired missing timeout_bond_pool".to_string(),
                            )
                        })?;
                }
                AutonomousBountyEventKind::BountyBecameClaimable
                | AutonomousBountyEventKind::SubmissionExpired
                | AutonomousBountyEventKind::SubmissionRejected => status = "claimable",
                AutonomousBountyEventKind::BountyClaimed => status = "claimed",
                AutonomousBountyEventKind::SubmissionAdded => status = "submitted",
                AutonomousBountyEventKind::BountySettled => {
                    status = "paid";
                    timeout_bond_pool = 0;
                }
                AutonomousBountyEventKind::BountyCancelled => {
                    status = "cancelled";
                    timeout_bond_pool = 0;
                }
                AutonomousBountyEventKind::CanonicalBountyCreated
                | AutonomousBountyEventKind::CanonicalBountyTermsCommitted
                | AutonomousBountyEventKind::CanonicalBountyEconomicsConfigured
                | AutonomousBountyEventKind::CanonicalBountyVerificationConfigured
                | AutonomousBountyEventKind::ExternalBountySubmitted
                | AutonomousBountyEventKind::RefundWithdrawn => {}
            }
        }
        let verification_mode = match creation_data["verification_mode"].as_u64() {
            Some(0) => "deterministic_module",
            Some(1) => "signed_quorum",
            Some(2) => "ai_judge_quorum",
            _ => {
                return Err(ChainBaseError::InvalidLogData(
                    "canonical bounty has an unknown verification mode".to_string(),
                ));
            }
        };
        let zero_address = "0x0000000000000000000000000000000000000000";
        let verifier_module = required_string("verifier_module")?;
        let verifier_module =
            (!verifier_module.eq_ignore_ascii_case(zero_address)).then_some(verifier_module);
        let terms_record = terms.get(&terms_hash.to_ascii_lowercase()).cloned();
        let mut validation_errors =
            validate_autonomous_terms_against_creation(&creation_data, terms_record.as_ref());
        if let Some(error) = active_terms_semantic_error(status, terms_record.as_ref()) {
            validation_errors.push(error);
        }
        let terms_valid = validation_errors.is_empty();
        let (verification_ready, verification_readiness_reason) = if !terms_valid {
            (false, "content-addressed terms are invalid or unavailable")
        } else if verification_mode != "deterministic_module" {
            (
                false,
                "quorum verifier service availability is not canonically attested",
            )
        } else if verifier_module.is_none() {
            (false, "deterministic verifier module is not configured")
        } else {
            (true, "deterministic verifier module is committed on-chain")
        };
        let item = AutonomousBountyFeedItem {
            bounty_id,
            bounty_contract,
            creator,
            status: status.to_string(),
            solver_reward: solver_reward.to_string(),
            verifier_reward: verifier_reward.to_string(),
            claim_bond: claim_bond.to_string(),
            timeout_bond_pool: timeout_bond_pool.to_string(),
            target_amount: target_amount.to_string(),
            funded_amount: funded_amount.to_string(),
            terms_hash: terms_hash.clone(),
            terms: terms_record,
            terms_valid,
            verification_mode: verification_mode.to_string(),
            verifier_module,
            verification_ready,
            verification_readiness_reason: verification_readiness_reason.to_string(),
            validation_errors,
            events,
        };
        if claimable_only && !autonomous_bounty_is_earning_ready(&item) {
            continue;
        }
        feed.push(item);
    }
    feed.sort_by(|left, right| {
        let left_block = left
            .events
            .last()
            .map(|event| event.block_number)
            .unwrap_or(0);
        let right_block = right
            .events
            .last()
            .map(|event| event.block_number)
            .unwrap_or(0);
        right_block.cmp(&left_block)
    });
    Ok(feed)
}

pub fn evm_event_topic(signature: &str) -> String {
    event_topic(signature)
}

pub fn normalize_evm_address(address: impl AsRef<str>) -> Result<String, ChainBaseError> {
    normalize_address(address)
}

pub fn keccak256_canonical_json(value: &Value) -> Result<String, ChainBaseError> {
    let bytes = canonical_json_bytes(value)?;
    Ok(format!("0x{}", hex::encode(Keccak256::digest(bytes))))
}

pub fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, ChainBaseError> {
    serde_json::to_vec(&canonical_json_value(value))
        .map_err(|error| ChainBaseError::InvalidCanonicalJson(error.to_string()))
}

pub fn sha256_utf8(value: &str) -> String {
    format!("0x{}", hex::encode(Sha256::digest(value.as_bytes())))
}

pub fn sha256_canonical_json(value: &Value) -> Result<String, ChainBaseError> {
    let bytes = canonical_json_bytes(value)?;
    Ok(format!("0x{}", hex::encode(Sha256::digest(bytes))))
}

pub fn build_autonomous_bounty_terms_record(
    creator_wallet: &str,
    mut document: AutonomousBountyTermsDocument,
    created_at: DateTime<Utc>,
) -> Result<AutonomousBountyTermsRecord, ChainBaseError> {
    let normalized_creator = normalize_evm_address(creator_wallet)?;
    if document.schema_version != "agent-bounties/terms-v1"
        || document.title.trim().is_empty()
        || document.title.len() > 200
        || document.goal.trim().is_empty()
        || document.goal.len() > 50_000
        || document.acceptance_criteria.is_empty()
        || document.acceptance_criteria.len() > 50
        || document
            .acceptance_criteria
            .iter()
            .any(|criterion| criterion.trim().is_empty() || criterion.len() > 10_000)
        || !document.contract_terms.is_object()
        || !document.verification_policy.is_object()
        || document.benchmark.is_null()
        || document.evidence_schema.is_null()
        || document
            .source_url
            .as_deref()
            .is_some_and(|url| !(url.starts_with("https://") || url.starts_with("http://")))
    {
        return Err(ChainBaseError::InvalidTermsDocument(
            "schema, contract terms, title, goal, criteria, benchmark, evidence schema, policy, or source URL is invalid"
                .to_string(),
        ));
    }
    validate_contract_terms_document(&normalized_creator, &document.contract_terms, created_at)?;
    validate_known_deterministic_module_semantics(&document)?;
    validate_claim_metadata(&mut document)?;
    let document_value = serde_json::to_value(&document)
        .map_err(|error| ChainBaseError::InvalidCanonicalJson(error.to_string()))?;
    if serde_json::to_vec(&document_value)
        .map_err(|error| ChainBaseError::InvalidCanonicalJson(error.to_string()))?
        .len()
        > 256 * 1024
    {
        return Err(ChainBaseError::TermsDocumentTooLarge);
    }
    let acceptance_value = serde_json::to_value(&document.acceptance_criteria)
        .map_err(|error| ChainBaseError::InvalidCanonicalJson(error.to_string()))?;
    Ok(AutonomousBountyTermsRecord {
        terms_hash: keccak256_canonical_json(&document_value)?,
        policy_hash: keccak256_canonical_json(&document.verification_policy)?,
        acceptance_criteria_hash: keccak256_canonical_json(&acceptance_value)?,
        benchmark_hash: keccak256_canonical_json(&document.benchmark)?,
        evidence_schema_hash: keccak256_canonical_json(&document.evidence_schema)?,
        creator_wallet: normalized_creator,
        document,
        created_at,
    })
}

fn leading_zero_work_v1_benchmark() -> Value {
    json!({
        "engine": "leading_zero_work_v1",
        "difficulty_bits": 16,
        "hash_function": "keccak256",
        "preimage_abi_types": [
            "bytes32",
            "uint64",
            "address",
            "bytes32",
            "bytes32",
            "bytes32",
            "uint256"
        ],
        "proof_encoding": "abi.encode(uint256 nonce)",
        "verifier_module": BASE_MAINNET_LEADING_ZERO_WORK_VERIFIER,
        "reference_command": "cargo run -p cli -- autonomous-mine-work-proof"
    })
}

fn validate_known_deterministic_module_semantics(
    document: &AutonomousBountyTermsDocument,
) -> Result<(), ChainBaseError> {
    let Some(policy) = document.verification_policy.as_object() else {
        return Ok(());
    };
    if policy.get("mechanism").and_then(Value::as_str) != Some("deterministic_module") {
        return Ok(());
    }
    let Some(module) = policy.get("verifier_module").and_then(Value::as_str) else {
        return Ok(());
    };
    let Some(contract_terms) = document.contract_terms.as_object() else {
        return Ok(());
    };
    let network = contract_terms_string(contract_terms, "network")?;
    if base_network_descriptor(network)?.chain_id != 8_453
        || !module.eq_ignore_ascii_case(BASE_MAINNET_LEADING_ZERO_WORK_VERIFIER)
    {
        return Ok(());
    }
    let Some(benchmark) = document.benchmark.as_object() else {
        return Err(ChainBaseError::InvalidTermsDocument(
            "the known leading-zero verifier benchmark must be an object".to_string(),
        ));
    };
    let mut semantic_benchmark = benchmark.clone();
    if semantic_benchmark
        .remove("suggested_interface")
        .is_some_and(|value| !value.is_string())
    {
        return Err(ChainBaseError::InvalidTermsDocument(
            "the leading-zero verifier suggested_interface annotation must be a string".to_string(),
        ));
    }
    if Value::Object(semantic_benchmark) != leading_zero_work_v1_benchmark() {
        return Err(ChainBaseError::InvalidTermsDocument(
            "the known leading-zero verifier must use its exact 16-bit scope-bound work benchmark; it cannot verify GitHub CI, task quality, acceptance criteria, or artifact contents"
                .to_string(),
        ));
    }
    Ok(())
}

fn active_terms_semantic_error(
    status: &str,
    terms: Option<&AutonomousBountyTermsRecord>,
) -> Option<String> {
    // Preserve settled history while failing closed before any future earning action.
    if status == "paid" || status == "cancelled" {
        return None;
    }
    terms.and_then(|record| {
        validate_known_deterministic_module_semantics(&record.document)
            .err()
            .map(|error| error.to_string())
    })
}

fn validate_claim_metadata(
    document: &mut AutonomousBountyTermsDocument,
) -> Result<(), ChainBaseError> {
    if let Some(policy) = document.agent_eligibility.as_mut() {
        if policy.required_capabilities.len() > 32
            || policy.wallet_allowlist.len() > 500
            || policy.wallet_denylist.len() > 500
            || policy.maximum_sponsored_bond_base_units > 1_000_000
            || (policy.sponsorship_allowed && policy.maximum_sponsored_bond_base_units == 0)
        {
            return Err(ChainBaseError::InvalidTermsDocument(
                "agent eligibility or sponsorship bounds are invalid".to_string(),
            ));
        }
        for index in 0..policy.required_capabilities.len() {
            if policy.required_capabilities[..index].contains(&policy.required_capabilities[index])
            {
                return Err(ChainBaseError::InvalidTermsDocument(
                    "required agent capabilities must be unique".to_string(),
                ));
            }
        }
        policy.wallet_allowlist = policy
            .wallet_allowlist
            .iter()
            .map(normalize_evm_address)
            .collect::<Result<Vec<_>, _>>()?;
        policy.wallet_denylist = policy
            .wallet_denylist
            .iter()
            .map(normalize_evm_address)
            .collect::<Result<Vec<_>, _>>()?;
        policy.wallet_allowlist.sort();
        policy.wallet_allowlist.dedup();
        policy.wallet_denylist.sort();
        policy.wallet_denylist.dedup();
        if policy
            .wallet_allowlist
            .iter()
            .any(|wallet| policy.wallet_denylist.contains(wallet))
        {
            return Err(ChainBaseError::InvalidTermsDocument(
                "a solver wallet cannot be both allowlisted and denylisted".to_string(),
            ));
        }
    }
    if let Some(policy) = document.claim_coordination.as_ref() {
        if !(60..=86_400).contains(&policy.exclusive_claim_seconds)
            || policy.waitlist_capacity == 0
            || policy.waitlist_capacity > 1_000
            || policy.takeover_grace_seconds > 3_600
        {
            return Err(ChainBaseError::InvalidTermsDocument(
                "claim exclusivity, waitlist, or takeover bounds are invalid".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_contract_terms_document(
    creator_wallet: &str,
    contract_terms: &Value,
    created_at: DateTime<Utc>,
) -> Result<(), ChainBaseError> {
    let object = contract_terms.as_object().ok_or_else(|| {
        ChainBaseError::InvalidTermsDocument("contract_terms must be an object".to_string())
    })?;
    if contract_terms_string(object, "protocol_version")? != "agent-bounties/autonomous-v1" {
        return Err(ChainBaseError::InvalidTermsDocument(
            "contract_terms protocol_version is unsupported".to_string(),
        ));
    }
    let committed_creator =
        normalize_evm_address(contract_terms_string(object, "creator_wallet")?)?;
    if committed_creator != creator_wallet {
        return Err(ChainBaseError::InvalidTermsDocument(
            "contract_terms creator_wallet does not match the publisher".to_string(),
        ));
    }
    let network_name = contract_terms_string(object, "network")?;
    let network = base_network_descriptor(network_name)?;
    let token = normalize_evm_address(contract_terms_string(object, "settlement_token")?)?;
    if token != normalize_evm_address(&network.native_usdc_token_address)? {
        return Err(ChainBaseError::InvalidTermsDocument(
            "contract_terms settlement_token is not native USDC for the committed network"
                .to_string(),
        ));
    }
    let solver_reward = contract_terms_money(object, "solver_reward", false)?;
    let verifier_reward = contract_terms_money(object, "verifier_reward", false)?;
    let claim_bond = contract_terms_money(object, "claim_bond", false)?;
    let initial_funding = contract_terms_money(object, "initial_funding", true)?;
    let target = solver_reward.checked_add(verifier_reward).ok_or_else(|| {
        ChainBaseError::InvalidTermsDocument("contract_terms reward target overflows".to_string())
    })?;
    if claim_bond != verifier_reward || initial_funding > target {
        return Err(ChainBaseError::InvalidTermsDocument(
            "claim bond must equal verifier reward and initial funding cannot exceed target"
                .to_string(),
        ));
    }
    let funding_deadline = contract_terms_u64(object, "funding_deadline")?;
    let claim_window = contract_terms_u64(object, "claim_window_seconds")?;
    let verification_window = contract_terms_u64(object, "verification_window_seconds")?;
    let now = u64::try_from(created_at.timestamp()).map_err(|_| {
        ChainBaseError::InvalidTermsDocument("created_at is before Unix epoch".to_string())
    })?;
    if funding_deadline <= now
        || funding_deadline > now.saturating_add(366 * 24 * 60 * 60)
        || claim_window == 0
        || claim_window > 30 * 24 * 60 * 60
        || verification_window == 0
        || verification_window > 30 * 24 * 60 * 60
    {
        return Err(ChainBaseError::InvalidTermsDocument(
            "contract_terms deadlines or windows are outside protocol bounds".to_string(),
        ));
    }
    let nonce = parse_bytes32(contract_terms_string(object, "creation_nonce")?)?;
    if nonce.iter().all(|byte| *byte == 0) {
        return Err(ChainBaseError::InvalidTermsDocument(
            "contract_terms creation_nonce cannot be zero".to_string(),
        ));
    }
    Ok(())
}

fn contract_terms_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a str, ChainBaseError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            ChainBaseError::InvalidTermsDocument(format!(
                "contract_terms {field} must be a non-empty string"
            ))
        })
}

fn contract_terms_u64(
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<u64, ChainBaseError> {
    object.get(field).and_then(Value::as_u64).ok_or_else(|| {
        ChainBaseError::InvalidTermsDocument(format!(
            "contract_terms {field} must be an unsigned integer"
        ))
    })
}

fn contract_terms_money(
    object: &serde_json::Map<String, Value>,
    field: &str,
    allow_zero: bool,
) -> Result<u64, ChainBaseError> {
    let money = object
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| {
            ChainBaseError::InvalidTermsDocument(format!(
                "contract_terms {field} must be a money object"
            ))
        })?;
    if money.get("currency").and_then(Value::as_str) != Some("usdc") {
        return Err(ChainBaseError::InvalidTermsDocument(format!(
            "contract_terms {field} must use usdc"
        )));
    }
    let amount = money.get("amount").and_then(Value::as_u64).ok_or_else(|| {
        ChainBaseError::InvalidTermsDocument(format!(
            "contract_terms {field}.amount must be an unsigned integer"
        ))
    })?;
    if !allow_zero && amount == 0 {
        return Err(ChainBaseError::InvalidTermsDocument(format!(
            "contract_terms {field}.amount must be positive"
        )));
    }
    Ok(amount)
}

fn canonical_json_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            let mut canonical = serde_json::Map::new();
            for key in keys {
                canonical.insert(key.clone(), canonical_json_value(&map[key]));
            }
            Value::Object(canonical)
        }
        Value::Array(values) => Value::Array(values.iter().map(canonical_json_value).collect()),
        other => other.clone(),
    }
}

pub fn evm_uint256_word(value: u128) -> String {
    word_hex(encode_uint256(value).expect("u128 always fits uint256"))
}

pub fn evm_address_word(address: &str) -> Result<String, ChainBaseError> {
    encode_address(address).map(word_hex)
}

pub fn evm_bytes32_word(value: &str) -> Result<String, ChainBaseError> {
    parse_bytes32(value).map(word_hex)
}

pub fn evm_words_data(words: &[String]) -> Result<String, ChainBaseError> {
    let mut data = String::from("0x");
    for word in words {
        let normalized = normalize_topic(word)?;
        data.push_str(normalized.strip_prefix("0x").expect("word has prefix"));
    }
    Ok(data)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutonomousEventSignature {
    CanonicalBountyCreated,
    CanonicalBountyTermsCommitted,
    CanonicalBountyEconomicsConfigured,
    CanonicalBountyVerificationConfigured,
    ExternalBountySubmitted,
    FundingAdded,
    BountyBecameClaimable,
    BountyClaimed,
    SubmissionAdded,
    SubmissionRejected,
    BountySettled,
    ClaimExpired,
    SubmissionExpired,
    BountyCancelled,
    RefundWithdrawn,
}

fn autonomous_event_signature(topic: &str) -> Option<AutonomousEventSignature> {
    let normalized = normalize_topic(topic).ok()?;
    let signatures = [
        (
            "CanonicalBountyCreated(bytes32,address,address,bytes32,bytes32,bytes32)",
            AutonomousEventSignature::CanonicalBountyCreated,
        ),
        (
            "CanonicalBountyTermsCommitted(bytes32,bytes32,bytes32,bytes32)",
            AutonomousEventSignature::CanonicalBountyTermsCommitted,
        ),
        (
            "CanonicalBountyEconomicsConfigured(bytes32,uint256,uint256,uint256,uint256,uint64,uint64,uint64)",
            AutonomousEventSignature::CanonicalBountyEconomicsConfigured,
        ),
        (
            "CanonicalBountyVerificationConfigured(bytes32,uint8,address,address,uint8,bytes32)",
            AutonomousEventSignature::CanonicalBountyVerificationConfigured,
        ),
        (
            "ExternalBountySubmitted(address,address,bytes32,bytes32,bytes32)",
            AutonomousEventSignature::ExternalBountySubmitted,
        ),
        (
            "FundingAdded(bytes32,address,uint256,uint256,uint256)",
            AutonomousEventSignature::FundingAdded,
        ),
        (
            "BountyBecameClaimable(bytes32,uint256)",
            AutonomousEventSignature::BountyBecameClaimable,
        ),
        (
            "BountyClaimed(bytes32,uint64,address,bytes32,bytes32,uint256,uint64)",
            AutonomousEventSignature::BountyClaimed,
        ),
        (
            "SubmissionAdded(bytes32,uint64,address,bytes32,bytes32,uint64)",
            AutonomousEventSignature::SubmissionAdded,
        ),
        (
            "SubmissionRejected(bytes32,uint64,address,uint256,uint256,bytes32)",
            AutonomousEventSignature::SubmissionRejected,
        ),
        (
            "BountySettled(bytes32,uint64,address,uint256,uint256,uint256,uint256,bytes32,bytes32,bytes32,bytes32)",
            AutonomousEventSignature::BountySettled,
        ),
        (
            "ClaimExpired(bytes32,uint64,address,uint256,uint256)",
            AutonomousEventSignature::ClaimExpired,
        ),
        (
            "SubmissionExpired(bytes32,uint64,address,uint256)",
            AutonomousEventSignature::SubmissionExpired,
        ),
        (
            "BountyCancelled(bytes32,uint256)",
            AutonomousEventSignature::BountyCancelled,
        ),
        (
            "RefundWithdrawn(bytes32,address,uint256,uint256,uint256)",
            AutonomousEventSignature::RefundWithdrawn,
        ),
    ];
    signatures
        .into_iter()
        .find_map(|(signature, kind)| (normalized == event_topic(signature)).then_some(kind))
}

fn require_topic_count(
    log: &EvmLog,
    expected: usize,
    event_name: &str,
) -> Result<(), ChainBaseError> {
    if log.topics.len() != expected {
        return Err(ChainBaseError::InvalidLogTopics(event_name.to_string()));
    }
    Ok(())
}

fn topic_word(log: &EvmLog, index: usize, event_name: &str) -> Result<[u8; 32], ChainBaseError> {
    log.topics
        .get(index)
        .ok_or_else(|| ChainBaseError::InvalidLogTopics(event_name.to_string()))
        .and_then(|topic| {
            parse_bytes32(topic)
                .map_err(|_| ChainBaseError::InvalidLogTopics(event_name.to_string()))
        })
}

fn topic_u64(log: &EvmLog, index: usize, event_name: &str) -> Result<u64, ChainBaseError> {
    word_to_u64(topic_word(log, index, event_name)?, event_name)
}

fn word_to_u64(word: [u8; 32], event_name: &str) -> Result<u64, ChainBaseError> {
    let value =
        word_to_u128(word).map_err(|_| ChainBaseError::InvalidLogData(event_name.to_string()))?;
    u64::try_from(value).map_err(|_| ChainBaseError::InvalidLogData(event_name.to_string()))
}

fn word_to_u8(word: [u8; 32], event_name: &str) -> Result<u8, ChainBaseError> {
    let value =
        word_to_u128(word).map_err(|_| ChainBaseError::InvalidLogData(event_name.to_string()))?;
    u8::try_from(value).map_err(|_| ChainBaseError::InvalidLogData(event_name.to_string()))
}

fn decode_expiry_event(
    log: &EvmLog,
    event_name: &str,
    kind: AutonomousBountyEventKind,
) -> Result<(AutonomousBountyEventKind, String, Value), ChainBaseError> {
    require_topic_count(log, 4, event_name)?;
    let words = decode_words(&log.data, 1, event_name)?;
    Ok((
        kind,
        word_hex(topic_word(log, 1, event_name)?),
        json!({
            "round": topic_u64(log, 2, event_name)?,
            "solver": address_from_word(topic_word(log, 3, event_name)?),
            "claim_bond_refunded": word_to_u128(words[0])?,
        }),
    ))
}

fn event_topic(signature: &str) -> String {
    let mut hasher = Keccak256::new();
    hasher.update(signature.as_bytes());
    format!("0x{}", hex::encode(hasher.finalize()))
}

fn normalize_topic(topic: &str) -> Result<String, ChainBaseError> {
    let word = parse_bytes32(topic)?;
    Ok(word_hex(word))
}

fn normalize_hash(hash: &str) -> Result<String, ChainBaseError> {
    normalize_topic(hash)
}

fn normalize_signed_transaction(transaction: &str) -> Result<String, ChainBaseError> {
    let trimmed = transaction.strip_prefix("0x").ok_or_else(|| {
        ChainBaseError::InvalidSignedTransaction("transaction must have 0x prefix".to_string())
    })?;
    if trimmed.is_empty()
        || !trimmed.len().is_multiple_of(2)
        || !trimmed
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(ChainBaseError::InvalidSignedTransaction(
            transaction.to_string(),
        ));
    }
    Ok(format!("0x{}", trimmed.to_ascii_lowercase()))
}

fn normalize_data(data: &str) -> Result<String, ChainBaseError> {
    let trimmed = data.strip_prefix("0x").unwrap_or(data);
    if !trimmed.len().is_multiple_of(2)
        || !trimmed
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(ChainBaseError::InvalidLogData("EVM RPC log".to_string()));
    }
    Ok(format!("0x{}", trimmed.to_ascii_lowercase()))
}

fn parse_rpc_quantity(value: &str) -> Result<u64, ChainBaseError> {
    let trimmed = value.strip_prefix("0x").ok_or_else(|| {
        ChainBaseError::InvalidRpcQuantity("quantity must have 0x prefix".to_string())
    })?;
    if trimmed.is_empty()
        || !trimmed
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(ChainBaseError::InvalidRpcQuantity(value.to_string()));
    }
    u64::from_str_radix(trimmed, 16)
        .map_err(|_| ChainBaseError::InvalidRpcQuantity(value.to_string()))
}

fn hex_quantity(value: u64) -> String {
    format!("0x{value:x}")
}

fn decode_words(
    data: &str,
    expected_words: usize,
    event_name: &str,
) -> Result<Vec<[u8; 32]>, ChainBaseError> {
    let trimmed = data.strip_prefix("0x").unwrap_or(data);
    if trimmed.len() != expected_words * 64
        || !trimmed
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(ChainBaseError::InvalidLogData(event_name.to_string()));
    }

    (0..expected_words)
        .map(|index| {
            let start = index * 64;
            parse_bytes32(&trimmed[start..start + 64])
                .map_err(|_| ChainBaseError::InvalidLogData(event_name.to_string()))
        })
        .collect()
}

fn word_to_u128(word: [u8; 32]) -> Result<u128, ChainBaseError> {
    if word[..16].iter().any(|byte| *byte != 0) {
        return Err(ChainBaseError::InvalidAmount);
    }
    let mut value = [0u8; 16];
    value.copy_from_slice(&word[16..]);
    Ok(u128::from_be_bytes(value))
}

fn address_from_word(word: [u8; 32]) -> String {
    format!("0x{}", hex::encode(&word[12..]))
}

fn word_hex(word: [u8; 32]) -> String {
    format!("0x{}", hex::encode(word))
}

fn log_key(log: &EvmLog) -> String {
    format!("{}:{}", log.tx_hash.to_ascii_lowercase(), log.log_index)
}

fn deterministic_log_id(log: &EvmLog) -> Id {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, log_key(log).as_bytes())
}

fn normalize_address(address: impl AsRef<str>) -> Result<String, ChainBaseError> {
    let address = address.as_ref();
    let trimmed = address.strip_prefix("0x").unwrap_or(address);
    if trimmed.len() != 40
        || !trimmed
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(ChainBaseError::InvalidAddress(address.to_string()));
    }
    Ok(format!("0x{}", trimmed.to_ascii_lowercase()))
}

fn parse_bytes32(value: &str) -> Result<[u8; 32], ChainBaseError> {
    let trimmed = value.strip_prefix("0x").unwrap_or(value);
    if trimmed.len() != 64
        || !trimmed
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(ChainBaseError::InvalidBytes32(value.to_string()));
    }
    let bytes = hex::decode(trimmed).map_err(|_| ChainBaseError::InvalidBytes32(value.into()))?;
    bytes
        .try_into()
        .map_err(|_| ChainBaseError::InvalidBytes32(value.to_string()))
}

fn money_to_uint256(amount: &Money) -> Result<u128, ChainBaseError> {
    u128::try_from(amount.amount).map_err(|_| ChainBaseError::InvalidAmount)
}

fn autonomous_money_to_uint256(amount: &Money, allow_zero: bool) -> Result<u128, ChainBaseError> {
    if !amount.currency.eq_ignore_ascii_case("usdc") {
        return Err(ChainBaseError::InvalidSettlementCurrency);
    }
    let value = money_to_uint256(amount)?;
    if value == 0 && !allow_zero {
        return Err(ChainBaseError::InvalidAmount);
    }
    Ok(value)
}

fn eip712_field(name: &str, field_type: &str) -> Eip712TypeField {
    Eip712TypeField {
        name: name.to_string(),
        field_type: field_type.to_string(),
    }
}

fn eip3009_typed_data(
    network: &BaseNetworkDescriptor,
    from: &str,
    to: &str,
    value: u128,
    valid_after: u64,
    valid_before: u64,
    nonce: &str,
) -> Eip3009AuthorizationTypedData {
    let mut types = BTreeMap::new();
    types.insert(
        "EIP712Domain".to_string(),
        vec![
            eip712_field("name", "string"),
            eip712_field("version", "string"),
            eip712_field("chainId", "uint256"),
            eip712_field("verifyingContract", "address"),
        ],
    );
    types.insert(
        "TransferWithAuthorization".to_string(),
        vec![
            eip712_field("from", "address"),
            eip712_field("to", "address"),
            eip712_field("value", "uint256"),
            eip712_field("validAfter", "uint256"),
            eip712_field("validBefore", "uint256"),
            eip712_field("nonce", "bytes32"),
        ],
    );
    Eip3009AuthorizationTypedData {
        types,
        domain: Eip712DomainData {
            name: if network.chain_id == 84_532 {
                "USDC".to_string()
            } else {
                "USD Coin".to_string()
            },
            version: "2".to_string(),
            chain_id: network.chain_id,
            verifying_contract: network.native_usdc_token_address.clone(),
        },
        primary_type: "TransferWithAuthorization".to_string(),
        message: Eip3009AuthorizationMessage {
            from: from.to_string(),
            to: to.to_string(),
            value: value.to_string(),
            valid_after: valid_after.to_string(),
            valid_before: valid_before.to_string(),
            nonce: nonce.to_ascii_lowercase(),
        },
    }
}

fn normalized_signature_v(v: u8) -> Result<u8, ChainBaseError> {
    match v {
        0 | 1 => Ok(v + 27),
        27 | 28 => Ok(v),
        _ => Err(ChainBaseError::InvalidVerificationConfiguration(
            "EIP-3009 signature v must be 0, 1, 27, or 28".to_string(),
        )),
    }
}

fn autonomous_create_param_words(
    create: &AutonomousBountyCreate,
) -> Result<Vec<[u8; 32]>, ChainBaseError> {
    let solver_reward = autonomous_money_to_uint256(&create.solver_reward, false)?;
    let verifier_reward = autonomous_money_to_uint256(&create.verifier_reward, true)?;
    let verifier_module = create
        .verifier_module
        .as_deref()
        .unwrap_or("0x0000000000000000000000000000000000000000");
    let verifier_reward_recipient = create
        .verifier_reward_recipient
        .as_deref()
        .unwrap_or("0x0000000000000000000000000000000000000000");
    Ok(vec![
        encode_uint256(solver_reward)?,
        encode_uint256(verifier_reward)?,
        parse_bytes32(&create.terms_hash)?,
        parse_bytes32(&create.policy_hash)?,
        parse_bytes32(&create.acceptance_criteria_hash)?,
        parse_bytes32(&create.benchmark_hash)?,
        parse_bytes32(&create.evidence_schema_hash)?,
        encode_uint256(create.funding_deadline.into())?,
        encode_uint256(create.claim_window_seconds.into())?,
        encode_uint256(create.verification_window_seconds.into())?,
        create.verification_mode.word()?,
        encode_address(verifier_module)?,
        encode_address(verifier_reward_recipient)?,
        encode_uint256(create.threshold.into())?,
    ])
}

fn normalized_verifiers(create: &AutonomousBountyCreate) -> Result<Vec<[u8; 32]>, ChainBaseError> {
    create
        .verifiers
        .iter()
        .map(|verifier| encode_address(verifier))
        .collect()
}

fn validate_autonomous_creation(
    create: &AutonomousBountyCreate,
    verifier_words: &[[u8; 32]],
) -> Result<(), ChainBaseError> {
    let solver_reward = autonomous_money_to_uint256(&create.solver_reward, false)?;
    let verifier_reward = autonomous_money_to_uint256(&create.verifier_reward, true)?;
    let initial_funding = autonomous_money_to_uint256(&create.initial_funding, true)?;
    let target = solver_reward
        .checked_add(verifier_reward)
        .ok_or(ChainBaseError::InvalidAmount)?;
    if target > u128::from(u64::MAX) {
        return Err(ChainBaseError::InvalidAmount);
    }
    if verifier_reward == 0 {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "verifier reward and solver claim bond must be positive".to_string(),
        ));
    }
    if initial_funding > target {
        return Err(ChainBaseError::InitialFundingExceedsTarget);
    }
    if create.funding_deadline == 0
        || create.claim_window_seconds == 0
        || create.verification_window_seconds == 0
    {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "funding deadline and work windows must be positive".to_string(),
        ));
    }
    let zero_address = "0x0000000000000000000000000000000000000000";
    let module = create.verifier_module.as_deref().unwrap_or(zero_address);
    let reward_recipient = create
        .verifier_reward_recipient
        .as_deref()
        .unwrap_or(zero_address);
    match create.verification_mode {
        AutonomousVerificationMode::DeterministicModule => {
            if module.eq_ignore_ascii_case(zero_address)
                || create.threshold != 1
                || !verifier_words.is_empty()
                || (verifier_reward > 0 && reward_recipient.eq_ignore_ascii_case(zero_address))
            {
                return Err(ChainBaseError::InvalidVerificationConfiguration(
                    "deterministic mode requires one module, threshold one, no signer set, and a reward recipient when verifier pay is nonzero".to_string(),
                ));
            }
        }
        AutonomousVerificationMode::SignedQuorum | AutonomousVerificationMode::AiJudgeQuorum => {
            if !module.eq_ignore_ascii_case(zero_address)
                || !reward_recipient.eq_ignore_ascii_case(zero_address)
                || verifier_words.is_empty()
                || verifier_words.len() > 8
                || create.threshold == 0
                || usize::from(create.threshold) > verifier_words.len()
                || verifier_words
                    .iter()
                    .any(|word| word.iter().all(|byte| *byte == 0))
            {
                return Err(ChainBaseError::InvalidVerificationConfiguration(
                    "quorum mode requires one to eight nonzero verifier wallets, a valid threshold, and no module or fixed reward recipient".to_string(),
                ));
            }
            let mut unique = HashSet::new();
            if verifier_words.iter().any(|word| !unique.insert(*word)) {
                return Err(ChainBaseError::InvalidVerificationConfiguration(
                    "verifier wallets must be unique".to_string(),
                ));
            }
            if create.verification_mode == AutonomousVerificationMode::AiJudgeQuorum
                && create.threshold < 2
            {
                return Err(ChainBaseError::InvalidVerificationConfiguration(
                    "AI judge settlement requires at least two signatures".to_string(),
                ));
            }
            if verifier_reward % u128::from(create.threshold) != 0 {
                return Err(ChainBaseError::InvalidVerificationConfiguration(
                    "verifier reward must divide evenly across the threshold".to_string(),
                ));
            }
        }
    }
    Ok(())
}

pub fn validate_autonomous_creation_against_terms(
    network: &str,
    create: &AutonomousBountyCreate,
    terms: &AutonomousBountyTermsRecord,
) -> Result<(), ChainBaseError> {
    validate_known_deterministic_module_semantics(&terms.document)?;
    let hashes_match = create.terms_hash.eq_ignore_ascii_case(&terms.terms_hash)
        && create.policy_hash.eq_ignore_ascii_case(&terms.policy_hash)
        && create
            .acceptance_criteria_hash
            .eq_ignore_ascii_case(&terms.acceptance_criteria_hash)
        && create
            .benchmark_hash
            .eq_ignore_ascii_case(&terms.benchmark_hash)
        && create
            .evidence_schema_hash
            .eq_ignore_ascii_case(&terms.evidence_schema_hash);
    if !hashes_match
        || !normalize_address(&create.creator)?
            .eq_ignore_ascii_case(&normalize_address(&terms.creator_wallet)?)
    {
        return Err(ChainBaseError::InvalidTermsDocument(
            "creator or content commitments do not match the published terms".to_string(),
        ));
    }
    let contract_terms = terms.document.contract_terms.as_object().ok_or_else(|| {
        ChainBaseError::InvalidTermsDocument("published contract_terms are unavailable".to_string())
    })?;
    let network_descriptor = base_network_descriptor(network)?;
    let committed_network =
        base_network_descriptor(contract_terms_string(contract_terms, "network")?)?;
    let committed_token =
        normalize_address(contract_terms_string(contract_terms, "settlement_token")?)?;
    let committed_creator =
        normalize_address(contract_terms_string(contract_terms, "creator_wallet")?)?;
    let solver_reward = autonomous_money_to_uint256(&create.solver_reward, false)?;
    let verifier_reward = autonomous_money_to_uint256(&create.verifier_reward, true)?;
    let initial_funding = autonomous_money_to_uint256(&create.initial_funding, true)?;
    let economics_match = u128::from(contract_terms_money(
        contract_terms,
        "solver_reward",
        false,
    )?) == solver_reward
        && u128::from(contract_terms_money(
            contract_terms,
            "verifier_reward",
            true,
        )?) == verifier_reward
        && u128::from(contract_terms_money(contract_terms, "claim_bond", true)?) == verifier_reward
        && u128::from(contract_terms_money(
            contract_terms,
            "initial_funding",
            true,
        )?) == initial_funding;
    let timing_match = contract_terms_u64(contract_terms, "funding_deadline")?
        == create.funding_deadline
        && contract_terms_u64(contract_terms, "claim_window_seconds")?
            == create.claim_window_seconds
        && contract_terms_u64(contract_terms, "verification_window_seconds")?
            == create.verification_window_seconds;
    let nonce_match = parse_bytes32(contract_terms_string(contract_terms, "creation_nonce")?)?
        == parse_bytes32(&create.creation_nonce)?;
    if committed_network.chain_id != network_descriptor.chain_id
        || committed_token != normalize_address(&network_descriptor.native_usdc_token_address)?
        || committed_creator != normalize_address(&create.creator)?
        || !economics_match
        || !timing_match
        || !nonce_match
    {
        return Err(ChainBaseError::InvalidTermsDocument(
            "contract economics, timing, network, token, creator, or nonce do not match the published terms"
                .to_string(),
        ));
    }
    let policy = &terms.document.verification_policy;
    let expected_mode = match policy.get("mechanism").and_then(Value::as_str) {
        Some("deterministic_module") => AutonomousVerificationMode::DeterministicModule,
        Some("signed_quorum") => AutonomousVerificationMode::SignedQuorum,
        Some("ai_judge_quorum") => AutonomousVerificationMode::AiJudgeQuorum,
        _ => {
            return Err(ChainBaseError::InvalidTermsDocument(
                "published verification mechanism is unsupported".to_string(),
            ))
        }
    };
    let expected_threshold = policy
        .get("threshold")
        .and_then(Value::as_u64)
        .and_then(|value| u8::try_from(value).ok())
        .ok_or_else(|| {
            ChainBaseError::InvalidTermsDocument(
                "published verification threshold is invalid".to_string(),
            )
        })?;
    let zero = "0x0000000000000000000000000000000000000000";
    let expected_module = normalize_address(
        policy
            .get("verifier_module")
            .and_then(Value::as_str)
            .unwrap_or(zero),
    )?;
    let expected_recipient = normalize_address(
        policy
            .get("verifier_reward_recipient")
            .and_then(Value::as_str)
            .unwrap_or(zero),
    )?;
    let actual_module = normalize_address(create.verifier_module.as_deref().unwrap_or(zero))?;
    let actual_recipient =
        normalize_address(create.verifier_reward_recipient.as_deref().unwrap_or(zero))?;
    let expected_verifiers = policy
        .get("verifiers")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| {
                    ChainBaseError::InvalidTermsDocument(
                        "published verifier is not an address".to_string(),
                    )
                })
                .and_then(normalize_address)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let actual_verifiers = create
        .verifiers
        .iter()
        .map(normalize_address)
        .collect::<Result<Vec<_>, _>>()?;
    if create.verification_mode != expected_mode
        || create.threshold != expected_threshold
        || actual_module != expected_module
        || actual_recipient != expected_recipient
        || actual_verifiers != expected_verifiers
    {
        return Err(ChainBaseError::InvalidTermsDocument(
            "contract verification configuration does not match the published policy".to_string(),
        ));
    }
    Ok(())
}

pub fn autonomous_bounty_create_from_terms(
    terms: &AutonomousBountyTermsRecord,
) -> Result<AutonomousBountyCreate, ChainBaseError> {
    validate_known_deterministic_module_semantics(&terms.document)?;
    let contract_terms = terms.document.contract_terms.as_object().ok_or_else(|| {
        ChainBaseError::InvalidTermsDocument("published contract_terms are unavailable".to_string())
    })?;
    let policy = terms
        .document
        .verification_policy
        .as_object()
        .ok_or_else(|| {
            ChainBaseError::InvalidTermsDocument(
                "published verification_policy is unavailable".to_string(),
            )
        })?;
    let mechanism = policy
        .get("mechanism")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ChainBaseError::InvalidTermsDocument(
                "verification_policy mechanism is required".to_string(),
            )
        })?;
    let verification_mode = match mechanism {
        "deterministic_module" => AutonomousVerificationMode::DeterministicModule,
        "signed_quorum" => AutonomousVerificationMode::SignedQuorum,
        "ai_judge_quorum" => AutonomousVerificationMode::AiJudgeQuorum,
        _ => {
            return Err(ChainBaseError::InvalidTermsDocument(
                "verification_policy mechanism is unsupported".to_string(),
            ))
        }
    };
    let optional_policy_address = |field: &str| -> Result<Option<String>, ChainBaseError> {
        match policy.get(field) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(value)) => Ok(Some(normalize_address(value)?)),
            _ => Err(ChainBaseError::InvalidTermsDocument(format!(
                "verification_policy {field} must be an address or null"
            ))),
        }
    };
    let verifiers = policy
        .get("verifiers")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ChainBaseError::InvalidTermsDocument(
                "verification_policy verifiers must be an array".to_string(),
            )
        })?
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| {
                    ChainBaseError::InvalidTermsDocument(
                        "verification_policy verifier must be an address".to_string(),
                    )
                })
                .and_then(normalize_address)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let threshold = policy
        .get("threshold")
        .and_then(Value::as_u64)
        .and_then(|value| u8::try_from(value).ok())
        .ok_or_else(|| {
            ChainBaseError::InvalidTermsDocument(
                "verification_policy threshold must fit uint8".to_string(),
            )
        })?;
    let money = |field: &str, allow_zero: bool| -> Result<Money, ChainBaseError> {
        let amount = contract_terms_money(contract_terms, field, allow_zero)?;
        let amount = i64::try_from(amount).map_err(|_| {
            ChainBaseError::InvalidTermsDocument(format!(
                "contract_terms {field} exceeds the supported amount range"
            ))
        })?;
        Ok(if amount == 0 {
            Money::zero("usdc")
        } else {
            Money::new(amount, "usdc").map_err(|_| ChainBaseError::InvalidAmount)?
        })
    };
    let create = AutonomousBountyCreate {
        creator: normalize_address(&terms.creator_wallet)?,
        solver_reward: money("solver_reward", false)?,
        verifier_reward: money("verifier_reward", false)?,
        terms_hash: terms.terms_hash.clone(),
        policy_hash: terms.policy_hash.clone(),
        acceptance_criteria_hash: terms.acceptance_criteria_hash.clone(),
        benchmark_hash: terms.benchmark_hash.clone(),
        evidence_schema_hash: terms.evidence_schema_hash.clone(),
        funding_deadline: contract_terms_u64(contract_terms, "funding_deadline")?,
        claim_window_seconds: contract_terms_u64(contract_terms, "claim_window_seconds")?,
        verification_window_seconds: contract_terms_u64(
            contract_terms,
            "verification_window_seconds",
        )?,
        verification_mode,
        verifier_module: optional_policy_address("verifier_module")?,
        verifier_reward_recipient: optional_policy_address("verifier_reward_recipient")?,
        verifiers,
        threshold,
        initial_funding: money("initial_funding", true)?,
        creation_nonce: contract_terms_string(contract_terms, "creation_nonce")?.to_string(),
    };
    let network = contract_terms_string(contract_terms, "network")?;
    validate_autonomous_creation_against_terms(network, &create, terms)?;
    Ok(create)
}

fn standing_meta_v2_publish_terms_intent(
    publisher: &str,
    canonical_terms: &[u8],
    parent: &StandingMetaV2ParentContext,
    terms: &AutonomousBountyTermsRecord,
    verifier_set_hash: &str,
    verifier_threshold: u8,
) -> Result<EvmTransactionIntent, ChainBaseError> {
    const FUNCTION: &str =
        "publish(bytes,(bytes32,uint64,bytes32,bytes32,bytes32,bytes32,bytes32,uint8))";
    let mut bytes = selector(FUNCTION).to_vec();
    bytes.extend_from_slice(&encode_uint256(9 * 32)?);
    for word in [
        parse_bytes32(&parent.bounty_id)?,
        encode_uint256(parent.round.into())?,
        parse_bytes32(&terms.policy_hash)?,
        parse_bytes32(&terms.acceptance_criteria_hash)?,
        parse_bytes32(&terms.benchmark_hash)?,
        parse_bytes32(&terms.evidence_schema_hash)?,
        parse_bytes32(verifier_set_hash)?,
        encode_uint256(verifier_threshold.into())?,
    ] {
        bytes.extend_from_slice(&word);
    }
    bytes.extend_from_slice(&encode_uint256(canonical_terms.len() as u128)?);
    bytes.extend_from_slice(canonical_terms);
    let padding = (32 - canonical_terms.len() % 32) % 32;
    bytes.resize(bytes.len() + padding, 0);
    Ok(EvmTransactionIntent {
        from: Some(normalize_address(publisher)?),
        to: BASE_MAINNET_STANDING_META_V2_TERMS_REGISTRY.to_string(),
        value_wei: 0,
        data: format!("0x{}", hex::encode(bytes)),
        function: FUNCTION.to_string(),
    })
}

fn encode_autonomous_create_call(
    params: &[[u8; 32]],
    verifiers: &[[u8; 32]],
    initial_funding: u128,
    creation_nonce: [u8; 32],
) -> Result<String, ChainBaseError> {
    const SIGNATURE: &str = "createBounty((uint256,uint256,bytes32,bytes32,bytes32,bytes32,bytes32,uint64,uint64,uint64,uint8,address,address,uint8),address[],uint256,bytes32)";
    if params.len() != 14 {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "factory parameter tuple must contain fourteen words".to_string(),
        ));
    }
    let mut bytes = selector(SIGNATURE).to_vec();
    for word in params {
        bytes.extend_from_slice(word);
    }
    bytes.extend_from_slice(&encode_uint256(17 * 32)?);
    bytes.extend_from_slice(&encode_uint256(initial_funding)?);
    bytes.extend_from_slice(&creation_nonce);
    bytes.extend_from_slice(&encode_uint256(verifiers.len() as u128)?);
    for verifier in verifiers {
        bytes.extend_from_slice(verifier);
    }
    Ok(format!("0x{}", hex::encode(bytes)))
}

#[allow(clippy::too_many_arguments)]
fn encode_autonomous_authorized_create_call(
    creator: &str,
    params: &[[u8; 32]],
    verifiers: &[[u8; 32]],
    initial_funding: u128,
    creation_nonce: [u8; 32],
    valid_after: u64,
    valid_before: u64,
    v: u8,
    r: [u8; 32],
    s: [u8; 32],
) -> Result<String, ChainBaseError> {
    const SIGNATURE: &str = "createBountyWithAuthorization(address,(uint256,uint256,bytes32,bytes32,bytes32,bytes32,bytes32,uint64,uint64,uint64,uint8,address,address,uint8),address[],uint256,bytes32,(uint256,uint256,bytes32,uint8,bytes32,bytes32))";
    if params.len() != 14 {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "factory parameter tuple must contain fourteen words".to_string(),
        ));
    }
    let mut bytes = selector(SIGNATURE).to_vec();
    bytes.extend_from_slice(&encode_address(creator)?);
    for word in params {
        bytes.extend_from_slice(word);
    }
    bytes.extend_from_slice(&encode_uint256(24 * 32)?);
    bytes.extend_from_slice(&encode_uint256(initial_funding)?);
    bytes.extend_from_slice(&creation_nonce);
    bytes.extend_from_slice(&encode_uint256(valid_after.into())?);
    bytes.extend_from_slice(&encode_uint256(valid_before.into())?);
    bytes.extend_from_slice(&creation_nonce);
    bytes.extend_from_slice(&encode_uint256(v.into())?);
    bytes.extend_from_slice(&r);
    bytes.extend_from_slice(&s);
    bytes.extend_from_slice(&encode_uint256(verifiers.len() as u128)?);
    for verifier in verifiers {
        bytes.extend_from_slice(verifier);
    }
    Ok(format!("0x{}", hex::encode(bytes)))
}

fn autonomous_bounty_id(
    chain_id: u64,
    factory: &str,
    creator: &str,
    creation_nonce: [u8; 32],
    params: &[[u8; 32]],
    verifiers: &[[u8; 32]],
) -> Result<[u8; 32], ChainBaseError> {
    if params.len() != 14 {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "factory parameter tuple must contain fourteen words".to_string(),
        ));
    }
    let mut encoded = Vec::with_capacity((20 + verifiers.len()) * 32);
    encoded.extend_from_slice(&encode_uint256(chain_id.into())?);
    encoded.extend_from_slice(&encode_address(factory)?);
    encoded.extend_from_slice(&encode_address(creator)?);
    encoded.extend_from_slice(&creation_nonce);
    for word in params {
        encoded.extend_from_slice(word);
    }
    encoded.extend_from_slice(&encode_uint256(19 * 32)?);
    encoded.extend_from_slice(&encode_uint256(verifiers.len() as u128)?);
    for verifier in verifiers {
        encoded.extend_from_slice(verifier);
    }
    Ok(Keccak256::digest(encoded).into())
}

fn predict_minimal_proxy_address(
    factory: &str,
    implementation: &str,
    salt: [u8; 32],
) -> Result<String, ChainBaseError> {
    let implementation = normalize_address(implementation)?;
    let factory = normalize_address(factory)?;
    let mut init_code = hex::decode("3d602d80600a3d3981f3363d3d373d3d3d363d73")
        .expect("minimal proxy prefix is valid hex");
    init_code.extend_from_slice(
        &hex::decode(&implementation[2..]).expect("normalized implementation is valid hex"),
    );
    init_code.extend_from_slice(
        &hex::decode("5af43d82803e903d91602b57fd5bf3").expect("minimal proxy suffix is valid hex"),
    );
    let init_code_hash = Keccak256::digest(init_code);
    let mut preimage = Vec::with_capacity(85);
    preimage.push(0xff);
    preimage
        .extend_from_slice(&hex::decode(&factory[2..]).expect("normalized factory is valid hex"));
    preimage.extend_from_slice(&salt);
    preimage.extend_from_slice(&init_code_hash);
    let hash = Keccak256::digest(preimage);
    Ok(format!("0x{}", hex::encode(&hash[12..])))
}

fn encode_call(signature: &str, words: Vec<[u8; 32]>) -> String {
    let mut bytes = selector(signature).to_vec();
    for word in words {
        bytes.extend_from_slice(&word);
    }
    format!("0x{}", hex::encode(bytes))
}

fn validate_atomic_sponsor_grant(grant: &AtomicClaimSponsorGrant) -> Result<(), ChainBaseError> {
    if grant.round == 0
        || grant.bond == 0
        || grant.valid_before <= grant.valid_after
        || grant.deadline <= grant.valid_after
        || grant.deadline > grant.valid_before
    {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "atomic sponsorship grant has invalid amount, round, or validity bounds".to_string(),
        ));
    }
    normalize_address(&grant.sponsor_contract)?;
    normalize_address(&grant.bounty_contract)?;
    normalize_address(&grant.solver)?;
    parse_bytes32(&grant.terms_hash)?;
    parse_bytes32(&grant.policy_hash)?;
    parse_bytes32(&grant.authorization_nonce)?;
    parse_bytes32(&grant.grant_nonce)?;
    Ok(())
}

fn atomic_sponsor_grant_digest(
    chain_id: u64,
    factory_contract: &str,
    grant: &AtomicClaimSponsorGrant,
) -> Result<String, ChainBaseError> {
    validate_atomic_sponsor_grant(grant)?;
    let sponsor = normalize_address(&grant.sponsor_contract)?;
    let factory = normalize_address(factory_contract)?;
    let bounty = normalize_address(&grant.bounty_contract)?;
    let solver = normalize_address(&grant.solver)?;
    let domain_typehash = keccak_word(
        b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );
    let grant_typehash = keccak_word(
        b"SponsoredClaim(address sponsor,address factory,address bounty,address solver,uint64 round,uint256 bond,bytes32 termsHash,bytes32 policyHash,bytes32 authorizationNonce,uint256 validAfter,uint256 validBefore,bytes32 grantNonce,uint256 deadline)",
    );
    let domain_separator = keccak_words(&[
        domain_typehash,
        keccak_word(b"Agent Bounties Atomic Claim Sponsor"),
        keccak_word(b"1"),
        encode_uint256(chain_id.into())?,
        encode_address(&sponsor)?,
    ]);
    let struct_hash = keccak_words(&[
        grant_typehash,
        encode_address(&sponsor)?,
        encode_address(&factory)?,
        encode_address(&bounty)?,
        encode_address(&solver)?,
        encode_uint256(grant.round.into())?,
        encode_uint256(grant.bond)?,
        parse_bytes32(&grant.terms_hash)?,
        parse_bytes32(&grant.policy_hash)?,
        parse_bytes32(&grant.authorization_nonce)?,
        encode_uint256(grant.valid_after.into())?,
        encode_uint256(grant.valid_before.into())?,
        parse_bytes32(&grant.grant_nonce)?,
        encode_uint256(grant.deadline.into())?,
    ]);
    let mut digest = Vec::with_capacity(66);
    digest.extend_from_slice(&[0x19, 0x01]);
    digest.extend_from_slice(&domain_separator);
    digest.extend_from_slice(&struct_hash);
    Ok(format!("0x{}", hex::encode(Keccak256::digest(digest))))
}

fn encode_atomic_sponsored_claim_call(
    grant: &AtomicClaimSponsorGrant,
    grant_signature: &[u8],
    solver_signature: &AutonomousBountyAuthorizationSignature,
) -> Result<String, ChainBaseError> {
    const FUNCTION: &str = "sponsorAndClaim((address,address,uint64,uint256,bytes32,bytes32,bytes32,uint256,uint256,bytes32,uint256),bytes,uint8,bytes32,bytes32)";
    let mut bytes = selector(FUNCTION).to_vec();
    let words = [
        encode_address(&grant.bounty_contract)?,
        encode_address(&grant.solver)?,
        encode_uint256(grant.round.into())?,
        encode_uint256(grant.bond)?,
        parse_bytes32(&grant.terms_hash)?,
        parse_bytes32(&grant.policy_hash)?,
        parse_bytes32(&grant.authorization_nonce)?,
        encode_uint256(grant.valid_after.into())?,
        encode_uint256(grant.valid_before.into())?,
        parse_bytes32(&grant.grant_nonce)?,
        encode_uint256(grant.deadline.into())?,
        encode_uint256(480_u128)?,
        encode_uint256(normalized_signature_v(solver_signature.v)?.into())?,
        parse_bytes32(&solver_signature.r)?,
        parse_bytes32(&solver_signature.s)?,
    ];
    for word in words {
        bytes.extend_from_slice(&word);
    }
    bytes.extend_from_slice(&encode_uint256(grant_signature.len() as u128)?);
    bytes.extend_from_slice(grant_signature);
    let padding = (32 - grant_signature.len() % 32) % 32;
    bytes.resize(bytes.len() + padding, 0);
    Ok(format!("0x{}", hex::encode(bytes)))
}

fn keccak_word(value: &[u8]) -> [u8; 32] {
    Keccak256::digest(value).into()
}

fn keccak_words(words: &[[u8; 32]]) -> [u8; 32] {
    let mut bytes = Vec::with_capacity(words.len() * 32);
    for word in words {
        bytes.extend_from_slice(word);
    }
    Keccak256::digest(bytes).into()
}

#[derive(Debug, Clone)]
struct EncodedAutonomousAttestation {
    verifier: [u8; 32],
    passed: [u8; 32],
    response_hash: [u8; 32],
    deadline: [u8; 32],
    signature: Vec<u8>,
}

fn encode_single_dynamic_bytes_call(signature: &str, value: &[u8]) -> String {
    let mut bytes = selector(signature).to_vec();
    bytes.extend_from_slice(&encode_uint256(32).expect("constant fits"));
    bytes.extend_from_slice(&encode_uint256(value.len() as u128).expect("length fits"));
    bytes.extend_from_slice(value);
    let padding = (32 - value.len() % 32) % 32;
    bytes.resize(bytes.len() + padding, 0);
    format!("0x{}", hex::encode(bytes))
}

fn encode_attestation_array_call(attestations: &[EncodedAutonomousAttestation]) -> String {
    const SIGNATURE: &str = "settleWithAttestations((address,bool,bytes32,uint256,bytes)[])";
    let tuples = attestations
        .iter()
        .map(|attestation| {
            let mut tuple = Vec::with_capacity(32 * 7);
            tuple.extend_from_slice(&attestation.verifier);
            tuple.extend_from_slice(&attestation.passed);
            tuple.extend_from_slice(&attestation.response_hash);
            tuple.extend_from_slice(&attestation.deadline);
            tuple.extend_from_slice(&encode_uint256(5 * 32).expect("constant fits"));
            tuple.extend_from_slice(
                &encode_uint256(attestation.signature.len() as u128).expect("length fits"),
            );
            tuple.extend_from_slice(&attestation.signature);
            let padding = (32 - attestation.signature.len() % 32) % 32;
            tuple.resize(tuple.len() + padding, 0);
            tuple
        })
        .collect::<Vec<_>>();

    let mut bytes = selector(SIGNATURE).to_vec();
    bytes.extend_from_slice(&encode_uint256(32).expect("constant fits"));
    bytes.extend_from_slice(&encode_uint256(tuples.len() as u128).expect("length fits"));
    let mut offset = tuples.len() * 32;
    for tuple in &tuples {
        bytes.extend_from_slice(&encode_uint256(offset as u128).expect("offset fits"));
        offset += tuple.len();
    }
    for tuple in tuples {
        bytes.extend_from_slice(&tuple);
    }
    format!("0x{}", hex::encode(bytes))
}

fn selector(signature: &str) -> [u8; 4] {
    let mut hasher = Keccak256::new();
    hasher.update(signature.as_bytes());
    let hash = hasher.finalize();
    [hash[0], hash[1], hash[2], hash[3]]
}

fn encode_address(address: &str) -> Result<[u8; 32], ChainBaseError> {
    let normalized = normalize_address(address)?;
    let raw = hex::decode(&normalized[2..])
        .map_err(|_| ChainBaseError::InvalidAddress(address.to_string()))?;
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(&raw);
    Ok(word)
}

fn encode_uint256(value: u128) -> Result<[u8; 32], ChainBaseError> {
    let mut word = [0u8; 32];
    word[16..].copy_from_slice(&value.to_be_bytes());
    Ok(word)
}

fn encode_bool(value: bool) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[31] = u8::from(value);
    word
}

fn parse_hex_bytes(value: &str) -> Result<Vec<u8>, ChainBaseError> {
    let Some(raw) = value.strip_prefix("0x") else {
        return Err(ChainBaseError::InvalidHexBytes(value.to_string()));
    };
    if raw.len() % 2 != 0 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ChainBaseError::InvalidHexBytes(value.to_string()));
    }
    hex::decode(raw).map_err(|_| ChainBaseError::InvalidHexBytes(value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::Money;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    #[test]
    fn hosted_relayer_derives_public_address_and_redacts_private_key() {
        let private_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let relayer = BaseTransactionRelayer::from_private_key(private_key).unwrap();
        assert_eq!(
            relayer.address(),
            "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"
        );
        let debug = format!("{relayer:?}");
        assert!(debug.contains(&relayer.address()));
        assert!(!debug.contains(private_key));
        assert_eq!(
            BaseTransactionRelayer::from_private_key("not-a-private-key").unwrap_err(),
            ChainBaseError::InvalidRelayerPrivateKey
        );
    }

    fn atomic_sponsor_vector() -> AtomicClaimSponsorGrant {
        AtomicClaimSponsorGrant {
            sponsor_contract: "0x1111111111111111111111111111111111111111".to_string(),
            bounty_contract: "0x3333333333333333333333333333333333333333".to_string(),
            solver: "0x4444444444444444444444444444444444444444".to_string(),
            round: 7,
            bond: 10_000,
            terms_hash: format!("0x{}", "aa".repeat(32)),
            policy_hash: format!("0x{}", "bb".repeat(32)),
            authorization_nonce: format!("0x{}", "cc".repeat(32)),
            valid_after: 0,
            valid_before: 2_000_000_000,
            grant_nonce: format!("0x{}", "dd".repeat(32)),
            deadline: 1_999_999_700,
        }
    }

    #[test]
    fn atomic_sponsor_digest_matches_independent_cast_vector() {
        let planner = AutonomousBountyTxPlanner::new(
            "0x2222222222222222222222222222222222222222",
            "0x5555555555555555555555555555555555555555",
        )
        .unwrap();
        assert_eq!(
            planner
                .atomic_sponsor_grant_digest("base-mainnet", &atomic_sponsor_vector())
                .unwrap(),
            "0xe37f8bbafd2b096b83b5485185e1af53e8ff12747d508afc2c42ac7aabdd3750"
        );
    }

    #[test]
    fn atomic_sponsor_plan_is_one_bounded_vault_call() {
        let planner = AutonomousBountyTxPlanner::new(
            "0x2222222222222222222222222222222222222222",
            "0x5555555555555555555555555555555555555555",
        )
        .unwrap();
        let signer = BaseTransactionRelayer::from_private_key(
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        )
        .unwrap();
        let grant = atomic_sponsor_vector();
        let digest = planner
            .atomic_sponsor_grant_digest("base-mainnet", &grant)
            .unwrap();
        let grant_signature = signer.sign_digest(&digest).unwrap();
        let grant_signature_bytes = parse_hex_bytes(&grant_signature).unwrap();
        assert_eq!(grant_signature_bytes.len(), 65);
        assert!(matches!(grant_signature_bytes[64], 27 | 28));

        let solver_signature = AutonomousBountyAuthorizationSignature {
            v: 27,
            r: format!("0x{}", "ee".repeat(32)),
            s: format!("0x{}", "0f".repeat(32)),
        };
        let relayer = "0x6666666666666666666666666666666666666666";
        let plan = planner
            .plan_atomic_sponsored_claim(
                "base-mainnet",
                &grant,
                &grant_signature,
                &solver_signature,
                relayer,
            )
            .unwrap();
        assert_eq!(plan.grant_digest, digest);
        assert_eq!(plan.sponsor_contract, grant.sponsor_contract);
        assert_eq!(plan.relay_transaction.from.as_deref(), Some(relayer));
        assert_eq!(plan.relay_transaction.to, grant.sponsor_contract);
        assert_eq!(plan.relay_transaction.value_wei, 0);
        assert!(plan.relay_transaction.data.starts_with("0xba3ddedd"));
        assert_eq!(
            parse_hex_bytes(&plan.relay_transaction.data).unwrap().len(),
            612
        );
    }

    #[test]
    fn hosted_relayer_provider_errors_redact_rpc_credentials() {
        let error = sanitize_relayer_provider_error(
            "request to https://user:secret@rpc.example/v2/api-key?token=private failed; fallback wss://rpc.example/ws-key unavailable",
        );
        let ChainBaseError::RelayerProvider(message) = error else {
            panic!("expected a relayer provider error");
        };
        assert_eq!(
            message,
            "request to [redacted-url] failed; fallback [redacted-url] unavailable"
        );
        assert!(!message.contains("secret"));
        assert!(!message.contains("api-key"));
        assert!(!message.contains("ws-key"));
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_RPC_URL backed by an unlocked Anvil chain"]
    async fn hosted_relayer_rehearsal_broadcasts_bounded_zero_value_transaction() {
        let rpc_url = std::env::var("AGENT_BOUNTIES_TEST_RPC_URL").unwrap();
        let private_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let relayer = BaseTransactionRelayer::from_private_key(private_key).unwrap();
        let intent = EvmTransactionIntent {
            from: Some(relayer.address()),
            to: relayer.address(),
            value_wei: 0,
            data: "0x12345678".to_string(),
            function: "boundedRelayHarness()".to_string(),
        };
        let transaction = relayer
            .simulate_and_broadcast(&rpc_url, 31_337, &intent, 100_000, 100_000_000_000)
            .await
            .unwrap();
        assert_eq!(transaction.relayer, relayer.address());
        assert!(transaction.gas_limit <= 100_000);
        assert!(transaction.max_fee_per_gas_wei <= 100_000_000_000);

        let mut receipt = None;
        for request_id in 1..=30 {
            receipt = fetch_transaction_receipt(&rpc_url, &transaction.tx_hash, request_id)
                .await
                .unwrap()
                .result;
            if receipt.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let receipt = receipt.expect("Anvil mined the relay transaction within three seconds");
        assert_eq!(receipt.succeeded().unwrap(), Some(true));
    }

    #[test]
    fn function_selectors_match_solidity_contract() {
        assert_eq!(
            hex::encode(selector("createBounty((uint256,uint256,bytes32,bytes32,bytes32,bytes32,bytes32,uint64,uint64,uint64,uint8,address,address,uint8),address[],uint256,bytes32)")),
            "9d2e414c"
        );
        assert_eq!(hex::encode(selector("fund(uint256)")), "ca1d209d");
        assert_eq!(
            hex::encode(selector(AUTONOMOUS_FUND_WITH_AUTHORIZATION_FUNCTION)),
            AUTONOMOUS_FUND_WITH_AUTHORIZATION_SELECTOR
        );
        assert_eq!(hex::encode(selector("claim()")), "4e71d92d");
        assert_eq!(hex::encode(selector("submit(bytes32,bytes32)")), "d26ff86e");
        assert_eq!(hex::encode(selector("verifyAndSettle(bytes)")), "ed827cee");
        assert_eq!(
            hex::encode(selector(
                "settleWithAttestations((address,bool,bytes32,uint256,bytes)[])"
            )),
            "e3457186"
        );
    }

    #[test]
    fn canonical_json_commitments_ignore_object_key_insertion_order() {
        let left = serde_json::from_str::<Value>(r#"{"z":1,"a":{"y":2,"b":3}}"#).unwrap();
        let right = serde_json::from_str::<Value>(r#"{"a":{"b":3,"y":2},"z":1}"#).unwrap();
        assert_eq!(
            keccak256_canonical_json(&left).unwrap(),
            keccak256_canonical_json(&right).unwrap()
        );
    }

    #[test]
    fn canonical_child_terms_plan_matches_solidity_commitments() {
        let plan = plan_canonical_child_bounty_terms(&CanonicalChildBountyTermsRequest {
            parent_bounty_id: "0x0000000000000000000000000000000000000000000000000000000000abcdef"
                .to_string(),
            parent_round: 0x0123456789abcdef,
            parent_solver: "0x3333333333333333333333333333333333333333".to_string(),
            parent_solver_reward: Money::new(900_000, "usdc").unwrap(),
            child_acceptance_criteria: vec![
                "Run the deterministic fixture suite.".to_string(),
                "Publish the passing output hash.".to_string(),
            ],
            verifier_module: "0x4444444444444444444444444444444444444444".to_string(),
        })
        .unwrap();

        assert_eq!(
            plan.acceptance_criteria_hash,
            keccak256_canonical_json(&json!(plan.acceptance_criteria)).unwrap()
        );
        assert_eq!(
            serde_json::to_string(&plan.benchmark).unwrap(),
            r#"{"parent_bounty_id":"0x0000000000000000000000000000000000000000000000000000000000abcdef","parent_round_hex":"0x0123456789abcdef","protocol":"agent-bounties/canonical-child-v1"}"#
        );
        assert_eq!(
            plan.benchmark_hash,
            keccak256_canonical_json(&plan.benchmark).unwrap()
        );
        assert_eq!(plan.minimum_child_target.amount, 900_000);
        assert_eq!(plan.required_child_status, "settled");
    }

    #[test]
    fn standing_meta_v2_child_preparation_is_exact_and_fully_funded() {
        let planner = AutonomousBountyTxPlanner::new(
            BASE_MAINNET_AUTONOMOUS_BOUNTY_FACTORY,
            BASE_MAINNET_AUTONOMOUS_BOUNTY_IMPLEMENTATION,
        )
        .unwrap();
        let created_at = DateTime::parse_from_rfc3339("2026-07-17T06:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let request = StandingMetaV2ChildPreparationRequest {
            network: Some("base-mainnet".to_string()),
            parent_bounty_contract: "0x43d42cb227d76588ab16693f14efd6cff851fa7a".to_string(),
            parent_solver: "0x1111111111111111111111111111111111111111".to_string(),
            intended_child_solver: "0x2222222222222222222222222222222222222222".to_string(),
            title: "Fix one deterministic parser regression".to_string(),
            goal: "Make the pinned failing fixture pass without weakening its assertion."
                .to_string(),
            acceptance_criteria: vec![
                "The pinned regression command exits zero.".to_string(),
                "The patch adds or preserves a failing-before, passing-after fixture.".to_string(),
            ],
            benchmark_source: StandingMetaV2BenchmarkSource {
                kind: "github_commit".to_string(),
                repository: "NSPG13/agent-bounties".to_string(),
                commit: "a".repeat(40),
                subdirectory: "crates/chain-base/tests".to_string(),
            },
            runner_manifest: RegressionSandboxPolicy {
                schema_version: "agent-bounties/regression-sandbox-v1".to_string(),
                image: format!("docker.io/library/alpine@sha256:{}", "b".repeat(64)),
                command: vec!["true".to_string()],
                workdir: "/workspace".to_string(),
                benchmark_digest: format!("sha256:{}", "c".repeat(64)),
                timeout_seconds: 30,
                cpu_millis: 500,
                memory_bytes: 128 * 1024 * 1024,
                pids_limit: 32,
                max_output_bytes: 64 * 1024,
                tmpfs_bytes: 64 * 1024 * 1024,
                max_source_bytes: 1024 * 1024,
                max_source_files: 100,
                max_benchmark_bytes: 1024 * 1024,
                max_benchmark_files: 100,
                platform: "linux/amd64".to_string(),
                test_seed: 7,
            },
            evidence_schema: None,
            verifier_reward: None,
            funding_deadline: None,
            claim_window_seconds: None,
            verification_window_seconds: None,
            creation_nonce: None,
            nonce_salt: Some("fixture-one".to_string()),
            source_url: Some("https://github.com/NSPG13/agent-bounties/issues/335".to_string()),
            discovery_source: None,
        };
        let parent = StandingMetaV2ParentContext {
            bounty_contract: request.parent_bounty_contract.clone(),
            bounty_id: "0x12ad2fa99de272728311a3eb07c3c741048382260cb91ba1e8f001ed3b5759d0"
                .to_string(),
            creator: "0x3333333333333333333333333333333333333333".to_string(),
            round: 1,
            solver_reward: Money::new(900_000, "usdc").unwrap(),
            funding_deadline: 1_791_676_800,
        };

        let plan = planner
            .plan_standing_meta_v2_child(&request, &parent, created_at)
            .unwrap();

        assert_eq!(plan.protocol_version, STANDING_META_V2_PROTOCOL_VERSION);
        assert_eq!(plan.task_verifier_threshold, 2);
        assert_eq!(
            plan.task_verifier_set_hash,
            BASE_MAINNET_STANDING_META_V2_VERIFIER_SET_HASH
        );
        assert_eq!(plan.task_verifiers, BASE_MAINNET_STANDING_META_V2_VERIFIERS);
        assert_eq!(plan.child_create.solver_reward.amount, 800_000);
        assert_eq!(plan.child_create.verifier_reward.amount, 100_000);
        assert_eq!(plan.child_create.initial_funding.amount, 900_000);
        assert_eq!(
            plan.child_create.verification_mode,
            AutonomousVerificationMode::SignedQuorum
        );
        assert_eq!(plan.child_create.threshold, 2);
        assert_eq!(plan.child_create.verifier_module, None);
        assert_eq!(plan.pre_claim_wallet_calls.len(), 3);
        assert_eq!(&plan.publish_terms.data[..10], "0x16d0f49a");
        assert_eq!(&plan.publish_terms.data[10..74], format!("{:064x}", 9 * 32));
        assert_eq!(
            plan.terms.terms_hash,
            format!(
                "0x{}",
                hex::encode(Keccak256::digest(
                    hex::decode(&plan.canonical_terms_hex[2..]).unwrap()
                ))
            )
        );
        assert_eq!(
            plan.terms.document.benchmark["parent_binding"]["parent_bounty_id"],
            parent.bounty_id
        );
        assert_eq!(
            plan.terms.document.benchmark["source"]["commit"],
            "a".repeat(40)
        );
        assert!(plan.parent_claim_timing.strict_timestamp_ordering);
        assert!(!plan.parent_claim_timing.same_block_claim_allowed);
        assert!(plan.next_action.contains("strictly later timestamp"));
        assert!(plan.child_creation.supports_single_wallet_batch);
        assert!(!plan.hosted_terms_published);
    }

    #[test]
    fn standing_meta_v2_child_preparation_rejects_same_solver_or_mutable_runner() {
        let planner = AutonomousBountyTxPlanner::new(
            BASE_MAINNET_AUTONOMOUS_BOUNTY_FACTORY,
            BASE_MAINNET_AUTONOMOUS_BOUNTY_IMPLEMENTATION,
        )
        .unwrap();
        let created_at = DateTime::parse_from_rfc3339("2026-07-17T06:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut request = StandingMetaV2ChildPreparationRequest {
            network: None,
            parent_bounty_contract: "0x3333333333333333333333333333333333333333".to_string(),
            parent_solver: "0x1111111111111111111111111111111111111111".to_string(),
            intended_child_solver: "0x1111111111111111111111111111111111111111".to_string(),
            title: "Pinned task".to_string(),
            goal: "Pass one immutable fixture.".to_string(),
            acceptance_criteria: vec!["The exact command exits zero.".to_string()],
            benchmark_source: StandingMetaV2BenchmarkSource {
                kind: "github_commit".to_string(),
                repository: "NSPG13/agent-bounties".to_string(),
                commit: "a".repeat(40),
                subdirectory: "crates/chain-base/tests".to_string(),
            },
            runner_manifest: RegressionSandboxPolicy {
                schema_version: "agent-bounties/regression-sandbox-v1".to_string(),
                image: "docker.io/library/alpine:latest".to_string(),
                command: vec!["true".to_string()],
                workdir: "/workspace".to_string(),
                benchmark_digest: format!("sha256:{}", "c".repeat(64)),
                timeout_seconds: 30,
                cpu_millis: 500,
                memory_bytes: 128 * 1024 * 1024,
                pids_limit: 32,
                max_output_bytes: 64 * 1024,
                tmpfs_bytes: 64 * 1024 * 1024,
                max_source_bytes: 1024 * 1024,
                max_source_files: 100,
                max_benchmark_bytes: 1024 * 1024,
                max_benchmark_files: 100,
                platform: "linux/amd64".to_string(),
                test_seed: 7,
            },
            evidence_schema: None,
            verifier_reward: None,
            funding_deadline: None,
            claim_window_seconds: None,
            verification_window_seconds: None,
            creation_nonce: None,
            nonce_salt: None,
            source_url: None,
            discovery_source: None,
        };
        let parent = StandingMetaV2ParentContext {
            bounty_contract: request.parent_bounty_contract.clone(),
            bounty_id: format!("0x{}", "a".repeat(64)),
            creator: "0x3333333333333333333333333333333333333333".to_string(),
            round: 1,
            solver_reward: Money::new(900_000, "usdc").unwrap(),
            funding_deadline: 1_791_676_800,
        };

        assert!(planner
            .plan_standing_meta_v2_child(&request, &parent, created_at)
            .unwrap_err()
            .to_string()
            .contains("runner manifest"));
        request.runner_manifest.image =
            format!("docker.io/library/alpine@sha256:{}", "b".repeat(64));
        request.intended_child_solver = "0x2222222222222222222222222222222222222222".to_string();
        request.benchmark_source.subdirectory = ".".to_string();
        assert!(planner
            .plan_standing_meta_v2_child(&request, &parent, created_at)
            .unwrap_err()
            .to_string()
            .contains("benchmark source"));
        request.benchmark_source.subdirectory = "crates/chain-base/tests".to_string();
        request.intended_child_solver = request.parent_solver.clone();
        assert!(planner
            .plan_standing_meta_v2_child(&request, &parent, created_at)
            .unwrap_err()
            .to_string()
            .contains("different wallets"));
        request.intended_child_solver = "0x2222222222222222222222222222222222222222".to_string();
        request.parent_solver = parent.creator.clone();
        assert!(planner
            .plan_standing_meta_v2_child(&request, &parent, created_at)
            .unwrap_err()
            .to_string()
            .contains("creator cannot claim"));
        request.parent_solver = "0x1111111111111111111111111111111111111111".to_string();
        request.parent_bounty_contract = "0x4444444444444444444444444444444444444444".to_string();
        assert!(planner
            .plan_standing_meta_v2_child(&request, &parent, created_at)
            .unwrap_err()
            .to_string()
            .contains("does not match"));
    }

    #[test]
    fn standing_meta_v2_benchmark_source_rejects_unknown_fields() {
        assert!(
            serde_json::from_value::<StandingMetaV2BenchmarkSource>(json!({
                "kind": "github_commit",
                "repository": "NSPG13/agent-bounties",
                "commit": "a".repeat(40),
                "subdirectory": "crates/chain-base/tests",
                "commmit": "typo"
            }))
            .is_err()
        );
    }

    #[test]
    fn standing_meta_v2_parent_context_accepts_only_exact_claimable_inventory() {
        let created_at = DateTime::parse_from_rfc3339("2026-07-17T02:11:34Z")
            .unwrap()
            .with_timezone(&Utc);
        let document: AutonomousBountyTermsDocument =
            serde_json::from_str(include_str!("../../../bounties/autonomous-v1/335.json")).unwrap();
        let terms = build_autonomous_bounty_terms_record(
            "0x1eaa1c68772cf76bc5f4e4174766076e33ace662",
            document,
            created_at,
        )
        .unwrap();
        let mut item = AutonomousBountyFeedItem {
            bounty_id: "0x12ad2fa99de272728311a3eb07c3c741048382260cb91ba1e8f001ed3b5759d0"
                .to_string(),
            bounty_contract: "0x43d42cb227d76588ab16693f14efd6cff851fa7a".to_string(),
            creator: terms.creator_wallet.clone(),
            status: "claimable".to_string(),
            solver_reward: "900000".to_string(),
            verifier_reward: "100000".to_string(),
            claim_bond: "100000".to_string(),
            timeout_bond_pool: "0".to_string(),
            target_amount: "1000000".to_string(),
            funded_amount: "1000000".to_string(),
            terms_hash: terms.terms_hash.clone(),
            terms: Some(terms),
            terms_valid: true,
            verification_mode: "deterministic_module".to_string(),
            verifier_module: Some(BASE_MAINNET_STANDING_META_V2_VERIFIER.to_string()),
            verification_ready: true,
            verification_readiness_reason: "exact deployed verifier".to_string(),
            validation_errors: vec![],
            events: vec![],
        };

        let context = standing_meta_v2_parent_context(&item).unwrap();
        assert_eq!(context.round, 1);
        assert_eq!(context.solver_reward.amount, 900_000);
        assert_eq!(context.funding_deadline, 1_791_676_800);

        item.verification_ready = false;
        assert!(standing_meta_v2_parent_context(&item)
            .unwrap_err()
            .to_string()
            .contains("not an exact"));
        item.verification_ready = true;
        item.verifier_module = Some(BASE_MAINNET_CANONICAL_CHILD_VERIFIER.to_string());
        assert!(standing_meta_v2_parent_context(&item)
            .unwrap_err()
            .to_string()
            .contains("not an exact"));
    }

    #[test]
    fn canonical_child_terms_plan_rejects_unbound_or_zero_inputs() {
        let mut request = CanonicalChildBountyTermsRequest {
            parent_bounty_id: format!("0x{}", "aa".repeat(32)),
            parent_round: 0,
            parent_solver: "0x3333333333333333333333333333333333333333".to_string(),
            parent_solver_reward: Money::new(1, "usdc").unwrap(),
            child_acceptance_criteria: vec!["Return a nonempty artifact.".to_string()],
            verifier_module: "0x4444444444444444444444444444444444444444".to_string(),
        };
        assert!(matches!(
            plan_canonical_child_bounty_terms(&request),
            Err(ChainBaseError::InvalidVerificationConfiguration(_))
        ));

        request.verifier_module = "0x4444444444444444444444444444444444444444".to_string();
        request.child_acceptance_criteria.clear();
        assert!(matches!(
            plan_canonical_child_bounty_terms(&request),
            Err(ChainBaseError::InvalidVerificationConfiguration(_))
        ));

        request.parent_round = 1;
        request.verifier_module = "0x0000000000000000000000000000000000000000".to_string();
        assert!(matches!(
            plan_canonical_child_bounty_terms(&request),
            Err(ChainBaseError::InvalidVerificationConfiguration(_))
        ));

        request.child_acceptance_criteria = vec!["Return a nonempty artifact.".to_string()];
        request.verifier_module = BASE_MAINNET_CANONICAL_CHILD_VERIFIER.to_string();
        let recursive_error = plan_canonical_child_bounty_terms(&request).unwrap_err();
        assert!(recursive_error
            .to_string()
            .contains("parent canonical-child verifier cannot verify its own child task"));

        request.verifier_module = BASE_MAINNET_LEADING_ZERO_WORK_VERIFIER.to_string();
        let canary_error = plan_canonical_child_bounty_terms(&request).unwrap_err();
        assert!(canary_error
            .to_string()
            .contains("leading-zero work canary cannot verify a canonical child task"));
    }

    #[test]
    fn autonomous_creation_plan_matches_solidity_abi_and_create2_vector() {
        let planner = AutonomousBountyTxPlanner::new(
            "0x1111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
        )
        .unwrap();
        let create = AutonomousBountyCreate {
            creator: "0x3333333333333333333333333333333333333333".to_string(),
            solver_reward: Money::new(900_000, "usdc").unwrap(),
            verifier_reward: Money::new(100_000, "usdc").unwrap(),
            terms_hash: format!("0x{}", "aa".repeat(32)),
            policy_hash: format!("0x{}", "bb".repeat(32)),
            acceptance_criteria_hash: format!("0x{}", "cc".repeat(32)),
            benchmark_hash: format!("0x{}", "dd".repeat(32)),
            evidence_schema_hash: format!("0x{}", "ee".repeat(32)),
            funding_deadline: 2_000_000_000,
            claim_window_seconds: 3_600,
            verification_window_seconds: 1_800,
            verification_mode: AutonomousVerificationMode::DeterministicModule,
            verifier_module: Some("0x4444444444444444444444444444444444444444".to_string()),
            verifier_reward_recipient: Some(
                "0x5555555555555555555555555555555555555555".to_string(),
            ),
            verifiers: vec![],
            threshold: 1,
            initial_funding: Money::new(1_000_000, "usdc").unwrap(),
            creation_nonce: format!("0x{}", "ff".repeat(32)),
        };

        let plan = planner.plan_creation("base-mainnet", &create).unwrap();

        assert_eq!(
            plan.bounty_id,
            "0xad1d0d3e0adb54b50d5905c3e5fb430ec76a58a2bd4153096cd87a155b105610"
        );
        assert_eq!(
            plan.predicted_bounty_contract,
            "0x5f0d4b53404996a8c293c0153210530c62c50ae4"
        );
        assert!(plan.create_bounty.data.starts_with("0x9d2e414c"));
        assert_eq!((plan.create_bounty.data.len() - 2) / 2, 580);
        let verifier_offset_start = 2 + 8 + (14 * 64);
        assert_eq!(
            &plan.create_bounty.data[verifier_offset_start..verifier_offset_start + 64],
            &format!("{:064x}", 17 * 32)
        );
        assert_eq!(plan.wallet_calls.len(), 2);
        assert_eq!(
            plan.approve.as_ref().unwrap().to,
            BASE_MAINNET_USDC_TOKEN_ADDRESS
        );
        assert_eq!(
            plan.eip3009_authorization.as_ref().unwrap().message.to,
            plan.predicted_bounty_contract
        );
        assert_eq!(
            plan.eip3009_authorization.as_ref().unwrap().domain.name,
            "USD Coin"
        );

        let authorized = planner
            .plan_authorized_creation(
                "base-mainnet",
                &create,
                &AutonomousBountyAuthorizationSignature {
                    v: 0,
                    r: format!("0x{}", "11".repeat(32)),
                    s: format!("0x{}", "22".repeat(32)),
                },
                Some("0x6666666666666666666666666666666666666666"),
            )
            .unwrap();
        assert!(authorized.relay_transaction.data.starts_with("0x61407894"));
        assert_eq!((authorized.relay_transaction.data.len() - 2) / 2, 804);
        let authorized_verifier_offset = 2 + 8 + (15 * 64);
        assert_eq!(
            &authorized.relay_transaction.data
                [authorized_verifier_offset..authorized_verifier_offset + 64],
            &format!("{:064x}", 24 * 32)
        );
        assert_eq!(authorized.bounty_id, plan.bounty_id);
        assert_eq!(
            authorized.predicted_bounty_contract,
            plan.predicted_bounty_contract
        );

        let mut zero_verifier_reward = create;
        zero_verifier_reward.verifier_reward = Money::zero("usdc");
        assert!(matches!(
            planner.plan_creation("base-mainnet", &zero_verifier_reward),
            Err(ChainBaseError::InvalidVerificationConfiguration(_))
        ));
    }

    #[test]
    fn autonomous_pooling_claim_and_submission_plans_are_wallet_ready() {
        let planner = AutonomousBountyTxPlanner::new(
            "0x1111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
        )
        .unwrap();
        let contribution_request = AutonomousBountyContribution {
            bounty_contract: "0x4444444444444444444444444444444444444444".to_string(),
            contributor: "0x3333333333333333333333333333333333333333".to_string(),
            amount: Money::new(250_000, "usdc").unwrap(),
            authorization_nonce: Some(format!("0x{}", "ef".repeat(32))),
            authorization_valid_before: Some(2_000_000_000),
        };
        let contribution = planner
            .plan_contribution("base-sepolia", &contribution_request)
            .unwrap();
        let authorized_contribution = planner
            .plan_authorized_contribution(
                "base-sepolia",
                &contribution_request,
                &AutonomousBountyAuthorizationSignature {
                    v: 28,
                    r: format!("0x{}", "11".repeat(32)),
                    s: format!("0x{}", "22".repeat(32)),
                },
                None,
            )
            .unwrap();
        let claim = planner
            .plan_claim(
                "base-sepolia",
                "0x4444444444444444444444444444444444444444",
                "0x3333333333333333333333333333333333333333",
                100_000,
                Some(&format!("0x{}", "aa".repeat(32))),
                Some(2_000_000_000),
            )
            .unwrap();
        let authorized_claim = planner
            .plan_authorized_claim(
                "base-sepolia",
                "0x4444444444444444444444444444444444444444",
                "0x3333333333333333333333333333333333333333",
                100_000,
                &format!("0x{}", "aa".repeat(32)),
                2_000_000_000,
                &AutonomousBountyAuthorizationSignature {
                    v: 27,
                    r: format!("0x{}", "11".repeat(32)),
                    s: format!("0x{}", "22".repeat(32)),
                },
                None,
            )
            .unwrap();
        let submission = planner
            .plan_submission(
                "0x4444444444444444444444444444444444444444",
                "0x3333333333333333333333333333333333333333",
                &format!("0x{}", "ab".repeat(32)),
                &format!("0x{}", "cd".repeat(32)),
            )
            .unwrap();
        let submission_authorization = planner
            .plan_submission_authorization(
                "base-mainnet",
                &AutonomousBountySubmissionAuthorizationRequest {
                    bounty_contract: "0x4444444444444444444444444444444444444444".to_string(),
                    bounty_id: format!("0x{}", "12".repeat(32)),
                    round: 1,
                    solver: "0x3333333333333333333333333333333333333333".to_string(),
                    submission_hash: format!("0x{}", "ab".repeat(32)),
                    evidence_hash: format!("0x{}", "cd".repeat(32)),
                    policy_hash: format!("0x{}", "ef".repeat(32)),
                    deadline: 2_000_000_000,
                },
            )
            .unwrap();

        assert_eq!(contribution.wallet_calls.len(), 2);
        assert!(contribution.fund.data.starts_with("0xca1d209d"));
        assert!(contribution.eip3009_authorization.is_some());
        assert!(authorized_contribution
            .relay_transaction
            .data
            .starts_with("0xe1c9e96f"));
        assert_eq!(claim.claim.data, "0x4e71d92d");
        assert_eq!(claim.wallet_calls.len(), 2);
        assert!(claim.eip3009_authorization.is_some());
        assert_eq!(
            claim.eip3009_authorization.as_ref().unwrap().domain.name,
            "USDC"
        );
        assert_eq!(authorized_claim.claim_bond, "100000");
        assert_eq!(
            authorized_claim.relay_transaction.function,
            "claimWithAuthorization(address,uint256,uint256,bytes32,uint8,bytes32,bytes32)"
        );
        assert_eq!(
            authorized_claim.relay_transaction.data,
            concat!(
                "0xea7b65f4",
                "0000000000000000000000003333333333333333333333333333333333333333",
                "0000000000000000000000000000000000000000000000000000000000000000",
                "0000000000000000000000000000000000000000000000000000000077359400",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "000000000000000000000000000000000000000000000000000000000000001b",
                "1111111111111111111111111111111111111111111111111111111111111111",
                "2222222222222222222222222222222222222222222222222222222222222222"
            )
        );
        assert!(submission.data.starts_with("0xd26ff86e"));
        assert_eq!(submission_authorization.primary_type, "Submit");
        assert_eq!(submission_authorization.domain.chain_id, 8_453);
        assert_eq!(submission_authorization.message.round, "1");
        assert_eq!(
            submission_authorization.message.submission_hash,
            format!("0x{}", "ab".repeat(32))
        );
    }

    #[test]
    fn autonomous_verification_and_settlement_plans_match_cast_vectors() {
        let planner = AutonomousBountyTxPlanner::new(
            "0x1111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
        )
        .unwrap();
        let bounty = "0x4444444444444444444444444444444444444444";
        let verifier = "0x1111111111111111111111111111111111111111";
        let response_hash = format!("0x{}", "aa".repeat(32));
        let typed_data = planner
            .plan_verification_attestation(
                "base-mainnet",
                &AutonomousVerificationAttestationRequest {
                    bounty_contract: bounty.to_string(),
                    bounty_id: format!("0x{}", "bb".repeat(32)),
                    round: 7,
                    verifier: verifier.to_string(),
                    submission_hash: format!("0x{}", "cc".repeat(32)),
                    evidence_hash: format!("0x{}", "dd".repeat(32)),
                    policy_hash: format!("0x{}", "ee".repeat(32)),
                    passed: true,
                    response_hash: response_hash.clone(),
                    deadline: 2_000_000_000,
                },
            )
            .unwrap();
        assert_eq!(typed_data.primary_type, "VerificationAttestation");
        assert_eq!(typed_data.domain.name, "Agent Bounties");
        assert_eq!(typed_data.domain.version, "1");
        assert_eq!(typed_data.domain.chain_id, 8_453);
        assert_eq!(typed_data.domain.verifying_contract, bounty);
        assert_eq!(typed_data.message.round, "7");
        assert!(typed_data.message.passed);

        let module = planner
            .plan_module_settlement(bounty, None, "0x010203")
            .unwrap();
        assert_eq!(
            module.data,
            "0xed827cee000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000030102030000000000000000000000000000000000000000000000000000000000"
        );

        let attestation = planner
            .plan_attestation_settlement(
                bounty,
                None,
                &[AutonomousSignedAttestation {
                    verifier: verifier.to_string(),
                    passed: true,
                    response_hash,
                    deadline: 123,
                    signature: "0x010203".to_string(),
                }],
            )
            .unwrap();
        assert_eq!(
            attestation.data,
            "0xe345718600000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000002000000000000000000000000011111111111111111111111111111111111111110000000000000000000000000000000000000000000000000000000000000001aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa000000000000000000000000000000000000000000000000000000000000007b00000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000030102030000000000000000000000000000000000000000000000000000000000"
        );
        assert_eq!(
            planner.plan_expire_claim(bounty, None).unwrap().data,
            encode_call("expireClaim()", vec![])
        );
        assert_eq!(
            planner.plan_expire_submission(bounty, None).unwrap().data,
            encode_call("expireSubmission()", vec![])
        );
    }

    #[test]
    fn decodes_canonical_creation_funding_and_settlement_evidence() {
        let decoder = AutonomousBountyLogDecoder;
        let bounty_id = format!("0x{}", "ab".repeat(32));
        let bounty = "0x2222222222222222222222222222222222222222";
        let creator = "0x3333333333333333333333333333333333333333";
        let created = decoder
            .decode(EvmLog {
                address: "0x1111111111111111111111111111111111111111".to_string(),
                topics: vec![
                    evm_event_topic(
                        "CanonicalBountyCreated(bytes32,address,address,bytes32,bytes32,bytes32)",
                    ),
                    bounty_id.clone(),
                    evm_address_word(bounty).unwrap(),
                    evm_address_word(creator).unwrap(),
                ],
                data: evm_words_data(&[
                    format!("0x{}", "01".repeat(32)),
                    format!("0x{}", "02".repeat(32)),
                    format!("0x{}", "03".repeat(32)),
                ])
                .unwrap(),
                tx_hash: format!("0x{}", "11".repeat(32)),
                block_number: 10,
                log_index: 0,
                occurred_at: None,
            })
            .unwrap();
        let committed = decoder
            .decode(EvmLog {
                address: "0x1111111111111111111111111111111111111111".to_string(),
                topics: vec![
                    evm_event_topic(
                        "CanonicalBountyTermsCommitted(bytes32,bytes32,bytes32,bytes32)",
                    ),
                    bounty_id.clone(),
                ],
                data: evm_words_data(&[
                    format!("0x{}", "07".repeat(32)),
                    format!("0x{}", "08".repeat(32)),
                    format!("0x{}", "09".repeat(32)),
                ])
                .unwrap(),
                tx_hash: format!("0x{}", "11".repeat(32)),
                block_number: 10,
                log_index: 1,
                occurred_at: None,
            })
            .unwrap();
        let economics = decoder
            .decode(EvmLog {
                address: "0x1111111111111111111111111111111111111111".to_string(),
                topics: vec![
                    evm_event_topic("CanonicalBountyEconomicsConfigured(bytes32,uint256,uint256,uint256,uint256,uint64,uint64,uint64)"),
                    bounty_id.clone(),
                ],
                data: evm_words_data(&[
                    evm_uint256_word(900_000),
                    evm_uint256_word(100_000),
                    evm_uint256_word(1_000_000),
                    evm_uint256_word(250_000),
                    evm_uint256_word(2_000_000_000),
                    evm_uint256_word(3_600),
                    evm_uint256_word(1_800),
                ]).unwrap(),
                tx_hash: format!("0x{}", "11".repeat(32)),
                block_number: 10,
                log_index: 2,
                occurred_at: None,
            })
            .unwrap();
        let verification = decoder
            .decode(EvmLog {
                address: "0x1111111111111111111111111111111111111111".to_string(),
                topics: vec![
                    evm_event_topic("CanonicalBountyVerificationConfigured(bytes32,uint8,address,address,uint8,bytes32)"),
                    bounty_id.clone(),
                ],
                data: evm_words_data(&[
                    evm_uint256_word(2),
                    evm_address_word("0x4444444444444444444444444444444444444444").unwrap(),
                    evm_address_word("0x5555555555555555555555555555555555555555").unwrap(),
                    evm_uint256_word(2),
                    format!("0x{}", "0a".repeat(32)),
                ]).unwrap(),
                tx_hash: format!("0x{}", "11".repeat(32)),
                block_number: 10,
                log_index: 3,
                occurred_at: None,
            })
            .unwrap();
        assert_eq!(
            created.kind,
            AutonomousBountyEventKind::CanonicalBountyCreated
        );
        assert_eq!(created.bounty_id, bounty_id);
        assert_eq!(created.data["bounty_contract"], bounty);
        assert_eq!(economics.data["initial_funding"], 250_000);
        assert_eq!(economics.data["claim_bond"], 100_000);
        assert_eq!(verification.data["verification_mode"], 2);

        let funding = decoder
            .decode(EvmLog {
                address: bounty.to_string(),
                topics: vec![
                    evm_event_topic("FundingAdded(bytes32,address,uint256,uint256,uint256)"),
                    bounty_id.clone(),
                    evm_address_word(creator).unwrap(),
                ],
                data: evm_words_data(&[
                    evm_uint256_word(750_000),
                    evm_uint256_word(1_000_000),
                    evm_uint256_word(1_000_000),
                ])
                .unwrap(),
                tx_hash: format!("0x{}", "22".repeat(32)),
                block_number: 11,
                log_index: 1,
                occurred_at: None,
            })
            .unwrap();
        assert_eq!(funding.kind, AutonomousBountyEventKind::FundingAdded);
        assert_eq!(funding.data["funded_amount"], 1_000_000);

        let settled = decoder
            .decode(EvmLog {
                address: bounty.to_string(),
                topics: vec![
                    evm_event_topic("BountySettled(bytes32,uint64,address,uint256,uint256,uint256,uint256,bytes32,bytes32,bytes32,bytes32)"),
                    bounty_id,
                    evm_uint256_word(1),
                    evm_address_word("0x4444444444444444444444444444444444444444").unwrap(),
                ],
                data: evm_words_data(&[
                    evm_uint256_word(900_000),
                    evm_uint256_word(100_000),
                    evm_uint256_word(0),
                    evm_uint256_word(100_000),
                    format!("0x{}", "04".repeat(32)),
                    format!("0x{}", "05".repeat(32)),
                    format!("0x{}", "02".repeat(32)),
                    format!("0x{}", "06".repeat(32)),
                ]).unwrap(),
                tx_hash: format!("0x{}", "33".repeat(32)),
                block_number: 12,
                log_index: 2,
                occurred_at: None,
            })
            .unwrap();
        assert_eq!(settled.kind, AutonomousBountyEventKind::BountySettled);
        assert_eq!(settled.data["solver_reward"], 900_000);
        assert_eq!(settled.data["solver_payout"], 1_000_000);
        assert_eq!(
            settled.data["verification_hash"],
            format!("0x{}", "06".repeat(32))
        );

        let feed = build_autonomous_bounty_feed(
            vec![
                created,
                committed,
                economics,
                verification,
                funding,
                settled,
            ],
            Vec::<AutonomousBountyTermsRecord>::new(),
            false,
        )
        .unwrap();
        assert_eq!(feed.len(), 1);
        assert_eq!(feed[0].status, "paid");
        assert_eq!(feed[0].funded_amount, "1000000");
        assert_eq!(feed[0].verification_mode, "ai_judge_quorum");
        assert!(!feed[0].verification_ready);
    }

    #[test]
    fn builds_content_addressed_autonomous_terms_commitments() {
        let now = Utc::now();
        let document = AutonomousBountyTermsDocument {
            schema_version: "agent-bounties/terms-v1".to_string(),
            contract_terms: json!({
                "protocol_version": "agent-bounties/autonomous-v1",
                "creator_wallet": "0x3333333333333333333333333333333333333333",
                "network": "base-mainnet",
                "settlement_token": BASE_MAINNET_USDC_TOKEN_ADDRESS,
                "solver_reward": {"amount": 900_000, "currency": "usdc"},
                "verifier_reward": {"amount": 100_000, "currency": "usdc"},
                "claim_bond": {"amount": 100_000, "currency": "usdc"},
                "initial_funding": {"amount": 1_000_000, "currency": "usdc"},
                "funding_deadline": now.timestamp() + 86_400,
                "claim_window_seconds": 3_600,
                "verification_window_seconds": 1_800,
                "creation_nonce": format!("0x{}", "11".repeat(32)),
            }),
            title: "Fix deterministic test".to_string(),
            goal: "Make the committed check pass.".to_string(),
            acceptance_criteria: vec!["cargo test exits zero".to_string()],
            benchmark: json!({"command": "cargo test", "exit_code": 0}),
            evidence_schema: json!({"required": ["commit", "check_run"]}),
            verification_policy: json!({
                "mechanism": "signed_quorum",
                "threshold": 1,
                "verifiers": ["0x4444444444444444444444444444444444444444"]
            }),
            source_url: Some("https://github.com/NSPG13/agent-bounties/issues/1".to_string()),
            discovery_source: Some("MCP discovery".to_string()),
            agent_eligibility: None,
            claim_coordination: None,
        };
        let record = build_autonomous_bounty_terms_record(
            "0x3333333333333333333333333333333333333333",
            document,
            now,
        )
        .unwrap();

        assert!(record.terms_hash.starts_with("0x"));
        assert_eq!(record.terms_hash.len(), 66);
        assert_ne!(record.terms_hash, record.policy_hash);
        assert_eq!(
            record.creator_wallet,
            "0x3333333333333333333333333333333333333333"
        );
        let mut create = AutonomousBountyCreate {
            creator: record.creator_wallet.clone(),
            solver_reward: Money::new(900_000, "usdc").unwrap(),
            verifier_reward: Money::new(100_000, "usdc").unwrap(),
            terms_hash: record.terms_hash.clone(),
            policy_hash: record.policy_hash.clone(),
            acceptance_criteria_hash: record.acceptance_criteria_hash.clone(),
            benchmark_hash: record.benchmark_hash.clone(),
            evidence_schema_hash: record.evidence_schema_hash.clone(),
            funding_deadline: u64::try_from(now.timestamp() + 86_400).unwrap(),
            claim_window_seconds: 3_600,
            verification_window_seconds: 1_800,
            verification_mode: AutonomousVerificationMode::SignedQuorum,
            verifier_module: None,
            verifier_reward_recipient: None,
            verifiers: vec!["0x4444444444444444444444444444444444444444".to_string()],
            threshold: 1,
            initial_funding: Money::new(1_000_000, "usdc").unwrap(),
            creation_nonce: format!("0x{}", "11".repeat(32)),
        };
        validate_autonomous_creation_against_terms("base-mainnet", &create, &record).unwrap();
        let derived = autonomous_bounty_create_from_terms(&record).unwrap();
        assert_eq!(derived.creator, create.creator);
        assert_eq!(derived.solver_reward, create.solver_reward);
        assert_eq!(derived.verifier_reward, create.verifier_reward);
        assert_eq!(derived.verification_mode, create.verification_mode);
        assert_eq!(derived.verifiers, create.verifiers);
        assert_eq!(derived.threshold, create.threshold);
        create.solver_reward = Money::new(800_000, "usdc").unwrap();
        assert!(
            validate_autonomous_creation_against_terms("base-mainnet", &create, &record).is_err()
        );
    }

    #[test]
    fn known_leading_zero_module_rejects_mismatched_benchmarks_everywhere() {
        let now = Utc::now();
        let mut document: AutonomousBountyTermsDocument =
            serde_json::from_str(include_str!("../../../bounties/autonomous-v1/244.json")).unwrap();
        document.contract_terms["funding_deadline"] = json!(now.timestamp() + 86_400);

        let valid = build_autonomous_bounty_terms_record(
            "0x884834E884d6e93462655A2820140aD03E6747bC",
            document.clone(),
            now,
        )
        .unwrap();
        let valid_create = autonomous_bounty_create_from_terms(&valid).unwrap();

        document.benchmark = json!({
            "engine": "github_ci",
            "required_checks": ["ci"],
            "required_conclusion": "success"
        });
        let publication_error = build_autonomous_bounty_terms_record(
            "0x884834E884d6e93462655A2820140aD03E6747bC",
            document.clone(),
            now,
        )
        .unwrap_err();
        assert!(publication_error.to_string().contains(
            "known leading-zero verifier must use its exact 16-bit scope-bound work benchmark"
        ));

        let mut legacy_record = valid;
        legacy_record.document = document;
        legacy_record.benchmark_hash =
            keccak256_canonical_json(&legacy_record.document.benchmark).unwrap();
        legacy_record.terms_hash =
            keccak256_canonical_json(&serde_json::to_value(&legacy_record.document).unwrap())
                .unwrap();
        let mut legacy_create = valid_create;
        legacy_create.benchmark_hash = legacy_record.benchmark_hash.clone();
        legacy_create.terms_hash = legacy_record.terms_hash.clone();

        assert!(validate_autonomous_creation_against_terms(
            "base-mainnet",
            &legacy_create,
            &legacy_record,
        )
        .is_err());
        assert!(autonomous_bounty_create_from_terms(&legacy_record).is_err());
        assert!(active_terms_semantic_error("claimable", Some(&legacy_record)).is_some());
        assert!(active_terms_semantic_error("submitted", Some(&legacy_record)).is_some());
        assert!(active_terms_semantic_error("paid", Some(&legacy_record)).is_none());
        assert!(active_terms_semantic_error("cancelled", Some(&legacy_record)).is_none());
    }

    #[test]
    fn deterministic_terms_accept_the_contract_zero_verifier_set() {
        let document: AutonomousBountyTermsDocument =
            serde_json::from_str(include_str!("../../../bounties/autonomous-v1/217.json")).unwrap();
        let record = build_autonomous_bounty_terms_record(
            "0x884834E884d6e93462655A2820140aD03E6747bC",
            document,
            Utc::now(),
        )
        .unwrap();
        let creation_data = json!({
            "terms_hash": record.terms_hash,
            "policy_hash": record.policy_hash,
            "acceptance_criteria_hash": record.acceptance_criteria_hash,
            "benchmark_hash": record.benchmark_hash,
            "evidence_schema_hash": record.evidence_schema_hash,
            "solver_reward": 900_000,
            "verifier_reward": 100_000,
            "claim_bond": 100_000,
            "initial_funding": 1_000_000,
            "funding_deadline": 1_791_676_800u64,
            "claim_window_seconds": 1_209_600,
            "verification_window_seconds": 1_209_600,
            "creation_nonce": "0x6a7751dfcd4709a50bf22722e9c9f4ac5a4ad9086d99b0d65a8a247959c36e3f",
            "creator": "0x884834e884d6e93462655a2820140ad03e6747bc",
            "verification_mode": 0,
            "threshold": 1,
            "verifier_module": "0x40adac5a1d00a725f77682f8940b893eaed31ecf",
            "verifier_reward_recipient": "0x884834e884d6e93462655a2820140ad03e6747bc",
            "verifier_set_hash": format!("0x{}", "00".repeat(32)),
        });

        assert!(
            validate_autonomous_terms_against_creation(&creation_data, Some(&record),).is_empty()
        );
    }

    #[test]
    fn creation_batch_uses_one_exact_aggregate_approval() {
        let planner = AutonomousBountyTxPlanner::new(
            "0x1111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
        )
        .unwrap();
        let first = AutonomousBountyCreate {
            creator: "0x3333333333333333333333333333333333333333".to_string(),
            solver_reward: Money::new(900_000, "usdc").unwrap(),
            verifier_reward: Money::new(100_000, "usdc").unwrap(),
            terms_hash: format!("0x{}", "aa".repeat(32)),
            policy_hash: format!("0x{}", "bb".repeat(32)),
            acceptance_criteria_hash: format!("0x{}", "cc".repeat(32)),
            benchmark_hash: format!("0x{}", "dd".repeat(32)),
            evidence_schema_hash: format!("0x{}", "ee".repeat(32)),
            funding_deadline: 2_000_000_000,
            claim_window_seconds: 3_600,
            verification_window_seconds: 1_800,
            verification_mode: AutonomousVerificationMode::SignedQuorum,
            verifier_module: None,
            verifier_reward_recipient: None,
            verifiers: vec!["0x4444444444444444444444444444444444444444".to_string()],
            threshold: 1,
            initial_funding: Money::new(1_000_000, "usdc").unwrap(),
            creation_nonce: format!("0x{}", "01".repeat(32)),
        };
        let mut second = first.clone();
        second.terms_hash = format!("0x{}", "ab".repeat(32));
        second.creation_nonce = format!("0x{}", "02".repeat(32));

        let batch = planner
            .plan_creation_batch("base-mainnet", &[first.clone(), second])
            .unwrap();
        assert_eq!(batch.total_initial_funding, "2000000");
        assert_eq!(batch.creations.len(), 2);
        assert_eq!(batch.wallet_calls.len(), 3);
        assert_eq!(batch.wallet_calls[0].function, "approve(address,uint256)");
        assert!(batch.wallet_calls[1].data.starts_with("0x9d2e414c"));
        assert!(batch.wallet_calls[2].data.starts_with("0x9d2e414c"));
        assert!(batch
            .approve
            .as_ref()
            .unwrap()
            .data
            .ends_with("00000000000000000000000000000000000000000000000000000000001e8480"));

        assert!(matches!(
            planner.plan_creation_batch("base-mainnet", &[first.clone(), first]),
            Err(ChainBaseError::InvalidVerificationConfiguration(message))
                if message.contains("duplicate bounty id")
        ));
    }

    #[test]
    fn seeded_mainnet_bounty_terms_match_committed_manifest_hashes() {
        let created_at = chrono::DateTime::parse_from_rfc3339("2026-07-10T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let creator = "0x884834E884d6e93462655A2820140aD03E6747bC";
        let cases = [
            (
                168,
                include_str!("../../../bounties/autonomous-v1/168.json"),
                "0x83d7f1c75921cf11a3eb7530d72f26272b3a031c1ed73380b7d41e2bdb82c878",
                "0x82da5ff5c09dd827ec70a45328d86f5b8d35ba4313afd8d058737d68d8ddfbeb",
                "0xa795332930322a841d1c608d39df8b29e3b21a664dadaf0780ee3d5c9e2a6f2b",
            ),
            (
                169,
                include_str!("../../../bounties/autonomous-v1/169.json"),
                "0x8c5090db8abad4d7ae34d0286a1c2e28fd4d672c0556abccaa2e9b5194995013",
                "0x804552709a19c26b6b15f4674e24c9aec95d06882558bfe28167c7cf4dc49bf4",
                "0xc908d8f36c9ec2899e0512496ef463957f5e5694c5ce65f277deddf6b14d624d",
            ),
            (
                170,
                include_str!("../../../bounties/autonomous-v1/170.json"),
                "0xe20033e97249d4fa480bf46043b0523c3ee4305bba578e8425568086c7908d31",
                "0x735d12ea387942bfbcd22acdecbbf905fe41a5159c5f09c1512dd27deb53700d",
                "0x0f5426744d1aa813e40c6ce3728db06977255d60384cd19e384397ec9de2dd13",
            ),
            (
                171,
                include_str!("../../../bounties/autonomous-v1/171.json"),
                "0x0942c645d944a488d463ceb4ffa53021798dd20293d5afe47730d009508f7944",
                "0xdb47ed8b500dc305c960c4872f02ee71ccf99c846b206eeae890416b0d778ab6",
                "0x749b4c3fa113ff74f0209862100988562d30184d453135a98ba649e81e453336",
            ),
        ];

        for (issue, json, terms_hash, criteria_hash, benchmark_hash) in cases {
            let document = serde_json::from_str::<AutonomousBountyTermsDocument>(json).unwrap();
            let record =
                build_autonomous_bounty_terms_record(creator, document, created_at).unwrap();
            assert_eq!(record.terms_hash, terms_hash, "issue {issue} terms");
            assert_eq!(
                record.policy_hash,
                "0x9b3cf2179e1a858d94198e9f03f439b5479a519910430541199178114d790dc1",
                "issue {issue} policy"
            );
            assert_eq!(
                record.acceptance_criteria_hash, criteria_hash,
                "issue {issue} criteria"
            );
            assert_eq!(
                record.benchmark_hash, benchmark_hash,
                "issue {issue} benchmark"
            );
            assert_eq!(
                record.evidence_schema_hash,
                "0x1aca62507de0bcde1a36e353b228321e6d35b4eaafe64a8fa5027f20b3a2f4e5",
                "issue {issue} evidence schema"
            );
        }
    }

    #[test]
    fn recovery_reservations_fail_closed_and_filter_earning_inventory() {
        let reserved_contract = "0x2222222222222222222222222222222222222222";
        let available_contract = "0x3333333333333333333333333333333333333333";
        let reservations = AutonomousBountyRecoveryReservations::parse_csv(Some(&format!(
            " 0x{} , {available_contract} ",
            "22".repeat(20).to_ascii_uppercase()
        )))
        .unwrap();
        assert!(reservations.contains(reserved_contract));
        assert!(reservations.contains(&available_contract.to_ascii_uppercase()));
        assert!(BUILTIN_RECOVERY_RESERVED_BOUNTY_CONTRACTS
            .iter()
            .all(|contract| reservations.contains(contract)));
        assert!(BUILTIN_RECOVERY_RESERVED_BOUNTY_CONTRACTS
            .iter()
            .all(|contract| AutonomousBountyRecoveryReservations::default().contains(contract)));
        assert!(matches!(
            AutonomousBountyRecoveryReservations::parse_csv(Some(&format!("{reserved_contract},"))),
            Err(ChainBaseError::InvalidVerificationConfiguration(_))
        ));

        let item = |bounty_contract: &str| AutonomousBountyFeedItem {
            bounty_id: format!("0x{}", "ab".repeat(32)),
            bounty_contract: bounty_contract.to_string(),
            creator: "0x4444444444444444444444444444444444444444".to_string(),
            status: "claimable".to_string(),
            solver_reward: "900000".to_string(),
            verifier_reward: "100000".to_string(),
            claim_bond: "100000".to_string(),
            timeout_bond_pool: "0".to_string(),
            target_amount: "1000000".to_string(),
            funded_amount: "1000000".to_string(),
            terms_hash: format!("0x{}", "aa".repeat(32)),
            terms: None,
            terms_valid: true,
            verification_mode: "deterministic_module".to_string(),
            verifier_module: Some("0x5555555555555555555555555555555555555555".to_string()),
            verification_ready: true,
            verification_readiness_reason: "deterministic verifier module is committed on-chain"
                .to_string(),
            validation_errors: Vec::new(),
            events: Vec::new(),
        };

        let only_reserved =
            AutonomousBountyRecoveryReservations::parse_csv(Some(reserved_contract)).unwrap();
        let mut full_feed = vec![item(reserved_contract), item(available_contract)];
        only_reserved.apply(&mut full_feed, false);
        assert_eq!(full_feed.len(), 2);
        assert!(!full_feed[0].verification_ready);
        assert_eq!(
            full_feed[0].verification_readiness_reason,
            RECOVERY_RESERVED_VERIFICATION_REASON
        );
        assert!(full_feed[1].verification_ready);

        let mut earning_feed = vec![item(reserved_contract), item(available_contract)];
        only_reserved.apply(&mut earning_feed, true);
        assert_eq!(earning_feed.len(), 1);
        assert_eq!(earning_feed[0].bounty_contract, available_contract);

        let mut verification_feed = vec![item(reserved_contract), item(available_contract)];
        only_reserved.exclude_from_verification_jobs(&mut verification_feed);
        assert_eq!(verification_feed.len(), 1);
        assert_eq!(verification_feed[0].bounty_contract, available_contract);
    }

    fn claimed_submission_fixture(claim_expires_at: u64) -> AutonomousBountyFeedItem {
        let bounty_id = format!("0x{}", "ab".repeat(32));
        let bounty_contract = "0x2222222222222222222222222222222222222222";
        let solver = "0x3333333333333333333333333333333333333333";
        AutonomousBountyFeedItem {
            bounty_id: bounty_id.clone(),
            bounty_contract: bounty_contract.to_string(),
            creator: "0x4444444444444444444444444444444444444444".to_string(),
            status: "claimed".to_string(),
            solver_reward: "1900000".to_string(),
            verifier_reward: "100000".to_string(),
            claim_bond: "100000".to_string(),
            timeout_bond_pool: "0".to_string(),
            target_amount: "2000000".to_string(),
            funded_amount: "2000000".to_string(),
            terms_hash: format!("0x{}", "aa".repeat(32)),
            terms: Some(AutonomousBountyTermsRecord {
                terms_hash: format!("0x{}", "aa".repeat(32)),
                policy_hash: format!("0x{}", "bb".repeat(32)),
                acceptance_criteria_hash: format!("0x{}", "01".repeat(32)),
                benchmark_hash: format!("0x{}", "02".repeat(32)),
                evidence_schema_hash: format!("0x{}", "03".repeat(32)),
                creator_wallet: "0x4444444444444444444444444444444444444444".to_string(),
                document: AutonomousBountyTermsDocument {
                    schema_version: "agent-bounties/terms-v1".to_string(),
                    contract_terms: json!({}),
                    title: "Deterministic claimed bounty".to_string(),
                    goal: "Prepare one exact submission".to_string(),
                    acceptance_criteria: vec!["fixture passes".to_string()],
                    benchmark: json!({"engine": "fixture"}),
                    evidence_schema: json!({"required": ["commit_sha"]}),
                    verification_policy: json!({
                        "mechanism": "deterministic_module",
                        "threshold": 1,
                        "verifier_module": "0x5555555555555555555555555555555555555555"
                    }),
                    source_url: Some(
                        "https://github.com/NSPG13/agent-bounties/issues/244".to_string(),
                    ),
                    discovery_source: Some("github-label:bounty".to_string()),
                    agent_eligibility: None,
                    claim_coordination: None,
                },
                created_at: Utc::now(),
            }),
            terms_valid: true,
            verification_mode: "deterministic_module".to_string(),
            verifier_module: Some("0x5555555555555555555555555555555555555555".to_string()),
            verification_ready: true,
            verification_readiness_reason: "deterministic verifier module is committed on-chain"
                .to_string(),
            validation_errors: Vec::new(),
            events: vec![AutonomousBountyEvent {
                id: Uuid::new_v4(),
                log_key: "100:0".to_string(),
                tx_hash: format!("0x{}", "11".repeat(32)),
                block_number: 100,
                log_index: 0,
                contract_address: bounty_contract.to_string(),
                bounty_id,
                kind: AutonomousBountyEventKind::BountyClaimed,
                data: json!({
                    "round": 2,
                    "solver": solver,
                    "bond": 100000,
                    "claim_expires_at": claim_expires_at
                }),
                occurred_at: Utc::now(),
            }],
        }
    }

    #[test]
    fn submission_preparation_binds_active_claim_hashes_and_relay_fields() {
        let item = claimed_submission_fixture(5_000);
        let solver = "0x3333333333333333333333333333333333333333";
        let artifact = "  https://github.com/owner/repo/commit/abc  ";
        let evidence = json!({"z": 2, "a": {"commit_sha": "abc"}});
        let planner = AutonomousBountyTxPlanner::new(
            "0x6666666666666666666666666666666666666666",
            "0x7777777777777777777777777777777777777777",
        )
        .unwrap();

        let prepared = build_autonomous_submission_preparation(
            &planner,
            "base-mainnet",
            &item,
            solver,
            artifact,
            evidence.clone(),
            1_000,
        )
        .unwrap();

        assert_eq!(prepared.current_bounty_state, "claimed");
        assert_eq!(prepared.expected_bounty_state, "submitted");
        assert_eq!(prepared.expected_canonical_event, "SubmissionAdded");
        assert_eq!(prepared.round, 2);
        assert_eq!(prepared.claim_expires_at, 5_000);
        assert_eq!(prepared.authorization_deadline, 2_800);
        assert_eq!(
            prepared.artifact_reference,
            "https://github.com/owner/repo/commit/abc"
        );
        assert_eq!(
            prepared.submission_hash,
            sha256_utf8(prepared.artifact_reference.as_str())
        );
        assert_eq!(
            prepared.evidence_hash,
            sha256_canonical_json(&json!({"a": {"commit_sha": "abc"}, "z": 2})).unwrap()
        );
        assert_eq!(prepared.signing_payload.message.round, "2");
        assert_eq!(prepared.signing_payload.message.deadline, "2800");
        assert_eq!(
            prepared.signing_payload.message.submission_hash,
            prepared.submission_hash
        );
        assert_eq!(prepared.unsigned_relay_envelope["signature"], Value::Null);
        assert_eq!(prepared.unsigned_relay_envelope["round"], 2);
        assert_eq!(prepared.evidence_publication["evidence"], evidence);
        assert_eq!(
            prepared.relay_issue_url.as_deref(),
            Some("https://github.com/NSPG13/agent-bounties/issues/244")
        );
    }

    #[test]
    fn submission_preparation_fails_closed_on_state_solver_time_and_evidence() {
        let planner = AutonomousBountyTxPlanner::new(
            "0x6666666666666666666666666666666666666666",
            "0x7777777777777777777777777777777777777777",
        )
        .unwrap();
        let solver = "0x3333333333333333333333333333333333333333";
        let prepare = |item: &AutonomousBountyFeedItem, solver: &str, evidence: Value| {
            build_autonomous_submission_preparation(
                &planner,
                "base-mainnet",
                item,
                solver,
                "https://example.com/artifact",
                evidence,
                1_000,
            )
        };

        let mut item = claimed_submission_fixture(5_000);
        assert!(matches!(
            prepare(
                &item,
                "0x8888888888888888888888888888888888888888",
                json!({})
            ),
            Err(ChainBaseError::InvalidSubmissionPreparation(_))
        ));
        item.verification_ready = false;
        assert!(matches!(
            prepare(&item, solver, json!({})),
            Err(ChainBaseError::InvalidSubmissionPreparation(_))
        ));
        item = claimed_submission_fixture(1_060);
        assert!(matches!(
            prepare(&item, solver, json!({})),
            Err(ChainBaseError::InvalidSubmissionPreparation(_))
        ));
        item = claimed_submission_fixture(5_000);
        assert!(matches!(
            prepare(&item, solver, json!(["not", "an", "object"])),
            Err(ChainBaseError::InvalidSubmissionEvidence(_))
        ));
        item.status = "submitted".to_string();
        assert!(matches!(
            prepare(&item, solver, json!({})),
            Err(ChainBaseError::InvalidSubmissionPreparation(_))
        ));
    }

    #[test]
    fn verifier_signing_scope_must_match_current_indexed_submission() {
        let bounty_id = format!("0x{}", "ab".repeat(32));
        let bounty_contract = "0x2222222222222222222222222222222222222222";
        let verifier = "0x4444444444444444444444444444444444444444";
        let submission_hash = format!("0x{}", "cc".repeat(32));
        let evidence_hash = format!("0x{}", "dd".repeat(32));
        let policy_hash = format!("0x{}", "bb".repeat(32));
        let mut item = AutonomousBountyFeedItem {
            bounty_id: bounty_id.clone(),
            bounty_contract: bounty_contract.to_string(),
            creator: "0x3333333333333333333333333333333333333333".to_string(),
            status: "submitted".to_string(),
            solver_reward: "900000".to_string(),
            verifier_reward: "100000".to_string(),
            claim_bond: "100000".to_string(),
            timeout_bond_pool: "0".to_string(),
            target_amount: "1000000".to_string(),
            funded_amount: "1000000".to_string(),
            terms_hash: format!("0x{}", "aa".repeat(32)),
            terms: Some(AutonomousBountyTermsRecord {
                terms_hash: format!("0x{}", "aa".repeat(32)),
                policy_hash: policy_hash.clone(),
                acceptance_criteria_hash: format!("0x{}", "01".repeat(32)),
                benchmark_hash: format!("0x{}", "02".repeat(32)),
                evidence_schema_hash: format!("0x{}", "03".repeat(32)),
                creator_wallet: "0x3333333333333333333333333333333333333333".to_string(),
                document: AutonomousBountyTermsDocument {
                    schema_version: "agent-bounties/terms-v1".to_string(),
                    contract_terms: json!({
                        "protocol_version": "agent-bounties/autonomous-v1",
                        "creator_wallet": "0x3333333333333333333333333333333333333333",
                        "network": "base-mainnet",
                        "settlement_token": BASE_MAINNET_USDC_TOKEN_ADDRESS,
                        "solver_reward": {"amount": 900_000, "currency": "usdc"},
                        "verifier_reward": {"amount": 100_000, "currency": "usdc"},
                        "claim_bond": {"amount": 100_000, "currency": "usdc"},
                        "initial_funding": {"amount": 1_000_000, "currency": "usdc"},
                        "funding_deadline": 2_000_000_000u64,
                        "claim_window_seconds": 3_600,
                        "verification_window_seconds": 1_800,
                        "creation_nonce": format!("0x{}", "11".repeat(32)),
                    }),
                    title: "Verify current submission".to_string(),
                    goal: "Bind signatures to current indexed evidence".to_string(),
                    acceptance_criteria: vec!["current hashes match".to_string()],
                    benchmark: json!({"engine": "github_ci"}),
                    evidence_schema: json!({"required": ["commit_sha"]}),
                    verification_policy: json!({
                        "mechanism": "signed_quorum",
                        "threshold": 1,
                        "verifiers": [verifier]
                    }),
                    source_url: None,
                    discovery_source: None,
                    agent_eligibility: None,
                    claim_coordination: None,
                },
                created_at: Utc::now(),
            }),
            terms_valid: true,
            verification_mode: "signed_quorum".to_string(),
            verifier_module: None,
            verification_ready: false,
            verification_readiness_reason:
                "quorum verifier service availability is not canonically attested".to_string(),
            validation_errors: vec![],
            events: vec![AutonomousBountyEvent {
                id: Uuid::new_v4(),
                log_key: "100:0".to_string(),
                tx_hash: format!("0x{}", "11".repeat(32)),
                block_number: 100,
                log_index: 0,
                contract_address: bounty_contract.to_string(),
                bounty_id: bounty_id.clone(),
                kind: AutonomousBountyEventKind::SubmissionAdded,
                data: json!({
                    "round": 2,
                    "solver": "0x5555555555555555555555555555555555555555",
                    "submission_hash": submission_hash,
                    "evidence_hash": evidence_hash,
                    "verification_expires_at": 200
                }),
                occurred_at: Utc::now(),
            }],
        };
        let mut request = AutonomousVerificationAttestationRequest {
            bounty_contract: bounty_contract.to_string(),
            bounty_id,
            round: 2,
            verifier: verifier.to_string(),
            submission_hash: format!("0x{}", "cc".repeat(32)),
            evidence_hash: format!("0x{}", "dd".repeat(32)),
            policy_hash,
            passed: true,
            response_hash: format!("0x{}", "ee".repeat(32)),
            deadline: 150,
        };

        assert!(!autonomous_bounty_is_earning_ready(&item));
        item.status = "claimable".to_string();
        assert!(!autonomous_bounty_is_earning_ready(&item));
        item.verification_ready = true;
        assert!(autonomous_bounty_is_earning_ready(&item));
        item.status = "submitted".to_string();
        item.verification_ready = false;

        validate_attestation_request_against_feed(&item, &request, 100).unwrap();
        item.terms_valid = false;
        assert!(matches!(
            validate_attestation_request_against_feed(&item, &request, 100),
            Err(ChainBaseError::InvalidAttestationScope(_))
        ));
        item.terms_valid = true;
        request.round = 3;
        assert!(matches!(
            validate_attestation_request_against_feed(&item, &request, 100),
            Err(ChainBaseError::InvalidAttestationScope(_))
        ));
        request.round = 2;
        request.deadline = 201;
        assert!(matches!(
            validate_attestation_request_against_feed(&item, &request, 100),
            Err(ChainBaseError::InvalidAttestationScope(_))
        ));

        let jobs = build_autonomous_verification_jobs(
            "base-mainnet",
            vec![item],
            vec![AutonomousSubmissionEvidenceRecord {
                network: "base-mainnet".to_string(),
                bounty_contract: bounty_contract.to_string(),
                bounty_id: format!("0x{}", "ab".repeat(32)),
                round: 2,
                solver_wallet: "0x5555555555555555555555555555555555555555".to_string(),
                artifact_reference: "https://example.com/artifact".to_string(),
                artifact_hash: format!("0x{}", "cc".repeat(32)),
                evidence: json!({"check": "passed"}),
                evidence_hash: format!("0x{}", "dd".repeat(32)),
                created_at: Utc::now(),
            }],
            100,
        )
        .unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].round, 2);
        assert_eq!(jobs[0].eligible_verifiers, vec![verifier.to_string()]);
        assert_eq!(jobs[0].current_solver_payout, "900000");
    }

    #[test]
    fn submission_evidence_preimages_must_match_indexed_hashes() {
        let artifact = "https://github.com/owner/repo/commit/abc";
        let evidence = json!({"z": 2, "a": {"commit_sha": "abc"}});
        let bounty_id = format!("0x{}", "ab".repeat(32));
        let bounty_contract = "0x2222222222222222222222222222222222222222";
        let solver = "0x3333333333333333333333333333333333333333";
        let item = AutonomousBountyFeedItem {
            bounty_id: bounty_id.clone(),
            bounty_contract: bounty_contract.to_string(),
            creator: "0x4444444444444444444444444444444444444444".to_string(),
            status: "submitted".to_string(),
            solver_reward: "900000".to_string(),
            verifier_reward: "100000".to_string(),
            claim_bond: "100000".to_string(),
            timeout_bond_pool: "0".to_string(),
            target_amount: "1000000".to_string(),
            funded_amount: "1000000".to_string(),
            terms_hash: format!("0x{}", "aa".repeat(32)),
            terms: None,
            terms_valid: false,
            verification_mode: "signed_quorum".to_string(),
            verifier_module: None,
            verification_ready: false,
            verification_readiness_reason: "content-addressed terms are invalid or unavailable"
                .to_string(),
            validation_errors: vec!["fixture omits terms".to_string()],
            events: vec![AutonomousBountyEvent {
                id: Uuid::new_v4(),
                log_key: "101:0".to_string(),
                tx_hash: format!("0x{}", "11".repeat(32)),
                block_number: 101,
                log_index: 0,
                contract_address: bounty_contract.to_string(),
                bounty_id: bounty_id.clone(),
                kind: AutonomousBountyEventKind::SubmissionAdded,
                data: json!({
                    "round": 3,
                    "solver": solver,
                    "submission_hash": sha256_utf8(artifact),
                    "evidence_hash": sha256_canonical_json(&evidence).unwrap(),
                    "verification_expires_at": 200
                }),
                occurred_at: Utc::now(),
            }],
        };

        let record = build_autonomous_submission_evidence_record(
            "base-mainnet",
            &item,
            bounty_contract,
            &bounty_id,
            3,
            solver,
            artifact,
            evidence.clone(),
            Utc::now(),
        )
        .unwrap();
        assert_eq!(record.artifact_hash, sha256_utf8(artifact));
        assert_eq!(
            record.evidence_hash,
            sha256_canonical_json(&json!({"a": {"commit_sha": "abc"}, "z": 2})).unwrap()
        );
        assert!(matches!(
            build_autonomous_submission_evidence_record(
                "base-mainnet",
                &item,
                bounty_contract,
                &bounty_id,
                3,
                solver,
                artifact,
                json!({"a": {"commit_sha": "different"}, "z": 2}),
                Utc::now(),
            ),
            Err(ChainBaseError::InvalidSubmissionEvidence(_))
        ));
    }

    #[test]
    fn plans_contract_log_query_for_autonomous_protocol_topics() {
        let query = BaseContractLogQuery::new(
            "0x1111111111111111111111111111111111111111",
            100,
            Some(120),
            autonomous_bounty_event_topics(),
        )
        .unwrap();
        let request = query.rpc_request(9);
        assert_eq!(request.params[0].from_block, "0x64");
        assert_eq!(request.params[0].to_block, "0x78");
        assert_eq!(request.params[0].topics[0].len(), 15);
        assert_eq!(
            request.params[0].address,
            EthGetLogsAddressFilter::One(query.contract)
        );
    }

    #[test]
    fn plans_batched_canonical_bounty_log_query() {
        let query = BaseMultiContractLogQuery::new(
            [
                "0x2222222222222222222222222222222222222222",
                "0x1111111111111111111111111111111111111111",
                "0x2222222222222222222222222222222222222222",
            ],
            100,
            Some(120),
            autonomous_bounty_event_topics(),
        )
        .unwrap();
        let request = query.rpc_request(10);
        assert_eq!(query.contracts.len(), 2);
        assert_eq!(
            request.params[0].address,
            EthGetLogsAddressFilter::Many(vec![
                "0x1111111111111111111111111111111111111111".to_string(),
                "0x2222222222222222222222222222222222222222".to_string(),
            ])
        );
    }

    #[test]
    fn base_network_descriptors_identify_rpc_env_vars() {
        let sepolia = base_network_descriptor("base-sepolia").unwrap();
        let mainnet = base_network_descriptor("8453").unwrap();

        assert_eq!(sepolia.chain_id, 84_532);
        assert_eq!(sepolia.rpc_url_env, "BASE_SEPOLIA_RPC_URL");
        assert_eq!(
            sepolia.native_usdc_token_address,
            BASE_SEPOLIA_USDC_TOKEN_ADDRESS
        );
        assert_eq!(mainnet.chain_id, 8_453);
        assert_eq!(mainnet.rpc_url_env, "BASE_MAINNET_RPC_URL");
        assert_eq!(
            mainnet.native_usdc_token_address,
            BASE_MAINNET_USDC_TOKEN_ADDRESS
        );
        assert_eq!(
            base_network_descriptor("optimism").unwrap_err(),
            ChainBaseError::UnknownNetwork("optimism".to_string())
        );
    }

    #[test]
    fn base_rpc_url_config_resolves_only_configured_networks() {
        let config = BaseRpcUrlConfig {
            base_sepolia: Some("https://sepolia.example".to_string()),
            base_mainnet: None,
        };

        let (network, url) = config.resolve("base-sepolia").unwrap();
        assert_eq!(network.name, "Base Sepolia");
        assert_eq!(url, "https://sepolia.example");
        assert_eq!(
            config.resolve("base-mainnet").unwrap_err(),
            ChainBaseError::MissingRpcUrl {
                network: "Base".to_string(),
                env_var: "BASE_MAINNET_RPC_URL".to_string()
            }
        );
    }

    #[test]
    fn rejects_reversed_log_query_range() {
        let error = BaseContractLogQuery::new(
            "0x1111111111111111111111111111111111111111",
            200,
            Some(199),
            autonomous_bounty_event_topics(),
        )
        .unwrap_err();

        assert_eq!(
            error,
            ChainBaseError::InvalidBlockRange {
                from_block: 200,
                to_block: 199
            }
        );
    }

    #[test]
    fn normalizes_rpc_logs_to_evm_logs() {
        let rpc_log = RpcEvmLog {
            address: "0x1111111111111111111111111111111111111111".to_string(),
            topics: vec![event_topic(
                "EscrowCreated(uint256,bytes32,address,address,uint256,bytes32)",
            )],
            data: "0xAABB".to_string(),
            transaction_hash: format!("0x{}", "ab".repeat(32)),
            block_number: "0xa".to_string(),
            log_index: "0x2".to_string(),
        };

        let log = rpc_log.to_evm_log().unwrap();

        assert_eq!(log.address, "0x1111111111111111111111111111111111111111");
        assert_eq!(log.data, "0xaabb");
        assert_eq!(log.tx_hash, format!("0x{}", "ab".repeat(32)));
        assert_eq!(log.block_number, 10);
        assert_eq!(log.log_index, 2);
    }

    #[test]
    fn accepts_bare_or_enveloped_rpc_log_submissions() {
        let log = RpcEvmLog {
            address: "0x1111111111111111111111111111111111111111".to_string(),
            topics: vec![event_topic("EscrowReleased(uint256,bytes32)")],
            data: "0x".to_string(),
            transaction_hash: format!("0x{}", "ab".repeat(32)),
            block_number: "0x1".to_string(),
            log_index: "0x0".to_string(),
        };
        let bare = serde_json::to_value(vec![log.clone()]).unwrap();
        let enveloped = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": [log]
        });

        let bare: RpcLogSubmission = serde_json::from_value(bare).unwrap();
        let enveloped: RpcLogSubmission = serde_json::from_value(enveloped).unwrap();

        assert_eq!(bare.into_logs().len(), 1);
        assert_eq!(enveloped.into_logs().len(), 1);
    }

    #[test]
    fn parses_json_rpc_provider_errors_without_logs() {
        let error = parse_eth_get_logs_response(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32000,
                "message": "block range too large"
            }
        }))
        .unwrap_err();

        assert_eq!(
            error,
            ChainBaseError::RpcProviderError {
                code: -32000,
                message: "block range too large".to_string()
            }
        );
    }

    #[test]
    fn builds_eth_block_number_request() {
        let request = eth_block_number_request(17);

        assert_eq!(request.jsonrpc, "2.0");
        assert_eq!(request.id, 17);
        assert_eq!(request.method, "eth_blockNumber");
        assert!(request.params.is_empty());
    }

    #[test]
    fn rejects_malformed_block_number_response() {
        let error = parse_eth_block_number_response(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": "not-hex"
        }))
        .unwrap_err();

        assert_eq!(
            error,
            ChainBaseError::InvalidRpcQuantity("quantity must have 0x prefix".to_string())
        );
    }

    #[tokio::test]
    async fn fetches_block_number_through_mock_transport() {
        let seen_request = Arc::new(Mutex::new(None));
        let transport = MockTransport {
            seen_request: seen_request.clone(),
            response: serde_json::json!({
                "jsonrpc": "2.0",
                "id": 7,
                "result": "0x2a"
            }),
        };

        let block_number = fetch_block_number_with_transport("https://rpc.example", 7, &transport)
            .await
            .unwrap();

        assert_eq!(block_number, 42);
        let request = seen_request.lock().unwrap().clone().unwrap();
        assert_eq!(request["method"], "eth_blockNumber");
        assert!(request["params"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn fetches_exact_block_timestamp_through_mock_transport() {
        let seen_request = Arc::new(Mutex::new(None));
        let transport = MockTransport {
            seen_request: seen_request.clone(),
            response: serde_json::json!({
                "jsonrpc": "2.0",
                "id": 8,
                "result": { "number": "0x2a", "timestamp": "0x669b8f00" }
            }),
        };

        let timestamp =
            fetch_block_timestamp_with_transport("https://rpc.example", 42, 8, &transport)
                .await
                .unwrap();

        assert_eq!(timestamp.timestamp(), 1_721_470_720);
        let request = seen_request.lock().unwrap().clone().unwrap();
        assert_eq!(request["method"], "eth_getBlockByNumber");
        assert_eq!(request["params"], serde_json::json!(["0x2a", false]));
    }

    #[tokio::test]
    async fn observes_erc20_balance_at_one_safe_block() {
        let seen_requests = Arc::new(Mutex::new(Vec::new()));
        let token = "0x1111111111111111111111111111111111111111";
        let account = "0x2222222222222222222222222222222222222222";
        let transport = SequenceTransport {
            seen_requests: seen_requests.clone(),
            responses: Mutex::new(VecDeque::from([
                json!({
                    "jsonrpc": "2.0",
                    "id": 90,
                    "result": {
                        "number": "0x2a",
                        "hash": format!("0x{}", "ab".repeat(32)),
                        "timestamp": "0x669b8f00"
                    }
                }),
                json!({
                    "jsonrpc": "2.0",
                    "id": 91,
                    "result": format!("0x{:064x}", 29_000_000_u128)
                }),
            ])),
        };

        let observation = observe_erc20_balance_safe_with_transport(
            "https://rpc.example",
            token,
            account,
            90,
            &transport,
        )
        .await
        .unwrap();

        assert_eq!(observation.balance, 29_000_000);
        assert_eq!(observation.safe_block_number, 42);
        assert_eq!(observation.account, account);
        let requests = seen_requests.lock().unwrap();
        assert_eq!(requests[0]["params"], json!(["safe", false]));
        assert_eq!(requests[1]["method"], "eth_call");
        assert_eq!(requests[1]["params"][0]["to"], token);
        assert_eq!(requests[1]["params"][1], "0x2a");
        assert_eq!(
            requests[1]["params"][0]["data"],
            format!("0x70a08231{:0>64}", account.trim_start_matches("0x"))
        );
    }

    #[tokio::test]
    async fn observes_leaderboard_paid_winner_at_one_safe_block() {
        let seen_requests = Arc::new(Mutex::new(Vec::new()));
        let contract = "0x3333333333333333333333333333333333333333";
        let winner = "0x4444444444444444444444444444444444444444";
        let award_id = solver_leaderboard_award_id(0, 1_728_000_000).unwrap();
        assert_eq!(
            award_id,
            "0x7e01072e59df59da214dce5f7c3e2ef0f8c8b9a2d636811251e865c1ffd8a774"
        );
        let transport = SequenceTransport {
            seen_requests: seen_requests.clone(),
            responses: Mutex::new(VecDeque::from([
                json!({
                    "jsonrpc": "2.0",
                    "id": 92,
                    "result": {
                        "number": "0x2a",
                        "hash": format!("0x{}", "cd".repeat(32)),
                        "timestamp": "0x669b8f00"
                    }
                }),
                json!({
                    "jsonrpc": "2.0",
                    "id": 93,
                    "result": format!("0x{:0>64}", winner.trim_start_matches("0x"))
                }),
            ])),
        };

        let observation = observe_solver_leaderboard_paid_winner_safe_with_transport(
            "https://rpc.example",
            contract,
            &award_id,
            92,
            &transport,
        )
        .await
        .unwrap();

        assert_eq!(observation.paid_winner.as_deref(), Some(winner));
        assert_eq!(observation.safe_block_number, 42);
        let requests = seen_requests.lock().unwrap();
        assert_eq!(requests[1]["params"][0]["to"], contract);
        assert_eq!(requests[1]["params"][1], "0x2a");
        assert_eq!(
            requests[1]["params"][0]["data"],
            format!("0x270eca05{}", award_id.trim_start_matches("0x"))
        );
    }

    #[test]
    fn builds_and_validates_raw_transaction_broadcast_requests() {
        let request = eth_send_raw_transaction_request("0xABCDEF", 77).unwrap();

        assert_eq!(request.method, "eth_sendRawTransaction");
        assert_eq!(request.id, 77);
        assert_eq!(request.params, vec!["0xabcdef"]);
        assert_eq!(
            eth_send_raw_transaction_request("abcdef", 1).unwrap_err(),
            ChainBaseError::InvalidSignedTransaction("transaction must have 0x prefix".to_string())
        );
        assert!(eth_send_raw_transaction_request("0xabc", 1).is_err());
    }

    #[test]
    fn parses_broadcast_provider_errors_and_tx_hashes() {
        let success = parse_eth_send_raw_transaction_response(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": format!("0x{}", "AB".repeat(32))
        }))
        .unwrap();
        assert_eq!(success.result, format!("0x{}", "ab".repeat(32)));

        let error = parse_eth_send_raw_transaction_response(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32000,
                "message": "replacement transaction underpriced"
            }
        }))
        .unwrap_err();
        assert_eq!(
            error,
            ChainBaseError::RpcProviderError {
                code: -32000,
                message: "replacement transaction underpriced".to_string()
            }
        );
    }

    #[tokio::test]
    async fn broadcasts_signed_transaction_through_mock_transport() {
        let seen_request = Arc::new(Mutex::new(None));
        let transport = MockTransport {
            seen_request: seen_request.clone(),
            response: serde_json::json!({
                "jsonrpc": "2.0",
                "id": 9,
                "result": format!("0x{}", "cd".repeat(32))
            }),
        };

        let response = broadcast_signed_transaction_with_transport(
            "https://rpc.example",
            "0x0102",
            9,
            &transport,
        )
        .await
        .unwrap();

        assert_eq!(response.result, format!("0x{}", "cd".repeat(32)));
        let request = seen_request.lock().unwrap().clone().unwrap();
        assert_eq!(request["method"], "eth_sendRawTransaction");
        assert_eq!(request["params"][0], "0x0102");
    }

    #[tokio::test]
    async fn fetches_receipt_and_normalizes_logs() {
        let tx_hash = format!("0x{}", "ef".repeat(32));
        let seen_request = Arc::new(Mutex::new(None));
        let transport = MockTransport {
            seen_request: seen_request.clone(),
            response: serde_json::json!({
                "jsonrpc": "2.0",
                "id": 10,
                "result": {
                    "transactionHash": tx_hash,
                    "blockNumber": "0x10",
                    "status": "0x1",
                    "logs": [{
                        "address": "0x1111111111111111111111111111111111111111",
                        "topics": [event_topic("EscrowReleased(uint256,bytes32)")],
                        "data": format!("0x{}", "22".repeat(32)),
                        "transactionHash": format!("0x{}", "ef".repeat(32)),
                        "blockNumber": "0x10",
                        "logIndex": "0x2"
                    }]
                }
            }),
        };

        let response = fetch_transaction_receipt_with_transport(
            "https://rpc.example",
            &format!("0x{}", "ef".repeat(32)),
            10,
            &transport,
        )
        .await
        .unwrap();
        let receipt = response.result.expect("receipt should be present");
        let logs = receipt.logs_to_evm_logs().unwrap();

        assert_eq!(receipt.block_number().unwrap(), Some(16));
        assert_eq!(receipt.succeeded().unwrap(), Some(true));
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].log_index, 2);
        let request = seen_request.lock().unwrap().clone().unwrap();
        assert_eq!(request["method"], "eth_getTransactionReceipt");
        assert_eq!(request["params"][0], format!("0x{}", "ef".repeat(32)));
    }

    fn canonical_factory_expected_state() -> AutonomousFactoryExpectedState {
        AutonomousFactoryExpectedState {
            protocol_version: "agent-bounties/autonomous-v1".to_string(),
            network: "base-mainnet".to_string(),
            chain_id: 8_453,
            factory_contract: "0x1111111111111111111111111111111111111111".to_string(),
            implementation_contract: "0x2222222222222222222222222222222222222222".to_string(),
            native_usdc_token_address: BASE_MAINNET_USDC_TOKEN_ADDRESS.to_string(),
            protocol_hash: AUTONOMOUS_BOUNTY_PROTOCOL_HASH.to_string(),
            factory_runtime_code_hash: format!("0x{}", "33".repeat(32)),
            implementation_runtime_code_hash: format!("0x{}", "44".repeat(32)),
        }
    }

    fn canonical_factory_rpc_responses(
        expected: &AutonomousFactoryExpectedState,
    ) -> VecDeque<Value> {
        let implementation_word = format!(
            "0x{:0>64}",
            expected.implementation_contract.trim_start_matches("0x")
        );
        let token_word = format!(
            "0x{:0>64}",
            expected.native_usdc_token_address.trim_start_matches("0x")
        );
        VecDeque::from(vec![
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "number": "0x10",
                    "hash": format!("0x{}", "aa".repeat(32)),
                    "timestamp": "0x669b8f00"
                }
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "result": { "codeHash": expected.factory_runtime_code_hash }
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "result": { "codeHash": expected.implementation_runtime_code_hash }
            }),
            json!({ "jsonrpc": "2.0", "id": 4, "result": expected.protocol_hash }),
            json!({ "jsonrpc": "2.0", "id": 5, "result": implementation_word }),
            json!({ "jsonrpc": "2.0", "id": 6, "result": token_word }),
        ])
    }

    struct SequenceTransport {
        seen_requests: Arc<Mutex<Vec<Value>>>,
        responses: Mutex<VecDeque<Value>>,
    }

    #[async_trait::async_trait]
    impl JsonRpcTransport for SequenceTransport {
        async fn post_json_value(
            &self,
            _rpc_url: &str,
            request: &Value,
        ) -> Result<Value, ChainBaseError> {
            self.seen_requests.lock().unwrap().push(request.clone());
            self.responses.lock().unwrap().pop_front().ok_or_else(|| {
                ChainBaseError::RpcTransport("mock response queue exhausted".to_string())
            })
        }
    }

    #[tokio::test]
    async fn safe_factory_verification_pins_exact_code_and_getters_to_one_block() {
        let expected = canonical_factory_expected_state();
        let seen_requests = Arc::new(Mutex::new(Vec::new()));
        let transport = SequenceTransport {
            seen_requests: seen_requests.clone(),
            responses: Mutex::new(canonical_factory_rpc_responses(&expected)),
        };

        let observation = verify_autonomous_factory_safe_state_with_transport(
            "https://mainnet.base.org",
            &expected,
            &transport,
        )
        .await
        .unwrap();

        assert_eq!(observation.safe_block_number, 16);
        assert_eq!(
            observation.safe_block_timestamp,
            u64::from_str_radix("669b8f00", 16).unwrap()
        );
        assert_eq!(observation.block_tag, "safe");
        assert_eq!(observation.factory_contract, expected.factory_contract);
        assert_eq!(
            observation.implementation_contract,
            expected.implementation_contract
        );
        assert_eq!(
            observation.factory_runtime_code_hash,
            expected.factory_runtime_code_hash
        );

        let requests = seen_requests.lock().unwrap();
        assert_eq!(requests.len(), 6);
        assert_eq!(requests[0]["method"], "eth_getBlockByNumber");
        assert_eq!(requests[0]["params"], json!(["safe", false]));
        assert_eq!(requests[1]["method"], "eth_getProof");
        assert_eq!(requests[1]["params"][2], "0x10");
        assert_eq!(requests[2]["params"][2], "0x10");
        for request in &requests[3..] {
            assert_eq!(request["method"], "eth_call");
            assert_eq!(request["params"][1], "0x10");
        }
    }

    #[tokio::test]
    async fn safe_factory_verification_fails_closed_on_token_mismatch() {
        let expected = canonical_factory_expected_state();
        let mut responses = canonical_factory_rpc_responses(&expected);
        let wrong_token = format!("0x{:0>64}", "5555555555555555555555555555555555555555");
        *responses.back_mut().unwrap() =
            json!({ "jsonrpc": "2.0", "id": 6, "result": wrong_token });
        let transport = SequenceTransport {
            seen_requests: Arc::new(Mutex::new(Vec::new())),
            responses: Mutex::new(responses),
        };

        let error = verify_autonomous_factory_safe_state_with_transport(
            "https://mainnet.base.org",
            &expected,
            &transport,
        )
        .await
        .unwrap_err();

        assert!(matches!(
            error,
            ChainBaseError::InvalidVerificationConfiguration(message)
                if message.contains("factory settlement token mismatch")
        ));
    }

    struct MockTransport {
        seen_request: Arc<Mutex<Option<Value>>>,
        response: Value,
    }

    #[async_trait::async_trait]
    impl JsonRpcTransport for MockTransport {
        async fn post_json_value(
            &self,
            _rpc_url: &str,
            request: &Value,
        ) -> Result<Value, ChainBaseError> {
            *self.seen_request.lock().unwrap() = Some(request.clone());
            Ok(self.response.clone())
        }
    }
}
