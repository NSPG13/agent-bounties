//! `agent-bounties/autonomous-v2`: an explicit platform fee on the solver reward and an optional
//! claim-eligibility gate (ADR 0006). The bounty contract surface is otherwise identical to v1,
//! so contribution, claim, settlement and refund planners are shared; only factory creation, the
//! EIP-712 domain version, the platform-fee forward call, and feed economics differ.

use super::*;

pub const AUTONOMOUS_V1_PROTOCOL_VERSION: &str = "agent-bounties/autonomous-v1";
pub const AUTONOMOUS_V2_PROTOCOL_VERSION: &str = "agent-bounties/autonomous-v2";
pub const AUTONOMOUS_V2_MAX_PLATFORM_FEE_BPS: u16 = 1_000;
pub const AUTONOMOUS_V2_EIP712_DOMAIN_VERSION: &str = "2";
const BPS_DENOMINATOR: u128 = 10_000;
const ZERO_ADDRESS: &str = "0x0000000000000000000000000000000000000000";
const V2_CREATE_PARAMS: &str = "(uint256,uint256,bytes32,bytes32,bytes32,bytes32,bytes32,uint64,uint64,uint64,uint8,address,address,uint8,address,bytes32)";
const V2_PARAM_WORDS: usize = 16;

/// Contract-terms keys that only autonomous-v2 bounties may publish.
const V2_ONLY_TERMS_KEYS: [&str; 5] = [
    "platform_fee_bps",
    "platform_fee",
    "platform_fee_recipient",
    "claim_eligibility_registry",
    "claim_eligibility_source",
];

pub fn autonomous_v2_create_bounty_function() -> String {
    format!("createBounty({V2_CREATE_PARAMS},address[],uint256,bytes32)")
}

pub fn autonomous_v2_create_bounty_with_authorization_function() -> String {
    format!("createBountyWithAuthorization(address,{V2_CREATE_PARAMS},address[],uint256,bytes32,(uint256,uint256,bytes32,uint8,bytes32,bytes32))")
}

/// `ceil(solverReward * platformFeeBps / 10_000)`, matching `agentBountyV2PlatformFee`.
pub fn autonomous_v2_platform_fee(
    solver_reward: u128,
    platform_fee_bps: u16,
) -> Result<u128, ChainBaseError> {
    if platform_fee_bps > AUTONOMOUS_V2_MAX_PLATFORM_FEE_BPS {
        return Err(ChainBaseError::InvalidVerificationConfiguration(format!(
            "platform fee {platform_fee_bps} bps exceeds the {AUTONOMOUS_V2_MAX_PLATFORM_FEE_BPS} bps cap"
        )));
    }
    solver_reward
        .checked_mul(u128::from(platform_fee_bps))
        .and_then(|value| value.checked_add(BPS_DENOMINATOR - 1))
        .map(|value| value / BPS_DENOMINATOR)
        .ok_or(ChainBaseError::InvalidAmount)
}

/// The immutable fee terms of one deployed `AgentBountyFactoryV2`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousV2FactoryFee {
    pub platform_fee_bps: u16,
    pub platform_fee_recipient: String,
}

impl AutonomousV2FactoryFee {
    fn validated(&self) -> Result<(u16, String), ChainBaseError> {
        let recipient = normalize_address(&self.platform_fee_recipient)?;
        autonomous_v2_platform_fee(0, self.platform_fee_bps)?;
        if (self.platform_fee_bps == 0) != (recipient == ZERO_ADDRESS) {
            return Err(ChainBaseError::InvalidVerificationConfiguration(
                "a platform fee needs a nonzero recipient, and a zero fee needs the zero address"
                    .to_string(),
            ));
        }
        Ok((self.platform_fee_bps, recipient))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousBountyV2Quote {
    pub protocol_version: String,
    pub solver_reward: String,
    pub verifier_reward: String,
    pub claim_bond: String,
    pub platform_fee_bps: u16,
    pub platform_fee: String,
    pub platform_fee_recipient: String,
    pub target_amount: String,
    pub fee_boundary: String,
}

pub fn quote_autonomous_v2_bounty(
    solver_reward: u128,
    verifier_reward: u128,
    fee: &AutonomousV2FactoryFee,
) -> Result<AutonomousBountyV2Quote, ChainBaseError> {
    let (bps, recipient) = fee.validated()?;
    let platform_fee = autonomous_v2_platform_fee(solver_reward, bps)?;
    let target = solver_reward
        .checked_add(verifier_reward)
        .and_then(|value| value.checked_add(platform_fee))
        .filter(|value| *value <= u128::from(u64::MAX))
        .ok_or(ChainBaseError::InvalidAmount)?;
    Ok(AutonomousBountyV2Quote {
        protocol_version: AUTONOMOUS_V2_PROTOCOL_VERSION.to_string(),
        solver_reward: solver_reward.to_string(),
        verifier_reward: verifier_reward.to_string(),
        claim_bond: verifier_reward.to_string(),
        platform_fee_bps: bps,
        platform_fee: platform_fee.to_string(),
        platform_fee_recipient: recipient,
        target_amount: target.to_string(),
        fee_boundary: "The poster funds the full target. The fee is paid to the fixed recipient only when the bounty settles; it stays escrowed through rejections and timeouts and is refunded with principal on cancellation.".to_string(),
    })
}

/// v2 creation request: the v1 fields plus the optional claim-eligibility gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousBountyV2Create {
    #[serde(flatten)]
    pub base: AutonomousBountyCreate,
    #[serde(default)]
    pub claim_eligibility_registry: Option<String>,
    #[serde(default)]
    pub claim_eligibility_source: Option<String>,
}

impl AutonomousBountyV2Create {
    fn gate_words(&self) -> Result<([u8; 32], [u8; 32]), ChainBaseError> {
        match (
            self.claim_eligibility_registry.as_deref(),
            self.claim_eligibility_source.as_deref(),
        ) {
            (None, None) => Ok(([0u8; 32], [0u8; 32])),
            (Some(registry), Some(source)) => {
                let registry_word = encode_address(registry)?;
                let source_word = parse_bytes32(source)?;
                if registry_word == [0u8; 32] || source_word == [0u8; 32] {
                    return Err(ChainBaseError::InvalidVerificationConfiguration(
                        "a claim-eligibility gate needs a nonzero registry and source".to_string(),
                    ));
                }
                Ok((registry_word, source_word))
            }
            _ => Err(ChainBaseError::InvalidVerificationConfiguration(
                "claim_eligibility_registry and claim_eligibility_source must be set together"
                    .to_string(),
            )),
        }
    }

    fn param_words(&self) -> Result<Vec<[u8; 32]>, ChainBaseError> {
        let mut words = autonomous_create_param_words(&self.base)?;
        let (registry, source) = self.gate_words()?;
        words.push(registry);
        words.push(source);
        Ok(words)
    }
}

/// Planned v2 creation, with the fee quote the poster must fund.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousBountyV2CreationPlan {
    #[serde(flatten)]
    pub plan: AutonomousBountyCreationPlan,
    pub quote: AutonomousBountyV2Quote,
}

