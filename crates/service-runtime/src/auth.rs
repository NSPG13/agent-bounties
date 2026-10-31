//! Shared caller identity. A wallet session proves one wallet, never a linked website account.
use alloy::primitives::{eip191_hash_message, Address, Signature};
use async_trait::async_trait;
use chrono::{DateTime, Duration, SecondsFormat, Timelike, Utc};
use db::{
    auth::{AuthStoreError, QuotaReservation, WalletChallenge},
    PostgresStore,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{str::FromStr, time::Duration as Timeout};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Principal {
    Anonymous,
    Operator,
    Account { account_id: String },
    Wallet { chain_id: u64, address: String },
}
impl Principal {
    pub fn key(&self) -> Option<String> {
        match self {
            Self::Anonymous => None,
            Self::Operator => Some("operator".into()),
            Self::Account { account_id } => Some(format!("account:{account_id}")),
            Self::Wallet { chain_id, address } => Some(format!(
                "wallet:{chain_id}:{}",
                address.to_ascii_lowercase()
            )),
        }
    }
    pub fn is_authenticated(&self) -> bool {
        !matches!(self, Self::Anonymous)
    }
}
#[derive(Debug, Error)]
pub enum AuthError {
    #[error("authentication required")]
    Unauthorized,
    #[error("authentication unavailable")]
    Unavailable,
    #[error("invalid or expired wallet challenge")]
    InvalidChallenge,
    #[error("invalid wallet signature")]
    InvalidSignature,
    #[error("invalid authentication request")]
    InvalidInput,
    #[error("idempotency key or owner conflicts")]
    Conflict,
    #[error("request limit exceeded")]
    QuotaExceeded,
}
impl From<AuthStoreError> for AuthError {
    fn from(e: AuthStoreError) -> Self {
        match e {
            AuthStoreError::Unavailable(_) => Self::Unavailable,
            AuthStoreError::InvalidChallenge => Self::InvalidChallenge,
            AuthStoreError::Conflict => Self::Conflict,
            AuthStoreError::QuotaExceeded => Self::QuotaExceeded,
            AuthStoreError::InvalidInput => Self::InvalidInput,
        }
    }
}
pub fn hash_secret(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
pub fn normalize_wallet(value: &str) -> Result<String, AuthError> {
    let address = Address::from_str(value).map_err(|_| AuthError::InvalidInput)?;
    if address == Address::ZERO {
        return Err(AuthError::InvalidInput);
    }
    Ok(format!("{address:#x}"))
}
fn new_secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

pub async fn authenticate_principal(
    store: Option<&PostgresStore>,
    operator: Option<&str>,
    direct: Option<&str>,
    authorization: Option<&str>,
    account_id: Option<&str>,
) -> Result<Principal, AuthError> {
    if crate::operator_token_is_authorized(operator, direct, authorization) {
        return Ok(Principal::Operator);
    }
    // Never let an invalid supplied credential silently fall back to a broader cookie identity.
    if direct.is_some() {
        return Err(AuthError::Unauthorized);
    }
    if let Some(auth) = authorization {
        let token = auth
            .strip_prefix("Bearer abws_")
            .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or(AuthError::Unauthorized)?;
        let session = store
            .ok_or(AuthError::Unavailable)?
            .find_auth_session(&hash_secret(&format!("abws_{token}")))
            .await?
            .ok_or(AuthError::Unauthorized)?;
        if let (Some(challenge), Some(signature)) =
            (&session.challenge, &session.contract_signature)
        {
            let network = match session.chain_id {
                8453 => "base-mainnet",
                84532 => "base-sepolia",
                _ => return Err(AuthError::Unauthorized),
            };
            let (_, url) = chain_base::BaseRpcUrlConfig::from_env()
                .resolve(network)
                .map_err(|_| AuthError::Unavailable)?;
            if RpcWalletVerifier::new(url, session.chain_id)?
                .verify(challenge, signature)
                .await?
                != WalletProofKind::Contract
            {
                return Err(AuthError::Unauthorized);
            }
        }
        return Ok(Principal::Wallet {
            chain_id: session.chain_id,
            address: session.address,
        });
    }
    if let Some(id) =
        account_id.filter(|id| id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Ok(Principal::Account {
            account_id: id.to_string(),
        });
    }
    Ok(Principal::Anonymous)
}
pub async fn authorize_resource(
    store: Option<&PostgresStore>,
    principal: &Principal,
    kind: &str,
    id: &str,
    permission: &str,
) -> Result<bool, AuthError> {
    if matches!(principal, Principal::Operator) {
        return Ok(true);
    }
    let Some(key) = principal.key() else {
        return Ok(false);
    };
    Ok(store
        .ok_or(AuthError::Unavailable)?
        .resource_is_authorized(kind, id, &key, permission)
        .await?)
}
pub async fn assign_resource_owner(
    store: &PostgresStore,
    principal: &Principal,
    kind: &str,
    id: &str,
) -> Result<(), AuthError> {
    let key = principal.key().ok_or(AuthError::Unauthorized)?;
    store
        .claim_resource_ownership(kind, id, &key)
        .await
        .map_err(Into::into)
}
pub async fn authorize_wallet(
    store: Option<&PostgresStore>,
    principal: &Principal,
    chain_id: u64,
    address: &str,
) -> Result<bool, AuthError> {
    let address = normalize_wallet(address)?;
    match principal {
        // Administrative resource access is not proof of control over a payout/solver wallet.
        Principal::Operator | Principal::Anonymous => Ok(false),
        Principal::Wallet {
            chain_id: own_chain,
            address: own_address,
        } => Ok(*own_chain == chain_id && own_address.eq_ignore_ascii_case(&address)),
        Principal::Account { account_id } => {
            let wallets = store
                .ok_or(AuthError::Unavailable)?
                .list_site_auth_wallets(account_id)
                .await
                .map_err(|_| AuthError::Unavailable)?;
            Ok(wallets.iter().any(|wallet| {
                wallet.chain_id == chain_id as i64 && wallet.address.eq_ignore_ascii_case(&address)
            }))
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub enum QuotaAction {
    Registration,
    Publication,
    Admission,
}
impl QuotaAction {
    fn policy(self) -> (&'static str, i64, i64) {
        match self {
            Self::Registration => ("registration", 5, 86400),
            Self::Publication => ("publication", 3, 86400),
            Self::Admission => ("admission", 10, 60),
        }
    }
}
pub async fn consume_quota_once(
    store: Option<&PostgresStore>,
    principal: &Principal,
    action: QuotaAction,
    idempotency_key: &str,
    payload_hash: &str,
) -> Result<QuotaReservation, AuthError> {
    let key = principal.key().ok_or(AuthError::Unauthorized)?;
    if idempotency_key.trim().is_empty() || idempotency_key.len() > 200 {
        return Err(AuthError::InvalidInput);
    }
    let (name, limit, window) = action.policy();
    store
        .ok_or(AuthError::Unavailable)?
        .consume_security_quota_once(
            &key,
            name,
            &hash_secret(idempotency_key),
            payload_hash,
            limit,
            window,
        )
        .await
        .map_err(Into::into)
}
#[derive(Debug, Clone, Serialize)]
pub struct ChallengeResponse {
    pub challenge_id: Uuid,
    pub message: String,
    pub expires_at: DateTime<Utc>,
    pub chain_id: u64,
    pub address: String,
}
#[derive(Serialize)]
pub struct SessionResponse {
    pub token: String,
    pub token_type: &'static str,
    pub expires_at: DateTime<Utc>,
    pub principal: Principal,
}
#[derive(Clone)]
pub struct AuthService {
    store: PostgresStore,
    domain: String,
    uri: String,
    chain_id: u64,
}
impl AuthService {
    pub fn new(store: PostgresStore, origin: &str, chain_id: u64) -> Result<Self, AuthError> {
        let parsed = url::Url::parse(origin).map_err(|_| AuthError::InvalidInput)?;
        if !matches!(chain_id, 8453 | 84532)
            || !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || parsed.path() != "/"
        {
            return Err(AuthError::InvalidInput);
        }
        if parsed.scheme() != "https"
            && !matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
        {
            return Err(AuthError::InvalidInput);
        }
        let domain = match parsed.port() {
            Some(port) => format!("{}:{port}", parsed.host_str().unwrap()),
            None => parsed.host_str().unwrap().to_string(),
        };
        Ok(Self {
            store,
            domain,
            uri: format!("{}/v1/auth/wallet/session", origin.trim_end_matches('/')),
            chain_id,
        })
    }
    pub async fn begin_challenge(
        &self,
        address: &str,
        ip_bucket: &str,
    ) -> Result<ChallengeResponse, AuthError> {
        let address = normalize_wallet(address)?;
        let id = Uuid::new_v4();
        let request_hash = hash_secret(&id.to_string());
        self.store
            .consume_security_quota_once(
                &format!(
                    "ip:{}",
                    hash_secret(&format!("{}:{ip_bucket}", self.domain))
                ),
                "nonce_ip",
                &request_hash,
                &request_hash,
                20,
                60,
            )
            .await?;
        self.store
            .consume_security_quota_once(
                &format!("wallet:{}:{address}", self.chain_id),
                "nonce_wallet",
                &request_hash,
                &request_hash,
                10,
                60,
            )
            .await?;
        let now = Utc::now()
            .with_nanosecond(0)
            .expect("zero nanoseconds valid");
        let expires = now + Duration::minutes(5);
        let nonce = new_secret();
        let checksum = Address::from_str(&address)
            .map_err(|_| AuthError::InvalidInput)?
            .to_checksum(None);
        let message=format!("{} wants you to sign in with your Ethereum account:\n{}\n\nSign in to Agentizable for 15 minutes. This does not link a website account or authorize a transaction, token approval, or payment.\n\nURI: {}\nVersion: 1\nChain ID: {}\nNonce: {}\nIssued At: {}\nExpiration Time: {}",self.domain,checksum,self.uri,self.chain_id,nonce,now.to_rfc3339_opts(SecondsFormat::Secs,true),(now+Duration::minutes(15)).to_rfc3339_opts(SecondsFormat::Secs,true));
        let record = WalletChallenge {
            id,
            domain: self.domain.clone(),
            chain_id: self.chain_id,
            address: address.clone(),
            message: message.clone(),
            created_at: now,
            expires_at: expires,
        };
        self.store.prune_security_auth_state().await?;
        self.store.create_wallet_challenge(&record).await?;
        Ok(ChallengeResponse {
            challenge_id: id,
            message,
            expires_at: expires,
            chain_id: self.chain_id,
            address,
        })
    }
    pub async fn complete_challenge(
        &self,
        id: Uuid,
        signature: &str,
        verifier: &dyn WalletSignatureVerifier,
    ) -> Result<SessionResponse, AuthError> {
        if signature.len() > 16386 || !signature.starts_with("0x") {
            return Err(AuthError::InvalidSignature);
        }
        let challenge = self.store.wallet_challenge_attempt(id).await?;
        if challenge.domain != self.domain
            || challenge.chain_id != self.chain_id
            || challenge.expires_at <= Utc::now()
        {
            return Err(AuthError::InvalidChallenge);
        }
        let proof_kind = verifier.verify(&challenge, signature).await?;
        let token = format!("abws_{}", new_secret());
        let session = self
            .store
            .consume_wallet_challenge_and_create_session(
                id,
                &hash_secret(&token),
                if proof_kind == WalletProofKind::Contract {
                    Some(signature)
                } else {
                    None
                },
            )
            .await?;
        Ok(SessionResponse {
            token,
            token_type: "Bearer",
            expires_at: session.expires_at,
            principal: Principal::Wallet {
                chain_id: session.chain_id,
                address: session.address,
            },
        })
    }
    pub async fn revoke(&self, authorization: &str) -> Result<(), AuthError> {
        let token = authorization
            .strip_prefix("Bearer abws_")
            .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or(AuthError::Unauthorized)?;
        self.store
            .revoke_auth_session(&hash_secret(&format!("abws_{token}")))
            .await
            .map_err(Into::into)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletProofKind {
    Eoa,
    Contract,
}
#[async_trait]
pub trait WalletSignatureVerifier: Send + Sync {
    async fn verify(
        &self,
        challenge: &WalletChallenge,
        signature: &str,
    ) -> Result<WalletProofKind, AuthError>;
}
#[derive(Clone)]
pub struct RpcWalletVerifier {
    client: reqwest::Client,
    url: String,
    chain_id: u64,
}
impl RpcWalletVerifier {
    pub fn new(url: String, chain_id: u64) -> Result<Self, AuthError> {
        let parsed = url::Url::parse(&url).map_err(|_| AuthError::InvalidInput)?;
        if !matches!(parsed.scheme(), "https" | "http") {
            return Err(AuthError::InvalidInput);
        }
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Timeout::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| AuthError::Unavailable)?,
            url,
            chain_id,
        })
    }
    async fn rpc(&self, method: &str, params: Value) -> Result<Value, AuthError> {
        let mut response = self
            .client
            .post(&self.url)
            .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
            .send()
            .await
            .map_err(|_| AuthError::Unavailable)?;
        if !response.status().is_success() {
            return Err(AuthError::Unavailable);
        }
        let mut data = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| AuthError::Unavailable)? {
            if data.len() + chunk.len() > 262144 {
                return Err(AuthError::Unavailable);
            }
            data.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&data).map_err(|_| AuthError::Unavailable)?;
        if value["jsonrpc"] != "2.0" || value["id"] != 1 || value.get("error").is_some() {
            return Err(AuthError::Unavailable);
        }
        value.get("result").cloned().ok_or(AuthError::Unavailable)
    }
}
#[async_trait]
impl WalletSignatureVerifier for RpcWalletVerifier {
    async fn verify(
        &self,
        challenge: &WalletChallenge,
        signature: &str,
    ) -> Result<WalletProofKind, AuthError> {
        if challenge.chain_id != self.chain_id {
            return Err(AuthError::InvalidChallenge);
        }
        let chain = self.rpc("eth_chainId", json!([])).await?;
        let observed = u64::from_str_radix(
            chain
                .as_str()
                .and_then(|s| s.strip_prefix("0x"))
                .ok_or(AuthError::Unavailable)?,
            16,
        )
        .map_err(|_| AuthError::Unavailable)?;
        if observed != self.chain_id {
            return Err(AuthError::Unavailable);
        }
        let block = self
            .rpc("eth_getBlockByNumber", json!(["safe", false]))
            .await?;
        let block_hash = block["hash"]
            .as_str()
            .filter(|s| s.len() == 66 && s.starts_with("0x"))
            .ok_or(AuthError::Unavailable)?;
        let block_ref = json!({"blockHash":block_hash,"requireCanonical":true});
        let code = self
            .rpc("eth_getCode", json!([challenge.address, block_ref]))
            .await?;
        let code = code
            .as_str()
            .and_then(|s| s.strip_prefix("0x"))
            .ok_or(AuthError::Unavailable)?;
        let code = hex::decode(code).map_err(|_| AuthError::Unavailable)?;
        if code.is_empty() {
            return verify_eoa_message(&challenge.message, signature, &challenge.address)
                .map(|_| WalletProofKind::Eoa);
        }
        // EIP-1271 at the same canonical safe block. Undeployed/counterfactual wallets are never accepted by magic wrappers.
        let signature_bytes = hex::decode(
            signature
                .strip_prefix("0x")
                .ok_or(AuthError::InvalidSignature)?,
        )
        .map_err(|_| AuthError::InvalidSignature)?;
        if signature_bytes.is_empty() || signature_bytes.len() > 8192 {
            return Err(AuthError::InvalidSignature);
        }
        let digest = eip191_hash_message(challenge.message.as_bytes());
        let mut calldata = vec![0x16, 0x26, 0xba, 0x7e];
        calldata.extend_from_slice(digest.as_slice());
        let mut offset = [0u8; 32];
        offset[31] = 64;
        calldata.extend_from_slice(&offset);
        let mut len = [0u8; 32];
        len[24..].copy_from_slice(&(signature_bytes.len() as u64).to_be_bytes());
        calldata.extend_from_slice(&len);
        calldata.extend_from_slice(&signature_bytes);
        calldata.resize(4 + 96 + signature_bytes.len().div_ceil(32) * 32, 0);
        let result=self.rpc("eth_call",json!([{"to":challenge.address,"data":format!("0x{}",hex::encode(calldata)),"gas":"0x7a120"},block_ref])).await?;
        let returned = result
            .as_str()
            .and_then(|s| s.strip_prefix("0x"))
            .and_then(|s| hex::decode(s).ok())
            .ok_or(AuthError::InvalidSignature)?;
        if returned.len() != 32
            || returned[..4] != [0x16, 0x26, 0xba, 0x7e]
            || returned[4..].iter().any(|b| *b != 0)
        {
            return Err(AuthError::InvalidSignature);
        }
        Ok(WalletProofKind::Contract)
    }
}
pub fn verify_eoa_message(message: &str, signature: &str, address: &str) -> Result<(), AuthError> {
    let sig = Signature::from_str(signature).map_err(|_| AuthError::InvalidSignature)?;
    if sig.normalize_s().is_some() {
        return Err(AuthError::InvalidSignature);
    }
    let signer = sig
        .recover_address_from_msg(message.as_bytes())
        .map_err(|_| AuthError::InvalidSignature)?;
    if format!("{signer:#x}") != normalize_wallet(address)? {
        return Err(AuthError::InvalidSignature);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::signers::{local::PrivateKeySigner, SignerSync};
    #[test]
    fn eoa_signature_is_bound_to_message_and_wallet() {
        let signer = PrivateKeySigner::random();
        let message = "localhost wants you to sign in";
        let signature = signer
            .sign_message_sync(message.as_bytes())
            .unwrap()
            .to_string();
        let signature = format!("0x{}", signature.trim_start_matches("0x"));
        assert!(
            verify_eoa_message(message, &signature, &format!("{:#x}", signer.address())).is_ok()
        );
        assert!(verify_eoa_message(
            "different nonce",
            &signature,
            &format!("{:#x}", signer.address())
        )
        .is_err());
        assert!(verify_eoa_message(
            message,
            &signature,
            "0x1111111111111111111111111111111111111111"
        )
        .is_err());
    }
    #[test]
    fn scopes_and_missing_operator_fail_closed() {
        assert_ne!(
            Principal::Account {
                account_id: "a".repeat(64)
            }
            .key(),
            Principal::Wallet {
                chain_id: 8453,
                address: "0x1111111111111111111111111111111111111111".into()
            }
            .key()
        );
        assert!(!crate::operator_token_is_authorized(None, None, None));
        assert!(!crate::operator_token_is_authorized(Some(""), None, None));
        assert!(normalize_wallet("0x0000000000000000000000000000000000000000").is_err());
    }

    #[tokio::test]
    async fn operator_resource_authority_cannot_impersonate_a_wallet() {
        let principal = authenticate_principal(
            None,
            Some("local-operator"),
            Some("local-operator"),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(
            authorize_resource(None, &principal, "bounty", "local-fixture", "write")
                .await
                .unwrap()
        );
        let address = "0x1111111111111111111111111111111111111111";
        assert!(!authorize_wallet(None, &principal, 8453, address)
            .await
            .unwrap());
        let wallet = Principal::Wallet {
            chain_id: 8453,
            address: address.into(),
        };
        assert!(authorize_wallet(None, &wallet, 8453, address)
            .await
            .unwrap());
        assert!(!authorize_wallet(None, &wallet, 84532, address)
            .await
            .unwrap());
        assert!(!authorize_wallet(
            None,
            &wallet,
            8453,
            "0x2222222222222222222222222222222222222222"
        )
        .await
        .unwrap());
    }
}

#[cfg(test)]
mod rpc_tests {
    use super::*;
    use std::io::{Read, Write};
    fn mock_rpc(results: Vec<Value>) -> (String, std::thread::JoinHandle<Vec<Value>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let join = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for result in results {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Timeout::from_secs(5)))
                    .unwrap();
                let mut data = Vec::new();
                let mut byte = [0u8; 1];
                while !data.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    data.push(byte[0]);
                    assert!(data.len() < 10000);
                }
                let header = String::from_utf8(data).unwrap();
                let length: usize = header
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .map(str::to_owned)
                    })
                    .unwrap()
                    .parse()
                    .unwrap();
                let mut data = vec![0; length];
                stream.read_exact(&mut data).unwrap();
                requests.push(serde_json::from_slice(&data).unwrap());
                let body = json!({"jsonrpc":"2.0","id":1,"result":result}).to_string();
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            }
            requests
        });
        (format!("http://{address}"), join)
    }
    fn challenge() -> WalletChallenge {
        WalletChallenge {
            id: Uuid::new_v4(),
            domain: "localhost".into(),
            chain_id: 8453,
            address: "0x1111111111111111111111111111111111111111".into(),
            message: "synthetic nonce, domain, address and expiry".into(),
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::minutes(5),
        }
    }
    #[tokio::test]
    async fn deployed_contract_checks_chain_safe_block_and_exact_magic() {
        let block = json!({"hash":format!("0x{}","ab".repeat(32))});
        let magic = format!("0x1626ba7e{}", "00".repeat(28));
        let (url, join) = mock_rpc(vec![
            json!("0x2105"),
            block.clone(),
            json!("0x6000"),
            json!(magic),
        ]);
        assert_eq!(
            RpcWalletVerifier::new(url, 8453)
                .unwrap()
                .verify(&challenge(), "0x1234")
                .await
                .unwrap(),
            WalletProofKind::Contract
        );
        let requests = join.join().unwrap();
        assert_eq!(requests[2]["params"][1], requests[3]["params"][1]);
        assert_eq!(requests[3]["params"][0]["gas"], "0x7a120");
        assert!(requests[3]["params"][0]["data"]
            .as_str()
            .unwrap()
            .starts_with("0x1626ba7e"));
        let (url, join) = mock_rpc(vec![
            json!("0x2105"),
            block,
            json!("0x6000"),
            json!("0xffffffff"),
        ]);
        assert!(RpcWalletVerifier::new(url, 8453)
            .unwrap()
            .verify(&challenge(), "0x1234")
            .await
            .is_err());
        join.join().unwrap();
        let (url, join) = mock_rpc(vec![json!("0x1")]);
        assert!(RpcWalletVerifier::new(url, 8453)
            .unwrap()
            .verify(&challenge(), "0x1234")
            .await
            .is_err());
        join.join().unwrap();
    }
}

