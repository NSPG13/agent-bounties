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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmTransactionIntent {
    pub from: Option<String>,
    pub to: String,
    pub value_wei: u128,
    pub data: String,
    pub function: String,
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

pub const BOUNDED_AGENT_WALLET_PROTOCOL_VERSION: &str = "agent-bounties/bounded-wallet-v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BoundedAgentWalletAction {
    Create {
        create: Box<AutonomousBountyCreate>,
    },
    Fund {
        bounty_contract: String,
        amount: Money,
    },
    Claim {
        bounty_contract: String,
        expected_claim_bond: Money,
    },
    Submit {
        bounty_contract: String,
        submission_hash: String,
        evidence_hash: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundedAgentWalletActionRequest {
    pub wallet_contract: String,
    pub delegate: String,
    pub policy_version: u64,
    pub delegate_nonce: u128,
    pub deadline: u64,
    pub action: BoundedAgentWalletAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundedAgentWalletAuthorizationMessage {
    pub wallet: String,
    pub action: String,
    pub payload_hash: String,
    pub nonce: String,
    pub deadline: String,
    pub policy_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundedAgentWalletAuthorizationTypedData {
    pub types: BTreeMap<String, Vec<Eip712TypeField>>,
    pub domain: Eip712DomainData,
    pub primary_type: String,
    pub message: BoundedAgentWalletAuthorizationMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundedAgentWalletActionPlan {
    pub protocol_version: String,
    pub network: BaseNetworkDescriptor,
    pub expected_factory_contract: String,
    pub wallet_contract: String,
    pub delegate: String,
    pub action: String,
    pub action_code: u8,
    pub spend_upper_bound: String,
    pub payload: String,
    pub payload_hash: String,
    pub direct_transaction: EvmTransactionIntent,
    pub relay_authorization: BoundedAgentWalletAuthorizationTypedData,
    pub predicted_bounty_contract: Option<String>,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundedAgentWalletAuthorizedActionPlan {
    pub protocol_version: String,
    pub network: BaseNetworkDescriptor,
    pub wallet_contract: String,
    pub action: String,
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

    pub fn plan_bounded_wallet_action(
        &self,
        network: &str,
        request: &BoundedAgentWalletActionRequest,
    ) -> Result<BoundedAgentWalletActionPlan, ChainBaseError> {
        if request.policy_version == 0 || request.deadline == 0 {
            return Err(ChainBaseError::InvalidVerificationConfiguration(
                "bounded-wallet policy version and action deadline must be positive".to_string(),
            ));
        }
        let network = base_network_descriptor(network)?;
        let wallet = normalize_address(&request.wallet_contract)?;
        let delegate = normalize_address(&request.delegate)?;
        let (
            action,
            action_code,
            spend_upper_bound,
            payload,
            direct_data,
            predicted_bounty_contract,
        ) = match &request.action {
            BoundedAgentWalletAction::Create { create } => {
                if normalize_address(&create.creator)? != wallet {
                    return Err(ChainBaseError::InvalidVerificationConfiguration(
                        "bounded-wallet bounty creator must equal the wallet contract".to_string(),
                    ));
                }
                let params = autonomous_create_param_words(create)?;
                let verifiers = normalized_verifiers(create)?;
                validate_autonomous_creation(create, &verifiers)?;
                let creation_nonce = parse_bytes32(&create.creation_nonce)?;
                let initial_funding = autonomous_money_to_uint256(&create.initial_funding, true)?;
                let arguments = encode_bounty_create_arguments(
                    &params,
                    &verifiers,
                    initial_funding,
                    creation_nonce,
                )?;
                let bounty_id = autonomous_bounty_id(
                    network.chain_id,
                    &self.factory_contract,
                    &wallet,
                    creation_nonce,
                    &params,
                    &verifiers,
                )?;
                let predicted = predict_minimal_proxy_address(
                    &self.factory_contract,
                    &self.implementation_contract,
                    bounty_id,
                )?;
                (
                        "create".to_string(),
                        0,
                        initial_funding,
                        arguments.clone(),
                        encode_selector_and_arguments(
                            "createBounty((uint256,uint256,bytes32,bytes32,bytes32,bytes32,bytes32,uint64,uint64,uint64,uint8,address,address,uint8),address[],uint256,bytes32)",
                            &arguments,
                        ),
                        Some(predicted),
                    )
            }
            BoundedAgentWalletAction::Fund {
                bounty_contract,
                amount,
            } => {
                let amount = autonomous_money_to_uint256(amount, false)?;
                let payload = encode_static_arguments(vec![
                    encode_address(bounty_contract)?,
                    encode_uint256(amount)?,
                ]);
                (
                    "fund".to_string(),
                    1,
                    amount,
                    payload.clone(),
                    encode_selector_and_arguments("fundBounty(address,uint256)", &payload),
                    None,
                )
            }
            BoundedAgentWalletAction::Claim {
                bounty_contract,
                expected_claim_bond,
            } => {
                let bond = autonomous_money_to_uint256(expected_claim_bond, false)?;
                let payload = encode_static_arguments(vec![encode_address(bounty_contract)?]);
                (
                    "claim".to_string(),
                    2,
                    bond,
                    payload.clone(),
                    encode_selector_and_arguments("claimBounty(address)", &payload),
                    None,
                )
            }
            BoundedAgentWalletAction::Submit {
                bounty_contract,
                submission_hash,
                evidence_hash,
            } => {
                let payload = encode_static_arguments(vec![
                    encode_address(bounty_contract)?,
                    parse_bytes32(submission_hash)?,
                    parse_bytes32(evidence_hash)?,
                ]);
                (
                    "submit".to_string(),
                    3,
                    0,
                    payload.clone(),
                    encode_selector_and_arguments(
                        "submitBounty(address,bytes32,bytes32)",
                        &payload,
                    ),
                    None,
                )
            }
        };
        let payload_hash = word_hex(Keccak256::digest(&payload).into());
        let direct_transaction = EvmTransactionIntent {
            from: Some(delegate.clone()),
            to: wallet.clone(),
            value_wei: 0,
            data: format!("0x{}", hex::encode(direct_data)),
            function: match action.as_str() {
                "create" => "createBounty((uint256,uint256,bytes32,bytes32,bytes32,bytes32,bytes32,uint64,uint64,uint64,uint8,address,address,uint8),address[],uint256,bytes32)",
                "fund" => "fundBounty(address,uint256)",
                "claim" => "claimBounty(address)",
                _ => "submitBounty(address,bytes32,bytes32)",
            }
            .to_string(),
        };
        let relay_authorization = bounded_wallet_typed_data(
            &network,
            &wallet,
            action_code,
            &payload_hash,
            request.delegate_nonce,
            request.deadline,
            request.policy_version,
        );
        Ok(BoundedAgentWalletActionPlan {
            protocol_version: BOUNDED_AGENT_WALLET_PROTOCOL_VERSION.to_string(),
            network,
            expected_factory_contract: self.factory_contract.clone(),
            wallet_contract: wallet,
            delegate,
            action,
            action_code,
            spend_upper_bound: spend_upper_bound.to_string(),
            payload: format!("0x{}", hex::encode(payload)),
            payload_hash,
            direct_transaction,
            relay_authorization,
            predicted_bounty_contract,
            evidence_boundary: "This unsigned plan is not authorization, funding, claim, submission, or payout evidence. Before signing, read the wallet's owner, factory, settlementToken, policy, policyVersion, delegateNonce, revoked, periodSpent, and lifetimeSpent on-chain and require exact agreement. Canonical events remain the only lifecycle and payment evidence.".to_string(),
        })
    }

    pub fn plan_bounded_wallet_authorized_action(
        &self,
        network: &str,
        request: &BoundedAgentWalletActionRequest,
        signature: &AutonomousBountyAuthorizationSignature,
        relayer: Option<&str>,
    ) -> Result<BoundedAgentWalletAuthorizedActionPlan, ChainBaseError> {
        let plan = self.plan_bounded_wallet_action(network, request)?;
        let signature = encode_signature_bytes(signature)?;
        let payload = parse_hex_bytes(&plan.payload)?;
        let data = encode_bounded_wallet_relay_call(
            plan.action_code,
            &payload,
            request.delegate_nonce,
            request.deadline,
            &signature,
        )?;
        Ok(BoundedAgentWalletAuthorizedActionPlan {
            protocol_version: plan.protocol_version,
            network: plan.network,
            wallet_contract: plan.wallet_contract.clone(),
            action: plan.action,
            relay_transaction: EvmTransactionIntent {
                from: relayer.map(normalize_address).transpose()?,
                to: plan.wallet_contract,
                value_wei: 0,
                data,
                function: "executeWithSignature(uint8,bytes,uint256,uint256,bytes)"
                    .to_string(),
            },
            evidence_boundary: "A delegate signature or relay transaction hash is not execution or payment evidence. Confirm the exact canonical action event and, for earnings, the canonical BountySettled event.".to_string(),
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
                "fundWithAuthorization(address,uint256,uint256,uint256,bytes32,uint8,bytes32,bytes32)",
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
            function: "fundWithAuthorization(address,uint256,uint256,uint256,bytes32,uint8,bytes32,bytes32)".to_string(),
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

pub fn validate_bounded_wallet_action_against_safe_state(
    request: &BoundedAgentWalletActionRequest,
    plan: &BoundedAgentWalletActionPlan,
    observation: &BoundedWalletSafeObservation,
) -> Result<(), ChainBaseError> {
    let expected_network = match plan.network.chain_id {
        8_453 => "base-mainnet",
        84_532 => "base-sepolia",
        _ => "unsupported",
    };
    if observation.protocol_version != BOUNDED_AGENT_WALLET_PROTOCOL_VERSION
        || observation.chain_id != plan.network.chain_id
        || observation.network != expected_network
    {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "bounded-wallet plan and safe observation identify different protocols or networks"
                .to_string(),
        ));
    }
    if normalize_address(&request.wallet_contract)? != observation.wallet_contract
        || normalize_address(&request.delegate)? != observation.policy.delegate
        || plan.wallet_contract != observation.wallet_contract
        || plan.delegate != observation.policy.delegate
        || plan.expected_factory_contract != observation.bounty_factory_contract
    {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "bounded-wallet plan does not match safe wallet identity or delegate state".to_string(),
        ));
    }
    let observed_nonce = observation.delegate_nonce.parse::<u128>().map_err(|_| {
        ChainBaseError::InvalidVerificationConfiguration(
            "bounded-wallet safe delegate nonce is invalid".to_string(),
        )
    })?;
    if !observation.active
        || observation.revoked
        || request.policy_version != observation.policy_version
        || request.delegate_nonce != observed_nonce
    {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "bounded-wallet policy is inactive, revoked, or stale".to_string(),
        ));
    }
    if request.deadline <= observation.safe_block_timestamp
        || request.deadline > observation.policy.valid_until
        || request.deadline > observation.safe_block_timestamp.saturating_add(900)
    {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "bounded-wallet action deadline must be live and no more than 15 minutes after the safe block"
                .to_string(),
        ));
    }
    let action_mask = 1u8.checked_shl(plan.action_code.into()).ok_or_else(|| {
        ChainBaseError::InvalidVerificationConfiguration(
            "bounded-wallet plan has an unsupported action code".to_string(),
        )
    })?;
    if observation.policy.allowed_actions & action_mask == 0 {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "bounded-wallet policy does not allow the planned action".to_string(),
        ));
    }
    if let BoundedAgentWalletAction::Create { create } = &request.action {
        let mode = match create.verification_mode {
            AutonomousVerificationMode::DeterministicModule => 0,
            AutonomousVerificationMode::SignedQuorum => 1,
            AutonomousVerificationMode::AiJudgeQuorum => 2,
        };
        if observation.policy.allowed_verification_modes & (1u8 << mode) == 0 {
            return Err(ChainBaseError::InvalidVerificationConfiguration(
                "bounded-wallet policy does not allow the bounty verification mode".to_string(),
            ));
        }
    }
    let spend = plan.spend_upper_bound.parse::<u128>().map_err(|_| {
        ChainBaseError::InvalidVerificationConfiguration(
            "bounded-wallet plan spend bound is invalid".to_string(),
        )
    })?;
    if spend == 0 {
        return Ok(());
    }
    let max_per_action = observation
        .policy
        .max_per_action
        .parse::<u128>()
        .map_err(|_| {
            ChainBaseError::InvalidVerificationConfiguration(
                "bounded-wallet safe per-action cap is invalid".to_string(),
            )
        })?;
    let max_per_period = observation
        .policy
        .max_per_period
        .parse::<u128>()
        .map_err(|_| {
            ChainBaseError::InvalidVerificationConfiguration(
                "bounded-wallet safe period cap is invalid".to_string(),
            )
        })?;
    let max_lifetime = observation
        .policy
        .max_lifetime_spend
        .parse::<u128>()
        .map_err(|_| {
            ChainBaseError::InvalidVerificationConfiguration(
                "bounded-wallet safe lifetime cap is invalid".to_string(),
            )
        })?;
    let observed_bucket = observation.period_bucket.parse::<u128>().map_err(|_| {
        ChainBaseError::InvalidVerificationConfiguration(
            "bounded-wallet safe period bucket is invalid".to_string(),
        )
    })?;
    let period_spent = observation.period_spent.parse::<u128>().map_err(|_| {
        ChainBaseError::InvalidVerificationConfiguration(
            "bounded-wallet safe period spend is invalid".to_string(),
        )
    })?;
    let lifetime_spent = observation.lifetime_spent.parse::<u128>().map_err(|_| {
        ChainBaseError::InvalidVerificationConfiguration(
            "bounded-wallet safe lifetime spend is invalid".to_string(),
        )
    })?;
    if observation.policy.period_seconds == 0 {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "bounded-wallet safe period length is zero".to_string(),
        ));
    }
    let current_bucket =
        u128::from(observation.safe_block_timestamp / observation.policy.period_seconds);
    let effective_period_spent = if current_bucket == observed_bucket {
        period_spent
    } else {
        0
    };
    if spend > max_per_action
        || effective_period_spent
            .checked_add(spend)
            .map_or(true, |next| next > max_per_period)
        || lifetime_spent
            .checked_add(spend)
            .map_or(true, |next| next > max_lifetime)
    {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "bounded-wallet plan exceeds the live per-action, period, or lifetime cap".to_string(),
        ));
    }
    Ok(())
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