impl AutonomousBountyTxPlanner {
    /// Plans `AgentBountyFactoryV2.createBounty` for a factory with the given immutable fee terms.
    pub fn plan_v2_creation(
        &self,
        network: &str,
        create: &AutonomousBountyV2Create,
        factory_fee: &AutonomousV2FactoryFee,
    ) -> Result<AutonomousBountyV2CreationPlan, ChainBaseError> {
        let network = base_network_descriptor(network)?;
        let base = &create.base;
        let creator = normalize_address(&base.creator)?;
        let solver_reward = autonomous_money_to_uint256(&base.solver_reward, false)?;
        let verifier_reward = autonomous_money_to_uint256(&base.verifier_reward, true)?;
        let quote = quote_autonomous_v2_bounty(solver_reward, verifier_reward, factory_fee)?;
        let platform_fee: u128 = quote
            .platform_fee
            .parse()
            .map_err(|_| ChainBaseError::InvalidAmount)?;
        let params = create.param_words()?;
        let verifiers = normalized_verifiers(base)?;
        validate_autonomous_creation(base, &verifiers, platform_fee)?;
        let creation_nonce = parse_bytes32(&base.creation_nonce)?;
        let bounty_id = autonomous_v2_bounty_id(
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
        let initial_funding = autonomous_money_to_uint256(&base.initial_funding, true)?;
        let create_bounty = EvmTransactionIntent {
            from: Some(creator.clone()),
            to: self.factory_contract.clone(),
            value_wei: 0,
            data: encode_v2_create_call(&params, &verifiers, initial_funding, creation_nonce)?,
            function: autonomous_v2_create_bounty_function(),
        };
        let approve = (initial_funding > 0)
            .then(|| -> Result<EvmTransactionIntent, ChainBaseError> {
                Ok(EvmTransactionIntent {
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
            })
            .transpose()?;
        let mut wallet_calls = Vec::with_capacity(2);
        wallet_calls.extend(approve.clone());
        wallet_calls.push(create_bounty.clone());
        let eip3009_authorization = (initial_funding > 0).then(|| {
            eip3009_typed_data(
                &network,
                &creator,
                &predicted_bounty_contract,
                initial_funding,
                0,
                base.funding_deadline,
                &base.creation_nonce,
            )
        });
        Ok(AutonomousBountyV2CreationPlan {
            plan: AutonomousBountyCreationPlan {
                protocol_version: AUTONOMOUS_V2_PROTOCOL_VERSION.to_string(),
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
                evidence_boundary: "A transaction plan or signature is not funding. Funding is applied only after a confirmed canonical v2 factory event and matching FundingAdded event from the predicted bounty contract. The target includes the platform fee, which is paid only at settlement.".to_string(),
            },
            quote,
        })
    }

    /// Plans the relayed `createBountyWithAuthorization` call for a signed EIP-3009 funding.
    pub fn plan_v2_authorized_creation(
        &self,
        network: &str,
        create: &AutonomousBountyV2Create,
        factory_fee: &AutonomousV2FactoryFee,
        signature: &AutonomousBountyAuthorizationSignature,
        relayer: Option<&str>,
    ) -> Result<AutonomousBountyAuthorizedCreationPlan, ChainBaseError> {
        let creation = self.plan_v2_creation(network, create, factory_fee)?;
        let initial_funding = autonomous_money_to_uint256(&create.base.initial_funding, false)?;
        let params = create.param_words()?;
        let verifiers = normalized_verifiers(&create.base)?;
        let creation_nonce = parse_bytes32(&create.base.creation_nonce)?;
        let v = normalized_signature_v(signature.v)?;
        let relay_transaction = EvmTransactionIntent {
            from: relayer.map(normalize_address).transpose()?,
            to: self.factory_contract.clone(),
            value_wei: 0,
            data: encode_v2_authorized_create_call(
                &create.base.creator,
                &params,
                &verifiers,
                initial_funding,
                creation_nonce,
                create.base.funding_deadline,
                v,
                parse_bytes32(&signature.r)?,
                parse_bytes32(&signature.s)?,
            )?,
            function: autonomous_v2_create_bounty_with_authorization_function(),
        };
        Ok(AutonomousBountyAuthorizedCreationPlan {
            protocol_version: creation.plan.protocol_version,
            network: creation.plan.network,
            bounty_id: creation.plan.bounty_id,
            predicted_bounty_contract: creation.plan.predicted_bounty_contract,
            relay_transaction,
            evidence_boundary: "A valid authorization and relayed transaction hash are not funding evidence. Recognize funding only after the canonical v2 factory creation event and matching FundingAdded log are confirmed.".to_string(),
        })
    }

    /// v2 bounties sign submissions under EIP-712 domain version "2".
    pub fn plan_v2_submission_authorization(
        &self,
        network: &str,
        request: &AutonomousBountySubmissionAuthorizationRequest,
    ) -> Result<AutonomousBountySubmissionAuthorizationTypedData, ChainBaseError> {
        let mut typed = self.plan_submission_authorization(network, request)?;
        typed.domain.version = AUTONOMOUS_V2_EIP712_DOMAIN_VERSION.to_string();
        Ok(typed)
    }

    /// v2 bounties verify quorum attestations under EIP-712 domain version "2".
    pub fn plan_v2_verification_attestation(
        &self,
        network: &str,
        request: &AutonomousVerificationAttestationRequest,
    ) -> Result<AutonomousVerificationAttestationTypedData, ChainBaseError> {
        let mut typed = self.plan_verification_attestation(network, request)?;
        typed.domain.version = AUTONOMOUS_V2_EIP712_DOMAIN_VERSION.to_string();
        Ok(typed)
    }

    /// Forwards a fee whose settlement-time transfer failed. Anyone may send it; the bounty pays
    /// only its fixed recipient.
    pub fn plan_v2_platform_fee_forward(
        &self,
        bounty_contract: &str,
    ) -> Result<EvmTransactionIntent, ChainBaseError> {
        Ok(EvmTransactionIntent {
            from: None,
            to: normalize_address(bounty_contract)?,
            value_wei: 0,
            data: encode_call("withdrawPlatformFee()", Vec::new()),
            function: "withdrawPlatformFee()".to_string(),
        })
    }
}

/// Checks a v2 creation request against its published terms: every v1 commitment, the v2
/// `protocol_version`, the factory's immutable fee terms, and the claim gate. Returns the platform
/// fee the funding target includes.
pub fn validate_autonomous_v2_creation_against_terms(
    network: &str,
    create: &AutonomousBountyV2Create,
    terms: &AutonomousBountyTermsRecord,
    factory_fee: &AutonomousV2FactoryFee,
) -> Result<u128, ChainBaseError> {
    validate_creation_against_terms_for_protocol(
        network,
        &create.base,
        terms,
        AUTONOMOUS_V2_PROTOCOL_VERSION,
    )?;
    let contract_terms = terms.document.contract_terms.as_object().ok_or_else(|| {
        ChainBaseError::InvalidTermsDocument("published contract_terms are unavailable".to_string())
    })?;
    let quote = quote_autonomous_v2_bounty(
        autonomous_money_to_uint256(&create.base.solver_reward, false)?,
        autonomous_money_to_uint256(&create.base.verifier_reward, true)?,
        factory_fee,
    )?;
    let platform_fee: u128 = quote
        .platform_fee
        .parse()
        .map_err(|_| ChainBaseError::InvalidAmount)?;
    let (registry, source) = create.gate_words()?;
    let mut planned = json!({
        "platform_fee_bps": quote.platform_fee_bps,
        "platform_fee": u64::try_from(platform_fee).map_err(|_| ChainBaseError::InvalidAmount)?,
        "platform_fee_recipient": quote.platform_fee_recipient,
    });
    if registry != [0u8; 32] {
        planned["claim_eligibility_registry"] = json!(address_from_word(registry));
        planned["claim_eligibility_source"] = json!(word_hex(source));
    }
    let mut errors = Vec::new();
    validate_v2_terms_against_creation(contract_terms, &planned, &mut errors);
    if let Some(error) = errors.into_iter().next() {
        return Err(ChainBaseError::InvalidTermsDocument(error));
    }
    Ok(platform_fee)
}

/// v2 public-earning gate: exact v2 terms plus the shared policy, with the fee in the target.
/// Gated (invoice-contractor) bounties are not open public inventory and are rejected here; plan
/// them through the invoice treasury path instead.
pub fn validate_autonomous_v2_creation_for_public_earning(
    network: &str,
    create: &AutonomousBountyV2Create,
    terms: &AutonomousBountyTermsRecord,
    factory_fee: &AutonomousV2FactoryFee,
) -> Result<u128, ChainBaseError> {
    let platform_fee =
        validate_autonomous_v2_creation_against_terms(network, create, terms, factory_fee)?;
    if create.claim_eligibility_registry.is_some() {
        return Err(ChainBaseError::InvalidTermsDocument(
            "claim-gated bounties are not open public earning inventory".to_string(),
        ));
    }
    validate_public_earning_policy(&create.base, terms, platform_fee)?;
    Ok(platform_fee)
}

/// One autonomous-v2 planning request, as accepted by `cli autonomous-v2-plan`. Every action
/// returns unsigned typed data or an unsigned transaction intent; none of them is funding, claim,
/// submission or settlement evidence.
#[derive(Debug, Clone, Deserialize)]
pub struct AutonomousV2PlanRequest {
    pub network: String,
    pub factory_contract: String,
    pub implementation_contract: String,
    #[serde(flatten)]
    pub action: AutonomousV2PlanAction,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AutonomousV2PlanAction {
    Quote {
        solver_reward: Money,
        verifier_reward: Money,
        factory_fee: AutonomousV2FactoryFee,
    },
    Create {
        create: AutonomousBountyV2Create,
        factory_fee: AutonomousV2FactoryFee,
    },
    AuthorizedCreate {
        create: AutonomousBountyV2Create,
        factory_fee: AutonomousV2FactoryFee,
        signature: AutonomousBountyAuthorizationSignature,
        #[serde(default)]
        relayer: Option<String>,
    },
    Claim {
        bounty_contract: String,
        solver: String,
        claim_bond: Money,
        #[serde(default)]
        authorization_nonce: Option<String>,
        #[serde(default)]
        authorization_valid_before: Option<u64>,
    },
    AuthorizedClaim {
        bounty_contract: String,
        solver: String,
        claim_bond: Money,
        authorization_nonce: String,
        authorization_valid_before: u64,
        signature: AutonomousBountyAuthorizationSignature,
        #[serde(default)]
        relayer: Option<String>,
    },
    SubmissionAuthorization {
        submission: AutonomousBountySubmissionAuthorizationRequest,
    },
    SubmissionRelay {
        bounty_contract: String,
        submission_hash: String,
        evidence_hash: String,
        deadline: u64,
        signature: String,
        #[serde(default)]
        relayer: Option<String>,
    },
    VerificationAttestation {
        attestation: AutonomousVerificationAttestationRequest,
    },
    AttestationSettlement {
        bounty_contract: String,
        #[serde(default)]
        caller: Option<String>,
        attestations: Vec<AutonomousSignedAttestation>,
    },
    PlatformFeeForward {
        bounty_contract: String,
    },
}

/// Plans one autonomous-v2 action. Claim plans reuse the shared v1 bounty surface and are
/// relabelled with the v2 protocol version.
pub fn plan_autonomous_v2_action(
    request: &AutonomousV2PlanRequest,
) -> Result<Value, ChainBaseError> {
    let planner = AutonomousBountyTxPlanner::new(
        &request.factory_contract,
        &request.implementation_contract,
    )?;
    let network = request.network.as_str();
    base_network_descriptor(network)?;
    match &request.action {
        AutonomousV2PlanAction::Quote {
            solver_reward,
            verifier_reward,
            factory_fee,
        } => to_plan_value(quote_autonomous_v2_bounty(
            autonomous_money_to_uint256(solver_reward, false)?,
            autonomous_money_to_uint256(verifier_reward, true)?,
            factory_fee,
        )?),
        AutonomousV2PlanAction::Create {
            create,
            factory_fee,
        } => to_plan_value(planner.plan_v2_creation(network, create, factory_fee)?),
        AutonomousV2PlanAction::AuthorizedCreate {
            create,
            factory_fee,
            signature,
            relayer,
        } => to_plan_value(planner.plan_v2_authorized_creation(
            network,
            create,
            factory_fee,
            signature,
            relayer.as_deref(),
        )?),
        AutonomousV2PlanAction::Claim {
            bounty_contract,
            solver,
            claim_bond,
            authorization_nonce,
            authorization_valid_before,
        } => {
            let mut plan = planner.plan_claim(
                network,
                bounty_contract,
                solver,
                autonomous_money_to_uint256(claim_bond, true)?,
                authorization_nonce.as_deref(),
                *authorization_valid_before,
            )?;
            plan.protocol_version = AUTONOMOUS_V2_PROTOCOL_VERSION.to_string();
            to_plan_value(plan)
        }
        AutonomousV2PlanAction::AuthorizedClaim {
            bounty_contract,
            solver,
            claim_bond,
            authorization_nonce,
            authorization_valid_before,
            signature,
            relayer,
        } => {
            let mut plan = planner.plan_authorized_claim(
                network,
                bounty_contract,
                solver,
                autonomous_money_to_uint256(claim_bond, false)?,
                authorization_nonce,
                *authorization_valid_before,
                signature,
                relayer.as_deref(),
            )?;
            plan.protocol_version = AUTONOMOUS_V2_PROTOCOL_VERSION.to_string();
            to_plan_value(plan)
        }
        AutonomousV2PlanAction::SubmissionAuthorization { submission } => {
            to_plan_value(planner.plan_v2_submission_authorization(network, submission)?)
        }
        AutonomousV2PlanAction::SubmissionRelay {
            bounty_contract,
            submission_hash,
            evidence_hash,
            deadline,
            signature,
            relayer,
        } => to_plan_value(planner.plan_signed_submission_relay(
            bounty_contract,
            submission_hash,
            evidence_hash,
            *deadline,
            signature,
            relayer.as_deref(),
        )?),
        AutonomousV2PlanAction::VerificationAttestation { attestation } => {
            to_plan_value(planner.plan_v2_verification_attestation(network, attestation)?)
        }
        AutonomousV2PlanAction::AttestationSettlement {
            bounty_contract,
            caller,
            attestations,
        } => to_plan_value(planner.plan_attestation_settlement(
            bounty_contract,
            caller.as_deref(),
            attestations,
        )?),
        AutonomousV2PlanAction::PlatformFeeForward { bounty_contract } => {
            to_plan_value(planner.plan_v2_platform_fee_forward(bounty_contract)?)
        }
    }
}

fn to_plan_value(plan: impl Serialize) -> Result<Value, ChainBaseError> {
    serde_json::to_value(plan)
        .map_err(|error| ChainBaseError::InvalidCanonicalJson(error.to_string()))
}

fn require_v2_params(params: &[[u8; 32]]) -> Result<(), ChainBaseError> {
    if params.len() != V2_PARAM_WORDS {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "v2 factory parameter tuple must contain sixteen words".to_string(),
        ));
    }
    Ok(())
}

fn encode_v2_create_call(
    params: &[[u8; 32]],
    verifiers: &[[u8; 32]],
    initial_funding: u128,
    creation_nonce: [u8; 32],
) -> Result<String, ChainBaseError> {
    require_v2_params(params)?;
    let mut bytes = selector(&autonomous_v2_create_bounty_function()).to_vec();
    params.iter().for_each(|word| bytes.extend_from_slice(word));
    // Head: 16 tuple words + verifiers offset + initialFunding + creationNonce.
    bytes.extend_from_slice(&encode_uint256(((V2_PARAM_WORDS + 3) * 32) as u128)?);
    bytes.extend_from_slice(&encode_uint256(initial_funding)?);
    bytes.extend_from_slice(&creation_nonce);
    bytes.extend_from_slice(&encode_uint256(verifiers.len() as u128)?);
    verifiers
        .iter()
        .for_each(|word| bytes.extend_from_slice(word));
    Ok(format!("0x{}", hex::encode(bytes)))
}

#[allow(clippy::too_many_arguments)]
fn encode_v2_authorized_create_call(
    creator: &str,
    params: &[[u8; 32]],
    verifiers: &[[u8; 32]],
    initial_funding: u128,
    creation_nonce: [u8; 32],
    valid_before: u64,
    v: u8,
    r: [u8; 32],
    s: [u8; 32],
) -> Result<String, ChainBaseError> {
    require_v2_params(params)?;
    let mut bytes = selector(&autonomous_v2_create_bounty_with_authorization_function()).to_vec();
    bytes.extend_from_slice(&encode_address(creator)?);
    params.iter().for_each(|word| bytes.extend_from_slice(word));
    // Head: creator + 16 tuple words + verifiers offset + initialFunding + creationNonce + 6 auth words.
    bytes.extend_from_slice(&encode_uint256(
        ((1 + V2_PARAM_WORDS + 3 + 6) * 32) as u128,
    )?);
    bytes.extend_from_slice(&encode_uint256(initial_funding)?);
    bytes.extend_from_slice(&creation_nonce);
    bytes.extend_from_slice(&encode_uint256(0)?);
    bytes.extend_from_slice(&encode_uint256(valid_before.into())?);
    bytes.extend_from_slice(&creation_nonce);
    bytes.extend_from_slice(&encode_uint256(v.into())?);
    bytes.extend_from_slice(&r);
    bytes.extend_from_slice(&s);
    bytes.extend_from_slice(&encode_uint256(verifiers.len() as u128)?);
    verifiers
        .iter()
        .for_each(|word| bytes.extend_from_slice(word));
    Ok(format!("0x{}", hex::encode(bytes)))
}

/// `keccak256(abi.encode(chainid, factory, creator, creationNonce, params, verifiers))`.
fn autonomous_v2_bounty_id(
    chain_id: u64,
    factory: &str,
    creator: &str,
    creation_nonce: [u8; 32],
    params: &[[u8; 32]],
    verifiers: &[[u8; 32]],
) -> Result<[u8; 32], ChainBaseError> {
    require_v2_params(params)?;
    let mut encoded = Vec::with_capacity((V2_PARAM_WORDS + 6 + verifiers.len()) * 32);
    encoded.extend_from_slice(&encode_uint256(chain_id.into())?);
    encoded.extend_from_slice(&encode_address(factory)?);
    encoded.extend_from_slice(&encode_address(creator)?);
    encoded.extend_from_slice(&creation_nonce);
    params
        .iter()
        .for_each(|word| encoded.extend_from_slice(word));
    encoded.extend_from_slice(&encode_uint256(((4 + V2_PARAM_WORDS + 1) * 32) as u128)?);
    encoded.extend_from_slice(&encode_uint256(verifiers.len() as u128)?);
    verifiers
        .iter()
        .for_each(|word| encoded.extend_from_slice(word));
    Ok(Keccak256::digest(encoded).into())
}

/// Platform fee projected from canonical v2 events. `status` is one of `pending`, `paid`,
/// `deferred`, `forwarded`, or `refundable`. Only `paid` and `forwarded` mean the recipient
/// received the fee.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousBountyPlatformFee {
    pub bps: u16,
    pub amount: String,
    pub recipient: String,
    pub status: String,
}

/// On-chain claim gate: only wallets with a current attestation from `source` in `registry`
/// can claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousBountyClaimEligibility {
    pub registry: String,
    pub source: String,
}

