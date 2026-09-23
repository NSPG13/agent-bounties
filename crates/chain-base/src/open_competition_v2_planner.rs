use super::{
    base_network_descriptor, normalize_evm_address, BaseNetworkDescriptor, ChainBaseError,
    Eip712DomainData, Eip712TypeField, EvmTransactionIntent,
};
use alloy::{
    primitives::{keccak256, Address, B256, I256, U256},
    sol,
    sol_types::{SolCall, SolValue},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};

/// Planner utilities for Open Competition V2.

pub const OPEN_COMPETITION_V2_GROTH16_ID: &str = "sp1-groth16";
pub const OPEN_COMPETITION_V2_PLONK_ID: &str = "sp1-plonk";
pub const OPEN_COMPETITION_V2_BASE_USDC: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
pub const OPEN_COMPETITION_V2_BASE_SEPOLIA_USDC: &str =
    "0x036CbD53842c5426634e7929541eC2318f3dCF7e";
sol! {
    struct CompetitionV2CreateParamsAbi {
        uint256 solverReward;
        uint256 keeperReward;
        uint64 fundingDeadline;
        uint64 proofWindowSeconds;
        uint8 winnerMode;
        uint8 scoreDirection;
        int256 scoreThreshold;
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
    }

    interface IERC20CompetitionV2 {
        function approve(address spender, uint256 amount) external returns (bool);
        function transfer(address recipient, uint256 amount) external returns (bool);
        function transferWithAuthorization(
            address from,
            address to,
            uint256 value,
            uint256 validAfter,
            uint256 validBefore,
            bytes32 nonce,
            uint8 v,
            bytes32 r,
            bytes32 s
        ) external;
    }

    interface IOpenCompetitionFactoryV2Beta3Planner {
        function createCompetition(
            CompetitionV2CreateParamsAbi params,
            uint256 initialFunding,
            bytes32 creationNonce,
            bytes32 acknowledgedRiskHash
        ) external returns (address competitionAddress, bytes32 bountyId);
    }

    interface IOpenCompetitionBountyV2Beta3Planner {
        function fund(uint256 requestedAmount, bytes32 acknowledgedRiskHash)
            external returns (uint256 acceptedAmount);
        function submitProof(bytes publicValues, bytes proofBytes) external;
        function submitProofFor(
            bytes publicValues,
            bytes proofBytes,
            uint256 authorizationDeadline,
            bytes solverSignature
        ) external;
        function finalizeBestScore() external;
        function cancelFunding() external;
        function expireCompetition() external;
        function cancelForUnavailableVerifier() external;
        function withdrawRefundFor(address contributor) external returns (uint256 amount);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionV2BrokerPaymentAuthorization {
    pub payer: String,
    pub recipient: String,
    pub amount: u64,
    pub valid_before: u64,
    pub nonce: String,
    pub v: u8,
    pub r: String,
    pub s: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenCompetitionV2WinnerMode {
    FirstProven,
    BestScore,
}

impl OpenCompetitionV2WinnerMode {
    fn abi_value(self) -> u8 {
        match self {
            Self::FirstProven => 0,
            Self::BestScore => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenCompetitionV2ScoreDirection {
    HigherIsBetter,
    LowerIsBetter,
}

impl OpenCompetitionV2ScoreDirection {
    fn abi_value(self) -> u8 {
        match self {
            Self::HigherIsBetter => 0,
            Self::LowerIsBetter => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenCompetitionV2ProofSystem {
    Groth16,
    Plonk,
}

impl OpenCompetitionV2ProofSystem {
    pub fn id(self) -> &'static str {
        match self {
            Self::Groth16 => OPEN_COMPETITION_V2_GROTH16_ID,
            Self::Plonk => OPEN_COMPETITION_V2_PLONK_ID,
        }
    }

    pub fn hash(self) -> B256 {
        keccak256(self.id())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionV2CreateParams {
    pub solver_reward: u128,
    pub keeper_reward: u128,
    pub funding_deadline: u64,
    pub proof_window_seconds: u64,
    pub winner_mode: OpenCompetitionV2WinnerMode,
    pub score_direction: OpenCompetitionV2ScoreDirection,
    pub score_threshold: String,
    pub proof_system: OpenCompetitionV2ProofSystem,
    pub program_vkey: String,
    pub source_hash: String,
    pub elf_hash: String,
    pub journal_schema_hash: String,
    pub metric_program_hash: String,
    pub execution_policy_hash: String,
    pub verification_policy_hash: String,
    pub settlement_policy_hash: String,
    pub beta_risk_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenCompetitionV2ProgramClassification {
    Reviewed,
    CustomUnreviewed,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionV2MetricProgramRelease {
    pub profile_id: String,
    pub classification: OpenCompetitionV2ProgramClassification,
    pub program_vkey: String,
    pub source_hash: String,
    pub elf_hash: String,
    pub journal_schema_hash: String,
    pub metric_program_hash: String,
    pub review_evidence_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionV2Release {
    pub protocol_version: String,
    pub network: String,
    pub source_commit: String,
    pub repository_subject_hash: String,
    pub sp1_source_commit: String,
    pub sp1_circuit_version: String,
    pub factory_contract: String,
    pub factory_runtime_code_hash: String,
    pub implementation_contract: String,
    pub implementation_runtime_code_hash: String,
    pub settlement_token: String,
    pub groth16_verifier: String,
    pub groth16_verifier_hash: String,
    pub groth16_verifier_runtime_code_hash: String,
    pub groth16_adapter: String,
    pub groth16_adapter_runtime_code_hash: String,
    pub plonk_verifier: String,
    pub plonk_verifier_hash: String,
    pub plonk_verifier_runtime_code_hash: String,
    pub plonk_adapter: String,
    pub plonk_adapter_runtime_code_hash: String,
    pub deployment_block: u64,
    pub release_hash: String,
    pub beta_risk_hash: String,
    pub public_creation_enabled: bool,
    #[serde(default)]
    pub proof_broker_enabled: bool,
    #[serde(default)]
    pub metric_programs: Vec<OpenCompetitionV2MetricProgramRelease>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionV2CreationRequest {
    pub release: OpenCompetitionV2Release,
    pub creator: String,
    pub creation_nonce: String,
    pub acknowledged_risk_hash: String,
    pub initial_funding: u128,
    pub params: OpenCompetitionV2CreateParams,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionV2CreationPlan {
    pub schema_version: String,
    pub protocol_version: String,
    pub network: BaseNetworkDescriptor,
    pub bounty_id: String,
    pub predicted_competition: String,
    pub funding_target: String,
    pub remaining_funding_after_creation: String,
    pub profitable_if_win: bool,
    pub wallet_calls: Vec<EvmTransactionIntent>,
    pub public_inventory_eligible_after_confirmation: bool,
    pub next_action: String,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionV2ActionPlan {
    pub schema_version: String,
    pub proto