pub const BASE_MAINNET_USDC_TOKEN_ADDRESS: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
pub const BASE_SEPOLIA_USDC_TOKEN_ADDRESS: &str = "0x036CbD53842c5426634e7929541eC2318f3dCF7e";
pub const AUTONOMOUS_BOUNTY_PROTOCOL_HASH: &str =
    "0x0afcbf01041498cc301207aa5cd21a838c522d8c057d9b29c2dd83d7d94053e7";

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedWalletDeploymentExpectedState {
    pub protocol_version: String,
    pub network: String,
    pub chain_id: u64,
    pub wallet_factory_contract: String,
    pub wallet_factory_runtime_code_hash: String,
    pub wallet_runtime_code_hash: String,
    pub bounty_factory_contract: String,
    pub native_usdc_token_address: String,
}

pub const CANONICAL_BASE_SEPOLIA_BOUNTY_FACTORY: &str =
    "0x95e28e0c270374cb1406f88e26ee68e49be50e92";
pub const CANONICAL_BASE_MAINNET_BOUNTY_FACTORY: &str =
    "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9";
pub const CANONICAL_BASE_SEPOLIA_BOUNDED_WALLET_FACTORY: &str =
    "0x38b5bec0b16d25ff1b0a6bb09f8f7f5a54dd3397";