pub(crate) struct AutonomousV2Projection {
    pub platform_fee: Option<AutonomousBountyPlatformFee>,
    pub claim_eligibility: Option<AutonomousBountyClaimEligibility>,
}

fn invalid(message: impl Into<String>) -> ChainBaseError {
    ChainBaseError::InvalidLogData(message.into())
}

/// Validates canonical economics and projects v2 fee and gate state. A bounty is v2 exactly when
/// its factory emitted `CanonicalBountyPlatformFeeConfigured`; inconsistent canonical data fails
/// closed, as v1 economics do.
pub(crate) fn project_autonomous_economics(
    creation_data: &Value,
    events: &[AutonomousBountyEvent],
    status: &str,
    solver_reward: u64,
    verifier_reward: u64,
    claim_bond: u64,
    target_amount: u64,
) -> Result<AutonomousV2Projection, ChainBaseError> {
    let count =
        |kind: AutonomousBountyEventKind| events.iter().filter(|event| event.kind == kind).count();
    let fee_configs = count(AutonomousBountyEventKind::CanonicalBountyPlatformFeeConfigured);
    let gate_configs = count(AutonomousBountyEventKind::CanonicalBountyClaimEligibilityConfigured);
    if fee_configs > 1 || gate_configs > 1 {
        return Err(invalid("duplicate v2 configuration event"));
    }
    let is_v2 = fee_configs == 1;
    if !is_v2
        && (gate_configs == 1
            || events.iter().any(|event| {
                matches!(
                    event.kind,
                    AutonomousBountyEventKind::PlatformFeePaid
                        | AutonomousBountyEventKind::PlatformFeeDeferred
                        | AutonomousBountyEventKind::PlatformFeeWithdrawn
                )
            }))
    {
        return Err(invalid(
            "v2 fee or gate events without a v2 fee configuration",
        ));
    }
    if claim_bond != verifier_reward {
        return Err(invalid("canonical bounty economics are inconsistent"));
    }
    if !is_v2 {
        if solver_reward.checked_add(verifier_reward) != Some(target_amount) {
            return Err(invalid("canonical bounty economics are inconsistent"));
        }
        return Ok(AutonomousV2Projection {
            platform_fee: None,
            claim_eligibility: None,
        });
    }

    let bps = creation_data["platform_fee_bps"]
        .as_u64()
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| invalid("CanonicalBountyPlatformFeeConfigured missing platform_fee_bps"))?;
    let fee = creation_data["platform_fee"]
        .as_u64()
        .ok_or_else(|| invalid("CanonicalBountyPlatformFeeConfigured missing platform_fee"))?;
    let recipient = creation_data["platform_fee_recipient"]
        .as_str()
        .ok_or_else(|| {
            invalid("CanonicalBountyPlatformFeeConfigured missing platform_fee_recipient")
        })?
        .to_ascii_lowercase();
    let expected_fee = autonomous_v2_platform_fee(u128::from(solver_reward), bps)
        .map_err(|_| invalid("canonical v2 platform fee exceeds the protocol cap"))?;
    let expected_target = u128::from(solver_reward) + u128::from(verifier_reward) + expected_fee;
    if u128::from(fee) != expected_fee
        || u128::from(target_amount) != expected_target
        || (bps == 0) != (recipient == ZERO_ADDRESS)
    {
        return Err(invalid("canonical v2 bounty economics are inconsistent"));
    }

    let mut fee_status = if status == "cancelled" {
        "refundable"
    } else {
        "pending"
    };
    for event in events {
        let amount_field = match event.kind {
            AutonomousBountyEventKind::PlatformFeePaid
            | AutonomousBountyEventKind::PlatformFeeDeferred => "platform_fee",
            AutonomousBountyEventKind::PlatformFeeWithdrawn => "amount",
            _ => continue,
        };
        let event_recipient = event.data["platform_fee_recipient"]
            .as_str()
            .unwrap_or_default();
        if event.data[amount_field].as_u64() != Some(fee)
            || !event_recipient.eq_ignore_ascii_case(&recipient)
        {
            return Err(invalid(format!(
                "{:?} does not match the configured platform fee",
                event.kind
            )));
        }
        fee_status = match (event.kind, fee_status) {
            (AutonomousBountyEventKind::PlatformFeePaid, "pending") => "paid",
            (AutonomousBountyEventKind::PlatformFeeDeferred, "pending") => "deferred",
            (AutonomousBountyEventKind::PlatformFeeWithdrawn, "deferred") => "forwarded",
            (kind, current) => {
                return Err(invalid(format!(
                    "{kind:?} is out of order after fee status {current}"
                )));
            }
        };
    }

    let claim_eligibility = (gate_configs == 1)
        .then(
            || -> Result<AutonomousBountyClaimEligibility, ChainBaseError> {
                Ok(AutonomousBountyClaimEligibility {
                    registry: creation_data["claim_eligibility_registry"]
                        .as_str()
                        .ok_or_else(|| invalid("claim-eligibility event missing registry"))?
                        .to_ascii_lowercase(),
                    source: creation_data["claim_eligibility_source"]
                        .as_str()
                        .ok_or_else(|| invalid("claim-eligibility event missing source"))?
                        .to_ascii_lowercase(),
                })
            },
        )
        .transpose()?;

    Ok(AutonomousV2Projection {
        platform_fee: Some(AutonomousBountyPlatformFee {
            bps,
            amount: fee.to_string(),
            recipient,
            status: fee_status.to_string(),
        }),
        claim_eligibility,
    })
}

