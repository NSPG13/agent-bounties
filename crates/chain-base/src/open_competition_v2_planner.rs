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

pub const OPEN_COMPETITION_V2_GROTH16_ID: &str = "sp1-groth16";
pub const OPEN_COMPETITION_V2_PLONK_ID: &str = "sp1-plonk";
pub const OPEN_COMPETITION_V2_BASE_USDC: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
pub const OPEN_COMPETITION_V2_BASE_SEPOLIA_USDC: &str =
    "0x036CbD53842c5426634e7929541eC2318f3dCF7e";
pub const OPEN_COMPETITION_V2_GROTH16_GATEWAY: &str = "0x397A5f7f3dBd538f23DE225B51f532c34448dA9B";
pub const OPEN_COMPETITION_V2_PLONK_GATEWAY: &str = "0x3B6041173B80E77f038f3F2C0f9744f04837185e";

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

    interface IOpenCompetitionFactoryV2Beta1Planner {
        function createCompetition(
            CompetitionV2CreateParamsAbi params,
            uint256 initialFunding,
            bytes32 creationNonce,
            bytes32 acknowledgedRiskHash
        ) external returns (address competitionAddress, bytes32 bountyId);
    }

    interface IOpenCompetitionBountyV2Beta1Planner {
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
        function cancelForUnavailableGateway() external;
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
    pub factory_contract: String,
    pub implementation_contract: String,
    pub settlement_token: String,
    pub groth16_adapter: String,
    pub plonk_adapter: String,
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
    pub protocol_version: String,
    pub network: BaseNetworkDescriptor,
    pub competition_contract: String,
    pub action: String,
    pub wallet_call: EvmTransactionIntent,
    pub next_action: String,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionV2ProofAuthorizationMessage {
    pub solver: String,
    pub solver_nonce: String,
    pub public_values_hash: String,
    pub proof_hash: String,
    pub authorization_deadline: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionV2ProofAuthorizationTypedData {
    pub types: BTreeMap<String, Vec<Eip712TypeField>>,
    pub domain: Eip712DomainData,
    pub primary_type: String,
    pub message: OpenCompetitionV2ProofAuthorizationMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionV2ProofPlan {
    pub schema_version: String,
    pub protocol_version: String,
    pub network: BaseNetworkDescriptor,
    pub competition_contract: String,
    pub solver: String,
    pub proof_system: OpenCompetitionV2ProofSystem,
    pub public_values_hash: String,
    pub proof_hash: String,
    pub direct_call: EvmTransactionIntent,
    pub relay_authorization: OpenCompetitionV2ProofAuthorizationTypedData,
    pub relay_call_after_signature: Option<EvmTransactionIntent>,
    pub evidence_boundary: String,
}

pub fn plan_open_competition_v2_creation(
    request: OpenCompetitionV2CreationRequest,
) -> Result<OpenCompetitionV2CreationPlan, ChainBaseError> {
    let descriptor = base_network_descriptor(&request.release.network)?;
    validate_release(&request.release, &descriptor)?;
    validate_create_params(
        &request.params,
        &request.acknowledged_risk_hash,
        request.initial_funding,
    )?;
    let creator = parse_address(&request.creator)?;
    let factory = parse_address(&request.release.factory_contract)?;
    let implementation = parse_address(&request.release.implementation_contract)?;
    let creation_nonce = parse_b256(&request.creation_nonce, "creation_nonce")?;
    require_nonzero(creation_nonce, "creation_nonce")?;
    let params = params_abi(&request.params)?;
    let bounty_id = keccak256(
        (
            U256::from(descriptor.chain_id),
            factory,
            creator,
            creation_nonce,
            params.clone(),
        )
            .abi_encode(),
    );
    let predicted = predict_clone(factory, implementation, bounty_id);
    let target = request
        .params
        .solver_reward
        .checked_add(request.params.keeper_reward)
        .ok_or(ChainBaseError::InvalidAmount)?;
    let mut wallet_calls = Vec::new();
    if request.initial_funding > 0 {
        let approval = IERC20CompetitionV2::approveCall {
            spender: predicted,
            amount: U256::from(request.initial_funding),
        };
        wallet_calls.push(intent(
            Some(creator),
            parse_address(&request.release.settlement_token)?,
            approval.abi_encode(),
            "approve(address,uint256)",
        ));
    }
    let create = IOpenCompetitionFactoryV2Beta1Planner::createCompetitionCall {
        params,
        initialFunding: U256::from(request.initial_funding),
        creationNonce: creation_nonce,
        acknowledgedRiskHash: parse_b256(
            &request.acknowledged_risk_hash,
            "acknowledged_risk_hash",
        )?,
    };
    wallet_calls.push(intent(
        Some(creator),
        factory,
        create.abi_encode(),
        "createCompetition((uint256,uint256,uint64,uint64,uint8,uint8,int256,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32),uint256,bytes32,bytes32)",
    ));
    Ok(OpenCompetitionV2CreationPlan {
        schema_version: "agent-bounties/open-competition-v2-creation-plan-v1".to_string(),
        protocol_version: super::OPEN_COMPETITION_V2_PROTOCOL_VERSION.to_string(),
        network: descriptor,
        bounty_id: format!("{bounty_id:#x}"),
        predicted_competition: format!("{predicted:#x}"),
        funding_target: target.to_string(),
        remaining_funding_after_creation: (target - request.initial_funding).to_string(),
        profitable_if_win: request.params.solver_reward > 0,
        wallet_calls,
        public_inventory_eligible_after_confirmation: request.initial_funding == target,
        next_action: "Execute wallet_calls in order, then wait for safe-block CompetitionActivatedV2 before publishing the competition as active.".to_string(),
        evidence_boundary: "This is unsigned transaction data. It is not canonical creation, funding, activation, proof acceptance, or payment evidence.".to_string(),
    })
}

pub fn validate_open_competition_v2_release(
    release: &OpenCompetitionV2Release,
) -> Result<(), ChainBaseError> {
    let descriptor = base_network_descriptor(&release.network)?;
    validate_release(release, &descriptor)
}

pub fn plan_open_competition_v2_funding(
    network: &str,
    settlement_token: &str,
    contributor: &str,
    competition: &str,
    amount: u128,
    acknowledged_risk_hash: &str,
) -> Result<Vec<EvmTransactionIntent>, ChainBaseError> {
    base_network_descriptor(network)?;
    if amount == 0 {
        return Err(v2_error("funding amount must be positive"));
    }
    let contributor = parse_address(contributor)?;
    let competition = parse_address(competition)?;
    let approval = IERC20CompetitionV2::approveCall {
        spender: competition,
        amount: U256::from(amount),
    };
    let funding = IOpenCompetitionBountyV2Beta1Planner::fundCall {
        requestedAmount: U256::from(amount),
        acknowledgedRiskHash: parse_b256(acknowledged_risk_hash, "acknowledged_risk_hash")?,
    };
    Ok(vec![
        intent(
            Some(contributor),
            parse_address(settlement_token)?,
            approval.abi_encode(),
            "approve(address,uint256)",
        ),
        intent(
            Some(contributor),
            competition,
            funding.abi_encode(),
            "fund(uint256,bytes32)",
        ),
    ])
}

pub fn plan_open_competition_v2_broker_payment(
    network: &str,
    settlement_token: &str,
    relayer: &str,
    authorization: &OpenCompetitionV2BrokerPaymentAuthorization,
) -> Result<EvmTransactionIntent, ChainBaseError> {
    let descriptor = base_network_descriptor(network)?;
    let expected_token = match descriptor.chain_id {
        8453 => OPEN_COMPETITION_V2_BASE_USDC,
        84532 => OPEN_COMPETITION_V2_BASE_SEPOLIA_USDC,
        _ => return Err(v2_error("V2 Beta1 supports Base and Base Sepolia only")),
    };
    if normalize_evm_address(settlement_token)? != normalize_evm_address(expected_token)? {
        return Err(v2_error("proof broker must settle in native Base USDC"));
    }
    if authorization.amount == 0 || authorization.valid_before == 0 {
        return Err(v2_error(
            "proof broker authorization amount and deadline are required",
        ));
    }
    if !matches!(authorization.v, 27 | 28) {
        return Err(v2_error("proof broker EIP-3009 v must be 27 or 28"));
    }
    let call = IERC20CompetitionV2::transferWithAuthorizationCall {
        from: parse_address(&authorization.payer)?,
        to: parse_address(&authorization.recipient)?,
        value: U256::from(authorization.amount),
        validAfter: U256::ZERO,
        validBefore: U256::from(authorization.valid_before),
        nonce: parse_b256(&authorization.nonce, "authorization_nonce")?,
        v: authorization.v,
        r: parse_b256(&authorization.r, "signature_r")?,
        s: parse_b256(&authorization.s, "signature_s")?,
    };
    Ok(intent(
        Some(parse_address(relayer)?),
        parse_address(settlement_token)?,
        call.abi_encode(),
        "transferWithAuthorization(address,address,uint256,uint256,uint256,bytes32,uint8,bytes32,bytes32)",
    ))
}

pub fn plan_open_competition_v2_broker_refund(
    network: &str,
    settlement_token: &str,
    broker: &str,
    recipient: &str,
    amount: u64,
) -> Result<EvmTransactionIntent, ChainBaseError> {
    let descriptor = base_network_descriptor(network)?;
    let expected_token = match descriptor.chain_id {
        8453 => OPEN_COMPETITION_V2_BASE_USDC,
        84532 => OPEN_COMPETITION_V2_BASE_SEPOLIA_USDC,
        _ => return Err(v2_error("V2 Beta1 supports Base and Base Sepolia only")),
    };
    if amount == 0
        || normalize_evm_address(settlement_token)? != normalize_evm_address(expected_token)?
    {
        return Err(v2_error("refund must be positive native Base USDC"));
    }
    let call = IERC20CompetitionV2::transferCall {
        recipient: parse_address(recipient)?,
        amount: U256::from(amount),
    };
    Ok(intent(
        Some(parse_address(broker)?),
        parse_address(settlement_token)?,
        call.abi_encode(),
        "transfer(address,uint256)",
    ))
}

pub fn open_competition_v2_broker_refund_digest(
    network: &str,
    settlement_token: &str,
    broker: &str,
    recipient: &str,
    amount: u64,
    valid_before: u64,
    nonce: &str,
) -> Result<String, ChainBaseError> {
    let descriptor = base_network_descriptor(network)?;
    let expected_token = match descriptor.chain_id {
        8453 => OPEN_COMPETITION_V2_BASE_USDC,
        84532 => OPEN_COMPETITION_V2_BASE_SEPOLIA_USDC,
        _ => return Err(v2_error("V2 Beta1 supports Base and Base Sepolia only")),
    };
    if amount == 0
        || valid_before == 0
        || normalize_evm_address(settlement_token)? != normalize_evm_address(expected_token)?
    {
        return Err(v2_error("refund authorization is invalid"));
    }
    let token = parse_address(settlement_token)?;
    let broker = parse_address(broker)?;
    let recipient = parse_address(recipient)?;
    let nonce = parse_b256(nonce, "refund_nonce")?;
    let domain_type_hash = keccak256(
        "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );
    let transfer_type_hash = keccak256(
        "TransferWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)",
    );
    let token_name = if descriptor.chain_id == 84_532 {
        "USDC"
    } else {
        "USD Coin"
    };
    let domain_separator = keccak256(
        (
            domain_type_hash,
            keccak256(token_name),
            keccak256("2"),
            U256::from(descriptor.chain_id),
            token,
        )
            .abi_encode(),
    );
    let authorization_hash = keccak256(
        (
            transfer_type_hash,
            broker,
            recipient,
            U256::from(amount),
            U256::ZERO,
            U256::from(valid_before),
            nonce,
        )
            .abi_encode(),
    );
    Ok(format!(
        "{:#x}",
        keccak256(
            [
                &[0x19_u8, 0x01_u8][..],
                domain_separator.as_slice(),
                authorization_hash.as_slice(),
            ]
            .concat()
        )
    ))
}

#[allow(clippy::too_many_arguments)]
pub fn plan_open_competition_v2_proof(
    network: &str,
    competition: &str,
    solver: &str,
    solver_nonce: u128,
    proof_system: OpenCompetitionV2ProofSystem,
    public_values: &[u8],
    proof: &[u8],
    authorization_deadline: u64,
    solver_signature: Option<&[u8]>,
) -> Result<OpenCompetitionV2ProofPlan, ChainBaseError> {
    if public_values.is_empty() || proof.is_empty() || authorization_deadline == 0 {
        return Err(v2_error(
            "proof inputs and authorization deadline are required",
        ));
    }
    let descriptor = base_network_descriptor(network)?;
    let competition = parse_address(competition)?;
    let solver = parse_address(solver)?;
    let public_values_hash = keccak256(public_values);
    let proof_hash = keccak256(proof);
    let direct = IOpenCompetitionBountyV2Beta1Planner::submitProofCall {
        publicValues: public_values.to_vec().into(),
        proofBytes: proof.to_vec().into(),
    };
    let mut types = BTreeMap::new();
    types.insert(
        "EIP712Domain".to_string(),
        vec![
            Eip712TypeField {
                name: "name".to_string(),
                field_type: "string".to_string(),
            },
            Eip712TypeField {
                name: "version".to_string(),
                field_type: "string".to_string(),
            },
            Eip712TypeField {
                name: "chainId".to_string(),
                field_type: "uint256".to_string(),
            },
            Eip712TypeField {
                name: "verifyingContract".to_string(),
                field_type: "address".to_string(),
            },
        ],
    );
    types.insert(
        "SubmitProof".to_string(),
        vec![
            Eip712TypeField {
                name: "solver".to_string(),
                field_type: "address".to_string(),
            },
            Eip712TypeField {
                name: "solverNonce".to_string(),
                field_type: "uint256".to_string(),
            },
            Eip712TypeField {
                name: "publicValuesHash".to_string(),
                field_type: "bytes32".to_string(),
            },
            Eip712TypeField {
                name: "proofHash".to_string(),
                field_type: "bytes32".to_string(),
            },
            Eip712TypeField {
                name: "authorizationDeadline".to_string(),
                field_type: "uint256".to_string(),
            },
        ],
    );
    let relay_call = solver_signature.map(|signature| {
        let call = IOpenCompetitionBountyV2Beta1Planner::submitProofForCall {
            publicValues: public_values.to_vec().into(),
            proofBytes: proof.to_vec().into(),
            authorizationDeadline: U256::from(authorization_deadline),
            solverSignature: signature.to_vec().into(),
        };
        intent(
            None,
            competition,
            call.abi_encode(),
            "submitProofFor(bytes,bytes,uint256,bytes)",
        )
    });
    Ok(OpenCompetitionV2ProofPlan {
        schema_version: "agent-bounties/open-competition-v2-proof-plan-v1".to_string(),
        protocol_version: super::OPEN_COMPETITION_V2_PROTOCOL_VERSION.to_string(),
        network: descriptor.clone(),
        competition_contract: format!("{competition:#x}"),
        solver: format!("{solver:#x}"),
        proof_system,
        public_values_hash: format!("{public_values_hash:#x}"),
        proof_hash: format!("{proof_hash:#x}"),
        direct_call: intent(
            Some(solver),
            competition,
            direct.abi_encode(),
            "submitProof(bytes,bytes)",
        ),
        relay_authorization: OpenCompetitionV2ProofAuthorizationTypedData {
            types,
            domain: Eip712DomainData {
                name: "Agent Bounties Open Competition V2 Beta1".to_string(),
                version: "1".to_string(),
                chain_id: descriptor.chain_id,
                verifying_contract: format!("{competition:#x}"),
            },
            primary_type: "SubmitProof".to_string(),
            message: OpenCompetitionV2ProofAuthorizationMessage {
                solver: format!("{solver:#x}"),
                solver_nonce: solver_nonce.to_string(),
                public_values_hash: format!("{public_values_hash:#x}"),
                proof_hash: format!("{proof_hash:#x}"),
                authorization_deadline: authorization_deadline.to_string(),
            },
        },
        relay_call_after_signature: relay_call,
        evidence_boundary: "A proof plan, signature, or relay transaction hash is not solver payment. Only a safe-block CompetitionSettledV2 event is payment evidence.".to_string(),
    })
}

pub fn plan_open_competition_v2_action(
    network: &str,
    competition: &str,
    caller: Option<&str>,
    action: &str,
    contributor: Option<&str>,
) -> Result<OpenCompetitionV2ActionPlan, ChainBaseError> {
    let descriptor = base_network_descriptor(network)?;
    let competition = parse_address(competition)?;
    let from = caller.map(parse_address).transpose()?;
    let (data, function, next_action) = match action {
        "finalize_best_score" => (
            IOpenCompetitionBountyV2Beta1Planner::finalizeBestScoreCall {}.abi_encode(),
            "finalizeBestScore()",
            "Wait for safe-block CompetitionSettledV2 and reconcile both solver and keeper transfers.",
        ),
        "cancel_funding" => (
            IOpenCompetitionBountyV2Beta1Planner::cancelFundingCall {}.abi_encode(),
            "cancelFunding()",
            "Wait for safe-block CompetitionCancelledV2, then call withdraw_refund_for for each contributor.",
        ),
        "expire_competition" => (
            IOpenCompetitionBountyV2Beta1Planner::expireCompetitionCall {}.abi_encode(),
            "expireCompetition()",
            "Wait for safe-block CompetitionCancelledV2, then return each contributor refund permissionlessly.",
        ),
        "cancel_unavailable_gateway" => (
            IOpenCompetitionBountyV2Beta1Planner::cancelForUnavailableGatewayCall {}.abi_encode(),
            "cancelForUnavailableGateway()",
            "Wait for safe-block CompetitionCancelledV2, then return each contributor refund permissionlessly.",
        ),
        "withdraw_refund_for" => {
            let contributor = contributor
                .ok_or_else(|| v2_error("contributor is required for withdraw_refund_for"))?;
            let call = IOpenCompetitionBountyV2Beta1Planner::withdrawRefundForCall {
                contributor: parse_address(contributor)?,
            };
            (
                call.abi_encode(),
                "withdrawRefundFor(address)",
                "Wait for safe-block CompetitionRefundWithdrawnV2 and confirm the contributor USDC transfer.",
            )
        }
        _ => return Err(v2_error("unsupported competition action")),
    };
    Ok(OpenCompetitionV2ActionPlan {
        schema_version: "agent-bounties/open-competition-v2-action-plan-v1".to_string(),
        protocol_version: super::OPEN_COMPETITION_V2_PROTOCOL_VERSION.to_string(),
        network: descriptor,
        competition_contract: format!("{competition:#x}"),
        action: action.to_string(),
        wallet_call: intent(from, competition, data, function),
        next_action: next_action.to_string(),
        evidence_boundary: "This unsigned call is not a state transition or payment. Reconcile the named safe-block canonical event before changing hosted state.".to_string(),
    })
}

fn validate_release(
    release: &OpenCompetitionV2Release,
    descriptor: &BaseNetworkDescriptor,
) -> Result<(), ChainBaseError> {
    if release.protocol_version != super::OPEN_COMPETITION_V2_PROTOCOL_VERSION {
        return Err(v2_error("release protocol version mismatch"));
    }
    parse_address(&release.factory_contract)?;
    parse_address(&release.implementation_contract)?;
    parse_address(&release.groth16_adapter)?;
    parse_address(&release.plonk_adapter)?;
    require_nonzero(
        parse_b256(&release.release_hash, "release_hash")?,
        "release_hash",
    )?;
    require_nonzero(
        parse_b256(&release.beta_risk_hash, "beta_risk_hash")?,
        "beta_risk_hash",
    )?;
    let expected_token = match descriptor.chain_id {
        8453 => OPEN_COMPETITION_V2_BASE_USDC,
        84532 => OPEN_COMPETITION_V2_BASE_SEPOLIA_USDC,
        _ => return Err(v2_error("V2 Beta1 supports Base and Base Sepolia only")),
    };
    if normalize_evm_address(&release.settlement_token)? != normalize_evm_address(expected_token)? {
        return Err(v2_error("release settlement token is not native Base USDC"));
    }
    let mut profile_ids = BTreeSet::new();
    let mut reviewed_profiles = 0_usize;
    for profile in &release.metric_programs {
        if profile.profile_id.trim().is_empty() || !profile_ids.insert(&profile.profile_id) {
            return Err(v2_error("release metric profile id is empty or duplicated"));
        }
        if profile.classification == OpenCompetitionV2ProgramClassification::Disabled {
            continue;
        }
        for (value, field) in [
            (&profile.program_vkey, "program_vkey"),
            (&profile.source_hash, "source_hash"),
            (&profile.elf_hash, "elf_hash"),
            (&profile.journal_schema_hash, "journal_schema_hash"),
            (&profile.metric_program_hash, "metric_program_hash"),
        ] {
            require_nonzero(parse_b256(value, field)?, field)?;
        }
        if profile.classification == OpenCompetitionV2ProgramClassification::Reviewed {
            require_nonzero(
                parse_b256(&profile.review_evidence_hash, "review_evidence_hash")?,
                "review_evidence_hash",
            )?;
            reviewed_profiles += 1;
        }
    }
    if release.proof_broker_enabled && reviewed_profiles == 0 {
        return Err(v2_error(
            "proof broker cannot be enabled without a reviewed metric profile",
        ));
    }
    Ok(())
}

fn validate_create_params(
    params: &OpenCompetitionV2CreateParams,
    acknowledged_risk_hash: &str,
    initial_funding: u128,
) -> Result<(), ChainBaseError> {
    if params.solver_reward == 0 || params.keeper_reward == 0 {
        return Err(v2_error("solver and keeper rewards must be positive"));
    }
    if params.keeper_reward > params.solver_reward / 20 {
        return Err(v2_error("keeper reward exceeds 5% of solver reward"));
    }
    let target = params
        .solver_reward
        .checked_add(params.keeper_reward)
        .ok_or(ChainBaseError::InvalidAmount)?;
    if initial_funding > target {
        return Err(v2_error("initial funding exceeds the competition target"));
    }
    if params.funding_deadline == 0 || params.proof_window_seconds == 0 {
        return Err(v2_error(
            "funding deadline and proof window must be positive",
        ));
    }
    if normalize_hash(acknowledged_risk_hash, "acknowledged_risk_hash")?
        != normalize_hash(&params.beta_risk_hash, "beta_risk_hash")?
    {
        return Err(v2_error("Beta1 risk hash was not acknowledged exactly"));
    }
    params_abi(params)?;
    Ok(())
}

fn params_abi(
    params: &OpenCompetitionV2CreateParams,
) -> Result<CompetitionV2CreateParamsAbi, ChainBaseError> {
    Ok(CompetitionV2CreateParamsAbi {
        solverReward: U256::from(params.solver_reward),
        keeperReward: U256::from(params.keeper_reward),
        fundingDeadline: params.funding_deadline,
        proofWindowSeconds: params.proof_window_seconds,
        winnerMode: params.winner_mode.abi_value(),
        scoreDirection: params.score_direction.abi_value(),
        scoreThreshold: I256::from_str(&params.score_threshold)
            .map_err(|_| v2_error("score_threshold must be an int256 decimal string"))?,
        proofSystem: params.proof_system.hash(),
        programVKey: required_hash(&params.program_vkey, "program_vkey")?,
        sourceHash: required_hash(&params.source_hash, "source_hash")?,
        elfHash: required_hash(&params.elf_hash, "elf_hash")?,
        journalSchemaHash: required_hash(&params.journal_schema_hash, "journal_schema_hash")?,
        metricProgramHash: required_hash(&params.metric_program_hash, "metric_program_hash")?,
        executionPolicyHash: required_hash(&params.execution_policy_hash, "execution_policy_hash")?,
        verificationPolicyHash: required_hash(
            &params.verification_policy_hash,
            "verification_policy_hash",
        )?,
        settlementPolicyHash: required_hash(
            &params.settlement_policy_hash,
            "settlement_policy_hash",
        )?,
        betaRiskHash: required_hash(&params.beta_risk_hash, "beta_risk_hash")?,
    })
}

fn intent(
    from: Option<Address>,
    to: Address,
    data: Vec<u8>,
    function: &str,
) -> EvmTransactionIntent {
    EvmTransactionIntent {
        from: from.map(|address| format!("{address:#x}")),
        to: format!("{to:#x}"),
        value_wei: 0,
        data: format!("0x{}", hex::encode(data)),
        function: function.to_string(),
    }
}

fn predict_clone(factory: Address, implementation: Address, salt: B256) -> Address {
    let mut creation_code = hex::decode("3d602d80600a3d3981f3").expect("valid clone prefix");
    creation_code
        .extend_from_slice(&hex::decode("363d3d373d3d3d363d73").expect("valid clone body prefix"));
    creation_code.extend_from_slice(implementation.as_slice());
    creation_code.extend_from_slice(
        &hex::decode("5af43d82803e903d91602b57fd5bf3").expect("valid clone suffix"),
    );
    let init_code_hash = keccak256(creation_code);
    let mut create2 = Vec::with_capacity(85);
    create2.push(0xff);
    create2.extend_from_slice(factory.as_slice());
    create2.extend_from_slice(salt.as_slice());
    create2.extend_from_slice(init_code_hash.as_slice());
    let hash = keccak256(create2);
    Address::from_slice(&hash.as_slice()[12..])
}

fn parse_address(value: &str) -> Result<Address, ChainBaseError> {
    normalize_evm_address(value)?
        .parse::<Address>()
        .map_err(|_| ChainBaseError::InvalidAddress(value.to_string()))
}

fn parse_b256(value: &str, field: &str) -> Result<B256, ChainBaseError> {
    B256::from_str(value).map_err(|_| v2_error(&format!("{field} must be bytes32 hex")))
}

fn required_hash(value: &str, field: &str) -> Result<B256, ChainBaseError> {
    let hash = parse_b256(value, field)?;
    require_nonzero(hash, field)?;
    Ok(hash)
}

fn require_nonzero(value: B256, field: &str) -> Result<(), ChainBaseError> {
    if value == B256::ZERO {
        return Err(v2_error(&format!("{field} must be nonzero")));
    }
    Ok(())
}

fn normalize_hash(value: &str, field: &str) -> Result<String, ChainBaseError> {
    Ok(format!("{:#x}", parse_b256(value, field)?))
}

fn v2_error(message: &str) -> ChainBaseError {
    ChainBaseError::InvalidOpenCompetitionV2(message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(byte: u8) -> String {
        format!("0x{}", hex::encode([byte; 32]))
    }

    fn release() -> OpenCompetitionV2Release {
        OpenCompetitionV2Release {
            protocol_version: super::super::OPEN_COMPETITION_V2_PROTOCOL_VERSION.to_string(),
            network: "base-sepolia".to_string(),
            factory_contract: "0x1111111111111111111111111111111111111111".to_string(),
            implementation_contract: "0x2222222222222222222222222222222222222222".to_string(),
            settlement_token: OPEN_COMPETITION_V2_BASE_SEPOLIA_USDC.to_string(),
            groth16_adapter: "0x3333333333333333333333333333333333333333".to_string(),
            plonk_adapter: "0x4444444444444444444444444444444444444444".to_string(),
            deployment_block: 1,
            release_hash: hash(1),
            beta_risk_hash: hash(2),
            public_creation_enabled: false,
            proof_broker_enabled: false,
            metric_programs: Vec::new(),
        }
    }

    fn params() -> OpenCompetitionV2CreateParams {
        OpenCompetitionV2CreateParams {
            solver_reward: 1_000_000,
            keeper_reward: 50_000,
            funding_deadline: 2_000_000_000,
            proof_window_seconds: 3600,
            winner_mode: OpenCompetitionV2WinnerMode::FirstProven,
            score_direction: OpenCompetitionV2ScoreDirection::HigherIsBetter,
            score_threshold: "0".to_string(),
            proof_system: OpenCompetitionV2ProofSystem::Groth16,
            program_vkey: hash(3),
            source_hash: hash(4),
            elf_hash: hash(5),
            journal_schema_hash: hash(6),
            metric_program_hash: hash(7),
            execution_policy_hash: hash(8),
            verification_policy_hash: hash(9),
            settlement_policy_hash: hash(10),
            beta_risk_hash: hash(2),
        }
    }

    #[test]
    fn creation_plan_is_exact_and_requires_risk_acknowledgement() {
        let request = OpenCompetitionV2CreationRequest {
            release: release(),
            creator: "0x5555555555555555555555555555555555555555".to_string(),
            creation_nonce: hash(11),
            acknowledged_risk_hash: hash(2),
            initial_funding: 1_050_000,
            params: params(),
        };
        let plan = plan_open_competition_v2_creation(request.clone()).unwrap();
        assert_eq!(plan.wallet_calls.len(), 2);
        assert!(plan.public_inventory_eligible_after_confirmation);
        assert_eq!(plan.funding_target, "1050000");
        assert_eq!(plan.remaining_funding_after_creation, "0");

        let mut rejected = request;
        rejected.acknowledged_risk_hash = hash(12);
        assert!(plan_open_competition_v2_creation(rejected).is_err());
    }

    #[test]
    fn planner_has_unlimited_entry_proof_paths_without_bonds() {
        let plan = plan_open_competition_v2_proof(
            "base-sepolia",
            "0x1111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
            7,
            OpenCompetitionV2ProofSystem::Plonk,
            &[1; 640],
            &[2; 96],
            2_000_000_000,
            None,
        )
        .unwrap();
        assert_eq!(plan.direct_call.function, "submitProof(bytes,bytes)");
        assert!(plan.relay_call_after_signature.is_none());
        assert_eq!(plan.relay_authorization.message.solver_nonce, "7");
    }

    #[test]
    fn refund_plan_is_permissionless_and_beneficiary_bound() {
        let plan = plan_open_competition_v2_action(
            "base-mainnet",
            "0x1111111111111111111111111111111111111111",
            None,
            "withdraw_refund_for",
            Some("0x2222222222222222222222222222222222222222"),
        )
        .unwrap();
        assert_eq!(plan.wallet_call.function, "withdrawRefundFor(address)");
        assert!(plan.wallet_call.from.is_none());
    }

    #[test]
    fn proof_broker_payment_and_refund_are_exact_usdc_calls() {
        let payment = plan_open_competition_v2_broker_payment(
            "base-mainnet",
            OPEN_COMPETITION_V2_BASE_USDC,
            "0x3333333333333333333333333333333333333333",
            &OpenCompetitionV2BrokerPaymentAuthorization {
                payer: "0x1111111111111111111111111111111111111111".to_string(),
                recipient: "0x2222222222222222222222222222222222222222".to_string(),
                amount: 150_000,
                valid_before: 2_000_000_000,
                nonce: hash(12),
                v: 27,
                r: hash(13),
                s: hash(14),
            },
        )
        .unwrap();
        assert_eq!(
            payment.to,
            OPEN_COMPETITION_V2_BASE_USDC.to_ascii_lowercase()
        );
        assert!(payment.function.starts_with("transferWithAuthorization"));
        assert_eq!(payment.value_wei, 0);

        let refund = plan_open_competition_v2_broker_refund(
            "base-mainnet",
            OPEN_COMPETITION_V2_BASE_USDC,
            "0x2222222222222222222222222222222222222222",
            "0x1111111111111111111111111111111111111111",
            150_000,
        )
        .unwrap();
        assert_eq!(refund.function, "transfer(address,uint256)");
        assert_eq!(refund.value_wei, 0);
    }
}