pub const CANONICAL_BASE_SEPOLIA_BOUNDED_WALLET_FACTORY_CODE_HASH: &str =
    "0x119c73cb4442cf5a792e6b9e0ed20f1b811f6596b76cb3377c766732a2235a4c";
pub const CANONICAL_BASE_SEPOLIA_BOUNDED_WALLET_CODE_HASH: &str =
    "0xca08c0045ab20776437a0443aeda5a5558126820043088e55a8040e2a0d03311";
pub const CANONICAL_BASE_MAINNET_BOUNDED_WALLET_FACTORY: &str =
    "0x372d05f2843e945d5903148dbb1572ae51bdd51b";
pub const CANONICAL_BASE_MAINNET_BOUNDED_WALLET_FACTORY_CODE_HASH: &str =
    "0x5db581ee8f2652bccc7a04eeae7cb368dd4bb94188fd675f2cbaba65d868f365";
pub const CANONICAL_BASE_MAINNET_BOUNDED_WALLET_CODE_HASH: &str =
    "0xd72086e6d3ed8cd9d2fdc73c299d043d55a49f16c7081de77a4477450b928756";

pub fn canonical_bounded_wallet_expected_state(
    network: &str,
) -> Result<BoundedWalletDeploymentExpectedState, ChainBaseError> {
    let descriptor = base_network_descriptor(network)?;
    let (
        wallet_factory_contract,
        wallet_factory_runtime_code_hash,
        wallet_runtime_code_hash,
        bounty_factory_contract,
    ) = match descriptor.chain_id {
        84_532 => (
            CANONICAL_BASE_SEPOLIA_BOUNDED_WALLET_FACTORY,
            CANONICAL_BASE_SEPOLIA_BOUNDED_WALLET_FACTORY_CODE_HASH,
            CANONICAL_BASE_SEPOLIA_BOUNDED_WALLET_CODE_HASH,
            CANONICAL_BASE_SEPOLIA_BOUNTY_FACTORY,
        ),
        8_453 => (
            CANONICAL_BASE_MAINNET_BOUNDED_WALLET_FACTORY,
            CANONICAL_BASE_MAINNET_BOUNDED_WALLET_FACTORY_CODE_HASH,
            CANONICAL_BASE_MAINNET_BOUNDED_WALLET_CODE_HASH,
            CANONICAL_BASE_MAINNET_BOUNTY_FACTORY,
        ),
        _ => unreachable!("base_network_descriptor returned an unsupported chain"),
    };
    Ok(BoundedWalletDeploymentExpectedState {
        protocol_version: BOUNDED_AGENT_WALLET_PROTOCOL_VERSION.to_string(),
        network: if descriptor.chain_id == 8_453 {
            "base-mainnet".to_string()
        } else {
            "base-sepolia".to_string()
        },
        chain_id: descriptor.chain_id,
        wallet_factory_contract: wallet_factory_contract.to_string(),
        wallet_factory_runtime_code_hash: wallet_factory_runtime_code_hash.to_string(),
        wallet_runtime_code_hash: wallet_runtime_code_hash.to_string(),
        bounty_factory_contract: bounty_factory_contract.to_string(),
        native_usdc_token_address: descriptor.native_usdc_token_address,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedWalletPolicyObservation {
    pub delegate: String,
    pub valid_after: u64,
    pub valid_until: u64,
    pub period_seconds: u64,
    pub max_per_action: String,
    pub max_per_period: String,
    pub max_lifetime_spend: String,
    pub allowed_actions: u8,
    pub allowed_verification_modes: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedWalletSafeObservation {
    pub protocol_version: String,
    pub network: String,
    pub chain_id: u64,
    pub safe_block_number: u64,
    pub safe_block_hash: String,
    pub safe_block_timestamp: u64,
    pub block_tag: String,
    pub wallet_factory_contract: String,
    pub wallet_contract: String,
    pub owner: String,
    pub bounty_factory_contract: String,
    pub native_usdc_token_address: String,
    pub wallet_factory_runtime_code_hash: String,
    pub wallet_runtime_code_hash: String,
    pub policy: BoundedWalletPolicyObservation,
    pub policy_version: u64,
    pub delegate_nonce: String,
    pub period_bucket: String,
    pub period_spent: String,
    pub lifetime_spent: String,
    pub revoked: bool,
    pub active: bool,
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

pub async fn inspect_bounded_wallet_safe_state(
    rpc_url: &str,
    expected: &BoundedWalletDeploymentExpectedState,
    wallet_contract: &str,
) -> Result<BoundedWalletSafeObservation, ChainBaseError> {
    inspect_bounded_wallet_safe_state_with_transport(
        rpc_url,
        expected,
        wallet_contract,
        &ReqwestJsonRpcTransport::default(),
    )
    .await
}

pub async fn inspect_bounded_wallet_safe_state_with_transport<T>(
    rpc_url: &str,
    expected: &BoundedWalletDeploymentExpectedState,
    wallet_contract: &str,
    transport: &T,
) -> Result<BoundedWalletSafeObservation, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let network = base_network_descriptor(&expected.network)?;
    if expected.protocol_version != BOUNDED_AGENT_WALLET_PROTOCOL_VERSION
        || network.chain_id != expected.chain_id
    {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "bounded-wallet manifest has an unsupported protocol or chain".to_string(),
        ));
    }

    let wallet_factory_contract = normalize_address(&expected.wallet_factory_contract)?;
    let wallet_contract = normalize_address(wallet_contract)?;
    let bounty_factory_contract = normalize_address(&expected.bounty_factory_contract)?;
    let native_usdc_token_address = normalize_address(&expected.native_usdc_token_address)?;
    let zero_address = "0x0000000000000000000000000000000000000000";
    if wallet_factory_contract == zero_address
        || wallet_contract == zero_address
        || bounty_factory_contract == zero_address
    {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "bounded-wallet deployment addresses must be nonzero".to_string(),
        ));
    }
    if native_usdc_token_address != normalize_address(&network.native_usdc_token_address)? {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "bounded-wallet settlement token does not match the Base network".to_string(),
        ));
    }
    let wallet_factory_runtime_code_hash =
        normalize_hash(&expected.wallet_factory_runtime_code_hash)?;
    let wallet_runtime_code_hash = normalize_hash(&expected.wallet_runtime_code_hash)?;
    let empty_code_hash = "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470";
    if wallet_factory_runtime_code_hash == empty_code_hash
        || wallet_runtime_code_hash == empty_code_hash
    {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "bounded-wallet runtime code hashes must not identify empty code".to_string(),
        ));
    }

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

    let observed_factory_hash = fetch_account_code_hash(
        rpc_url,
        &wallet_factory_contract,
        &exact_block,
        2,
        transport,
    )
    .await?;
    require_canonical_match(
        "bounded-wallet factory runtime code hash",
        &observed_factory_hash,
        &wallet_factory_runtime_code_hash,
    )?;
    let observed_wallet_hash =
        fetch_account_code_hash(rpc_url, &wallet_contract, &exact_block, 3, transport).await?;
    require_canonical_match(
        "bounded-wallet runtime code hash",
        &observed_wallet_hash,
        &wallet_runtime_code_hash,
    )?;

    let factory_bounty_word = fetch_contract_word(
        rpc_url,
        &wallet_factory_contract,
        &encode_call("bountyFactory()", Vec::new()),
        &exact_block,
        4,
        transport,
    )
    .await?;
    let factory_bounty = address_from_word(parse_bytes32(&factory_bounty_word)?);
    require_canonical_match(
        "bounded-wallet factory bounty factory",
        &factory_bounty,
        &bounty_factory_contract,
    )?;

    let factory_token_word = fetch_contract_word(
        rpc_url,
        &wallet_factory_contract,
        &encode_call("settlementToken()", Vec::new()),
        &exact_block,
        5,
        transport,
    )
    .await?;
    let factory_token = address_from_word(parse_bytes32(&factory_token_word)?);
    require_canonical_match(
        "bounded-wallet factory settlement token",
        &factory_token,
        &native_usdc_token_address,
    )?;

    let registered_word = fetch_contract_word(
        rpc_url,
        &wallet_factory_contract,
        &encode_call(
            "isFactoryWallet(address)",
            vec![encode_address(&wallet_contract)?],
        ),
        &exact_block,
        6,
        transport,
    )
    .await?;
    if word_to_u128(parse_bytes32(&registered_word)?)? != 1 {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "wallet is not registered by the canonical bounded-wallet factory".to_string(),
        ));
    }

    let owner_word = fetch_contract_word(
        rpc_url,
        &wallet_contract,
        &encode_call("owner()", Vec::new()),
        &exact_block,
        7,
        transport,
    )
    .await?;
    let owner = address_from_word(parse_bytes32(&owner_word)?);
    if owner == zero_address {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "bounded-wallet owner must be nonzero".to_string(),
        ));
    }

    let wallet_bounty_word = fetch_contract_word(
        rpc_url,
        &wallet_contract,
        &encode_call("factory()", Vec::new()),
        &exact_block,
        8,
        transport,
    )
    .await?;
    let wallet_bounty = address_from_word(parse_bytes32(&wallet_bounty_word)?);
    require_canonical_match(
        "bounded-wallet bounty factory",
        &wallet_bounty,
        &bounty_factory_contract,
    )?;

    let wallet_token_word = fetch_contract_word(
        rpc_url,
        &wallet_contract,
        &encode_call("settlementToken()", Vec::new()),
        &exact_block,
        9,
        transport,
    )
    .await?;
    let wallet_token = address_from_word(parse_bytes32(&wallet_token_word)?);
    require_canonical_match(
        "bounded-wallet settlement token",
        &wallet_token,
        &native_usdc_token_address,
    )?;

    let policy_words = fetch_contract_words(
        rpc_url,
        &wallet_contract,
        &encode_call("policy()", Vec::new()),
        &exact_block,
        10,
        9,
        transport,
    )
    .await?;
    let delegate = address_from_word(policy_words[0]);
    if delegate == zero_address {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "bounded-wallet delegate must be nonzero".to_string(),
        ));
    }
    let valid_after = rpc_word_to_u64(policy_words[1], "policy validAfter")?;
    let valid_until = rpc_word_to_u64(policy_words[2], "policy validUntil")?;
    let period_seconds = rpc_word_to_u64(policy_words[3], "policy periodSeconds")?;
    let max_per_action = word_to_u128(policy_words[4])?;
    let max_per_period = word_to_u128(policy_words[5])?;
    let max_lifetime_spend = word_to_u128(policy_words[6])?;
    let allowed_actions = rpc_word_to_u8(policy_words[7], "policy allowedActions")?;
    let allowed_verification_modes =
        rpc_word_to_u8(policy_words[8], "policy allowedVerificationModes")?;
    if valid_until <= valid_after
        || period_seconds == 0
        || max_per_action == 0
        || max_per_period == 0
        || max_lifetime_spend == 0
        || allowed_actions == 0
        || allowed_verification_modes == 0
    {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "bounded-wallet policy is structurally inactive".to_string(),
        ));
    }

    let policy_version = fetch_contract_u64(
        rpc_url,
        &wallet_contract,
        "policyVersion()",
        &exact_block,
        11,
        transport,
    )
    .await?;
    if policy_version == 0 {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "bounded-wallet policy version must be positive".to_string(),
        ));
    }
    let delegate_nonce = fetch_contract_u128(
        rpc_url,
        &wallet_contract,
        "delegateNonce()",
        &exact_block,
        12,
        transport,
    )
    .await?;
    let period_bucket = fetch_contract_u128(
        rpc_url,
        &wallet_contract,
        "periodBucket()",
        &exact_block,
        13,
        transport,
    )
    .await?;
    let period_spent = fetch_contract_u128(
        rpc_url,
        &wallet_contract,
        "periodSpent()",
        &exact_block,
        14,
        transport,
    )
    .await?;
    let lifetime_spent = fetch_contract_u128(
        rpc_url,
        &wallet_contract,
        "lifetimeSpent()",
        &exact_block,
        15,
        transport,
    )
    .await?;
    let revoked_word = fetch_contract_word(
        rpc_url,
        &wallet_contract,
        &encode_call("revoked()", Vec::new()),
        &exact_block,
        16,
        transport,
    )
    .await?;
    let revoked_raw = word_to_u128(parse_bytes32(&revoked_word)?)?;
    if revoked_raw > 1 {
        return Err(ChainBaseError::InvalidRpcResponse(
            "bounded-wallet revoked() returned a non-boolean word".to_string(),
        ));
    }
    let revoked = revoked_raw == 1;
    let active =
        !revoked && safe_block_timestamp >= valid_after && safe_block_timestamp <= valid_until;

    Ok(BoundedWalletSafeObservation {
        protocol_version: expected.protocol_version.clone(),
        network: expected.network.clone(),
        chain_id: expected.chain_id,
        safe_block_number,
        safe_block_hash,
        safe_block_timestamp,
        block_tag: "safe".to_string(),
        wallet_factory_contract,
        wallet_contract,
        owner,
        bounty_factory_contract,
        native_usdc_token_address,
        wallet_factory_runtime_code_hash,
        wallet_runtime_code_hash,
        policy: BoundedWalletPolicyObservation {
            delegate,
            valid_after,
            valid_until,
            period_seconds,
            max_per_action: max_per_action.to_string(),
            max_per_period: max_per_period.to_string(),
            max_lifetime_spend: max_lifetime_spend.to_string(),
            allowed_actions,
            allowed_verification_modes,
        },
        policy_version,
        delegate_nonce: delegate_nonce.to_string(),
        period_bucket: period_bucket.to_string(),
        period_spent: period_spent.to_string(),
        lifetime_spent: lifetime_spent.to_string(),
        revoked,
        active,
        evidence_boundary: "This observation proves exact factory and wallet code, immutable configuration, registration, owner, and policy state at one Base safe block. It does not authorize an action and does not prove funding, completion, payout, or settlement.".to_string(),
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
                    "method": "eth_getCode",
                    "params": [address, block]
                }),
            )
            .await?,
        request_id,
        "eth_getCode",
    )?;
    let encoded = result.as_str().ok_or_else(|| {
        ChainBaseError::InvalidRpcResponse("eth_getCode result is not hex bytecode".to_string())
    })?;
    let bytes = parse_hex_bytes(encoded)?;
    Ok(format!("0x{}", hex::encode(Keccak256::digest(bytes))))
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