/// Checks that published `contract_terms` commit to exactly the on-chain v2 fee and gate, and
/// that v1 terms publish none of the v2-only keys.
pub(crate) fn validate_v2_terms_against_creation(
    contract_terms: &serde_json::Map<String, Value>,
    creation_data: &Value,
    errors: &mut Vec<String>,
) {
    let is_v2 = creation_data.get("platform_fee_bps").is_some();
    let expected_version = if is_v2 {
        AUTONOMOUS_V2_PROTOCOL_VERSION
    } else {
        AUTONOMOUS_V1_PROTOCOL_VERSION
    };
    if contract_terms
        .get("protocol_version")
        .and_then(Value::as_str)
        != Some(expected_version)
    {
        errors.push("contract_terms protocol_version does not match the contract".to_string());
    }
    if !is_v2 {
        if V2_ONLY_TERMS_KEYS
            .iter()
            .any(|key| contract_terms.contains_key(*key))
        {
            errors.push("contract_terms declare v2 fee or gate fields for a v1 bounty".to_string());
        }
        return;
    }
    if contract_terms
        .get("platform_fee_bps")
        .and_then(Value::as_u64)
        != creation_data["platform_fee_bps"].as_u64()
    {
        errors.push("contract_terms platform_fee_bps does not match the contract".to_string());
    }
    match contract_terms_money(contract_terms, "platform_fee", true) {
        Ok(amount) if creation_data["platform_fee"].as_u64() == Some(amount) => {}
        Ok(_) => errors.push("contract_terms platform_fee does not match the contract".to_string()),
        Err(error) => errors.push(error.to_string()),
    }
    let same_address = |key: &str, event_key: &str| {
        contract_terms
            .get(key)
            .and_then(Value::as_str)
            .zip(creation_data[event_key].as_str())
            .is_some_and(|(terms, event)| terms.eq_ignore_ascii_case(event))
    };
    if !same_address("platform_fee_recipient", "platform_fee_recipient") {
        errors
            .push("contract_terms platform_fee_recipient does not match the contract".to_string());
    }
    let gated = creation_data.get("claim_eligibility_registry").is_some();
    let terms_gated = contract_terms.contains_key("claim_eligibility_registry")
        || contract_terms.contains_key("claim_eligibility_source");
    if gated != terms_gated
        || (gated
            && (!same_address("claim_eligibility_registry", "claim_eligibility_registry")
                || !same_address("claim_eligibility_source", "claim_eligibility_source")))
    {
        errors.push("contract_terms claim eligibility does not match the contract".to_string());
    }
}

