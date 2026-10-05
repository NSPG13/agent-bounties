//! `ParticipantEligibilityRegistry` attestation planning. The registry's single immutable attester
//! signs an EIP-712 `ParticipantAttestation`; anyone may relay `register`. The invoice contractor
//! registry uses this to admit attested contractors to `agent-bounties/invoice-contractor-v1`
//! claim gates. A signed attestation or relay hash is not registration evidence; only the
//! registry's `ParticipantAttested` event is.

use super::*;

pub const PARTICIPANT_REGISTRY_EIP712_NAME: &str = "Agent Bounties Participant Registry";
pub const PARTICIPANT_REGISTRY_EIP712_VERSION: &str = "1";
const PARTICIPANT_ATTESTATION_TYPE: &str =
    "ParticipantAttestation(address wallet,bytes32 participantId,bytes32 sourceHash,uint64 validUntil,uint256 nonce)";
const REGISTER_FUNCTION: &str = "register(address,bytes32,bytes32,uint64,bytes)";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParticipantAttestationRequest {
    pub registry: String,
    pub wallet: String,
    pub participant_id: String,
    pub source_hash: String,
    pub valid_until: u64,
    /// The registry's current `nonces(wallet)`.
    pub nonce: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParticipantAttestationMessage {
    pub wallet: String,
    pub participant_id: String,
    pub source_hash: String,
    pub valid_until: String,
    pub nonce: String,
}

/// `eth_signTypedData_v4` payload for the registry attester.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParticipantAttestationTypedData {
    pub types: BTreeMap<String, Vec<Eip712TypeField>>,
    pub domain: Eip712DomainData,
    pub primary_type: String,
    pub message: ParticipantAttestationMessage,
}

struct ParticipantAttestationWords {
    registry: String,
    wallet: [u8; 32],
    participant_id: [u8; 32],
    source_hash: [u8; 32],
    valid_until: [u8; 32],
    nonce: [u8; 32],
}

impl ParticipantAttestationRequest {
    fn words(&self) -> Result<ParticipantAttestationWords, ChainBaseError> {
        let participant_id = parse_bytes32(&self.participant_id)?;
        let source_hash = parse_bytes32(&self.source_hash)?;
        let wallet = encode_address(&self.wallet)?;
        if wallet == [0u8; 32]
            || participant_id == [0u8; 32]
            || source_hash == [0u8; 32]
            || self.valid_until == 0
        {
            return Err(ChainBaseError::InvalidVerificationConfiguration(
                "participant attestation needs a wallet, participant id, source, and expiry"
                    .to_string(),
            ));
        }
        Ok(ParticipantAttestationWords {
            registry: normalize_address(&self.registry)?,
            wallet,
            participant_id,
            source_hash,
            valid_until: encode_uint256(self.valid_until.into())?,
            nonce: encode_uint256(self.nonce.into())?,
        })
    }
}

pub fn plan_participant_attestation(
    network: &str,
    request: &ParticipantAttestationRequest,
) -> Result<ParticipantAttestationTypedData, ChainBaseError> {
    let network = base_network_descriptor(network)?;
    let words = request.words()?;
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
        "ParticipantAttestation".to_string(),
        vec![
            eip712_field("wallet", "address"),
            eip712_field("participantId", "bytes32"),
            eip712_field("sourceHash", "bytes32"),
            eip712_field("validUntil", "uint64"),
            eip712_field("nonce", "uint256"),
        ],
    );
    Ok(ParticipantAttestationTypedData {
        types,
        domain: Eip712DomainData {
            name: PARTICIPANT_REGISTRY_EIP712_NAME.to_string(),
            version: PARTICIPANT_REGISTRY_EIP712_VERSION.to_string(),
            chain_id: network.chain_id,
            verifying_contract: words.registry,
        },
        primary_type: "ParticipantAttestation".to_string(),
        message: ParticipantAttestationMessage {
            wallet: normalize_address(&request.wallet)?,
            participant_id: word_hex(words.participant_id),
            source_hash: word_hex(words.source_hash),
            valid_until: request.valid_until.to_string(),
            nonce: request.nonce.to_string(),
        },
    })
}