async fn fetch_contract_words<T>(
    rpc_url: &str,
    contract: &str,
    data: &str,
    block: &str,
    request_id: u64,
    expected_words: usize,
    transport: &T,
) -> Result<Vec<[u8; 32]>, ChainBaseError>
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
    let encoded = result.as_str().ok_or_else(|| {
        ChainBaseError::InvalidRpcResponse("eth_call result is not ABI data".to_string())
    })?;
    decode_words(encoded, expected_words, "eth_call result").map_err(|_| {
        ChainBaseError::InvalidRpcResponse(format!(
            "eth_call result is not exactly {expected_words} ABI words"
        ))
    })
}

async fn fetch_contract_u128<T>(
    rpc_url: &str,
    contract: &str,
    signature: &str,
    block: &str,
    request_id: u64,
    transport: &T,
) -> Result<u128, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let word = fetch_contract_word(
        rpc_url,
        contract,
        &encode_call(signature, Vec::new()),
        block,
        request_id,
        transport,
    )
    .await?;
    word_to_u128(parse_bytes32(&word)?)
}

async fn fetch_contract_u64<T>(
    rpc_url: &str,
    contract: &str,
    signature: &str,
    block: &str,
    request_id: u64,
    transport: &T,
) -> Result<u64, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let value =
        fetch_contract_u128(rpc_url, contract, signature, block, request_id, transport).await?;
    u64::try_from(value).map_err(|_| {
        ChainBaseError::InvalidRpcResponse(format!("{signature} result exceeds uint64"))
    })
}