/// Validates v2 fee and gate commitments in a terms document before publication and returns the
/// platform fee the funding target must include. v1 documents must not carry v2-only keys.
pub(crate) fn validate_contract_terms_fee_commitment(
    object: &serde_json::Map<String, Value>,
    protocol_version: &str,
    solver_reward: u64,
) -> Result<u64, ChainBaseError> {
    let reject = |message: &str| Err(ChainBaseError::InvalidTermsDocument(message.to_string()));
    if protocol_version == AUTONOMOUS_V1_PROTOCOL_VERSION {
        if V2_ONLY_TERMS_KEYS
            .iter()
            .any(|key| object.contains_key(*key))
        {
            return reject("contract_terms v2 fee or gate fields require protocol_version agent-bounties/autonomous-v2");
        }
        return Ok(0);
    }
    let bps = contract_terms_u64(object, "platform_fee_bps")?;
    let bps = u16::try_from(bps)
        .ok()
        .filter(|bps| *bps <= AUTONOMOUS_V2_MAX_PLATFORM_FEE_BPS);
    let Some(bps) = bps else {
        return reject("contract_terms platform_fee_bps exceeds the protocol cap");
    };
    let fee = contract_terms_money(object, "platform_fee", true)?;
    let expected = autonomous_v2_platform_fee(u128::from(solver_reward), bps)?;
    if u128::from(fee) != expected {
        return reject(
            "contract_terms platform_fee must equal ceil(solver_reward * platform_fee_bps / 10000)",
        );
    }
    let recipient =
        normalize_evm_address(contract_terms_string(object, "platform_fee_recipient")?)?;
    if (bps == 0) != (recipient == ZERO_ADDRESS) {
        return reject(
            "contract_terms platform_fee_recipient must be nonzero exactly when the fee is nonzero",
        );
    }
    match (
        object.get("claim_eligibility_registry"),
        object.get("claim_eligibility_source"),
    ) {
        (None, None) => {}
        (Some(_), Some(_)) => {
            let registry = normalize_evm_address(contract_terms_string(
                object,
                "claim_eligibility_registry",
            )?)?;
            let source = parse_bytes32(contract_terms_string(object, "claim_eligibility_source")?)?;
            if registry == ZERO_ADDRESS || source == [0u8; 32] {
                return reject(
                    "contract_terms claim eligibility registry and source must be nonzero",
                );
            }
        }
        _ => {
            return reject(
                "contract_terms claim eligibility registry and source must be set together",
            )
        }
    }
    Ok(fee)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/autonomous-v2-loop.json");

    fn fixture() -> Value {
        serde_json::from_str(FIXTURE).expect("fixture is valid JSON")
    }

    fn fixture_events() -> Vec<AutonomousBountyEvent> {
        let logs: Vec<RpcEvmLog> = serde_json::from_value(fixture()["logs"].clone()).unwrap();
        decode_autonomous_bounty_logs(rpc_logs_to_evm_logs(logs).unwrap()).unwrap()
    }

    fn bounty_address(label: &str) -> String {
        fixture()["bounties"][label].as_str().unwrap().to_string()
    }

    fn feed_from(
        events: Vec<AutonomousBountyEvent>,
    ) -> Result<Vec<AutonomousBountyFeedItem>, ChainBaseError> {
        build_autonomous_bounty_feed(events, Vec::new(), false)
    }

    fn item(label: &str) -> AutonomousBountyFeedItem {
        let address = bounty_address(label);
        feed_from(fixture_events())
            .unwrap()
            .into_iter()
            .find(|item| item.bounty_contract == address)
            .unwrap_or_else(|| panic!("{label} missing from feed"))
    }

    fn events_for(label: &str) -> Vec<AutonomousBountyEvent> {
        let address = bounty_address(label);
        let events = fixture_events();
        let bounty_id = events
            .iter()
            .find(|event| event.data["bounty_contract"].as_str() == Some(address.as_str()))
            .unwrap()
            .bounty_id
            .clone();
        events
            .into_iter()
            .filter(|event| event.bounty_id == bounty_id)
            .collect()
    }

    fn factory_fee() -> AutonomousV2FactoryFee {
        AutonomousV2FactoryFee {
            platform_fee_bps: 750,
            platform_fee_recipient: fixture()["platform_fee_recipient"]
                .as_str()
                .unwrap()
                .to_string(),
        }
    }

    fn contractor_source() -> String {
        word_hex(Keccak256::digest(b"agent-bounties/invoice-contractor-v1").into())
    }

    #[test]
    fn decodes_every_log_emitted_by_the_compiled_v2_contracts() {
        let events = fixture_events();
        assert_eq!(events.len(), 54, "every factory and bounty log must decode");
        let count = |kind| events.iter().filter(|event| event.kind == kind).count();
        assert_eq!(count(AutonomousBountyEventKind::CanonicalBountyCreated), 5);
        assert_eq!(
            count(AutonomousBountyEventKind::CanonicalBountyPlatformFeeConfigured),
            5
        );
        assert_eq!(
            count(AutonomousBountyEventKind::CanonicalBountyClaimEligibilityConfigured),
            1
        );
        assert_eq!(count(AutonomousBountyEventKind::PlatformFeePaid), 2);
        assert_eq!(count(AutonomousBountyEventKind::PlatformFeeDeferred), 1);
        assert_eq!(count(AutonomousBountyEventKind::PlatformFeeWithdrawn), 1);
        assert_eq!(count(AutonomousBountyEventKind::SubmissionRejected), 1);
        let fee_config = events
            .iter()
            .find(|event| {
                event.kind == AutonomousBountyEventKind::CanonicalBountyPlatformFeeConfigured
            })
            .unwrap();
        assert_eq!(fee_config.data["platform_fee_bps"], 750);
        assert_eq!(fee_config.data["platform_fee"], 75_000);
        assert_eq!(
            fee_config.data["platform_fee_recipient"],
            fixture()["platform_fee_recipient"]
        );
        let gate = events
            .iter()
            .find(|event| {
                event.kind == AutonomousBountyEventKind::CanonicalBountyClaimEligibilityConfigured
            })
            .unwrap();
        assert_eq!(gate.data["claim_eligibility_source"], contractor_source());
    }

    #[test]
    fn projects_fee_status_and_economics_for_every_v2_scenario() {
        let recipient = fixture()["platform_fee_recipient"]
            .as_str()
            .unwrap()
            .to_string();
        let fee = |status: &str| {
            Some(AutonomousBountyPlatformFee {
                bps: 750,
                amount: "75000".to_string(),
                recipient: recipient.clone(),
                status: status.to_string(),
            })
        };

        let paid = item("paid_after_reject");
        assert_eq!(paid.status, "paid");
        assert_eq!(
            paid.protocol_version.as_deref(),
            Some(AUTONOMOUS_V2_PROTOCOL_VERSION)
        );
        assert_eq!(paid.target_amount, "1175000");
        assert_eq!(paid.claim_bond, "100000");
        assert_eq!(paid.platform_fee, fee("paid"));
        assert_eq!(paid.claim_eligibility, None);

        assert_eq!(item("fee_deferred_then_forwarded").status, "paid");
        assert_eq!(
            item("fee_deferred_then_forwarded").platform_fee,
            fee("forwarded")
        );

        let gated = item("contractor_gated");
        assert_eq!(gated.status, "claimed");
        assert_eq!(gated.platform_fee, fee("pending"));
        let gate = gated
            .claim_eligibility
            .expect("gated bounty exposes its gate");
        assert_eq!(gate.source, contractor_source());
        assert_eq!(gate.registry.len(), 42);

        let cancelled = item("cancelled_and_refunded");
        assert_eq!(cancelled.status, "cancelled");
        assert_eq!(cancelled.platform_fee, fee("refundable"));
    }

    #[test]
    fn gated_bounties_never_appear_ready_to_earn() {
        let mut gated = item("contractor_gated");
        gated.status = "claimable".to_string();
        gated.terms_valid = true;
        gated.verification_ready = true;
        assert!(!autonomous_bounty_is_earning_ready(&gated));
        gated.claim_eligibility = None;
        assert!(autonomous_bounty_is_earning_ready(&gated));
    }

    #[test]
    fn v1_items_serialize_without_v2_fields() {
        let mut v1 = item("paid_after_reject");
        v1.protocol_version = None;
        v1.platform_fee = None;
        v1.claim_eligibility = None;
        let value = serde_json::to_value(&v1).unwrap();
        for key in ["protocol_version", "platform_fee", "claim_eligibility"] {
            assert!(
                value.get(key).is_none(),
                "{key} must be omitted for v1 items"
            );
        }
    }

    #[test]
    fn inconsistent_v2_economics_fail_closed() {
        let tamper = |label: &str, change: &dyn Fn(&mut Vec<AutonomousBountyEvent>)| {
            let mut events = events_for(label);
            change(&mut events);
            feed_from(events)
        };
        let set_fee = |events: &mut Vec<AutonomousBountyEvent>, fee: u64| {
            for event in events.iter_mut() {
                if event.kind == AutonomousBountyEventKind::CanonicalBountyPlatformFeeConfigured {
                    event.data["platform_fee"] = json!(fee);
                }
            }
        };
        // Wrong fee amount, even if the target still adds up.
        assert!(tamper("cancelled_and_refunded", &|events| set_fee(events, 74_999)).is_err());
        // A v2 target read without its fee configuration no longer adds up as v1.
        assert!(tamper("cancelled_and_refunded", &|events| {
            events.retain(|event| {
                event.kind != AutonomousBountyEventKind::CanonicalBountyPlatformFeeConfigured
            })
        })
        .is_err());
        // Fee payment events without a v2 fee configuration.
        assert!(tamper("paid_after_reject", &|events| {
            events.retain(|event| {
                event.kind != AutonomousBountyEventKind::CanonicalBountyPlatformFeeConfigured
            })
        })
        .is_err());
        // A second payment of the same fee.
        assert!(tamper("paid_after_reject", &|events| {
            let mut duplicate = events
                .iter()
                .find(|event| event.kind == AutonomousBountyEventKind::PlatformFeePaid)
                .unwrap()
                .clone();
            duplicate.log_index += 1_000;
            events.push(duplicate);
        })
        .is_err());
        // A forward with no deferral before it.
        assert!(tamper("fee_deferred_then_forwarded", &|events| {
            events.retain(|event| event.kind != AutonomousBountyEventKind::PlatformFeeDeferred)
        })
        .is_err());
        // A paid fee to a different recipient.
        assert!(tamper("paid_after_reject", &|events| {
            for event in events.iter_mut() {
                if event.kind == AutonomousBountyEventKind::PlatformFeePaid {
                    event.data["platform_fee_recipient"] =
                        json!("0x00000000000000000000000000000000000000aa");
                }
            }
        })
        .is_err());
        // The untampered events still project.
        assert!(feed_from(events_for("paid_after_reject")).is_ok());
    }

    #[test]
    fn fee_math_matches_contract_rounding_and_cap() {
        assert_eq!(autonomous_v2_platform_fee(1_000_000, 750).unwrap(), 75_000);
        assert_eq!(autonomous_v2_platform_fee(1_001, 750).unwrap(), 76);
        assert_eq!(autonomous_v2_platform_fee(10_000, 750).unwrap(), 750);
        assert_eq!(autonomous_v2_platform_fee(1, 750).unwrap(), 1);
        assert_eq!(autonomous_v2_platform_fee(1_000_000, 0).unwrap(), 0);
        assert_eq!(
            autonomous_v2_platform_fee(1_000_000, 1_000).unwrap(),
            100_000
        );
        assert!(autonomous_v2_platform_fee(1_000_000, 1_001).is_err());

        let quote = quote_autonomous_v2_bounty(1_000_000, 100_000, &factory_fee()).unwrap();
        assert_eq!(quote.platform_fee, "75000");
        assert_eq!(quote.target_amount, "1175000");
        assert_eq!(quote.claim_bond, "100000");
        let without_recipient = AutonomousV2FactoryFee {
            platform_fee_bps: 750,
            platform_fee_recipient: ZERO_ADDRESS.to_string(),
        };
        assert!(quote_autonomous_v2_bounty(1_000_000, 100_000, &without_recipient).is_err());
        let recipient_without_fee = AutonomousV2FactoryFee {
            platform_fee_bps: 0,
            platform_fee_recipient: factory_fee().platform_fee_recipient,
        };
        assert!(quote_autonomous_v2_bounty(1_000_000, 100_000, &recipient_without_fee).is_err());
    }

    fn reconstructed_create(label: &str) -> AutonomousBountyV2Create {
        let events = events_for(label);
        let data = |kind: AutonomousBountyEventKind| {
            events
                .iter()
                .find(|event| event.kind == kind)
                .map(|event| event.data.clone())
        };
        let created = data(AutonomousBountyEventKind::CanonicalBountyCreated).unwrap();
        let terms = data(AutonomousBountyEventKind::CanonicalBountyTermsCommitted).unwrap();
        let economics =
            data(AutonomousBountyEventKind::CanonicalBountyEconomicsConfigured).unwrap();
        let verification =
            data(AutonomousBountyEventKind::CanonicalBountyVerificationConfigured).unwrap();
        let gate = data(AutonomousBountyEventKind::CanonicalBountyClaimEligibilityConfigured);
        let usdc = |field: &str| Money {
            amount: economics[field].as_i64().unwrap(),
            currency: "usdc".to_string(),
        };
        let text = |value: &Value, field: &str| value[field].as_str().unwrap().to_string();
        AutonomousBountyV2Create {
            base: AutonomousBountyCreate {
                creator: text(&created, "creator"),
                solver_reward: usdc("solver_reward"),
                verifier_reward: usdc("verifier_reward"),
                terms_hash: text(&created, "terms_hash"),
                policy_hash: text(&created, "policy_hash"),
                acceptance_criteria_hash: text(&terms, "acceptance_criteria_hash"),
                benchmark_hash: text(&terms, "benchmark_hash"),
                evidence_schema_hash: text(&terms, "evidence_schema_hash"),
                funding_deadline: economics["funding_deadline"].as_u64().unwrap(),
                claim_window_seconds: economics["claim_window_seconds"].as_u64().unwrap(),
                verification_window_seconds: economics["verification_window_seconds"]
                    .as_u64()
                    .unwrap(),
                verification_mode: AutonomousVerificationMode::DeterministicModule,
                verifier_module: Some(text(&verification, "verifier_module")),
                verifier_reward_recipient: Some(text(&verification, "verifier_reward_recipient")),
                verifiers: Vec::new(),
                threshold: 1,
                initial_funding: usdc("initial_funding"),
                creation_nonce: text(&created, "creation_nonce"),
            },
            claim_eligibility_registry: gate
                .as_ref()
                .map(|gate| text(gate, "claim_eligibility_registry")),
            claim_eligibility_source: gate
                .as_ref()
                .map(|gate| text(gate, "claim_eligibility_source")),
        }
    }

    fn planner() -> AutonomousBountyTxPlanner {
        let fixture = fixture();
        AutonomousBountyTxPlanner::new(
            fixture["factory"].as_str().unwrap(),
            fixture["implementation"].as_str().unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn planned_creation_matches_solidity_calldata_bounty_id_and_address() {
        let fixture = fixture();
        for label in [
            "paid_after_reject",
            "contractor_gated",
            "cancelled_and_refunded",
        ] {
            let create = reconstructed_create(label);
            let planned = planner()
                .plan_v2_creation("base-sepolia", &create, &factory_fee())
                .unwrap();
            assert_eq!(
                planned.plan.create_bounty.data,
                fixture["create_bounty_calldata"][label].as_str().unwrap(),
                "{label} createBounty calldata must be byte-identical to the Solidity call"
            );
            assert_eq!(
                planned.plan.predicted_bounty_contract,
                bounty_address(label),
                "{label} CREATE2 address"
            );
            let created_id = &events_for(label)[0].bounty_id;
            assert_eq!(&planned.plan.bounty_id, created_id, "{label} bounty id");
            assert_eq!(
                planned.plan.protocol_version,
                AUTONOMOUS_V2_PROTOCOL_VERSION
            );
            assert_eq!(planned.quote.target_amount, "1175000");
        }
    }

    #[test]
    fn planned_authorized_creation_matches_the_abi_encoder() {
        let reference = &fixture()["authorized_create_reference"];
        let signature = AutonomousBountyAuthorizationSignature {
            v: reference["v"].as_u64().unwrap() as u8,
            r: reference["r"].as_str().unwrap().to_string(),
            s: reference["s"].as_str().unwrap().to_string(),
        };
        let planned = planner()
            .plan_v2_authorized_creation(
                "base-sepolia",
                &reconstructed_create("contractor_gated"),
                &factory_fee(),
                &signature,
                None,
            )
            .unwrap();
        assert_eq!(
            planned.relay_transaction.data,
            reference["calldata"].as_str().unwrap()
        );
        assert_eq!(
            planned.relay_transaction.function,
            autonomous_v2_create_bounty_with_authorization_function()
        );
    }

    #[test]
    fn planner_rejects_funding_above_the_fee_inclusive_target_and_half_gates() {
        let mut create = reconstructed_create("paid_after_reject");
        create.base.initial_funding.amount = 1_175_001;
        assert!(planner()
            .plan_v2_creation("base-sepolia", &create, &factory_fee())
            .is_err());
        let mut half_gate = reconstructed_create("contractor_gated");
        half_gate.claim_eligibility_source = None;
        assert!(planner()
            .plan_v2_creation("base-sepolia", &half_gate, &factory_fee())
            .is_err());
    }

    #[test]
    fn v2_signing_uses_domain_version_two_and_v1_is_unchanged() {
        let bounty = bounty_address("paid_after_reject");
        let submission = AutonomousBountySubmissionAuthorizationRequest {
            bounty_contract: bounty.clone(),
            bounty_id: format!("0x{}", "ab".repeat(32)),
            solver: "0x1111111111111111111111111111111111111111".to_string(),
            round: 1,
            submission_hash: format!("0x{}", "01".repeat(32)),
            evidence_hash: format!("0x{}", "02".repeat(32)),
            policy_hash: format!("0x{}", "03".repeat(32)),
            deadline: 1_900_000_000,
        };
        let v2 = planner()
            .plan_v2_submission_authorization("base-sepolia", &submission)
            .unwrap();
        let v1 = planner()
            .plan_submission_authorization("base-sepolia", &submission)
            .unwrap();
        assert_eq!(v2.domain.version, "2");
        assert_eq!(v1.domain.version, "1");
        assert_eq!(v2.message.round, v1.message.round);
        let forward = planner().plan_v2_platform_fee_forward(&bounty).unwrap();
        assert_eq!(
            forward.data,
            format!("0x{}", hex::encode(selector("withdrawPlatformFee()")))
        );
    }

    fn v2_contract_terms() -> serde_json::Map<String, Value> {
        json!({
            "protocol_version": AUTONOMOUS_V2_PROTOCOL_VERSION,
            "platform_fee_bps": 750,
            "platform_fee": {"amount": 75000, "currency": "usdc"},
            "platform_fee_recipient": "0xfee0000000000000000000000000000000000fee",
        })
        .as_object()
        .unwrap()
        .clone()
    }

    #[test]
    fn terms_publication_requires_exact_v2_fee_commitments() {
        let terms = v2_contract_terms();
        assert_eq!(
            validate_contract_terms_fee_commitment(
                &terms,
                AUTONOMOUS_V2_PROTOCOL_VERSION,
                1_000_000
            )
            .unwrap(),
            75_000
        );
        let with = |key: &str, value: Value| {
            let mut terms = v2_contract_terms();
            terms.insert(key.to_string(), value);
            validate_contract_terms_fee_commitment(
                &terms,
                AUTONOMOUS_V2_PROTOCOL_VERSION,
                1_000_000,
            )
        };
        assert!(with("platform_fee", json!({"amount": 74999, "currency": "usdc"})).is_err());
        assert!(with("platform_fee_bps", json!(1001)).is_err());
        assert!(with("platform_fee_recipient", json!(ZERO_ADDRESS)).is_err());
        assert!(with(
            "claim_eligibility_registry",
            json!("0x9fe46736679d2d9a65f0992f2272de9f3c7fa6e0")
        )
        .is_err());
        let mut gated = v2_contract_terms();
        gated.insert(
            "claim_eligibility_registry".to_string(),
            json!("0x9fe46736679d2d9a65f0992f2272de9f3c7fa6e0"),
        );
        gated.insert(
            "claim_eligibility_source".to_string(),
            json!(contractor_source()),
        );
        assert!(validate_contract_terms_fee_commitment(
            &gated,
            AUTONOMOUS_V2_PROTOCOL_VERSION,
            1_000_000
        )
        .is_ok());
        // v1 terms cannot advertise a fee or gate they do not have.
        assert!(validate_contract_terms_fee_commitment(
            &terms,
            AUTONOMOUS_V1_PROTOCOL_VERSION,
            1_000_000
        )
        .is_err());
        let v1 = json!({"protocol_version": AUTONOMOUS_V1_PROTOCOL_VERSION});
        assert_eq!(
            validate_contract_terms_fee_commitment(
                v1.as_object().unwrap(),
                AUTONOMOUS_V1_PROTOCOL_VERSION,
                1_000_000
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn published_terms_must_match_on_chain_fee_and_gate() {
        let paid = item("paid_after_reject");
        let creation = json!({
            "platform_fee_bps": 750,
            "platform_fee": 75000,
            "platform_fee_recipient": paid.platform_fee.as_ref().unwrap().recipient,
        });
        let mut errors = Vec::new();
        validate_v2_terms_against_creation(&v2_contract_terms(), &creation, &mut errors);
        assert!(errors.is_empty(), "{errors:?}");

        let mut wrong_recipient = v2_contract_terms();
        wrong_recipient.insert(
            "platform_fee_recipient".to_string(),
            json!("0x00000000000000000000000000000000000000aa"),
        );
        let mut errors = Vec::new();
        validate_v2_terms_against_creation(&wrong_recipient, &creation, &mut errors);
        assert_eq!(errors.len(), 1);

        let mut gated_creation = creation.clone();
        gated_creation["claim_eligibility_registry"] =
            json!("0x9fe46736679d2d9a65f0992f2272de9f3c7fa6e0");
        gated_creation["claim_eligibility_source"] = json!(contractor_source());
        let mut errors = Vec::new();
        validate_v2_terms_against_creation(&v2_contract_terms(), &gated_creation, &mut errors);
        assert_eq!(
            errors,
            vec!["contract_terms claim eligibility does not match the contract".to_string()]
        );

        let v1_creation = json!({});
        let mut errors = Vec::new();
        validate_v2_terms_against_creation(&v2_contract_terms(), &v1_creation, &mut errors);
        assert_eq!(
            errors.len(),
            2,
            "v1 bounty with v2 terms: wrong version and v2-only keys"
        );
    }

    fn plan_request(fixture: &Value, step: &Value) -> AutonomousV2PlanRequest {
        let mut request = json!({
            "network": "base-sepolia",
            "factory_contract": fixture["factory"],
            "implementation_contract": fixture["implementation"],
        });
        for (key, value) in step["request"].as_object().unwrap() {
            request[key] = value.clone();
        }
        serde_json::from_value(request).expect("recorded request parses")
    }

    fn planned_field(plan: Value, field: &Value) -> Value {
        match field.as_str() {
            Some(field) => plan[field].clone(),
            None => plan,
        }
    }

    /// The capture tool drove a quorum bounty from creation to settlement using only
    /// `cli autonomous-v2-plan` output, `cast wallet sign --data` signatures, and a separate
    /// relayer. Replanning every recorded request must reproduce exactly the typed data that was
    /// signed and the calldata the contracts accepted, and the relayed transactions must carry
    /// the canonical creation, funding, claim, submission, settlement and fee events.
    #[test]
    fn gasless_quorum_loop_executes_current_planner_output() {
        let fixture = fixture();
        let gasless = &fixture["gasless_loop"];
        let steps = gasless["steps"].as_array().unwrap();
        let names: Vec<&str> = steps
            .iter()
            .map(|step| step["step"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "authorized_create",
                "authorized_claim",
                "submission_relay",
                "attestation_settlement"
            ]
        );
        let relayer = steps[0]["relay"]["relayer"].as_str().unwrap().to_string();
        for step in steps {
            for authorization in step["authorizations"].as_array().unwrap() {
                let planned = plan_autonomous_v2_action(&plan_request(&fixture, authorization))
                    .expect("authorization replans");
                assert_eq!(
                    planned_field(planned, &authorization["typed_data_field"]),
                    authorization["typed_data"],
                    "{} typed data drifted from what was signed",
                    step["step"]
                );
                assert_ne!(authorization["signer"].as_str(), Some(relayer.as_str()));
            }
            let relay = &step["relay"];
            let intent = planned_field(
                plan_autonomous_v2_action(&plan_request(&fixture, relay)).expect("relay replans"),
                &relay["intent_field"],
            );
            assert_eq!(
                intent["data"], relay["calldata"],
                "{} calldata",
                step["step"]
            );
            assert_eq!(intent["to"], relay["to"], "{} destination", step["step"]);
        }

        let bounty = gasless["bounty"].as_str().unwrap();
        assert_eq!(bounty, bounty_address("gasless_quorum"));
        let events = events_for("gasless_quorum");
        let kinds_in = |step: usize| -> Vec<AutonomousBountyEventKind> {
            let hash = steps[step]["relay"]["transaction_hash"].as_str().unwrap();
            events
                .iter()
                .filter(|event| event.tx_hash.eq_ignore_ascii_case(hash))
                .map(|event| event.kind)
                .collect()
        };
        use AutonomousBountyEventKind as Kind;
        let created = kinds_in(0);
        for kind in [
            Kind::CanonicalBountyCreated,
            Kind::CanonicalBountyPlatformFeeConfigured,
            Kind::FundingAdded,
            Kind::BountyBecameClaimable,
        ] {
            assert!(created.contains(&kind), "creation relay lacks {kind:?}");
        }
        assert_eq!(kinds_in(1), [Kind::BountyClaimed]);
        assert_eq!(kinds_in(2), [Kind::SubmissionAdded]);
        // `_settle` pays the fee before it emits `BountySettled`.
        assert_eq!(kinds_in(3), [Kind::PlatformFeePaid, Kind::BountySettled]);

        let poster = steps[0]["authorizations"][0]["signer"].as_str().unwrap();
        let solver = steps[1]["authorizations"][0]["signer"].as_str().unwrap();
        let find = |kind| events.iter().find(|event| event.kind == kind).unwrap();
        assert_eq!(
            find(Kind::CanonicalBountyCreated).data["bounty_contract"],
            bounty
        );
        assert_eq!(find(Kind::FundingAdded).data["contributor"], poster);
        assert_eq!(find(Kind::FundingAdded).data["amount"], 1_175_000);
        assert_eq!(find(Kind::BountyClaimed).data["solver"], solver);
        assert_eq!(find(Kind::BountySettled).data["solver"], solver);
        assert_eq!(find(Kind::BountySettled).data["solver_payout"], 1_100_000);
        assert_eq!(find(Kind::PlatformFeePaid).data["platform_fee"], 75_000);

        let item = item("gasless_quorum");
        assert_eq!(item.status, "paid");
        assert_eq!(item.target_amount, "1175000");
        assert_eq!(
            item.platform_fee.map(|fee| fee.status),
            Some("paid".to_string())
        );
    }

    #[test]
    fn plan_requests_reject_unknown_networks_and_bad_submission_signatures() {
        let fixture = fixture();
        let relay = &fixture["gasless_loop"]["steps"][2]["relay"];
        let mut request = plan_request(&fixture, relay);
        assert!(plan_autonomous_v2_action(&request).is_ok());
        request.network = "ethereum".to_string();
        assert!(plan_autonomous_v2_action(&request).is_err());

        let mut short = plan_request(&fixture, relay);
        if let AutonomousV2PlanAction::SubmissionRelay { signature, .. } = &mut short.action {
            signature.truncate(signature.len() - 2);
        }
        assert!(plan_autonomous_v2_action(&short).is_err());
    }
}
