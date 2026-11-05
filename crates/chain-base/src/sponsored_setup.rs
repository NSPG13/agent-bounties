//! Fixed, zero-value setup actions on reviewed immutable releases.
//! Wallet deployment creates an empty wallet; this endpoint cannot move principal.
use super::*;
use crate::creator_open_service::{call, code, rpc, verify_wallet_signature};
use alloy::{
    primitives::{keccak256, Signature},
    sol,
    sol_types::{SolCall, SolStruct, SolValue},
};
use serde_json::{json, Value};

pub const SCHEMA: &str = "agent-bounties/sponsored-setup-v1";
fn bad(message: &str) -> ChainBaseError {
    ChainBaseError::InvalidRelayIntent(message.into())
}

sol! {
    struct SetupPolicy {
        address delegate;
        uint64 validAfter; uint64 validUntil; uint64 periodSeconds;
        uint256 maxPerAction; uint256 maxPerPeriod; uint256 maxLifetimeSpend; uint256 maxBountyTarget;
        uint8 allowedActions; uint8 allowedVerificationModes;
        address deterministicVerifierModule;
        bytes32 signedQuorumVerifierSetHash; bytes32 aiJudgeVerifierSetHash;
    }
    struct SponsoredSetup {
        address owner; address target; bytes32 callHash; bytes32 releaseHash; uint256 validBefore;
    }
    struct ParticipantAttestation {
        address wallet; bytes32 participantId; bytes32 sourceHash; uint64 validUntil; uint256 nonce;
    }
    interface SetupAbi {
        function createWallet(address owner, SetupPolicy policy, bytes32 userSalt) external returns(address);
        function predictWallet(address owner, SetupPolicy policy, bytes32 userSalt) external view returns(address);
        function implementation() external view returns(address);
        function WALLET_VERSION() external view returns(bytes32);
        function bountyFactory() external view returns(address);
        function factory() external view returns(address);
        function deploymentFactory() external view returns(address);
        function settlementToken() external view returns(address);
        function isFactoryWallet(address wallet) external view returns(bool);
        function owner() external view returns(address);
        function policyHash() external view returns(bytes32);
        function attester() external view returns(address);
        function nonces(address wallet) external view returns(uint256);
        function participants(address wallet) external view returns(bytes32 participantId, bytes32 sourceHash, uint64 registeredAt, uint64 validUntil);
        function register(address wallet, bytes32 participantId, bytes32 sourceHash, uint64 validUntil, bytes signature) external;
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    pub network: String,
    pub bounty_factory: Address,
    pub bounty_factory_code_hash: B256,
    pub wallet_factory: Address,
    pub wallet_factory_code_hash: B256,
    pub wallet_implementation: Address,
    pub wallet_implementation_code_hash: B256,
    pub participant_registry: Address,
    pub participant_registry_code_hash: B256,
    pub deployment_block: u64,
}
impl Release {
    pub fn validate(&self) -> Result<(), ChainBaseError> {
        base_network_descriptor(&self.network)?;
        let addresses = [
            self.bounty_factory,
            self.wallet_factory,
            self.wallet_implementation,
            self.participant_registry,
        ];
        if self.deployment_block == 0
            || addresses.iter().any(|a| a.is_zero())
            || addresses.into_iter().collect::<HashSet<_>>().len() != 4
            || [
                self.bounty_factory_code_hash,
                self.wallet_factory_code_hash,
                self.wallet_implementation_code_hash,
                self.participant_registry_code_hash,
            ]
            .iter()
            .any(B256::is_zero)
        {
            return Err(bad("invalid setup release"));
        }
        Ok(())
    }
    pub fn hash(&self) -> Result<B256, ChainBaseError> {
        self.validate()?;
        Ok(keccak256(
            serde_json::to_vec(self).map_err(|_| bad("invalid release"))?,
        ))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SponsoredSetupPolicy {
    pub delegate: String,
    pub valid_after: u64,
    pub valid_until: u64,
    pub period_seconds: u64,
    pub max_per_action_micro_usdc: u64,
    pub max_per_period_micro_usdc: u64,
    pub max_lifetime_micro_usdc: u64,
    pub max_bounty_target_micro_usdc: u64,
    pub allowed_actions: u8,
    pub allowed_verification_modes: u8,
    pub deterministic_verifier_module: String,
    pub signed_quorum_verifier_set_hash: String,
    pub ai_judge_verifier_set_hash: String,
}
impl Policy {
    pub fn abi(&self) -> Result<SetupPolicy, ChainBaseError> {
        let p = SetupPolicy {
            delegate: parse_alloy_address(&self.delegate)?,
            validAfter: self.valid_after,
            validUntil: self.valid_until,
            periodSeconds: self.period_seconds,
            maxPerAction: U256::from(self.max_per_action_micro_usdc),
            maxPerPeriod: U256::from(self.max_per_period_micro_usdc),
            maxLifetimeSpend: U256::from(self.max_lifetime_micro_usdc),
            maxBountyTarget: U256::from(self.max_bounty_target_micro_usdc),
            allowedActions: self.allowed_actions,
            allowedVerificationModes: self.allowed_verification_modes,
            deterministicVerifierModule: parse_alloy_address(&self.deterministic_verifier_module)?,
            signedQuorumVerifierSetHash: B256::from(parse_bytes32(
                &self.signed_quorum_verifier_set_hash,
            )?),
            aiJudgeVerifierSetHash: B256::from(parse_bytes32(&self.ai_judge_verifier_set_hash)?),
        };
        if p.delegate.is_zero()
            || p.validUntil <= p.validAfter
            || p.allowedActions == 0
            || p.allowedActions & !15 != 0
            || p.allowedVerificationModes == 0
            || p.allowedVerificationModes & !7 != 0
            || self.max_bounty_target_micro_usdc == 0
            || ((p.allowedActions & 7) != 0
                && [
                    self.period_seconds,
                    self.max_per_action_micro_usdc,
                    self.max_per_period_micro_usdc,
                    self.max_lifetime_micro_usdc,
                ]
                .contains(&0))
            || (p.allowedVerificationModes & 1 != 0) == p.deterministicVerifierModule.is_zero()
            || (p.allowedVerificationModes & 2 != 0) == p.signedQuorumVerifierSetHash.is_zero()
            || (p.allowedVerificationModes & 4 != 0) == p.aiJudgeVerifierSetHash.is_zero()
            || [
                self.max_per_action_micro_usdc,
                self.max_per_period_micro_usdc,
                self.max_lifetime_micro_usdc,
                self.max_bounty_target_micro_usdc,
            ]
            .iter()
            .any(|v| *v > 9_007_199_254_740_991)
        {
            return Err(bad("invalid bounded wallet policy"));
        }
        Ok(p)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SponsoredSetupAction {
    WalletDeploy {
        policy: SponsoredSetupPolicy,
        user_salt: String,
    },
    ParticipantRegister {
        participant_id: String,
        source_hash: String,
        valid_until: u64,
        registry_nonce: u64,
        attester_signature: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SponsoredSetupRequest {
    pub network: String,
    pub owner: String,
    pub valid_before: u64,
    pub action: SponsoredSetupAction,
}
pub type Policy = SponsoredSetupPolicy;
pub type Action = SponsoredSetupAction;
pub type Request = SponsoredSetupRequest;
#[derive(Clone, Serialize)]
pub struct Prepared {
    pub schema: &'static str,
    pub request: Request,
    pub release: Release,
    pub digest: B256,
    pub typed_data: Value,
    pub transaction: EvmTransactionIntent,
    pub predicted_wallet: Option<Address>,
}
pub fn prepare(release: &Release, request: &Request) -> Result<Prepared, ChainBaseError> {
    release.validate()?;
    if release.network != request.network {
        return Err(bad("setup network mismatch"));
    }
    let owner = parse_alloy_address(&request.owner)?;
    if owner.is_zero() || request.valid_before == 0 {
        return Err(bad("invalid setup identity"));
    }
    let (target, data, predicted_wallet, function) = match &request.action {
        Action::WalletDeploy { policy, user_salt } => {
            let policy = policy.abi()?;
            let salt = B256::from(parse_bytes32(user_salt)?);
            let effective = keccak256((owner, salt, keccak256(policy.abi_encode())).abi_encode());
            let init = hex::decode(format!(
                "3d602d80600a3d3981f3363d3d373d3d3d363d73{}5af43d82803e903d91602b57fd5bf3",
                hex::encode(release.wallet_implementation)
            ))
            .unwrap();
            let wallet = release.wallet_factory.create2(effective, keccak256(init));
            (
                release.wallet_factory,
                SetupAbi::createWalletCall {
                    owner,
                    policy,
                    userSalt: salt,
                }
                .abi_encode(),
                Some(wallet),
                "createWallet",
            )
        }
        Action::ParticipantRegister {
            participant_id,
            source_hash,
            valid_until,
            attester_signature,
            ..
        } => {
            let id = B256::from(parse_bytes32(participant_id)?);
            let source = B256::from(parse_bytes32(source_hash)?);
            let signature = parse_alloy_bytes(attester_signature)?;
            if id.is_zero()
                || source.is_zero()
                || signature.len() != 65
                || ![27, 28].contains(&signature[64])
            {
                return Err(bad("invalid participant registration"));
            }
            let parsed = Signature::try_from(signature.as_ref())
                .map_err(|_| bad("invalid attester signature"))?;
            if parsed.normalize_s().is_some() {
                return Err(bad("noncanonical attester signature"));
            }
            (
                release.participant_registry,
                SetupAbi::registerCall {
                    wallet: owner,
                    participantId: id,
                    sourceHash: source,
                    validUntil: *valid_until,
                    signature,
                }
                .abi_encode(),
                None,
                "register",
            )
        }
    };
    let chain = base_network_descriptor(&request.network)?.chain_id;
    let message = SponsoredSetup {
        owner,
        target,
        callHash: keccak256(&data),
        releaseHash: release.hash()?,
        validBefore: U256::from(request.valid_before),
    };
    let domain = alloy::sol_types::Eip712Domain {
        name: Some("Agent Bounties Sponsored Setup".into()),
        version: Some("1".into()),
        chain_id: Some(U256::from(chain)),
        verifying_contract: Some(target),
        salt: None,
    };
    let typed_data = json!({"types":{"EIP712Domain":[{"name":"name","type":"string"},{"name":"version","type":"string"},{"name":"chainId","type":"uint256"},{"name":"verifyingContract","type":"address"}],"SponsoredSetup":[{"name":"owner","type":"address"},{"name":"target","type":"address"},{"name":"callHash","type":"bytes32"},{"name":"releaseHash","type":"bytes32"},{"name":"validBefore","type":"uint256"}]},"domain":{"name":"Agent Bounties Sponsored Setup","version":"1","chainId":chain,"verifyingContract":target},"primaryType":"SponsoredSetup","message":{"owner":owner,"target":target,"callHash":message.callHash,"releaseHash":message.releaseHash,"validBefore":request.valid_before.to_string()}});
    Ok(Prepared {
        schema: SCHEMA,
        request: request.clone(),
        release: release.clone(),
        digest: message.eip712_signing_hash(&domain),
        typed_data,
        transaction: EvmTransactionIntent {
            from: None,
            to: format!("{target:#x}"),
            value_wei: 0,
            data: format!("0x{}", hex::encode(data)),
            function: function.into(),
        },
        predicted_wallet,
    })
}
/// Verify every immutable pointer and exact bytecode at one fresh canonical block.
pub async fn verify_release<T: JsonRpcTransport + ?Sized>(
    t: &T,
    url: &str,
    r: &Release,
    block: &BaseBlockIdentity,
    now: u64,
) -> Result<(), ChainBaseError> {
    r.validate()?;
    let n = base_network_descriptor(&r.network)?;
    if block.number < r.deployment_block
        || block.timestamp > now.saturating_add(30)
        || now.saturating_sub(block.timestamp) > 180
        || rpc(t, url, "eth_chainId", json!([])).await? != json!(format!("0x{:x}", n.chain_id))
    {
        return Err(bad("setup chain state is not fresh"));
    }
    for (a, h) in [
        (r.bounty_factory, r.bounty_factory_code_hash),
        (r.wallet_factory, r.wallet_factory_code_hash),
        (r.wallet_implementation, r.wallet_implementation_code_hash),
        (r.participant_registry, r.participant_registry_code_hash),
    ] {
        let c = code(t, url, a, block).await?;
        if c.is_empty() || keccak256(c) != h {
            return Err(bad("unreviewed setup bytecode"));
        }
    }
    if call(
        t,
        url,
        r.wallet_implementation,
        SetupAbi::WALLET_VERSIONCall {},
        block,
    )
    .await?
        != keccak256("agent-bounties/bounded-wallet/v2".as_bytes())
    {
        return Err(bad("unsupported wallet implementation version"));
    }
    let token: Address = n
        .native_usdc_token_address
        .parse()
        .map_err(|_| bad("invalid native USDC"))?;
    if call(
        t,
        url,
        r.wallet_factory,
        SetupAbi::implementationCall {},
        block,
    )
    .await?
        != r.wallet_implementation
        || call(
            t,
            url,
            r.wallet_factory,
            SetupAbi::bountyFactoryCall {},
            block,
        )
        .await?
            != r.bounty_factory
        || call(
            t,
            url,
            r.wallet_implementation,
            SetupAbi::factoryCall {},
            block,
        )
        .await?
            != r.bounty_factory
        || call(
            t,
            url,
            r.wallet_implementation,
            SetupAbi::deploymentFactoryCall {},
            block,
        )
        .await?
            != r.wallet_factory
    {
        return Err(bad("setup factory pointers changed"));
    }
    for a in [r.bounty_factory, r.wallet_factory, r.wallet_implementation] {
        if call(t, url, a, SetupAbi::settlementTokenCall {}, block).await? != token {
            return Err(bad("wrong setup settlement token"));
        }
    }
    Ok(())
}
/// State proof is setup readiness only, never bounty funding or solver payment.
pub async fn complete<T: JsonRpcTransport + ?Sized>(
    t: &T,
    url: &str,
    p: &Prepared,
    block: &BaseBlockIdentity,
) -> Result<bool, ChainBaseError> {
    let owner = parse_alloy_address(&p.request.owner)?;
    match &p.request.action {
        Action::WalletDeploy { policy, user_salt } => {
            let wallet = p
                .predicted_wallet
                .ok_or_else(|| bad("missing wallet prediction"))?;
            if call(
                t,
                url,
                p.release.wallet_factory,
                SetupAbi::predictWalletCall {
                    owner,
                    policy: policy.abi()?,
                    userSalt: B256::from(parse_bytes32(user_salt)?),
                },
                block,
            )
            .await?
                != wallet
            {
                return Err(bad("wallet prediction mismatch"));
            }
            if code(t, url, wallet, block).await?.is_empty() {
                return Ok(false);
            }
            let expected = hex::decode(format!(
                "363d3d373d3d3d363d73{}5af43d82803e903d91602b57fd5bf3",
                hex::encode(p.release.wallet_implementation)
            ))
            .unwrap();
            if code(t, url, wallet, block).await?.as_ref() != expected
                || !call(
                    t,
                    url,
                    p.release.wallet_factory,
                    SetupAbi::isFactoryWalletCall { wallet },
                    block,
                )
                .await?
                || call(t, url, wallet, SetupAbi::ownerCall {}, block).await? != owner
                || call(t, url, wallet, SetupAbi::policyHashCall {}, block).await?
                    != keccak256(policy.abi()?.abi_encode())
            {
                return Err(bad("deployed wallet authority mismatch"));
            }
            Ok(true)
        }
        Action::ParticipantRegister {
            participant_id,
            source_hash,
            valid_until,
            registry_nonce,
            ..
        } => {
            let r = call(
                t,
                url,
                p.release.participant_registry,
                SetupAbi::participantsCall { wallet: owner },
                block,
            )
            .await?;
            let nonce = call(
                t,
                url,
                p.release.participant_registry,
                SetupAbi::noncesCall { wallet: owner },
                block,
            )
            .await?;
            Ok(
                r.participantId == B256::from(parse_bytes32(participant_id)?)
                    && r.sourceHash == B256::from(parse_bytes32(source_hash)?)
                    && r.registeredAt > 0
                    && r.registeredAt <= block.timestamp
                    && r.validUntil == *valid_until
                    && nonce
                        == U256::from(*registry_nonce)
                            .checked_add(U256::from(1))
                            .ok_or_else(|| bad("nonce overflow"))?,
            )
        }
    }
}
pub async fn verify_authority<T: JsonRpcTransport + ?Sized>(
    t: &T,
    url: &str,
    p: &Prepared,
    signature: &str,
    block: &BaseBlockIdentity,
) -> Result<(), ChainBaseError> {
    let signature = parse_alloy_bytes(signature)?;
    verify_wallet_signature(
        t,
        url,
        parse_alloy_address(&p.request.owner)?,
        p.digest,
        &signature,
        block,
    )
    .await
}
pub async fn verify_fresh<T: JsonRpcTransport + ?Sized>(
    t: &T,
    url: &str,
    p: &Prepared,
    block: &BaseBlockIdentity,
    now: u64,
) -> Result<(), ChainBaseError> {
    if p.request.valid_before <= now.saturating_add(30)
        || p.request.valid_before > now.saturating_add(7200)
    {
        return Err(bad("setup approval expired or overlong"));
    }
    match &p.request.action {
        Action::WalletDeploy { policy, .. } => {
            if policy.valid_until <= now {
                return Err(bad("wallet policy expired"));
            }
        }
        Action::ParticipantRegister {
            participant_id,
            source_hash,
            valid_until,
            registry_nonce,
            attester_signature,
        } => {
            if *valid_until <= now
                || *valid_until > now.saturating_add(365 * 86400)
                || call(
                    t,
                    url,
                    p.release.participant_registry,
                    SetupAbi::noncesCall {
                        wallet: parse_alloy_address(&p.request.owner)?,
                    },
                    block,
                )
                .await?
                    != U256::from(*registry_nonce)
            {
                return Err(bad("registration nonce or validity changed"));
            }
            let message = ParticipantAttestation {
                wallet: parse_alloy_address(&p.request.owner)?,
                participantId: B256::from(parse_bytes32(participant_id)?),
                sourceHash: B256::from(parse_bytes32(source_hash)?),
                validUntil: *valid_until,
                nonce: U256::from(*registry_nonce),
            };
            let digest = message.eip712_signing_hash(&alloy::sol_types::Eip712Domain {
                name: Some("Agent Bounties Participant Registry".into()),
                version: Some("1".into()),
                chain_id: Some(U256::from(
                    base_network_descriptor(&p.request.network)?.chain_id,
                )),
                verifying_contract: Some(p.release.participant_registry),
                salt: None,
            });
            let s: Signature = attester_signature
                .parse()
                .map_err(|_| bad("invalid attester signature"))?;
            if s.recover_address_from_prehash(&digest)
                .map_err(|_| bad("invalid attester signature"))?
                != call(
                    t,
                    url,
                    p.release.participant_registry,
                    SetupAbi::attesterCall {},
                    block,
                )
                .await?
            {
                return Err(bad("wrong registry attester"));
            }
        }
    }
    Ok(())
}
pub async fn simulate<T: JsonRpcTransport + ?Sized>(
    t: &T,
    url: &str,
    p: &Prepared,
    relayer: &str,
    block: &BaseBlockIdentity,
) -> Result<(), ChainBaseError> {
    rpc(t,url,"eth_call",json!([{"from":relayer,"to":p.transaction.to,"data":p.transaction.data,"value":"0x0","gas":"0x16e360"},{"blockHash":block.hash,"requireCanonical":true}])).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Release, Request) {
        let release = Release {
            network: "base-sepolia".into(),
            bounty_factory: Address::repeat_byte(1),
            bounty_factory_code_hash: B256::repeat_byte(2),
            wallet_factory: Address::repeat_byte(3),
            wallet_factory_code_hash: B256::repeat_byte(4),
            wallet_implementation: Address::repeat_byte(5),
            wallet_implementation_code_hash: B256::repeat_byte(6),
            participant_registry: Address::repeat_byte(7),
            participant_registry_code_hash: B256::repeat_byte(8),
            deployment_block: 1,
        };
        let policy = Policy {
            delegate: format!("{:#x}", Address::repeat_byte(10)),
            valid_after: 1_999_999_000,
            valid_until: 2_000_086_000,
            period_seconds: 86400,
            max_per_action_micro_usdc: 5_000_000,
            max_per_period_micro_usdc: 10_000_000,
            max_lifetime_micro_usdc: 20_000_000,
            max_bounty_target_micro_usdc: 100_000_000,
            allowed_actions: 15,
            allowed_verification_modes: 2,
            deterministic_verifier_module: format!("{:#x}", Address::ZERO),
            signed_quorum_verifier_set_hash: format!("{:#x}", B256::repeat_byte(11)),
            ai_judge_verifier_set_hash: format!("{:#x}", B256::ZERO),
        };
        (
            release,
            Request {
                network: "base-sepolia".into(),
                owner: format!("{:#x}", Address::repeat_byte(9)),
                valid_before: 2_000_003_600,
                action: Action::WalletDeploy {
                    policy,
                    user_salt: format!("{:#x}", B256::repeat_byte(12)),
                },
            },
        )
    }
    #[test]
    fn setup_vectors_and_owner_authority_bind_every_field() {
        let (release, request) = fixture();
        let prepared = prepare(&release, &request).unwrap();
        let signer = PrivateKeySigner::random();
        let signature = signer.sign_hash_sync(&prepared.digest).unwrap();
        for field in ["owner", "valid_before", "action"] {
            let mut value = serde_json::to_value(&request).unwrap();
            if field == "owner" {
                value[field] = json!(format!("{:#x}", Address::repeat_byte(13)));
            } else if field == "valid_before" {
                value[field] = json!(2_000_003_599_u64);
            } else {
                value[field]["policy"]["max_per_action_micro_usdc"] = json!(6_000_000);
            }
            let changed = prepare(&release, &serde_json::from_value(value).unwrap()).unwrap();
            assert_ne!(changed.digest, prepared.digest);
            assert_ne!(
                signature
                    .recover_address_from_prehash(&changed.digest)
                    .unwrap(),
                signer.address()
            );
        }
        let mut other = release.clone();
        other.wallet_factory_code_hash = B256::repeat_byte(14);
        assert_ne!(prepare(&other, &request).unwrap().digest, prepared.digest);
        let mut invalid = serde_json::to_value(&request).unwrap();
        invalid["action"]["policy"]["allowed_actions"] = json!(16);
        assert!(prepare(&release, &serde_json::from_value(invalid).unwrap()).is_err());
        let mut second = request.clone();
        second.action = Action::ParticipantRegister {
            participant_id: format!("{:#x}", B256::repeat_byte(1)),
            source_hash: format!("{:#x}", B256::repeat_byte(2)),
            valid_until: 2_000_086_400,
            registry_nonce: 0,
            attester_signature: format!(
                "0x{}",
                hex::encode(
                    signer
                        .sign_hash_sync(&B256::repeat_byte(1))
                        .unwrap()
                        .as_bytes()
                )
            ),
        };
        // Deterministic signature for the cross-language fixture; this is not a live attestation.
        if let Action::ParticipantRegister {
            attester_signature, ..
        } = &mut second.action
        {
            let key: PrivateKeySigner = format!("{:064x}", 1).parse().unwrap();
            *attester_signature = format!(
                "0x{}",
                hex::encode(
                    key.sign_hash_sync(&B256::repeat_byte(1))
                        .unwrap()
                        .as_bytes()
                )
            );
        }
        let vectors = json!([prepared, prepare(&release, &second).unwrap()]);
        if std::env::var("UPDATE_SPONSORED_SETUP_FIXTURE").as_deref() == Ok("1") {
            std::fs::write(
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/fixtures/sponsored-setup-vectors.json"
                ),
                serde_json::to_string_pretty(&vectors).unwrap() + "\n",
            )
            .unwrap();
        } else {
            assert_eq!(
                vectors,
                serde_json::from_str::<Value>(include_str!(
                    "../fixtures/sponsored-setup-vectors.json"
                ))
                .unwrap()
            );
        }
    }
}