fn rpc_word_to_u64(word: [u8; 32], field: &str) -> Result<u64, ChainBaseError> {
    let value = word_to_u128(word)?;
    u64::try_from(value)
        .map_err(|_| ChainBaseError::InvalidRpcResponse(format!("{field} exceeds uint64")))
}

fn rpc_word_to_u8(word: [u8; 32], field: &str) -> Result<u8, ChainBaseError> {
    let value = word_to_u128(word)?;
    u8::try_from(value)
        .map_err(|_| ChainBaseError::InvalidRpcResponse(format!("{field} exceeds uint8")))
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
    let artifact_reference = artifact_reference.trim();
    if item.status != "submitted"
        || round == 0
        || artifact_reference.is_empty()
        || artifact_reference.len() > 16 * 1024
        || !item.bounty_contract.eq_ignore_ascii_case(bounty_contract)
        || !item.bounty_id.eq_ignore_ascii_case(bounty_id)
    {
        return Err(ChainBaseError::InvalidSubmissionEvidence(
            "bounty identity, state, round, or artifact reference is invalid".to_string(),
        ));
    }
    let evidence_size = serde_json::to_vec(&evidence)
        .map_err(|error| ChainBaseError::InvalidCanonicalJson(error.to_string()))?
        .len();
    if evidence_size > 256 * 1024 || !evidence.is_object() {
        return Err(ChainBaseError::InvalidSubmissionEvidence(
            "evidence must be an object no larger than 256 KiB".to_string(),
        ));
    }
    let solver_wallet = normalize_evm_address(solver_wallet)?;
    let bounty_contract = normalize_evm_address(bounty_contract)?;
    let artifact_hash = sha256_utf8(artifact_reference);
    let evidence_hash = sha256_canonical_json(&evidence)?;
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
        artifact_reference: artifact_reference.to_string(),
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
        let validation_errors =
            validate_autonomous_terms_against_creation(&creation_data, terms_record.as_ref());
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
    let canonical = canonical_json_value(value);
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|error| ChainBaseError::InvalidCanonicalJson(error.to_string()))?;
    Ok(format!("0x{}", hex::encode(Keccak256::digest(bytes))))
}

pub fn sha256_utf8(value: &str) -> String {
    format!("0x{}", hex::encode(Sha256::digest(value.as_bytes())))
}

pub fn sha256_canonical_json(value: &Value) -> Result<String, ChainBaseError> {
    let canonical = canonical_json_value(value);
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|error| ChainBaseError::InvalidCanonicalJson(error.to_string()))?;
    Ok(format!("0x{}", hex::encode(Sha256::digest(bytes))))
}

