//! Exact, versioned requests for the creator-open candidate. No caller-supplied calldata.
use crate::{creator_open as protocol, *};
use alloy::{
    primitives::{keccak256, Address, Bytes, Signature, B256, U256},
    sol_types::{SolCall, SolStruct},
};
use protocol::{abi, CreatorOpenTerms, Terms};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    pub protocol: String,
    pub network: String,
    pub factory: Address,
    pub implementation: Address,
    pub factory_code_hash: B256,
    pub implementation_code_hash: B256,
    pub deployment_block: u64,
}
fn bad(message: &str) -> ChainBaseError {
    ChainBaseError::InvalidTermsDocument(format!("creator-open: {message}"))
}
impl Release {
    pub fn validate(&self) -> Result<(), ChainBaseError> {
        if self.protocol != protocol::PROTOCOL
            || !["base-mainnet", "base-sepolia"].contains(&self.network.as_str())
            || self.factory.is_zero()
            || self.implementation.is_zero()
            || self.factory == self.implementation
            || self.factory_code_hash.is_zero()
            || self.implementation_code_hash.is_zero()
            || self.deployment_block == 0
        {
            return Err(bad("incomplete release manifest"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CreatorOpenAction {
    Create {
        #[schema(value_type = String)]
        token_nonce: B256,
    },
    Submit {
        #[schema(value_type = String)]
        solver: Address,
        #[schema(value_type = String)]
        submission_hash: B256,
        #[schema(value_type = String)]
        evidence_hash: B256,
        #[schema(value_type = String)]
        token_nonce: B256,
    },
    Review {
        entry: u64,
        #[schema(value_type = String)]
        solver: Address,
        #[schema(value_type = String)]
        submission_hash: B256,
        #[schema(value_type = String)]
        evidence_hash: B256,
        passed: bool,
        #[schema(value_type = String)]
        response_hash: B256,
    },
    Increase {
        new_reward_micro_usdc: u64,
        #[schema(value_type = String)]
        token_nonce: B256,
    },
    Cancel,
    Expire,
    RefundBond {
        entry: u64,
    },
    RefundPrincipal,
}
/// Public evidence is stored only after verifying the solver's signed hash.
/// The service never fetches the supplied URL.
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreatorOpenPublicEvidence {
    pub artifact_url: String,
    pub artifact_sha256: String,
    pub notes: String,
}
impl CreatorOpenPublicEvidence {
    pub fn commitment(&self) -> Result<B256, ChainBaseError> {
        let url =
            reqwest::Url::parse(&self.artifact_url).map_err(|_| bad("invalid evidence URL"))?;
        if self.artifact_url.len() > 2048
            || url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || self.notes.len() > 8000
            || self.artifact_sha256.len() != 71
            || !self.artifact_sha256.starts_with("sha256:")
            || !self.artifact_sha256[7..]
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(bad("invalid public evidence"));
        }
        Ok(keccak256(canonical_json_bytes(
            &serde_json::to_value(self).map_err(|_| bad("invalid evidence JSON"))?,
        )?))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreatorOpenRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_evidence: Option<CreatorOpenPublicEvidence>,
    /// Original immutable terms determine the address even after an increase.
    pub original_terms: CreatorOpenTerms,
    #[schema(value_type = String)]
    pub user_salt: B256,
    pub expected_revision: u64,
    pub expected_solver_reward_micro_usdc: u64,
    pub valid_before: u64,
    pub action: CreatorOpenAction,
}
#[derive(Clone, Default, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreatorOpenSignatures {
    #[serde(default)]
    #[schema(value_type = String)]
    pub token: Bytes,
    #[serde(default)]
    #[schema(value_type = String)]
    pub action: Bytes,
}
pub type Action = CreatorOpenAction;
pub type Request = CreatorOpenRequest;
pub type Signatures = CreatorOpenSignatures;

#[derive(Clone, Serialize)]
pub struct SignatureRequirement {
    pub signer: Address,
    pub digest: B256,
    pub typed_data: Value,
}
#[derive(Serialize)]
pub struct Prepared {
    pub protocol: &'static str,
    pub network: String,
    pub bounty_contract: Address,
    pub approval_hash: B256,
    pub request: Request,
    pub token_authorization: Option<SignatureRequirement>,
    pub action_authorization: Option<SignatureRequirement>,
    pub customer_principal_micro_usdc: String,
    pub customer_gas_wei: &'static str,
    pub evidence_boundary: &'static str,
    #[serde(skip)]
    pub transaction: EvmTransactionIntent,
    #[serde(skip)]
    pub quota_wallet: Address,
    #[serde(skip)]
    pub current_terms: Terms,
    #[serde(skip)]
    pub next_terms: Option<Terms>,
}
fn requirement<S: SolStruct + Serialize>(
    signer: Address,
    message: &S,
    domain: alloy::sol_types::Eip712Domain,
) -> SignatureRequirement {
    SignatureRequirement {
        signer,
        digest: message.eip712_signing_hash(&domain),
        typed_data: protocol::typed_data(message, domain),
    }
}
pub fn prepare(
    release: &Release,
    request: &Request,
    signatures: &Signatures,
) -> Result<Prepared, ChainBaseError> {
    release.validate()?;
    let r = request;
    if r.original_terms.network != release.network
        || r.expected_revision == 0
        || r.expected_revision == u64::MAX
        || r.valid_before == 0
        || signatures.token.len() > 4096
        || signatures.action.len() > 4096
    {
        return Err(bad("invalid request boundary"));
    }
    if let Some(evidence) = &r.public_evidence {
        if !matches!(r.action,Action::Submit { evidence_hash,.. } if evidence_hash==evidence.commitment()?)
        {
            return Err(bad(
                "public evidence must match the exact signed submission hash",
            ));
        }
    }
    // Validate historical terms without reinterpreting their window as a fresh creation.
    let historical_time = r
        .original_terms
        .submission_deadline
        .checked_sub(1)
        .ok_or_else(|| bad("missing deadline"))?;
    let original = r.original_terms.config(historical_time)?;
    let current_terms =
        if r.expected_solver_reward_micro_usdc == r.original_terms.solver_reward_micro_usdc {
            r.original_terms.clone()
        } else {
            r.original_terms
                .increased(r.expected_solver_reward_micro_usdc, historical_time)?
        };
    let config = current_terms.config(historical_time)?;
    let bounty = protocol::predict(
        release.factory,
        release.implementation,
        &original,
        r.user_salt,
    );
    let network = base_network_descriptor(&release.network)?;
    let token = network
        .native_usdc_token_address
        .parse::<Address>()
        .map_err(|_| bad("invalid token"))?;
    let domain = protocol::domain(network.chain_id, bounty);
    let token_domain = protocol::token_domain(network.chain_id, token);
    let mut token_requirement = None;
    let mut action_requirement = None;
    let mut next_terms = None;
    let mut principal = 0u128;
    let mut wallet = config.creator;
    let mut authorization = |from: Address,
                             value: u128,
                             nonce: B256|
     -> Result<abi::TokenAuthorization, ChainBaseError> {
        if nonce.is_zero() {
            return Err(bad("missing token nonce"));
        }
        let message = abi::ReceiveWithAuthorization {
            from,
            to: bounty,
            value: U256::from(value),
            validAfter: U256::ZERO,
            validBefore: U256::from(r.valid_before),
            nonce,
        };
        token_requirement = Some(requirement(from, &message, token_domain.clone()));
        principal = value;
        Ok(abi::TokenAuthorization {
            validAfter: U256::ZERO,
            validBefore: U256::from(r.valid_before),
            nonce,
            signature: signatures.token.clone(),
        })
    };
    let transaction = match r.action {
        Action::Create { token_nonce } => {
            if r.expected_revision != 1
                || r.expected_solver_reward_micro_usdc != r.original_terms.solver_reward_micro_usdc
            {
                return Err(bad("creation changed terms"));
            }
            let auth = authorization(
                config.creator,
                config.solverReward + config.reviewReward,
                token_nonce,
            )?;
            protocol::plan_create(release.factory, original, r.user_salt, auth)
        }
        Action::Submit {
            solver,
            submission_hash,
            evidence_hash,
            token_nonce,
        } => {
            if solver.is_zero()
                || solver == config.creator
                || submission_hash.is_zero()
                || evidence_hash.is_zero()
            {
                return Err(bad("invalid submission commitments"));
            }
            wallet = solver;
            let auth = authorization(solver, config.reviewReward, token_nonce)?;
            let message = abi::OpenSubmission {
                solver,
                submissionHash: submission_hash,
                evidenceHash: evidence_hash,
                termsHash: config.termsHash,
                revision: r.expected_revision,
                tokenNonce: token_nonce,
                deadline: U256::from(r.valid_before),
            };
            action_requirement = Some(requirement(solver, &message, domain));
            protocol::plan_submit(
                bounty,
                abi::Submission {
                    solver,
                    submissionHash: submission_hash,
                    evidenceHash: evidence_hash,
                    termsHash: config.termsHash,
                    revision: r.expected_revision,
                    deadline: U256::from(r.valid_before),
                },
                auth,
                signatures.action.clone(),
            )
        }
        Action::Review {
            entry,
            solver,
            submission_hash,
            evidence_hash,
            passed,
            response_hash,
        } => {
            if entry == 0
                || solver.is_zero()
                || submission_hash.is_zero()
                || evidence_hash.is_zero()
                || response_hash.is_zero()
            {
                return Err(bad("invalid review commitments"));
            }
            let message = abi::CreatorReview {
                entry,
                solver,
                submissionHash: submission_hash,
                evidenceHash: evidence_hash,
                termsHash: config.termsHash,
                revision: r.expected_revision,
                passed,
                responseHash: response_hash,
                deadline: U256::from(r.valid_before),
            };
            action_requirement = Some(requirement(config.creator, &message, domain));
            protocol::plan_review(
                bounty,
                entry,
                passed,
                response_hash,
                r.valid_before,
                signatures.action.clone(),
            )
        }
        Action::Increase {
            new_reward_micro_usdc,
            token_nonce,
        } => {
            let new_terms = current_terms.increased(new_reward_micro_usdc, historical_time)?;
            let new_config = new_terms.config(historical_time)?;
            let auth = authorization(
                config.creator,
                new_config.solverReward - config.solverReward,
                token_nonce,
            )?;
            let message = abi::IncreaseReward {
                revision: r.expected_revision,
                termsHash: config.termsHash,
                newTermsHash: new_config.termsHash,
                newReward: U256::from(new_config.solverReward),
                tokenNonce: token_nonce,
                deadline: U256::from(r.valid_before),
            };
            action_requirement = Some(requirement(config.creator, &message, domain));
            next_terms = Some(new_terms);
            protocol::plan_increase(
                bounty,
                new_config.solverReward,
                new_config.termsHash,
                auth,
                r.valid_before,
                signatures.action.clone(),
            )
        }
        Action::Cancel => {
            let message = abi::CancelBounty {
                revision: r.expected_revision,
                termsHash: config.termsHash,
                deadline: U256::from(r.valid_before),
            };
            action_requirement = Some(requirement(config.creator, &message, domain));
            protocol::plan_cancel(bounty, r.valid_before, signatures.action.clone())
        }
        Action::Expire => protocol::plan_expire(bounty),
        Action::RefundBond { entry } => {
            if entry == 0 {
                return Err(bad("missing entry"));
            }
            protocol::plan_refund_bond(bounty, entry)
        }
        Action::RefundPrincipal => protocol::plan_refund_principal(bounty),
    };
    if token_requirement.is_none() && !signatures.token.is_empty()
        || action_requirement.is_none() && !signatures.action.is_empty()
    {
        return Err(bad("unexpected signature"));
    }
    // Hash exact semantic authority, not alternative ECDSA/1271 signature encodings.
    let approval_hash = keccak256(canonical_json_bytes(
        &json!({"release":release,"request":r}),
    )?);
    Ok(Prepared{protocol:protocol::PROTOCOL,network:release.network.clone(),bounty_contract:bounty,approval_hash,request:r.clone(),token_authorization:token_requirement,action_authorization:action_requirement,customer_principal_micro_usdc:principal.to_string(),customer_gas_wei:"0",evidence_boundary:"Preparation and transaction hashes are not funding or payment. The platform pays gas only within its shared daily allowance; it never switches to customer-paid gas.",transaction,quota_wallet:wallet,current_terms,next_terms})
}

alloy::sol! {
    interface Read {
        function settlementToken() external view returns(address);
        function balanceOf(address wallet) external view returns(uint256);
        function implementation() external view returns(address);
        function factory() external view returns(address);
        function isCanonicalBounty(address bounty) external view returns(bool);
        function bountyId() external view returns(bytes32);
        function creator() external view returns(address);
        function termsHash() external view returns(bytes32);
        function revision() external view returns(uint64);
        function solverReward() external view returns(uint256);
        function reviewReward() external view returns(uint256);
        function status() external view returns(uint8);
        function entryCount() external view returns(uint64);
        function pendingEntries() external view returns(uint16);
        function activeEntry(address solver) external view returns(uint64);
        function refundablePrincipal() external view returns(uint256);
        function outstandingBonds() external view returns(uint256);
        function winner() external view returns(uint64);
        function entries(uint64 id) external view returns(address solver,bytes32 submissionHash,bytes32 evidenceHash,uint64 reviewDeadline,uint8 status);
        function isValidSignature(bytes32 digest,bytes signature) external view returns(bytes4);
    }
}
pub async fn principal_balance<T: JsonRpcTransport + ?Sized>(
    transport: &T,
    url: &str,
    prepared: &Prepared,
    block: &BaseBlockIdentity,
) -> Result<Option<U256>, ChainBaseError> {
    let Some(authority) = &prepared.token_authorization else {
        return Ok(None);
    };
    let token = base_network_descriptor(&prepared.network)?
        .native_usdc_token_address
        .parse::<Address>()
        .map_err(|_| bad("invalid token"))?;
    call(
        transport,
        url,
        token,
        Read::balanceOfCall {
            wallet: authority.signer,
        },
        block,
    )
    .await
    .map(Some)
}
/// All calls use an EIP-1898 block hash with requireCanonical, preventing mixed-fork reads.
pub async fn rpc<T: JsonRpcTransport + ?Sized>(
    transport: &T,
    url: &str,
    method: &str,
    params: Value,
) -> Result<Value, ChainBaseError> {
    rpc_result(
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            transport.post_json_value(
                url,
                &json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}),
            ),
        )
        .await
        .map_err(|_| ChainBaseError::RpcTransport("creator-open RPC check timed out".into()))??,
        1,
        method,
    )
}
fn pinned(block: &BaseBlockIdentity) -> Value {
    json!({"blockHash":block.hash,"requireCanonical":true})
}
pub(crate) async fn code<T: JsonRpcTransport + ?Sized>(
    t: &T,
    url: &str,
    address: Address,
    block: &BaseBlockIdentity,
) -> Result<Bytes, ChainBaseError> {
    let v = rpc(t, url, "eth_getCode", json!([address, pinned(block)])).await?;
    v.as_str()
        .ok_or_else(|| bad("missing code"))?
        .parse()
        .map_err(|_| bad("invalid code"))
}
pub(crate) async fn call<T: JsonRpcTransport + ?Sized, C: SolCall>(
    t: &T,
    url: &str,
    address: Address,
    call: C,
    block: &BaseBlockIdentity,
) -> Result<C::Return, ChainBaseError> {
    let v=rpc(t,url,"eth_call",json!([{"to":address,"data":format!("0x{}",hex::encode(call.abi_encode())),"gas":if C::SIGNATURE=="isValidSignature(bytes32,bytes)" {"0x30d40"} else {"0x1e8480"}},pinned(block)])).await?;
    let data: Bytes = v
        .as_str()
        .ok_or_else(|| bad("missing call result"))?
        .parse()
        .map_err(|_| bad("invalid call result"))?;
    C::abi_decode_returns_validate(&data).map_err(|_| bad("malformed state result"))
}
pub async fn verify_release<T: JsonRpcTransport + ?Sized>(
    t: &T,
    url: &str,
    release: &Release,
    block: &BaseBlockIdentity,
    now: u64,
) -> Result<(), ChainBaseError> {
    release.validate()?;
    if block.number < release.deployment_block
        || block.timestamp > now.saturating_add(30)
        || now.saturating_sub(block.timestamp) > 180
    {
        return Err(bad("release state is not fresh"));
    }
    let network = base_network_descriptor(&release.network)?;
    if rpc(t, url, "eth_chainId", json!([])).await? != json!(format!("0x{:x}", network.chain_id)) {
        return Err(bad("RPC network mismatch"));
    }
    let (factory, implementation) = tokio::try_join!(
        code(t, url, release.factory, block),
        code(t, url, release.implementation, block)
    )?;
    if keccak256(factory) != release.factory_code_hash
        || keccak256(implementation) != release.implementation_code_hash
    {
        return Err(bad("unreviewed release bytecode"));
    }
    let (token, implementation, factory) = tokio::try_join!(
        call(t, url, release.factory, Read::settlementTokenCall {}, block),
        call(t, url, release.factory, Read::implementationCall {}, block),
        call(t, url, release.implementation, Read::factoryCall {}, block)
    )?;
    if token
        != network
            .native_usdc_token_address
            .parse::<Address>()
            .map_err(|_| bad("invalid token"))?
        || implementation != release.implementation
        || factory != release.factory
    {
        return Err(bad("release immutable mismatch"));
    }
    Ok(())
}
pub async fn verify_fresh_state<T: JsonRpcTransport + ?Sized>(
    t: &T,
    url: &str,
    release: &Release,
    p: &Prepared,
    block: &BaseBlockIdentity,
    now: u64,
) -> Result<(), ChainBaseError> {
    if p.request.valid_before <= now.saturating_add(30)
        || p.request.valid_before > now.saturating_add(7200)
    {
        return Err(bad("authorization must expire within two hours"));
    }
    verify_release(t, url, release, block, now).await?;
    let canonical = call(
        t,
        url,
        release.factory,
        Read::isCanonicalBountyCall {
            bounty: p.bounty_contract,
        },
        block,
    )
    .await?;
    if matches!(p.request.action, Action::Create { .. }) {
        p.request.original_terms.validate(now)?;
        if canonical || !code(t, url, p.bounty_contract, block).await?.is_empty() {
            return Err(bad(
                "creation already exists; reconcile original transaction",
            ));
        }
        return Ok(());
    }
    if !canonical {
        return Err(bad("unrecognized bounty"));
    }
    let actual_code = code(t, url, p.bounty_contract, block).await?;
    if actual_code.as_ref() != &protocol::clone_init_code(release.implementation)[10..] {
        return Err(bad("wrong clone bytecode"));
    }
    let (id, creator, terms, revision, reward, review) = tokio::try_join!(
        call(t, url, p.bounty_contract, Read::bountyIdCall {}, block),
        call(t, url, p.bounty_contract, Read::creatorCall {}, block),
        call(t, url, p.bounty_contract, Read::termsHashCall {}, block),
        call(t, url, p.bounty_contract, Read::revisionCall {}, block),
        call(t, url, p.bounty_contract, Read::solverRewardCall {}, block),
        call(t, url, p.bounty_contract, Read::reviewRewardCall {}, block)
    )?;
    let expected = p
        .current_terms
        .config(p.current_terms.submission_deadline - 1)?;
    if id != expected.bountyId
        || creator != expected.creator
        || terms != expected.termsHash
        || revision != p.request.expected_revision
        || reward != U256::from(expected.solverReward)
        || review != U256::from(expected.reviewReward)
    {
        return Err(bad("stale or mismatched terms; prepare again"));
    }
    let (status, count) = tokio::try_join!(
        call(t, url, p.bounty_contract, Read::statusCall {}, block),
        call(t, url, p.bounty_contract, Read::entryCountCall {}, block)
    )?;
    match p.request.action {
        Action::Submit { solver, .. } => {
            let (active, pending) = tokio::try_join!(
                call(
                    t,
                    url,
                    p.bounty_contract,
                    Read::activeEntryCall { solver },
                    block
                ),
                call(
                    t,
                    url,
                    p.bounty_contract,
                    Read::pendingEntriesCall {},
                    block
                )
            )?;
            if status != 1
                || now >= expected.submissionDeadline
                || active != 0
                || pending >= expected.maxPendingEntries
            {
                return Err(bad("submission is closed or a review is already pending"));
            }
        }
        Action::Review { .. } if status != 1 => return Err(bad("review is closed")),
        Action::Increase { .. }
            if status != 1 || count != 0 || now >= expected.submissionDeadline =>
        {
            return Err(bad("reward cannot change after work starts or closes"))
        }
        Action::Cancel if status != 1 || count != 0 => {
            return Err(bad("cancellation requires no submissions"))
        }
        Action::Expire
            if status != 1
                || now
                    <= expected
                        .submissionDeadline
                        .saturating_add(expected.reviewWindow) =>
        {
            return Err(bad("bounty has not expired"))
        }
        Action::RefundBond { entry } => {
            let e = call(
                t,
                url,
                p.bounty_contract,
                Read::entriesCall { id: entry },
                block,
            )
            .await?;
            if e.status != 1 || (status == 1 && now <= e.reviewDeadline) {
                return Err(bad("bond is not refundable yet"));
            }
        }
        Action::RefundPrincipal
            if status != 3
                || call(
                    t,
                    url,
                    p.bounty_contract,
                    Read::refundablePrincipalCall {},
                    block,
                )
                .await?
                    == U256::ZERO =>
        {
            return Err(bad("principal is not refundable"));
        }
        _ => {}
    }
    if let Action::Review {
        entry,
        solver,
        submission_hash,
        evidence_hash,
        ..
    } = p.request.action
    {
        let actual = call(
            t,
            url,
            p.bounty_contract,
            Read::entriesCall { id: entry },
            block,
        )
        .await?;
        if actual.status != 1
            || actual.solver != solver
            || actual.submissionHash != submission_hash
            || actual.evidenceHash != evidence_hash
            || U256::from(actual.reviewDeadline) < U256::from(p.request.valid_before)
        {
            return Err(bad("stale review entry"));
        }
    }
    Ok(())
}
pub async fn verify_signatures<T: JsonRpcTransport + ?Sized>(
    t: &T,
    url: &str,
    p: &Prepared,
    signatures: &Signatures,
    block: &BaseBlockIdentity,
) -> Result<(), ChainBaseError> {
    for (requirement, signature) in [
        (&p.token_authorization, &signatures.token),
        (&p.action_authorization, &signatures.action),
    ] {
        let Some(required) = requirement else {
            continue;
        };
        verify_wallet_signature(t, url, required.signer, required.digest, signature, block).await?;
    }
    Ok(())
}
pub(crate) async fn verify_wallet_signature<T: JsonRpcTransport + ?Sized>(
    t: &T,
    url: &str,
    signer: Address,
    digest: B256,
    signature: &Bytes,
    block: &BaseBlockIdentity,
) -> Result<(), ChainBaseError> {
    if signature.len() > 4096 {
        return Err(bad("signature too large"));
    }
    if signature.is_empty() {
        return Err(bad("missing signature"));
    }
    if code(t, url, signer, block).await?.is_empty() {
        let signature =
            Signature::try_from(signature.as_ref()).map_err(|_| bad("invalid signature"))?;
        if signature.normalize_s().is_some()
            || signature
                .recover_address_from_prehash(&digest)
                .map_err(|_| bad("invalid signature"))?
                != signer
        {
            return Err(bad("wrong signer"));
        }
    } else {
        let magic = call(
            t,
            url,
            signer,
            Read::isValidSignatureCall {
                digest,
                signature: signature.clone(),
            },
            block,
        )
        .await?;
        if magic.as_slice() != [0x16, 0x26, 0xba, 0x7e] {
            return Err(bad("contract wallet rejected signature"));
        }
    }
    Ok(())
}
/// Simulate the exact signed call before consuming a durable attempt or gas quota.
pub async fn simulate<T: JsonRpcTransport + ?Sized>(
    t: &T,
    url: &str,
    p: &Prepared,
    from: &str,
    block: &BaseBlockIdentity,
) -> Result<(), ChainBaseError> {
    let v=rpc(t,url,"eth_call",json!([{"from":from,"to":p.transaction.to,"data":p.transaction.data,"value":"0x0","gas":"0x16e360"},pinned(block)])).await?;
    let _: Bytes = v
        .as_str()
        .ok_or_else(|| bad("missing simulation result"))?
        .parse()
        .map_err(|_| bad("invalid simulation result"))?;
    Ok(())
}

/// Match the exact protocol event in a receipt whose canonical block and success
/// have already been verified. Ordinary transfer logs never prove settlement.
pub fn receipt_outcome(
    release: &Release,
    p: &Prepared,
    receipt: &RpcTransactionReceipt,
    block: &BaseBlockIdentity,
) -> Result<Option<Value>, ChainBaseError> {
    use alloy::sol_types::SolEvent;
    if receipt.block_number()? != Some(block.number)
        || receipt.block_hash.as_deref() != Some(block.hash.as_str())
        || receipt.succeeded()? != Some(true)
    {
        return Err(bad("receipt is not canonical and successful"));
    }
    let mut logs = receipt.logs_to_evm_logs()?;
    for log in &mut logs {
        if log.block_number != block.number || log.tx_hash != receipt.transaction_hash {
            return Err(bad("inconsistent receipt log"));
        }
        log.occurred_at = chrono::DateTime::from_timestamp(
            i64::try_from(block.timestamp).map_err(|_| bad("bad block timestamp"))?,
            0,
        );
    }
    if matches!(p.request.action, Action::Create { .. }) {
        let state = protocol::project(
            &p.request.original_terms,
            release.factory,
            release.implementation,
            p.request.user_salt,
            block.timestamp,
            &logs,
        )?;
        return Ok((state.status=="open").then(||json!({"event":"CreatorOpenBountyCreated","funded":true,"solver_paid":false,"projection":state,"transaction_hash":receipt.transaction_hash,"block":block})));
    }
    let c = p
        .current_terms
        .config(p.current_terms.submission_deadline - 1)?;
    for log in logs.iter().filter(|l| {
        l.address
            .eq_ignore_ascii_case(&format!("{:#x}", p.bounty_contract))
    }) {
        let topics = log
            .topics
            .iter()
            .map(|v| v.parse::<B256>().map_err(|_| bad("invalid topic")))
            .collect::<Result<Vec<_>, _>>()?;
        let data = log
            .data
            .parse::<Bytes>()
            .map_err(|_| bad("invalid event data"))?;
        macro_rules! event {
            ($ty:ty) => {{
                if topics.first() != Some(&<$ty>::SIGNATURE_HASH) {
                    continue;
                }
                let e = <$ty>::decode_raw_log_validate(topics.clone(), &data)
                    .map_err(|_| bad("invalid protocol event"))?;
                if e.bountyId != c.bountyId
                    || e.encode_data() != data.as_ref()
                    || e.encode_log_data().topics() != topics
                {
                    return Err(bad("noncanonical protocol event"));
                }
                e
            }};
        }
        let (matches, name, paid) = match p.request.action {
            Action::Submit {
                solver,
                submission_hash,
                evidence_hash,
                ..
            } => {
                let e = event!(abi::OpenSubmissionAdded);
                (
                    e.solver == solver
                        && e.submissionHash == submission_hash
                        && e.evidenceHash == evidence_hash
                        && e.termsHash == c.termsHash
                        && e.revision == p.request.expected_revision
                        && e.bond == U256::from(c.reviewReward)
                        && e.reviewDeadline == block.timestamp.saturating_add(c.reviewWindow),
                    "OpenSubmissionAdded",
                    false,
                )
            }
            Action::Review {
                entry,
                solver,
                submission_hash,
                evidence_hash,
                passed: true,
                response_hash,
            } => {
                let e = event!(abi::OpenBountySettled);
                (
                    e.entry == entry
                        && e.solver == solver
                        && e.submissionHash == submission_hash
                        && e.evidenceHash == evidence_hash
                        && e.responseHash == response_hash
                        && e.termsHash == c.termsHash
                        && e.solverReward == U256::from(c.solverReward)
                        && e.bondReturned == U256::from(c.reviewReward)
                        && e.reviewReward == U256::from(c.reviewReward),
                    "OpenBountySettled",
                    true,
                )
            }
            Action::Review {
                entry,
                solver,
                passed: false,
                response_hash,
                ..
            } => {
                let e = event!(abi::OpenSubmissionRejected);
                (
                    e.entry == entry
                        && e.solver == solver
                        && e.responseHash == response_hash
                        && e.reviewReward == U256::from(c.reviewReward),
                    "OpenSubmissionRejected",
                    false,
                )
            }
            Action::Increase {
                new_reward_micro_usdc,
                ..
            } => {
                let e = event!(abi::OpenRewardIncreased);
                let n = p
                    .next_terms
                    .as_ref()
                    .ok_or_else(|| bad("missing increased terms"))?
                    .config(c.submissionDeadline - 1)?;
                (
                    e.revision == p.request.expected_revision + 1
                        && e.oldTermsHash == c.termsHash
                        && e.newTermsHash == n.termsHash
                        && e.oldReward == U256::from(c.solverReward)
                        && e.newReward == U256::from(new_reward_micro_usdc),
                    "OpenRewardIncreased",
                    false,
                )
            }
            Action::Cancel | Action::Expire => {
                let _e = event!(abi::OpenBountyCancelled);
                (true, "OpenBountyCancelled", false)
            }
            Action::RefundBond { entry } => {
                let e = event!(abi::OpenBondRefunded);
                (
                    e.entry == entry && e.amount == U256::from(c.reviewReward),
                    "OpenBondRefunded",
                    false,
                )
            }
            Action::RefundPrincipal => {
                let e = event!(abi::OpenPrincipalRefunded);
                (
                    e.creator == c.creator
                        && e.amount == U256::from(c.solverReward + c.reviewReward),
                    "OpenPrincipalRefunded",
                    false,
                )
            }
            Action::Create { .. } => unreachable!(),
        };
        if matches {
            return Ok(Some(
                json!({"event":name,"funded":false,"solver_paid":paid,"transaction_hash":receipt.transaction_hash,"block":block,"log_index":log.log_index}),
            ));
        }
    }
    Ok(None)
}

/// Cross-check the event projection against state at that exact canonical block.
/// Incomplete eth_getLogs responses must never advertise a stale open bounty.
pub async fn verify_projection<T: JsonRpcTransport + ?Sized>(
    t: &T,
    url: &str,
    p: &protocol::Projection,
    block: &BaseBlockIdentity,
) -> Result<(), ChainBaseError> {
    let address = p
        .contract
        .parse()
        .map_err(|_| bad("invalid projection address"))?;
    let (state, revision, reward, review, terms, count, pending, winner, principal, bonds) = tokio::try_join!(
        call(t, url, address, Read::statusCall {}, block),
        call(t, url, address, Read::revisionCall {}, block),
        call(t, url, address, Read::solverRewardCall {}, block),
        call(t, url, address, Read::reviewRewardCall {}, block),
        call(t, url, address, Read::termsHashCall {}, block),
        call(t, url, address, Read::entryCountCall {}, block),
        call(t, url, address, Read::pendingEntriesCall {}, block),
        call(t, url, address, Read::winnerCall {}, block),
        call(t, url, address, Read::refundablePrincipalCall {}, block),
        call(t, url, address, Read::outstandingBondsCall {}, block)
    )?;
    let expected_state = match p.status.as_str() {
        "open" => 1,
        "settled" => 2,
        "cancelled" => 3,
        _ => 0,
    };
    if state != expected_state
        || revision != p.revision
        || reward != U256::from(p.solver_reward)
        || review != U256::from(p.review_reward)
        || format!("{terms:#x}") != p.terms_hash
        || count != p.entries.len() as u64
        || pending as usize != p.entries.values().filter(|e| e.status == "pending").count()
        || winner != p.winner.unwrap_or(0)
        || principal != U256::from(p.principal)
        || bonds != U256::from(p.outstanding_bonds)
    {
        return Err(bad("event history does not match canonical contract state"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct UnresponsiveTransport;
    #[async_trait::async_trait]
    impl JsonRpcTransport for UnresponsiveTransport {
        async fn post_json_value(
            &self,
            _url: &str,
            _request: &Value,
        ) -> Result<Value, ChainBaseError> {
            std::future::pending().await
        }
    }
    #[tokio::test]
    async fn rpc_timeout_is_an_availability_failure_not_invalid_customer_terms() {
        let error = rpc(
            &UnresponsiveTransport,
            "http://unused.invalid",
            "eth_call",
            json!([]),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, ChainBaseError::RpcTransport(_)));
    }

    fn fixture() -> (Release, Request) {
        (
            Release {
                protocol: protocol::PROTOCOL.into(),
                network: "base-sepolia".into(),
                factory: Address::repeat_byte(1),
                implementation: Address::repeat_byte(2),
                factory_code_hash: B256::repeat_byte(3),
                implementation_code_hash: B256::repeat_byte(4),
                deployment_block: 1,
            },
            Request {
                public_evidence: None,
                original_terms: Terms {
                    schema: protocol::PROTOCOL.into(),
                    network: "base-sepolia".into(),
                    bounty_id: format!("{:#x}", B256::repeat_byte(5)),
                    creator: format!("{:#x}", Address::repeat_byte(6)),
                    title: "Open review".into(),
                    goal: "Deliver evidence".into(),
                    acceptance_criteria: vec!["Exact evidence".into()],
                    solver_reward_micro_usdc: 20_000_000,
                    review_reward_micro_usdc: 1_000_000,
                    submission_deadline: 2_000_000_000,
                    review_window_seconds: 3600,
                    max_pending_entries: 8,
                    disclosure: protocol::DISCLOSURE.into(),
                },
                user_salt: B256::repeat_byte(7),
                expected_revision: 1,
                expected_solver_reward_micro_usdc: 20_000_000,
                valid_before: 1_990_000_000,
                action: Action::Create {
                    token_nonce: B256::repeat_byte(8),
                },
            },
        )
    }
    #[test]
    fn browser_preparation_vectors_match_the_rust_authority() {
        let (release, mut request) = fixture();
        let hash = B256::repeat_byte(10);
        let solver = Address::repeat_byte(9);
        let public = CreatorOpenPublicEvidence {
            artifact_url: "https://example.org/deliverable".into(),
            artifact_sha256: format!("sha256:{}", "ab".repeat(32)),
            notes: "Check the committed artifact before deciding.".into(),
        };
        let mut plans = Vec::new();
        for action in [
            Action::Create { token_nonce: hash },
            Action::Submit {
                solver,
                submission_hash: hash,
                evidence_hash: public.commitment().unwrap(),
                token_nonce: hash,
            },
            Action::Review {
                entry: 1,
                solver,
                submission_hash: hash,
                evidence_hash: public.commitment().unwrap(),
                passed: true,
                response_hash: hash,
            },
            Action::Increase {
                new_reward_micro_usdc: 25_000_000,
                token_nonce: hash,
            },
            Action::Cancel,
            Action::Expire,
            Action::RefundBond { entry: 1 },
            Action::RefundPrincipal,
        ] {
            request.public_evidence =
                matches!(&action, Action::Submit { .. }).then(|| public.clone());
            request.action = action;
            plans.push(
                serde_json::to_value(prepare(&release, &request, &Signatures::default()).unwrap())
                    .unwrap(),
            );
        }
        let value = json!({"release":release,"now":1_989_999_400u64,"plans":plans});
        if let Ok(path) = std::env::var("CREATOR_OPEN_BROWSER_FIXTURE_EXPORT") {
            std::fs::write(
                path,
                format!("{}\n", serde_json::to_string_pretty(&value).unwrap()),
            )
            .unwrap();
        } else {
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../site/fixtures/creator-open-preparations.json"
            );
            let stored: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            assert_eq!(
                stored, value,
                "review and regenerate the shared browser authority vectors"
            );
        }
        let mut altered = public;
        altered.artifact_url = "https://example.org/changed".into();
        request.action = Action::Submit {
            solver,
            submission_hash: hash,
            evidence_hash: hash,
            token_nonce: hash,
        };
        request.public_evidence = Some(altered);
        assert!(prepare(&release, &request, &Signatures::default()).is_err());
    }
    #[test]
    fn all_customer_actions_have_exact_zero_value_plans_and_minimum_signatures() {
        let (release, mut r) = fixture();
        let solver = Address::repeat_byte(9);
        let hash = B256::repeat_byte(10);
        for (action, token, authority, amount) in [
            (
                Action::Create { token_nonce: hash },
                true,
                false,
                "21000000",
            ),
            (
                Action::Submit {
                    solver,
                    submission_hash: hash,
                    evidence_hash: hash,
                    token_nonce: hash,
                },
                true,
                true,
                "1000000",
            ),
            (
                Action::Review {
                    entry: 1,
                    solver,
                    submission_hash: hash,
                    evidence_hash: hash,
                    passed: true,
                    response_hash: hash,
                },
                false,
                true,
                "0",
            ),
            (
                Action::Increase {
                    new_reward_micro_usdc: 25_000_000,
                    token_nonce: hash,
                },
                true,
                true,
                "5000000",
            ),
            (Action::Cancel, false, true, "0"),
            (Action::Expire, false, false, "0"),
            (Action::RefundBond { entry: 1 }, false, false, "0"),
            (Action::RefundPrincipal, false, false, "0"),
        ] {
            r.action = action;
            let p = prepare(&release, &r, &Signatures::default()).unwrap();
            assert_eq!(p.transaction.value_wei, 0);
            assert_eq!(p.customer_gas_wei, "0");
            assert_eq!(p.customer_principal_micro_usdc, amount);
            assert_eq!(p.token_authorization.is_some(), token);
            assert_eq!(p.action_authorization.is_some(), authority);
            if let Some(auth) = p.token_authorization {
                assert_eq!(auth.typed_data["primaryType"], "ReceiveWithAuthorization");
                assert_eq!(
                    auth.typed_data["message"]["to"],
                    format!("{:#x}", p.bounty_contract)
                );
            }
            let serialized =
                serde_json::to_value(prepare(&release, &r, &Signatures::default()).unwrap())
                    .unwrap();
            assert!(
                serialized.get("transaction").is_none(),
                "preparation never supplies a direct-send fallback"
            );
        }
    }
    #[test]
    fn approval_is_immutable_but_independent_of_signature_encoding() {
        let (release, r) = fixture();
        let original = prepare(&release, &r, &Signatures::default()).unwrap();
        let signed = prepare(
            &release,
            &r,
            &Signatures {
                token: vec![1; 65].into(),
                action: Bytes::new(),
            },
        )
        .unwrap();
        assert_eq!(original.approval_hash, signed.approval_hash);
        for field in ["user_salt", "valid_before", "goal", "reward"] {
            let mut changed = r.clone();
            match field {
                "user_salt" => changed.user_salt = B256::repeat_byte(99),
                "valid_before" => changed.valid_before += 1,
                "goal" => changed.original_terms.goal.push('!'),
                _ => {
                    changed.original_terms.solver_reward_micro_usdc += 1;
                    changed.expected_solver_reward_micro_usdc += 1;
                }
            }
            assert_ne!(
                prepare(&release, &changed, &Signatures::default())
                    .unwrap()
                    .approval_hash,
                original.approval_hash
            );
        }
        let mut different = release.clone();
        different.implementation_code_hash = B256::repeat_byte(90);
        assert_ne!(
            prepare(&different, &r, &Signatures::default())
                .unwrap()
                .approval_hash,
            original.approval_hash
        );
    }
    #[test]
    fn unknown_authority_and_invalid_economics_are_rejected() {
        let (release, mut r) = fixture();
        let mut value = serde_json::to_value(&r).unwrap();
        value["calldata"] = "0x1234".into();
        assert!(serde_json::from_value::<Request>(value).is_err());
        let mut value = serde_json::to_value(&r).unwrap();
        value["action"]["destination"] = format!("{:#x}", Address::repeat_byte(99)).into();
        assert!(serde_json::from_value::<Request>(value).is_err());
        r.action = Action::Increase {
            new_reward_micro_usdc: 19_000_000,
            token_nonce: B256::repeat_byte(8),
        };
        assert!(prepare(&release, &r, &Signatures::default()).is_err());
        r.action = Action::Cancel;
        assert!(prepare(
            &release,
            &r,
            &Signatures {
                token: vec![1; 65].into(),
                action: Bytes::new()
            }
        )
        .is_err());
        r.expected_revision = u64::MAX;
        assert!(prepare(&release, &r, &Signatures::default()).is_err());
    }
}