#[cfg(test)]
mod durable_tests {
    use super::*;
    use alloy::signers::{local::PrivateKeySigner, SignerSync};
    struct LocalEoa;
    #[async_trait]
    impl WalletSignatureVerifier for LocalEoa {
        async fn verify(&self, c: &WalletChallenge, s: &str) -> Result<WalletProofKind, AuthError> {
            verify_eoa_message(&c.message, s, &c.address).map(|_| WalletProofKind::Eoa)
        }
    }
    #[tokio::test]
    #[ignore = "requires isolated SECURITY_AUTH_TEST_DATABASE_URL on localhost"]
    async fn siwe_exchange_replay_domain_scopes_and_revocation() {
        let url = std::env::var("SECURITY_AUTH_TEST_DATABASE_URL").unwrap();
        db::test_support::assert_isolated_security_database(&url);
        let store = PostgresStore::connect(&url).await.unwrap();
        store.migrate().await.unwrap();
        let service = AuthService::new(store.clone(), "http://localhost:18083", 8453).unwrap();
        let signer = PrivateKeySigner::random();
        let address = format!("{:#x}", signer.address());
        let challenge = service
            .begin_challenge(&address, &Uuid::new_v4().to_string())
            .await
            .unwrap();
        assert!(challenge
            .message
            .contains("localhost:18083 wants you to sign in"));
        assert!(challenge.message.contains("Chain ID: 8453"));
        assert!(challenge.expires_at <= Utc::now() + Duration::minutes(5));
        let signature = signer
            .sign_message_sync(challenge.message.as_bytes())
            .unwrap()
            .to_string();
        let signature = format!("0x{}", signature.trim_start_matches("0x"));
        let different_domain =
            AuthService::new(store.clone(), "http://localhost:18084", 8453).unwrap();
        assert!(matches!(
            different_domain
                .complete_challenge(challenge.challenge_id, &signature, &LocalEoa)
                .await,
            Err(AuthError::InvalidChallenge)
        ));
        let session = service
            .complete_challenge(challenge.challenge_id, &signature, &LocalEoa)
            .await
            .unwrap();
        assert!(matches!(
            service
                .complete_challenge(challenge.challenge_id, &signature, &LocalEoa)
                .await,
            Err(AuthError::InvalidChallenge)
        ));
        let header = format!("Bearer {}", session.token);
        let account = "a".repeat(64);
        let principal = authenticate_principal(
            Some(&store),
            Some("operator-fixture"),
            None,
            Some(&header),
            Some(&account),
        )
        .await
        .unwrap();
        assert_eq!(
            principal,
            Principal::Wallet {
                chain_id: 8453,
                address: address.clone()
            }
        );
        assert!(authorize_wallet(Some(&store), &principal, 8453, &address)
            .await
            .unwrap());
        assert!(!authorize_wallet(Some(&store), &principal, 84532, &address)
            .await
            .unwrap());
        assert!(
            !authorize_resource(Some(&store), &principal, "account", &account, "read")
                .await
                .unwrap()
        );
        assert_eq!(
            authenticate_principal(Some(&store), None, None, None, Some(&account))
                .await
                .unwrap(),
            Principal::Account {
                account_id: account
            }
        );
        assert!(
            authenticate_principal(Some(&store), None, None, Some("Bearer bad"), None)
                .await
                .is_err()
        );
        service.revoke(&header).await.unwrap();
        assert!(
            authenticate_principal(Some(&store), None, None, Some(&header), None)
                .await
                .is_err()
        );
    }
}