pub fn build_autonomous_bounty_terms_record(
    creator_wallet: &str,
    document: AutonomousBountyTermsDocument,
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
            name: "USD Coin".to_string(),
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

fn bounded_wallet_typed_data(
    network: &BaseNetworkDescriptor,
    wallet: &str,
    action: u8,
    payload_hash: &str,
    nonce: u128,
    deadline: u64,
    policy_version: u64,
) -> BoundedAgentWalletAuthorizationTypedData {
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
        "AgentAction".to_string(),
        vec![
            eip712_field("wallet", "address"),
            eip712_field("action", "uint8"),
            eip712_field("payloadHash", "bytes32"),
            eip712_field("nonce", "uint256"),
            eip712_field("deadline", "uint256"),
            eip712_field("policyVersion", "uint64"),
        ],
    );
    BoundedAgentWalletAuthorizationTypedData {
        types,
        domain: Eip712DomainData {
            name: "Agent Bounties Bounded Wallet".to_string(),
            version: "1".to_string(),
            chain_id: network.chain_id,
            verifying_contract: wallet.to_string(),
        },
        primary_type: "AgentAction".to_string(),
        message: BoundedAgentWalletAuthorizationMessage {
            wallet: wallet.to_string(),
            action: action.to_string(),
            payload_hash: payload_hash.to_string(),
            nonce: nonce.to_string(),
            deadline: deadline.to_string(),
            policy_version: policy_version.to_string(),
        },
    }
}

fn encode_signature_bytes(
    signature: &AutonomousBountyAuthorizationSignature,
) -> Result<Vec<u8>, ChainBaseError> {
    let mut bytes = Vec::with_capacity(65);
    bytes.extend_from_slice(&parse_bytes32(&signature.r)?);
    bytes.extend_from_slice(&parse_bytes32(&signature.s)?);
    bytes.push(normalized_signature_v(signature.v)?);
    Ok(bytes)
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

fn encode_autonomous_create_call(
    params: &[[u8; 32]],
    verifiers: &[[u8; 32]],
    initial_funding: u128,
    creation_nonce: [u8; 32],
) -> Result<String, ChainBaseError> {
    const SIGNATURE: &str = "createBounty((uint256,uint256,bytes32,bytes32,bytes32,bytes32,bytes32,uint64,uint64,uint64,uint8,address,address,uint8),address[],uint256,bytes32)";
    let arguments =
        encode_bounty_create_arguments(params, verifiers, initial_funding, creation_nonce)?;
    Ok(format!(
        "0x{}",
        hex::encode(encode_selector_and_arguments(SIGNATURE, &arguments))
    ))
}

fn encode_bounty_create_arguments(
    params: &[[u8; 32]],
    verifiers: &[[u8; 32]],
    initial_funding: u128,
    creation_nonce: [u8; 32],
) -> Result<Vec<u8>, ChainBaseError> {
    if params.len() != 14 {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "factory parameter tuple must contain fourteen words".to_string(),
        ));
    }
    let mut bytes = Vec::new();
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
    Ok(bytes)
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
    let arguments = encode_static_arguments(words);
    let bytes = encode_selector_and_arguments(signature, &arguments);
    format!("0x{}", hex::encode(bytes))
}

fn encode_static_arguments(words: Vec<[u8; 32]>) -> Vec<u8> {
    words.into_iter().flatten().collect()
}

fn encode_selector_and_arguments(signature: &str, arguments: &[u8]) -> Vec<u8> {
    let mut bytes = selector(signature).to_vec();
    bytes.extend_from_slice(arguments);
    bytes
}

fn encode_bounded_wallet_relay_call(
    action: u8,
    payload: &[u8],
    nonce: u128,
    deadline: u64,
    signature: &[u8],
) -> Result<String, ChainBaseError> {
    const SIGNATURE: &str = "executeWithSignature(uint8,bytes,uint256,uint256,bytes)";
    let payload_tail = encode_dynamic_bytes(payload)?;
    let signature_tail = encode_dynamic_bytes(signature)?;
    let mut arguments = Vec::with_capacity(5 * 32 + payload_tail.len() + signature_tail.len());
    arguments.extend_from_slice(&encode_uint256(action.into())?);
    arguments.extend_from_slice(&encode_uint256(5 * 32)?);
    arguments.extend_from_slice(&encode_uint256(nonce)?);
    arguments.extend_from_slice(&encode_uint256(deadline.into())?);
    arguments.extend_from_slice(&encode_uint256((5 * 32 + payload_tail.len()) as u128)?);
    arguments.extend_from_slice(&payload_tail);
    arguments.extend_from_slice(&signature_tail);
    Ok(format!(
        "0x{}",
        hex::encode(encode_selector_and_arguments(SIGNATURE, &arguments))
    ))
}