/// `ParticipantEligibilityRegistry.attestationDigest`, computed offline.
pub fn participant_attestation_digest(
    network: &str,
    request: &ParticipantAttestationRequest,
) -> Result<[u8; 32], ChainBaseError> {
    let network = base_network_descriptor(network)?;
    let words = request.words()?;
    let keccak = |bytes: &[u8]| -> [u8; 32] { Keccak256::digest(bytes).into() };
    let mut struct_data = keccak(PARTICIPANT_ATTESTATION_TYPE.as_bytes()).to_vec();
    for word in [
        words.wallet,
        words.participant_id,
        words.source_hash,
        words.valid_until,
        words.nonce,
    ] {
        struct_data.extend_from_slice(&word);
    }
    let mut domain = keccak(
        b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    )
    .to_vec();
    domain.extend_from_slice(&keccak(PARTICIPANT_REGISTRY_EIP712_NAME.as_bytes()));
    domain.extend_from_slice(&keccak(PARTICIPANT_REGISTRY_EIP712_VERSION.as_bytes()));
    domain.extend_from_slice(&encode_uint256(network.chain_id.into())?);
    domain.extend_from_slice(&encode_address(&words.registry)?);
    let mut digest = b"\x19\x01".to_vec();
    digest.extend_from_slice(&keccak(&domain));
    digest.extend_from_slice(&keccak(&struct_data));
    Ok(keccak(&digest))
}

/// Recovers the signer of a 65-byte attestation signature, with the same low-s and v rules as the
/// registry.
pub fn recover_participant_attester(
    network: &str,
    request: &ParticipantAttestationRequest,
    signature: &str,
) -> Result<String, ChainBaseError> {
    let digest = participant_attestation_digest(network, request)?;
    let bytes = parse_hex_bytes(signature)?;
    let invalid = || {
        ChainBaseError::InvalidVerificationConfiguration(
            "participant attestation signature must be 65 bytes, low-s, with v 27 or 28"
                .to_string(),
        )
    };
    if bytes.len() != 65 || !matches!(bytes[64], 27 | 28) {
        return Err(invalid());
    }
    let signature = alloy::primitives::Signature::from_raw(&bytes).map_err(|_| invalid())?;
    if signature.normalize_s().is_some() {
        return Err(invalid());
    }
    let recovered = signature
        .recover_address_from_prehash(&B256::from(digest))
        .map_err(|_| invalid())?;
    normalize_address(recovered.to_string())
}

