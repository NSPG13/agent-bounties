//! Separate, undeployed creator-open protocol. Planners never imply funding or payment.
use super::{base_network_descriptor, canonical_json_bytes, ChainBaseError, EvmTransactionIntent};
use alloy::{
    primitives::{keccak256, Address, Bytes, B256, U256},
    sol_types::{SolCall, SolStruct, SolValue},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const PROTOCOL: &str = "agent-bounties/creator-open-v1";
pub const DISCLOSURE: &str = "The creator judges the published criteria. The first passing creator verdict pays one solver. Rejected entries pay their bond to the creator; unreviewed entries recover only their bond after their review deadline or another winner. There is no exclusive claim or guarantee of payment for work.";

pub mod abi {
    alloy::sol! {
        #[derive(Debug, serde::Serialize, serde::Deserialize)]
        struct Config {
            bytes32 bountyId; address creator; uint128 solverReward; uint128 reviewReward;
            bytes32 termsHash; bytes32 acceptanceCriteriaHash; bytes32 evidenceSchemaHash;
            uint64 submissionDeadline; uint64 reviewWindow; uint16 maxPendingEntries;
        }
        #[derive(Debug, serde::Serialize, serde::Deserialize)]
        struct TokenAuthorization { uint256 validAfter; uint256 validBefore; bytes32 nonce; bytes signature; }
        #[derive(Debug, serde::Serialize, serde::Deserialize)]
        struct Submission { address solver; bytes32 submissionHash; bytes32 evidenceHash; bytes32 termsHash; uint64 revision; uint256 deadline; }
        #[derive(Debug, serde::Serialize, serde::Deserialize)]
        struct OpenSubmission { address solver; bytes32 submissionHash; bytes32 evidenceHash; bytes32 termsHash; uint64 revision; bytes32 tokenNonce; uint256 deadline; }
        #[derive(Debug, serde::Serialize, serde::Deserialize)]
        struct CreatorReview { uint64 entry; address solver; bytes32 submissionHash; bytes32 evidenceHash; bytes32 termsHash; uint64 revision; bool passed; bytes32 responseHash; uint256 deadline; }
        #[derive(Debug, serde::Serialize, serde::Deserialize)]
        struct IncreaseReward { uint64 revision; bytes32 termsHash; bytes32 newTermsHash; uint256 newReward; bytes32 tokenNonce; uint256 deadline; }
        #[derive(Debug, serde::Serialize, serde::Deserialize)]
        struct CancelBounty { uint64 revision; bytes32 termsHash; uint256 deadline; }
        #[derive(Debug, serde::Serialize, serde::Deserialize)]
        struct ReceiveWithAuthorization { address from; address to; uint256 value; uint256 validAfter; uint256 validBefore; bytes32 nonce; }
        interface Factory {
            function createWithAuthorization(Config config, bytes32 userSalt, TokenAuthorization authorization) external returns(address);
        }
        interface Bounty {
            function submitAuthorized(Submission submission, TokenAuthorization authorization, bytes signature) external returns(uint64);
            function reviewAuthorized(uint64 id, bool passed, bytes32 responseHash, uint256 deadline, bytes signature) external;
            function increaseRewardAuthorized(uint128 newReward, bytes32 newTermsHash, TokenAuthorization authorization, uint256 deadline, bytes signature) external;
            function cancelAuthorized(uint256 deadline, bytes signature) external;
            function expire() external;
            function refundBond(uint64 id) external;
            function refundPrincipal() external;
        }
        event CreatorOpenBountyCreated(bytes32 indexed bountyId, address indexed bounty, address indexed creator, bytes32 termsHash, bytes32 configurationHash, bytes32 userSalt);
        event OpenBountyFunded(bytes32 indexed bountyId, address indexed creator, uint256 principal, bytes32 termsHash);
        event OpenSubmissionAdded(bytes32 indexed bountyId, uint64 indexed entry, address indexed solver, bytes32 submissionHash, bytes32 evidenceHash, bytes32 termsHash, uint64 revision, uint64 reviewDeadline, uint256 bond);
        event OpenSubmissionRejected(bytes32 indexed bountyId, uint64 indexed entry, address indexed solver, bytes32 responseHash, uint256 reviewReward);
        event OpenBountySettled(bytes32 indexed bountyId, uint64 indexed entry, address indexed solver, uint256 solverReward, uint256 bondReturned, uint256 reviewReward, bytes32 submissionHash, bytes32 evidenceHash, bytes32 termsHash, bytes32 responseHash);
        event OpenBondRefunded(bytes32 indexed bountyId, uint64 indexed entry, address indexed solver, uint256 amount);
        event OpenRewardIncreased(bytes32 indexed bountyId, uint64 revision, bytes32 oldTermsHash, bytes32 newTermsHash, uint256 oldReward, uint256 newReward);
        event OpenBountyCancelled(bytes32 indexed bountyId);
        event OpenPrincipalRefunded(bytes32 indexed bountyId, address indexed creator, uint256 amount);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreatorOpenTerms {
    pub schema: String,
    pub network: String,
    pub bounty_id: String,
    pub creator: String,
    pub title: String,
    pub goal: String,
    pub acceptance_criteria: Vec<String>,
    pub solver_reward_micro_usdc: u64,
    pub review_reward_micro_usdc: u64,
    pub submission_deadline: u64,
    pub review_window_seconds: u64,
    pub max_pending_entries: u16,
    pub disclosure: String,
}

pub type Terms = CreatorOpenTerms;

fn invalid(message: &str) -> ChainBaseError {
    ChainBaseError::InvalidTermsDocument(format!("creator-open: {message}"))
}
fn address(value: &str) -> Result<Address, ChainBaseError> {
    let parsed = value
        .parse::<Address>()
        .map_err(|_| invalid("invalid address"))?;
    if parsed.is_zero() {
        return Err(invalid("zero address"));
    }
    Ok(parsed)
}
fn hash(value: &str) -> Result<B256, ChainBaseError> {
    let parsed = value
        .parse::<B256>()
        .map_err(|_| invalid("invalid commitment"))?;
    if parsed.is_zero() {
        return Err(invalid("zero commitment"));
    }
    Ok(parsed)
}
pub fn evidence_schema() -> Value {
    json!({"type":"object","required":["artifact_url","artifact_sha256"],"properties":{
        "artifact_url":{"type":"string","pattern":"^https://"},
        "artifact_sha256":{"type":"string","pattern":"^sha256:[0-9a-f]{64}$"}}})
}
fn json_hash(value: &impl Serialize) -> Result<B256, ChainBaseError> {
    Ok(keccak256(canonical_json_bytes(
        &serde_json::to_value(value).map_err(|_| invalid("invalid JSON"))?,
    )?))
}
impl Terms {
    pub fn validate(&self, now: u64) -> Result<(), ChainBaseError> {
        base_network_descriptor(&self.network)?;
        address(&self.creator)?;
        hash(&self.bounty_id)?;
        if self.schema != PROTOCOL || self.disclosure != DISCLOSURE {
            return Err(invalid("protocol or disclosure mismatch"));
        }
        if self.title.trim().is_empty()
            || self.title.len() > 200
            || self.goal.trim().is_empty()
            || self.goal.len() > 16000
            || self.acceptance_criteria.is_empty()
            || self.acceptance_criteria.len() > 64
            || self
                .acceptance_criteria
                .iter()
                .any(|s| s.trim().is_empty() || s.len() > 4000)
        {
            return Err(invalid("missing or oversized public requirements"));
        }
        if self.solver_reward_micro_usdc == 0
            || self.review_reward_micro_usdc == 0
            || self.solver_reward_micro_usdc > 9_000_000_000_000_000
            || self.review_reward_micro_usdc > 9_000_000_000_000_000
        {
            return Err(invalid("rewards outside exact integer range"));
        }
        if self.submission_deadline <= now
            || self.submission_deadline > now.saturating_add(366 * 86400)
            || !(3600..=30 * 86400).contains(&self.review_window_seconds)
            || !(1..=64).contains(&self.max_pending_entries)
        {
            return Err(invalid("invalid work or review window or queue cap"));
        }
        Ok(())
    }
    pub fn config(&self, now: u64) -> Result<abi::Config, ChainBaseError> {
        self.validate(now)?;
        Ok(abi::Config {
            bountyId: hash(&self.bounty_id)?,
            creator: address(&self.creator)?,
            solverReward: self.solver_reward_micro_usdc.into(),
            reviewReward: self.review_reward_micro_usdc.into(),
            termsHash: json_hash(self)?,
            acceptanceCriteriaHash: json_hash(&self.acceptance_criteria)?,
            evidenceSchemaHash: json_hash(&evidence_schema())?,
            submissionDeadline: self.submission_deadline,
            reviewWindow: self.review_window_seconds,
            maxPendingEntries: self.max_pending_entries,
        })
    }
    pub fn increased(&self, new_reward: u64, now: u64) -> Result<Self, ChainBaseError> {
        if new_reward <= self.solver_reward_micro_usdc {
            return Err(invalid("reward must increase"));
        }
        let mut revised = self.clone();
        revised.solver_reward_micro_usdc = new_reward;
        revised.validate(now)?;
        Ok(revised)
    }
}

pub fn clone_init_code(implementation: Address) -> Vec<u8> {
    [
        hex::decode("3d602d80600a3d3981f3363d3d373d3d3d363d73").unwrap(),
        implementation.to_vec(),
        hex::decode("5af43d82803e903d91602b57fd5bf3").unwrap(),
    ]
    .concat()
}
pub fn predict(
    factory: Address,
    implementation: Address,
    config: &abi::Config,
    salt: B256,
) -> Address {
    let salt = keccak256((config.clone(), salt).abi_encode_params());
    factory.create2(salt, keccak256(clone_init_code(implementation)))
}
pub fn domain(chain: u64, bounty: Address) -> alloy::sol_types::Eip712Domain {
    alloy::sol_types::eip712_domain! {name:"Agent Bounties Creator Open",version:"1",chain_id:chain,verifying_contract:bounty,}
}
pub fn token_domain(chain: u64, token: Address) -> alloy::sol_types::Eip712Domain {
    alloy::sol_types::eip712_domain! {name:if chain==84532 {"USDC"} else {"USD Coin"},version:"2",chain_id:chain,verifying_contract:token,}
}
/// Clients must show and sign the exact EIP-712 envelope; this helper carries no authority.
pub fn typed_data<S>(message: &S, domain: alloy::sol_types::Eip712Domain) -> Value
where
    S: SolStruct + Serialize,
{
    let types = S::eip712_encode_type();
    // These protocol structs have no nested message types. Derive the field
    // list from Alloy's canonical type so encoding and the displayed envelope agree.
    let fields = types
        .split_once('(')
        .unwrap()
        .1
        .trim_end_matches(')')
        .split(',')
        .map(|field| {
            let (ty, name) = field.split_once(' ').unwrap();
            json!({"name":name,"type":ty})
        })
        .collect::<Vec<_>>();
    json!({"domain":{"name":domain.name,"version":domain.version,"chainId":domain.chain_id.map(|n|n.to_string()),"verifyingContract":domain.verifying_contract},"primaryType":S::NAME,"types":{
        "EIP712Domain":[{"name":"name","type":"string"},{"name":"version","type":"string"},{"name":"chainId","type":"uint256"},{"name":"verifyingContract","type":"address"}],
        S::NAME:fields},"message":message})
}
fn intent<C: SolCall>(target: Address, call: C) -> EvmTransactionIntent {
    EvmTransactionIntent {
        from: None,
        to: format!("{target:#x}"),
        value_wei: 0,
        data: format!("0x{}", hex::encode(call.abi_encode())),
        function: C::SIGNATURE.into(),
    }
}
pub fn plan_create(
    factory: Address,
    config: abi::Config,
    salt: B256,
    authorization: abi::TokenAuthorization,
) -> EvmTransactionIntent {
    intent(
        factory,
        abi::Factory::createWithAuthorizationCall {
            config,
            userSalt: salt,
            authorization,
        },
    )
}
pub fn plan_submit(
    bounty: Address,
    submission: abi::Submission,
    authorization: abi::TokenAuthorization,
    signature: Bytes,
) -> EvmTransactionIntent {
    intent(
        bounty,
        abi::Bounty::submitAuthorizedCall {
            submission,
            authorization,
            signature,
        },
    )
}
pub fn plan_review(
    bounty: Address,
    id: u64,
    passed: bool,
    response: B256,
    deadline: u64,
    signature: Bytes,
) -> EvmTransactionIntent {
    intent(
        bounty,
        abi::Bounty::reviewAuthorizedCall {
            id,
            passed,
            responseHash: response,
            deadline: U256::from(deadline),
            signature,
        },
    )
}
pub fn plan_increase(
    bounty: Address,
    reward: u128,
    terms: B256,
    authorization: abi::TokenAuthorization,
    deadline: u64,
    signature: Bytes,
) -> EvmTransactionIntent {
    intent(
        bounty,
        abi::Bounty::increaseRewardAuthorizedCall {
            newReward: reward,
            newTermsHash: terms,
            authorization,
            deadline: U256::from(deadline),
            signature,
        },
    )
}
pub fn plan_cancel(bounty: Address, deadline: u64, signature: Bytes) -> EvmTransactionIntent {
    intent(
        bounty,
        abi::Bounty::cancelAuthorizedCall {
            deadline: U256::from(deadline),
            signature,
        },
    )
}
pub fn plan_expire(bounty: Address) -> EvmTransactionIntent {
    intent(bounty, abi::Bounty::expireCall {})
}
pub fn plan_refund_bond(bounty: Address, id: u64) -> EvmTransactionIntent {
    intent(bounty, abi::Bounty::refundBondCall { id })
}
pub fn plan_refund_principal(bounty: Address) -> EvmTransactionIntent {
    intent(bounty, abi::Bounty::refundPrincipalCall {})
}

#[derive(Debug, Serialize)]
pub struct CreationPreparation {
    pub protocol: &'static str,
    pub network: String,
    pub bounty_contract: String,
    pub terms_hash: String,
    pub configuration_hash: String,
    pub customer_principal_micro_usdc: String,
    pub customer_gas_wei: &'static str,
    pub authorization: Value,
    pub authorization_digest: String,
    pub config: abi::Config,
    pub user_salt: String,
    pub evidence_boundary: &'static str,
}
pub fn prepare_creation(
    terms: &Terms,
    factory: Address,
    implementation: Address,
    salt: B256,
    nonce: B256,
    now: u64,
) -> Result<CreationPreparation, ChainBaseError> {
    if factory.is_zero() || implementation.is_zero() || nonce.is_zero() {
        return Err(invalid("missing deployment or authorization identity"));
    }
    let config = terms.config(now)?;
    let bounty = predict(factory, implementation, &config, salt);
    let network = base_network_descriptor(&terms.network)?;
    let principal = U256::from(config.solverReward) + U256::from(config.reviewReward);
    let authorization = abi::ReceiveWithAuthorization {
        from: config.creator,
        to: bounty,
        value: principal,
        validAfter: U256::ZERO,
        validBefore: U256::from(now.saturating_add(3600)),
        nonce,
    };
    let domain = token_domain(
        network.chain_id,
        address(&network.native_usdc_token_address)?,
    );
    Ok(CreationPreparation{protocol:PROTOCOL,network:terms.network.clone(),bounty_contract:format!("{bounty:#x}"),
        terms_hash:format!("{:#x}",config.termsHash),configuration_hash:format!("{:#x}",keccak256(config.abi_encode())),
        customer_principal_micro_usdc:principal.to_string(),customer_gas_wei:"0",
        authorization_digest:format!("{:#x}",authorization.eip712_signing_hash(&domain)),
        authorization:typed_data(&authorization,domain),config,user_salt:format!("{salt:#x}"),
        evidence_boundary:"Preparation is not authorization, funding, or a deployed release. Only a verified canonical creation and exact funded event make a bounty open."})
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub solver: String,
    pub submission_hash: String,
    pub evidence_hash: String,
    pub review_deadline: u64,
    pub status: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Projection {
    pub protocol: String,
    pub contract: String,
    pub bounty_id: String,
    pub creator: String,
    pub terms_hash: String,
    pub revision: u64,
    pub solver_reward: u128,
    pub review_reward: u128,
    pub submission_deadline: u64,
    pub review_window: u64,
    pub max_pending: u16,
    pub status: String,
    pub principal: u128,
    pub outstanding_bonds: u128,
    pub entries: std::collections::BTreeMap<u64, Entry>,
    pub winner: Option<u64>,
    pub settlement_transaction: Option<String>,
}

/// The caller supplies only logs from a pinned factory/clone, with receipts and
/// canonical block identities reconciled by the indexer. A log decoder alone is
/// never proof of canonicality. Rebuild from canonical logs after a reorg.
pub fn project(
    terms: &Terms,
    factory: Address,
    implementation: Address,
    salt: B256,
    created_at: u64,
    logs: &[super::EvmLog],
) -> Result<Projection, ChainBaseError> {
    use alloy::sol_types::SolEvent;
    let config = terms.config(created_at)?;
    let bounty = predict(factory, implementation, &config, salt);
    let mut p = Projection {
        protocol: PROTOCOL.into(),
        contract: format!("{bounty:#x}"),
        bounty_id: terms.bounty_id.clone(),
        creator: format!("{:#x}", config.creator),
        terms_hash: format!("{:#x}", config.termsHash),
        revision: 1,
        solver_reward: config.solverReward,
        review_reward: config.reviewReward,
        submission_deadline: config.submissionDeadline,
        review_window: config.reviewWindow,
        max_pending: config.maxPendingEntries,
        status: "unfunded".into(),
        principal: 0,
        outstanding_bonds: 0,
        entries: Default::default(),
        winner: None,
        settlement_transaction: None,
    };
    let mut ordered = logs.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|l| (l.block_number, l.log_index));
    let mut seen = std::collections::BTreeSet::new();
    let mut funded = None;
    let mut created = None;
    for log in ordered {
        if !seen.insert((log.block_number, log.log_index)) {
            return Err(invalid("duplicate canonical log position"));
        }
        let emitter = address(&log.address)?;
        let topics = log
            .topics
            .iter()
            .map(|s| s.parse::<B256>().map_err(|_| invalid("invalid log topic")))
            .collect::<Result<Vec<_>, _>>()?;
        let data = hex::decode(
            log.data
                .strip_prefix("0x")
                .ok_or_else(|| invalid("invalid event data"))?,
        )
        .map_err(|_| invalid("invalid event data"))?;
        let topic = *topics
            .first()
            .ok_or_else(|| invalid("missing event topic"))?;
        let observed = log
            .occurred_at
            .and_then(|time| u64::try_from(time.timestamp()).ok());
        // Ignore unrelated token-transfer logs, but never accept a recognized
        // event with the wrong emitter, bounty identity or exact ABI length.
        macro_rules! decode {
            ($event:ty,$expected:expr) => {{
                if observed.is_none_or(|time| time < created_at) {
                    return Err(invalid("missing verified event timestamp"));
                }
                if emitter != $expected {
                    return Err(invalid("wrong event emitter"));
                }
                let event = <$event>::decode_raw_log_validate(topics.clone(), &data)
                    .map_err(|_| invalid("malformed event"))?;
                if event.bountyId != config.bountyId
                    || event.encode_data() != data
                    || event.encode_log_data().topics() != topics.as_slice()
                {
                    return Err(invalid("wrong bounty or noncanonical event bytes"));
                }
                event
            }};
        }
        let amount = |v: U256| u128::try_from(v).map_err(|_| invalid("event amount overflow"));
        match topic {
            t if t == abi::CreatorOpenBountyCreated::SIGNATURE_HASH => {
                let e = decode!(abi::CreatorOpenBountyCreated, factory);
                if observed != Some(created_at)
                    || created.is_some()
                    || e.bounty != bounty
                    || e.creator != config.creator
                    || e.termsHash != config.termsHash
                    || e.configurationHash != keccak256(config.abi_encode())
                    || e.userSalt != salt
                {
                    return Err(invalid("creation commitment mismatch"));
                }
                created = Some(log.tx_hash.clone());
            }
            t if t == abi::OpenBountyFunded::SIGNATURE_HASH => {
                let e = decode!(abi::OpenBountyFunded, bounty);
                if observed != Some(created_at)
                    || funded.is_some()
                    || e.creator != config.creator
                    || e.termsHash != config.termsHash
                    || amount(e.principal)? != p.solver_reward + p.review_reward
                {
                    return Err(invalid("funding commitment mismatch"));
                }
                funded = Some(log.tx_hash.clone());
                p.principal = amount(e.principal)?;
                p.status = "open".into();
            }
            t if t == abi::OpenSubmissionAdded::SIGNATURE_HASH => {
                let e = decode!(abi::OpenSubmissionAdded, bounty);
                if p.status != "open"
                    || e.entry != p.entries.len() as u64 + 1
                    || e.solver.is_zero()
                    || e.solver == config.creator
                    || e.termsHash != hash(&p.terms_hash)?
                    || e.revision != p.revision
                    || amount(e.bond)? != p.review_reward
                    || e.submissionHash.is_zero()
                    || e.evidenceHash.is_zero()
                    || e.reviewDeadline < config.reviewWindow
                    || e.reviewDeadline - config.reviewWindow >= config.submissionDeadline
                    || observed != Some(e.reviewDeadline - config.reviewWindow)
                    || p.entries.values().filter(|v| v.status == "pending").count()
                        >= p.max_pending as usize
                    || p.entries
                        .values()
                        .any(|v| v.status == "pending" && v.solver == format!("{:#x}", e.solver))
                {
                    return Err(invalid("invalid submission transition"));
                }
                p.entries.insert(
                    e.entry,
                    Entry {
                        solver: format!("{:#x}", e.solver),
                        submission_hash: format!("{:#x}", e.submissionHash),
                        evidence_hash: format!("{:#x}", e.evidenceHash),
                        review_deadline: e.reviewDeadline,
                        status: "pending".into(),
                    },
                );
                p.outstanding_bonds += p.review_reward;
            }
            t if t == abi::OpenSubmissionRejected::SIGNATURE_HASH => {
                let e = decode!(abi::OpenSubmissionRejected, bounty);
                if p.entries
                    .get(&e.entry)
                    .is_none_or(|entry| observed.unwrap() > entry.review_deadline)
                    || p.status != "open"
                    || amount(e.reviewReward)? != p.review_reward
                    || e.responseHash.is_zero()
                {
                    return Err(invalid("invalid rejection"));
                }
                close(&mut p, e.entry, e.solver, "rejected")?;
            }
            t if t == abi::OpenBountySettled::SIGNATURE_HASH => {
                let e = decode!(abi::OpenBountySettled, bounty);
                let entry = p
                    .entries
                    .get(&e.entry)
                    .ok_or_else(|| invalid("unknown winning entry"))?;
                if observed.unwrap() > entry.review_deadline
                    || p.status != "open"
                    || p.winner.is_some()
                    || amount(e.solverReward)? != p.solver_reward
                    || amount(e.reviewReward)? != p.review_reward
                    || amount(e.bondReturned)? != p.review_reward
                    || e.submissionHash != hash(&entry.submission_hash)?
                    || e.evidenceHash != hash(&entry.evidence_hash)?
                    || e.termsHash != hash(&p.terms_hash)?
                    || e.responseHash.is_zero()
                {
                    return Err(invalid("invalid settlement"));
                }
                close(&mut p, e.entry, e.solver, "winner")?;
                p.winner = Some(e.entry);
                p.status = "settled".into();
                p.principal = 0;
                p.settlement_transaction = Some(log.tx_hash.clone());
            }
            t if t == abi::OpenBondRefunded::SIGNATURE_HASH => {
                let e = decode!(abi::OpenBondRefunded, bounty);
                if p.entries.get(&e.entry).is_none_or(|entry| {
                    p.status == "open" && observed.unwrap() <= entry.review_deadline
                }) || amount(e.amount)? != p.review_reward
                {
                    return Err(invalid("wrong bond refund"));
                }
                close(&mut p, e.entry, e.solver, "refunded")?;
            }
            t if t == abi::OpenRewardIncreased::SIGNATURE_HASH => {
                let e = decode!(abi::OpenRewardIncreased, bounty);
                let next = amount(e.newReward)?;
                let revised = terms.increased(
                    u64::try_from(next).map_err(|_| invalid("reward overflow"))?,
                    created_at,
                )?;
                if json_hash(&revised)? != e.newTermsHash
                    || observed.unwrap() >= p.submission_deadline
                    || p.status != "open"
                    || !p.entries.is_empty()
                    || e.revision != p.revision + 1
                    || e.oldTermsHash != hash(&p.terms_hash)?
                    || amount(e.oldReward)? != p.solver_reward
                    || next <= p.solver_reward
                    || e.newTermsHash.is_zero()
                    || e.newTermsHash == e.oldTermsHash
                {
                    return Err(invalid("invalid reward revision"));
                }
                p.principal += next - p.solver_reward;
                p.solver_reward = next;
                p.revision = e.revision;
                p.terms_hash = format!("{:#x}", e.newTermsHash);
            }
            t if t == abi::OpenBountyCancelled::SIGNATURE_HASH => {
                decode!(abi::OpenBountyCancelled, bounty);
                if p.status != "open"
                    || !p.entries.is_empty()
                        && observed.unwrap() <= p.submission_deadline + p.review_window
                {
                    return Err(invalid("invalid cancellation"));
                }
                p.status = "cancelled".into();
            }
            t if t == abi::OpenPrincipalRefunded::SIGNATURE_HASH => {
                let e = decode!(abi::OpenPrincipalRefunded, bounty);
                if p.status != "cancelled"
                    || p.principal == 0
                    || amount(e.amount)? != p.principal
                    || e.creator != config.creator
                {
                    return Err(invalid("invalid principal refund"));
                }
                p.principal = 0;
            }
            _ => {}
        }
    }
    if created.is_none() || funded != created {
        return Err(invalid("missing atomic canonical funding"));
    }
    Ok(p)
}
fn close(p: &mut Projection, id: u64, solver: Address, status: &str) -> Result<(), ChainBaseError> {
    let entry = p
        .entries
        .get_mut(&id)
        .ok_or_else(|| invalid("unknown entry"))?;
    if entry.status != "pending" || entry.solver != format!("{solver:#x}") {
        return Err(invalid("entry already closed or wrong solver"));
    }
    entry.status = status.into();
    p.outstanding_bonds = p
        .outstanding_bonds
        .checked_sub(p.review_reward)
        .ok_or_else(|| invalid("bond insolvency"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn terms() -> Terms {
        Terms {
            schema: PROTOCOL.into(),
            network: "base-sepolia".into(),
            bounty_id: format!("{:#x}", keccak256("bounty")),
            creator: format!("{:#x}", Address::repeat_byte(1)),
            title: "Deliver a model".into(),
            goal: "Publish the specified files".into(),
            acceptance_criteria: vec!["Every stated dimension matches".into()],
            solver_reward_micro_usdc: 20_300_000,
            review_reward_micro_usdc: 1_000_000,
            submission_deadline: 1_800_000_000 + 90 * 86400,
            review_window_seconds: 2 * 86400,
            max_pending_entries: 8,
            disclosure: DISCLOSURE.into(),
        }
    }
    #[test]
    fn long_work_window_has_short_independent_authorization_and_exact_principal() {
        let t = terms();
        let p = prepare_creation(
            &t,
            Address::repeat_byte(2),
            Address::repeat_byte(3),
            B256::ZERO,
            B256::repeat_byte(4),
            1_800_000_000,
        )
        .unwrap();
        assert_eq!(p.customer_principal_micro_usdc, "21300000");
        assert_eq!(p.customer_gas_wei, "0");
        assert_eq!(p.authorization["primaryType"], "ReceiveWithAuthorization");
        assert_eq!(p.authorization["domain"]["name"], "USDC");
        assert_eq!(
            p.authorization["message"]["validBefore"],
            json!(U256::from(1_800_003_600u64))
        );
        for field in [
            "goal",
            "creator",
            "title",
            "disclosure",
            "max_pending_entries",
            "review_window_seconds",
            "submission_deadline",
            "solver_reward_micro_usdc",
        ] {
            let mut changed = serde_json::to_value(&t).unwrap();
            changed[field] = match field {
                "goal" | "title" => json!("changed"),
                "creator" => json!(format!("{:#x}", Address::repeat_byte(4))),
                "disclosure" => json!("omitted risk"),
                "max_pending_entries" => json!(9),
                "review_window_seconds" => json!(86400),
                "submission_deadline" => json!(1_800_000_000u64 + 91 * 86400),
                _ => json!(30_300_000),
            };
            let changed: Terms = serde_json::from_value(changed).unwrap();
            if let Ok(other) = prepare_creation(
                &changed,
                Address::repeat_byte(2),
                Address::repeat_byte(3),
                B256::ZERO,
                B256::repeat_byte(4),
                1_800_000_000,
            ) {
                assert_ne!(p.bounty_contract, other.bounty_contract, "{field}");
                assert_ne!(
                    p.authorization_digest, other.authorization_digest,
                    "{field}"
                );
            }
        }
    }
    #[test]
    fn increasing_reward_preserves_every_other_term_and_invalidates_commitment() {
        let old = terms();
        let revised = old.increased(30_300_000, 1_800_000_000).unwrap();
        let mut expected = old.clone();
        expected.solver_reward_micro_usdc = 30_300_000;
        assert_eq!(revised, expected);
        assert_ne!(
            old.config(1_800_000_000).unwrap().termsHash,
            revised.config(1_800_000_000).unwrap().termsHash
        );
        assert!(old.increased(20_000_000, 1_800_000_000).is_err());
        let mut too_long = old;
        too_long.submission_deadline += 277 * 86400;
        assert!(too_long.validate(1_800_000_000).is_err());
    }
    #[test]
    fn rust_matches_executed_solidity_vectors_for_every_action() {
        let v: Value = serde_json::from_str(include_str!(
            "../../../contracts/base-escrow/fixtures/creator-open-vectors.json"
        ))
        .unwrap();
        let bytes =
            |name: &str| hex::decode(v[name].as_str().unwrap().trim_start_matches("0x")).unwrap();
        let config = abi::Config::abi_decode(&bytes("config")).unwrap();
        let s = abi::Submission::abi_decode(&bytes("submission")).unwrap();
        let a = abi::TokenAuthorization::abi_decode(&bytes("tokenAuthorization")).unwrap();
        let factory = v["factory"].as_str().unwrap().parse::<Address>().unwrap();
        let implementation = v["implementation"]
            .as_str()
            .unwrap()
            .parse::<Address>()
            .unwrap();
        let bounty = v["bounty"].as_str().unwrap().parse::<Address>().unwrap();
        assert_eq!(
            predict(factory, implementation, &config, B256::ZERO),
            bounty
        );
        let d = domain(84532, bounty);
        assert_eq!(format!("{:#x}", d.separator()), v["domain"]);
        let signed = abi::OpenSubmission {
            solver: s.solver,
            submissionHash: s.submissionHash,
            evidenceHash: s.evidenceHash,
            termsHash: s.termsHash,
            revision: s.revision,
            tokenNonce: a.nonce,
            deadline: s.deadline,
        };
        assert_eq!(
            format!("{:#x}", signed.eip712_signing_hash(&d)),
            v["submissionDigest"]
        );
        let response = keccak256("vector response");
        let next = keccak256("vector terms");
        let deadline = u64::try_from(s.deadline).unwrap();
        let review = abi::CreatorReview {
            entry: 1,
            solver: s.solver,
            submissionHash: s.submissionHash,
            evidenceHash: s.evidenceHash,
            termsHash: s.termsHash,
            revision: 1,
            passed: true,
            responseHash: response,
            deadline: s.deadline,
        };
        assert_eq!(
            format!("{:#x}", review.eip712_signing_hash(&d)),
            v["reviewDigest"]
        );
        let increase = abi::IncreaseReward {
            revision: 1,
            termsHash: s.termsHash,
            newTermsHash: next,
            newReward: U256::from(30_300_000),
            tokenNonce: a.nonce,
            deadline: s.deadline,
        };
        assert_eq!(
            format!("{:#x}", increase.eip712_signing_hash(&d)),
            v["increaseDigest"]
        );
        let cancel = abi::CancelBounty {
            revision: 1,
            termsHash: s.termsHash,
            deadline: s.deadline,
        };
        assert_eq!(
            format!("{:#x}", cancel.eip712_signing_hash(&d)),
            v["cancelDigest"]
        );
        let sig = Bytes::from(vec![0x11, 0x22, 0x33]);
        for (field, plan) in [
            (
                "create",
                plan_create(factory, config, B256::ZERO, a.clone()),
            ),
            ("submit", plan_submit(bounty, s, a.clone(), sig.clone())),
            (
                "review",
                plan_review(bounty, 1, true, response, deadline, sig.clone()),
            ),
            (
                "increase",
                plan_increase(bounty, 30_300_000, next, a, deadline, sig.clone()),
            ),
            ("cancel", plan_cancel(bounty, deadline, sig)),
            ("expire", plan_expire(bounty)),
            ("refundBond", plan_refund_bond(bounty, 1)),
            ("refundPrincipal", plan_refund_principal(bounty)),
        ] {
            assert_eq!(plan.data, v[field], "{field}");
            assert_eq!(plan.value_wei, 0);
            assert!(plan.from.is_none());
        }
    }
}

#[cfg(test)]
mod projection_tests {
    use super::*;
    use alloy::sol_types::SolEvent;
    use chrono::DateTime;
    fn log<E: SolEvent>(address: Address, event: E, index: u64, at: u64) -> super::super::EvmLog {
        let data = event.encode_log_data();
        super::super::EvmLog {
            address: format!("{address:#x}"),
            topics: data.topics().iter().map(|v| format!("{v:#x}")).collect(),
            data: format!("0x{}", hex::encode(&data.data)),
            tx_hash: format!(
                "{:#x}",
                B256::repeat_byte(if index < 2 { 1 } else { index as u8 })
            ),
            block_number: 100 + index,
            log_index: index,
            occurred_at: DateTime::from_timestamp(at as i64, 0),
        }
    }
    fn fixtures() -> (Terms, Address, Address, Address, Vec<super::super::EvmLog>) {
        let terms = super::tests::terms();
        let config = terms.config(1_800_000_000).unwrap();
        let factory = Address::repeat_byte(2);
        let implementation = Address::repeat_byte(3);
        let bounty = predict(factory, implementation, &config, B256::ZERO);
        let logs = vec![
            log(
                bounty,
                abi::OpenBountyFunded {
                    bountyId: config.bountyId,
                    creator: config.creator,
                    principal: U256::from(21_300_000),
                    termsHash: config.termsHash,
                },
                0,
                1_800_000_000,
            ),
            log(
                factory,
                abi::CreatorOpenBountyCreated {
                    bountyId: config.bountyId,
                    bounty,
                    creator: config.creator,
                    termsHash: config.termsHash,
                    configurationHash: keccak256(config.abi_encode()),
                    userSalt: B256::ZERO,
                },
                1,
                1_800_000_000,
            ),
        ];
        (terms, factory, implementation, bounty, logs)
    }
    #[test]
    fn projection_only_pays_from_exact_versioned_canonical_events_and_rejects_double_settlement() {
        let (t, f, i, b, mut logs) = fixtures();
        let c = t.config(1_800_000_000).unwrap();
        let solver = Address::repeat_byte(5);
        let submit = abi::OpenSubmissionAdded {
            bountyId: c.bountyId,
            entry: 1,
            solver,
            submissionHash: keccak256("result"),
            evidenceHash: keccak256("proof"),
            termsHash: c.termsHash,
            revision: 1,
            reviewDeadline: 1_800_000_001 + 2 * 86400,
            bond: U256::from(1_000_000),
        };
        logs.push(log(b, submit, 2, 1_800_000_001));
        let pending = project(&t, f, i, B256::ZERO, 1_800_000_000, &logs).unwrap();
        assert!(pending.settlement_transaction.is_none());
        assert_eq!(pending.outstanding_bonds, 1_000_000);
        let settle = abi::OpenBountySettled {
            bountyId: c.bountyId,
            entry: 1,
            solver,
            solverReward: U256::from(20_300_000),
            bondReturned: U256::from(1_000_000),
            reviewReward: U256::from(1_000_000),
            submissionHash: keccak256("result"),
            evidenceHash: keccak256("proof"),
            termsHash: c.termsHash,
            responseHash: keccak256("approved"),
        };
        logs.push(log(b, settle.clone(), 3, 1_800_000_002));
        let paid = project(&t, f, i, B256::ZERO, 1_800_000_000, &logs).unwrap();
        assert_eq!(paid.winner, Some(1));
        assert_eq!(paid.principal + paid.outstanding_bonds, 0);
        for mutation in ["emitter", "hash", "amount", "late", "duplicate"] {
            let mut bad = logs.clone();
            match mutation {
                "emitter" => bad[3].address = format!("{:#x}", Address::repeat_byte(9)),
                "hash" => {
                    let mut altered = settle.clone();
                    altered.submissionHash = keccak256("other");
                    bad[3] = log(b, altered, 3, 1_800_000_002);
                }
                "amount" => {
                    let mut altered = settle.clone();
                    altered.solverReward = U256::from(1);
                    bad[3] = log(b, altered, 3, 1_800_000_002);
                }
                "late" => {
                    bad[3].occurred_at = DateTime::from_timestamp(1_800_000_002 + 3 * 86400, 0)
                }
                _ => bad.push(log(b, settle.clone(), 4, 1_800_000_003)),
            }
            assert!(
                project(&t, f, i, B256::ZERO, 1_800_000_000, &bad).is_err(),
                "{mutation}"
            );
        }
        // Reorg replacement is a complete rebuild, so removing the settlement
        // removes the paid assertion rather than preserving stale payout state.
        logs.pop();
        assert!(project(&t, f, i, B256::ZERO, 1_800_000_000, &logs)
            .unwrap()
            .settlement_transaction
            .is_none());
    }
    #[test]
    fn unproven_funding_and_changed_terms_are_never_open() {
        let (t, f, i, _, logs) = fixtures();
        assert!(project(&t, f, i, B256::ZERO, 1_800_000_000, &logs[1..]).is_err());
        let mut altered = t;
        altered.goal = "a different promise".into();
        assert!(project(&altered, f, i, B256::ZERO, 1_800_000_000, &logs).is_err());
    }
}