fn encode_dynamic_bytes(value: &[u8]) -> Result<Vec<u8>, ChainBaseError> {
    let mut bytes = Vec::with_capacity(32 + value.len() + 31);
    bytes.extend_from_slice(&encode_uint256(value.len() as u128)?);
    bytes.extend_from_slice(value);
    let padding = (32 - value.len() % 32) % 32;
    bytes.resize(bytes.len() + padding, 0);
    Ok(bytes)
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
    fn function_selectors_match_solidity_contract() {
        assert_eq!(
            hex::encode(selector("createBounty((uint256,uint256,bytes32,bytes32,bytes32,bytes32,bytes32,uint64,uint64,uint64,uint8,address,address,uint8),address[],uint256,bytes32)")),
            "9d2e414c"
        );
        assert_eq!(hex::encode(selector("fund(uint256)")), "ca1d209d");
        assert_eq!(
            hex::encode(selector("fundWithAuthorization(address,uint256,uint256,uint256,bytes32,uint8,bytes32,bytes32)")),
            "e1c9e96f"
        );
        assert_eq!(hex::encode(selector("claim()")), "4e71d92d");
        assert_eq!(hex::encode(selector("submit(bytes32,bytes32)")), "d26ff86e");
        assert_eq!(
            hex::encode(selector("fundBounty(address,uint256)")),
            "f0206e56"
        );
        assert_eq!(hex::encode(selector("claimBounty(address)")), "98ff8075");
        assert_eq!(
            hex::encode(selector("submitBounty(address,bytes32,bytes32)")),
            "856191ac"
        );
        assert_eq!(
            hex::encode(selector(
                "executeWithSignature(uint8,bytes,uint256,uint256,bytes)"
            )),
            "7272147c"
        );
        assert_eq!(hex::encode(selector("verifyAndSettle(bytes)")), "ed827cee");
        assert_eq!(
            hex::encode(selector(
                "settleWithAttestations((address,bool,bytes32,uint256,bytes)[])"
            )),
            "e3457186"
        );
    }

    #[test]
    fn bounded_wallet_fund_plan_matches_cast_and_binds_delegate_policy() {
        let planner = AutonomousBountyTxPlanner::new(
            "0x1111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
        )
        .unwrap();
        let request = BoundedAgentWalletActionRequest {
            wallet_contract: "0x3333333333333333333333333333333333333333".to_string(),
            delegate: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            policy_version: 3,
            delegate_nonce: 7,
            deadline: 2_000_000_000,
            action: BoundedAgentWalletAction::Fund {
                bounty_contract: "0x4444444444444444444444444444444444444444".to_string(),
                amount: Money::new(25, "usdc").unwrap(),
            },
        };

        let plan = planner
            .plan_bounded_wallet_action("base-sepolia", &request)
            .unwrap();

        assert_eq!(plan.action_code, 1);
        assert_eq!(plan.spend_upper_bound, "25");
        assert_eq!(
            plan.payload,
            format!(
                "0x{}{}",
                "0".repeat(24),
                format!("{}{:064x}", "44".repeat(20), 25)
            )
        );
        assert_eq!(
            plan.payload_hash,
            "0xe465fccd2205a9756702c4215d477404ca62f0abd2d9d645cd83bbb7e5f95e04"
        );
        assert_eq!(
            plan.direct_transaction.data,
            format!("0xf0206e56{}", &plan.payload[2..])
        );
        assert_eq!(
            plan.relay_authorization.domain.verifying_contract,
            request.wallet_contract
        );
        assert_eq!(plan.relay_authorization.message.action, "1");
        assert_eq!(plan.relay_authorization.message.nonce, "7");
        assert_eq!(plan.relay_authorization.message.policy_version, "3");
    }

    #[test]
    fn bounded_wallet_plan_requires_fresh_safe_policy_and_remaining_caps() {
        let planner = AutonomousBountyTxPlanner::new(
            "0x1111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
        )
        .unwrap();
        let request = BoundedAgentWalletActionRequest {
            wallet_contract: "0x3333333333333333333333333333333333333333".to_string(),
            delegate: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            policy_version: 3,
            delegate_nonce: 7,
            deadline: 2_000_000_000,
            action: BoundedAgentWalletAction::Fund {
                bounty_contract: "0x4444444444444444444444444444444444444444".to_string(),
                amount: Money::new(25, "usdc").unwrap(),
            },
        };
        let plan = planner
            .plan_bounded_wallet_action("base-sepolia", &request)
            .unwrap();
        let observation = BoundedWalletSafeObservation {
            protocol_version: BOUNDED_AGENT_WALLET_PROTOCOL_VERSION.to_string(),
            network: "base-sepolia".to_string(),
            chain_id: 84_532,
            safe_block_number: 100,
            safe_block_hash: format!("0x{}", "11".repeat(32)),
            safe_block_timestamp: 1_999_999_500,
            block_tag: "safe".to_string(),
            wallet_factory_contract: "0x5555555555555555555555555555555555555555".to_string(),
            wallet_contract: request.wallet_contract.clone(),
            owner: "0x6666666666666666666666666666666666666666".to_string(),
            bounty_factory_contract: planner.factory_contract.clone(),
            native_usdc_token_address: BASE_SEPOLIA_USDC_TOKEN_ADDRESS.to_string(),
            wallet_factory_runtime_code_hash: format!("0x{}", "22".repeat(32)),
            wallet_runtime_code_hash: format!("0x{}", "33".repeat(32)),
            policy: BoundedWalletPolicyObservation {
                delegate: request.delegate.clone(),
                valid_after: 1_999_999_000,
                valid_until: 2_000_000_500,
                period_seconds: 86_400,
                max_per_action: "100".to_string(),
                max_per_period: "1000".to_string(),
                max_lifetime_spend: "2000".to_string(),
                allowed_actions: 15,
                allowed_verification_modes: 1,
            },
            policy_version: request.policy_version,
            delegate_nonce: request.delegate_nonce.to_string(),
            period_bucket: (1_999_999_500u128 / 86_400).to_string(),
            period_spent: "900".to_string(),
            lifetime_spent: "1000".to_string(),
            revoked: false,
            active: true,
            evidence_boundary: "test".to_string(),
        };

        validate_bounded_wallet_action_against_safe_state(&request, &plan, &observation).unwrap();

        let mut exhausted = observation.clone();
        exhausted.period_spent = "990".to_string();
        let error = validate_bounded_wallet_action_against_safe_state(&request, &plan, &exhausted)
            .unwrap_err();
        assert!(matches!(
            error,
            ChainBaseError::InvalidVerificationConfiguration(message)
                if message.contains("exceeds")
        ));
    }

    #[test]
    fn bounded_wallet_relay_call_matches_cast_vector() {
        let planner = AutonomousBountyTxPlanner::new(
            "0x1111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
        )
        .unwrap();
        let request = BoundedAgentWalletActionRequest {
            wallet_contract: "0x3333333333333333333333333333333333333333".to_string(),
            delegate: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            policy_version: 3,
            delegate_nonce: 7,
            deadline: 2_000_000_000,
            action: BoundedAgentWalletAction::Fund {
                bounty_contract: "0x4444444444444444444444444444444444444444".to_string(),
                amount: Money::new(25, "usdc").unwrap(),
            },
        };
        let signature = AutonomousBountyAuthorizationSignature {
            v: 27,
            r: format!("0x{}", "11".repeat(32)),
            s: format!("0x{}", "22".repeat(32)),
        };

        let plan = planner
            .plan_bounded_wallet_authorized_action(
                "base-sepolia",
                &request,
                &signature,
                Some("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            )
            .unwrap();

        assert_eq!(
            plan.relay_transaction.data,
            "0x7272147c000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000007000000000000000000000000000000000000000000000000000000007735940000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000040000000000000000000000000444444444444444444444444444444444444444400000000000000000000000000000000000000000000000000000000000000190000000000000000000000000000000000000000000000000000000000000041111111111111111111111111111111111111111111111111111111111111111122222222222222222222222222222222222222222222222222222222222222221b00000000000000000000000000000000000000000000000000000000000000"
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
        let factory_code = "0x6001600055";
        let implementation_code = "0x6002600055";
        AutonomousFactoryExpectedState {
            protocol_version: "agent-bounties/autonomous-v1".to_string(),
            network: "base-mainnet".to_string(),
            chain_id: 8_453,
            factory_contract: "0x1111111111111111111111111111111111111111".to_string(),
            implementation_contract: "0x2222222222222222222222222222222222222222".to_string(),
            native_usdc_token_address: BASE_MAINNET_USDC_TOKEN_ADDRESS.to_string(),
            protocol_hash: AUTONOMOUS_BOUNTY_PROTOCOL_HASH.to_string(),
            factory_runtime_code_hash: format!(
                "0x{}",
                hex::encode(Keccak256::digest(parse_hex_bytes(factory_code).unwrap()))
            ),
            implementation_runtime_code_hash: format!(
                "0x{}",
                hex::encode(Keccak256::digest(
                    parse_hex_bytes(implementation_code).unwrap()
                ))
            ),
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
                "result": "0x6001600055"
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "result": "0x6002600055"
            }),
            json!({ "jsonrpc": "2.0", "id": 4, "result": expected.protocol_hash }),
            json!({ "jsonrpc": "2.0", "id": 5, "result": implementation_word }),
            json!({ "jsonrpc": "2.0", "id": 6, "result": token_word }),
        ])
    }

    fn bounded_wallet_expected_state() -> BoundedWalletDeploymentExpectedState {
        let factory_code = "0x6003600055";
        let wallet_code = "0x6004600055";
        BoundedWalletDeploymentExpectedState {
            protocol_version: BOUNDED_AGENT_WALLET_PROTOCOL_VERSION.to_string(),
            network: "base-mainnet".to_string(),
            chain_id: 8_453,
            wallet_factory_contract: "0x1111111111111111111111111111111111111111".to_string(),
            wallet_factory_runtime_code_hash: format!(
                "0x{}",
                hex::encode(Keccak256::digest(parse_hex_bytes(factory_code).unwrap()))
            ),
            wallet_runtime_code_hash: format!(
                "0x{}",
                hex::encode(Keccak256::digest(parse_hex_bytes(wallet_code).unwrap()))
            ),
            bounty_factory_contract: "0x2222222222222222222222222222222222222222".to_string(),
            native_usdc_token_address: BASE_MAINNET_USDC_TOKEN_ADDRESS.to_string(),
        }
    }

    fn abi_address_word(address: &str) -> String {
        format!("{:0>64}", address.trim_start_matches("0x"))
    }

    fn abi_uint_word(value: u128) -> String {
        format!("{value:064x}")
    }

    fn bounded_wallet_rpc_responses(
        expected: &BoundedWalletDeploymentExpectedState,
        owner: &str,
        delegate: &str,
    ) -> VecDeque<Value> {
        let policy = format!(
            "0x{}{}{}{}{}{}{}{}{}",
            abi_address_word(delegate),
            abi_uint_word(1),
            abi_uint_word(2_000_000_000),
            abi_uint_word(86_400),
            abi_uint_word(1_000_000),
            abi_uint_word(2_000_000),
            abi_uint_word(5_000_000),
            abi_uint_word(15),
            abi_uint_word(1),
        );
        let one = format!("0x{}", abi_uint_word(1));
        let zero = format!("0x{}", abi_uint_word(0));
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
            json!({ "jsonrpc": "2.0", "id": 2, "result": "0x6003600055" }),
            json!({ "jsonrpc": "2.0", "id": 3, "result": "0x6004600055" }),
            json!({ "jsonrpc": "2.0", "id": 4, "result": format!("0x{}", abi_address_word(&expected.bounty_factory_contract)) }),
            json!({ "jsonrpc": "2.0", "id": 5, "result": format!("0x{}", abi_address_word(&expected.native_usdc_token_address)) }),
            json!({ "jsonrpc": "2.0", "id": 6, "result": one }),
            json!({ "jsonrpc": "2.0", "id": 7, "result": format!("0x{}", abi_address_word(owner)) }),
            json!({ "jsonrpc": "2.0", "id": 8, "result": format!("0x{}", abi_address_word(&expected.bounty_factory_contract)) }),
            json!({ "jsonrpc": "2.0", "id": 9, "result": format!("0x{}", abi_address_word(&expected.native_usdc_token_address)) }),
            json!({ "jsonrpc": "2.0", "id": 10, "result": policy }),
            json!({ "jsonrpc": "2.0", "id": 11, "result": format!("0x{}", abi_uint_word(1)) }),
            json!({ "jsonrpc": "2.0", "id": 12, "result": zero }),
            json!({ "jsonrpc": "2.0", "id": 13, "result": format!("0x{}", abi_uint_word(19_923)) }),
            json!({ "jsonrpc": "2.0", "id": 14, "result": format!("0x{}", abi_uint_word(250_000)) }),
            json!({ "jsonrpc": "2.0", "id": 15, "result": format!("0x{}", abi_uint_word(500_000)) }),
            json!({ "jsonrpc": "2.0", "id": 16, "result": format!("0x{}", abi_uint_word(0)) }),
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
        assert_eq!(requests[1]["method"], "eth_getCode");
        assert_eq!(requests[1]["params"][1], "0x10");
        assert_eq!(requests[2]["params"][1], "0x10");
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

    #[tokio::test]
    async fn bounded_wallet_inspection_pins_code_registration_and_policy_to_one_safe_block() {
        let expected = bounded_wallet_expected_state();
        let wallet = "0x4444444444444444444444444444444444444444";
        let owner = "0x5555555555555555555555555555555555555555";
        let delegate = "0x6666666666666666666666666666666666666666";
        let seen_requests = Arc::new(Mutex::new(Vec::new()));
        let transport = SequenceTransport {
            seen_requests: seen_requests.clone(),
            responses: Mutex::new(bounded_wallet_rpc_responses(&expected, owner, delegate)),
        };

        let observation = inspect_bounded_wallet_safe_state_with_transport(
            "https://mainnet.base.org",
            &expected,
            wallet,
            &transport,
        )
        .await
        .unwrap();

        assert_eq!(observation.safe_block_number, 16);
        assert_eq!(observation.wallet_contract, wallet);
        assert_eq!(observation.owner, owner);
        assert_eq!(observation.policy.delegate, delegate);
        assert_eq!(observation.policy.max_per_action, "1000000");
        assert_eq!(observation.policy.allowed_actions, 15);
        assert_eq!(observation.policy.allowed_verification_modes, 1);
        assert_eq!(observation.policy_version, 1);
        assert_eq!(observation.delegate_nonce, "0");
        assert_eq!(observation.period_spent, "250000");
        assert_eq!(observation.lifetime_spent, "500000");
        assert!(observation.active);

        let requests = seen_requests.lock().unwrap();
        assert_eq!(requests.len(), 16);
        assert_eq!(requests[0]["params"], json!(["safe", false]));
        for request in &requests[1..] {
            assert_eq!(
                request["params"][request["params"].as_array().unwrap().len() - 1],
                "0x10"
            );
        }
    }

    #[tokio::test]
    async fn bounded_wallet_inspection_rejects_unregistered_wallet() {
        let expected = bounded_wallet_expected_state();
        let wallet = "0x4444444444444444444444444444444444444444";
        let mut responses = bounded_wallet_rpc_responses(
            &expected,
            "0x5555555555555555555555555555555555555555",
            "0x6666666666666666666666666666666666666666",
        );
        responses[5] = json!({
            "jsonrpc": "2.0",
            "id": 6,
            "result": format!("0x{}", abi_uint_word(0))
        });
        let transport = SequenceTransport {
            seen_requests: Arc::new(Mutex::new(Vec::new())),
            responses: Mutex::new(responses),
        };

        let error = inspect_bounded_wallet_safe_state_with_transport(
            "https://mainnet.base.org",
            &expected,
            wallet,
            &transport,
        )
        .await
        .unwrap_err();

        assert!(matches!(
            error,
            ChainBaseError::InvalidVerificationConfiguration(message)
                if message.contains("not registered")
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