/// Plans the relayed `register` call after checking that `expected_attester` signed exactly this
/// attestation.
pub fn plan_participant_registration(
    network: &str,
    request: &ParticipantAttestationRequest,
    signature: &str,
    expected_attester: &str,
    relayer: Option<&str>,
) -> Result<EvmTransactionIntent, ChainBaseError> {
    if recover_participant_attester(network, request, signature)?
        != normalize_address(expected_attester)?
    {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "participant attestation was not signed by the registry attester".to_string(),
        ));
    }
    let words = request.words()?;
    let signature = parse_hex_bytes(signature)?;
    let mut bytes = selector(REGISTER_FUNCTION).to_vec();
    for word in [
        words.wallet,
        words.participant_id,
        words.source_hash,
        words.valid_until,
    ] {
        bytes.extend_from_slice(&word);
    }
    bytes.extend_from_slice(&encode_uint256(5 * 32)?);
    bytes.extend_from_slice(&encode_uint256(signature.len() as u128)?);
    bytes.extend_from_slice(&signature);
    bytes.resize(bytes.len() + (32 - signature.len() % 32) % 32, 0);
    Ok(EvmTransactionIntent {
        from: relayer.map(normalize_address).transpose()?,
        to: words.registry,
        value_wei: 0,
        data: format!("0x{}", hex::encode(bytes)),
        function: REGISTER_FUNCTION.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Anvil development key 3, the attester in the autonomous-v2 fixture capture.
    const ATTESTER_KEY: &str = "0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6";
    const ATTESTER: &str = "0x90f79bf6eb2c4f870365e785982e1f101e93b906";

    fn request() -> ParticipantAttestationRequest {
        ParticipantAttestationRequest {
            registry: "0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0".to_string(),
            wallet: "0x70997970c51812dc3a010c7d01b50e0d17dc79c8".to_string(),
            participant_id: word_hex(
                Keccak256::digest(b"agent-bounties/invoice-contractor-v1:ctr_42").into(),
            ),
            source_hash: word_hex(
                Keccak256::digest(b"agent-bounties/invoice-contractor-v1").into(),
            ),
            valid_until: 1_800_000_000,
            nonce: 0,
        }
    }

    fn sign(request: &ParticipantAttestationRequest, key: &str) -> String {
        let signer: PrivateKeySigner = key.parse().unwrap();
        let digest = participant_attestation_digest("base-sepolia", request).unwrap();
        let signature = signer.sign_hash_sync(&B256::from(digest)).unwrap();
        format!("0x{}", hex::encode(signature.as_bytes()))
    }

    /// `attestationDigest` read from a `ParticipantEligibilityRegistry` deployed on a local Anvil
    /// chain with chain id 84532 (first deployment from Anvil key 0, so its address is fixed).
    #[test]
    fn digest_equals_the_registry_contract() {
        let mut request = request();
        request.registry = "0x5FbDB2315678afecb367f032d93F642f64180aa3".to_string();
        assert_eq!(
            word_hex(participant_attestation_digest("base-sepolia", &request).unwrap()),
            "0x805c90b17de3c44841883f5a4e5d669dd341a0252c85281b58a99e5aeb664760"
        );
    }

    #[test]
    fn typed_data_matches_the_registry_domain_and_type() {
        let typed = plan_participant_attestation("base-sepolia", &request()).unwrap();
        assert_eq!(typed.domain.name, "Agent Bounties Participant Registry");
        assert_eq!(typed.domain.version, "1");
        assert_eq!(typed.domain.chain_id, 84_532);
        assert_eq!(
            typed.domain.verifying_contract,
            "0x9fe46736679d2d9a65f0992f2272de9f3c7fa6e0"
        );
        let fields: Vec<_> = typed.types["ParticipantAttestation"]
            .iter()
            .map(|field| format!("{} {}", field.field_type, field.name))
            .collect();
        assert_eq!(
            format!("ParticipantAttestation({})", fields.join(",")),
            PARTICIPANT_ATTESTATION_TYPE
        );
        assert_eq!(typed.message.valid_until, "1800000000");
    }

    #[test]
    fn registration_requires_the_registry_attester_signature() {
        let request = request();
        let signature = sign(&request, ATTESTER_KEY);
        assert_eq!(
            recover_participant_attester("base-sepolia", &request, &signature).unwrap(),
            ATTESTER
        );
        let plan =
            plan_participant_registration("base-sepolia", &request, &signature, ATTESTER, None)
                .unwrap();
        assert_eq!(plan.to, "0x9fe46736679d2d9a65f0992f2272de9f3c7fa6e0");
        assert!(plan
            .data
            .starts_with(&format!("0x{}", hex::encode(selector(REGISTER_FUNCTION)))));
        // 4 + 4 static words + offset + length + 65 bytes padded to 96.
        assert_eq!((plan.data.len() - 2) / 2, 4 + 32 * 6 + 96);

        let other = sign(
            &request,
            "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d",
        );
        assert!(
            plan_participant_registration("base-sepolia", &request, &other, ATTESTER, None)
                .is_err()
        );
        let mut renewed = request.clone();
        renewed.nonce = 1;
        assert!(
            plan_participant_registration("base-sepolia", &renewed, &signature, ATTESTER, None)
                .is_err(),
            "a signature never carries over to another nonce"
        );
        assert!(plan_participant_registration(
            "base-mainnet",
            &request,
            &signature,
            ATTESTER,
            None
        )
        .is_err());
        assert!(plan_participant_registration(
            "base-sepolia",
            &request,
            &signature[..130],
            ATTESTER,
            None
        )
        .is_err());
    }
}
