use chrono::{DateTime, Utc};
use domain::{EscrowStatus, Id, Money, PaymentRail};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha3::{Digest, Keccak256};
use std::{
    collections::{HashMap, HashSet},
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
pub struct BaseEscrowCreate {
    pub bounty_id: Id,
    pub payer: String,
    pub token: String,
    pub amount: Money,
    pub terms_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscrowRecipient {
    pub address: String,
    pub amount: Money,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseEscrowRelease {
    pub escrow_id: Id,
    pub recipients: Vec<EscrowRecipient>,
    pub proof_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseEscrowReleaseCall {
    pub onchain_escrow_id: u128,
    pub recipients: Vec<EscrowRecipient>,
    pub proof_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmTransactionIntent {
    pub from: Option<String>,
    pub to: String,
    pub value_wei: u128,
    pub data: String,
    pub function: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseEscrowFundingPlan {
    pub network: BaseNetworkDescriptor,
    pub approve: EvmTransactionIntent,
    pub create_escrow: EvmTransactionIntent,
}

#[derive(Debug, Clone)]
pub struct BaseEscrowTxPlanner {
    pub escrow_contract: String,
}

impl BaseEscrowTxPlanner {
    pub fn new(escrow_contract: impl Into<String>) -> Result<Self, ChainBaseError> {
        let escrow_contract = normalize_address(escrow_contract.into())?;
        Ok(Self { escrow_contract })
    }

    pub fn plan_funding(
        &self,
        create: &BaseEscrowCreate,
    ) -> Result<BaseEscrowFundingPlan, ChainBaseError> {
        self.plan_funding_for_network("base-sepolia", create)
    }

    pub fn plan_funding_for_network(
        &self,
        network: &str,
        create: &BaseEscrowCreate,
    ) -> Result<BaseEscrowFundingPlan, ChainBaseError> {
        let network = base_network_descriptor(network)?;
        let token = normalize_address(&create.token)?;
        let payer = normalize_address(&create.payer)?;
        let amount = money_to_uint256(&create.amount)?;
        let approve = EvmTransactionIntent {
            from: Some(payer),
            to: token.clone(),
            value_wei: 0,
            data: encode_call(
                "approve(address,uint256)",
                vec![
                    encode_address(&self.escrow_contract)?,
                    encode_uint256(amount)?,
                ],
            ),
            function: "approve(address,uint256)".to_string(),
        };
        let create_escrow = self.create_escrow(create)?;
        Ok(BaseEscrowFundingPlan {
            network,
            approve,
            create_escrow,
        })
    }

    pub fn create_escrow(
        &self,
        create: &BaseEscrowCreate,
    ) -> Result<EvmTransactionIntent, ChainBaseError> {
        let token = normalize_address(&create.token)?;
        let payer = normalize_address(&create.payer)?;
        let terms_hash = parse_bytes32(&create.terms_hash)?;
        let bounty_id = bytes32_from_uuid(create.bounty_id);
        let amount = money_to_uint256(&create.amount)?;
        Ok(EvmTransactionIntent {
            from: Some(payer),
            to: self.escrow_contract.clone(),
            value_wei: 0,
            data: encode_call(
                "createEscrow(bytes32,address,uint256,bytes32)",
                vec![
                    bounty_id,
                    encode_address(&token)?,
                    encode_uint256(amount)?,
                    terms_hash,
                ],
            ),
            function: "createEscrow(bytes32,address,uint256,bytes32)".to_string(),
        })
    }

    pub fn release(
        &self,
        release: &BaseEscrowReleaseCall,
    ) -> Result<EvmTransactionIntent, ChainBaseError> {
        if release.onchain_escrow_id == 0 {
            return Err(ChainBaseError::InvalidEscrowId);
        }
        validate_release_recipients(&release.recipients)?;
        let proof_hash = parse_bytes32(&release.proof_hash)?;
        let recipient_words = release
            .recipients
            .iter()
            .map(|recipient| encode_address(&recipient.address))
            .collect::<Result<Vec<_>, _>>()?;
        let amount_words = release
            .recipients
            .iter()
            .map(|recipient| money_to_uint256(&recipient.amount).and_then(encode_uint256))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(EvmTransactionIntent {
            from: None,
            to: self.escrow_contract.clone(),
            value_wei: 0,
            data: encode_dynamic_call(
                "release(uint256,address[],uint256[],bytes32)",
                encode_uint256(release.onchain_escrow_id)?,
                recipient_words,
                amount_words,
                proof_hash,
            ),
            function: "release(uint256,address[],uint256[],bytes32)".to_string(),
        })
    }

    pub fn refund(
        &self,
        onchain_escrow_id: u128,
        reason_hash: &str,
    ) -> Result<EvmTransactionIntent, ChainBaseError> {
        if onchain_escrow_id == 0 {
            return Err(ChainBaseError::InvalidEscrowId);
        }
        Ok(EvmTransactionIntent {
            from: None,
            to: self.escrow_contract.clone(),
            value_wei: 0,
            data: encode_call(
                "refund(uint256,bytes32)",
                vec![
                    encode_uint256(onchain_escrow_id)?,
                    parse_bytes32(reason_hash)?,
                ],
            ),
            function: "refund(uint256,bytes32)".to_string(),
        })
    }

    pub fn mark_disputed(
        &self,
        onchain_escrow_id: u128,
        dispute_hash: &str,
    ) -> Result<EvmTransactionIntent, ChainBaseError> {
        if onchain_escrow_id == 0 {
            return Err(ChainBaseError::InvalidEscrowId);
        }
        Ok(EvmTransactionIntent {
            from: None,
            to: self.escrow_contract.clone(),
            value_wei: 0,
            data: encode_call(
                "markDisputed(uint256,bytes32)",
                vec![
                    encode_uint256(onchain_escrow_id)?,
                    parse_bytes32(dispute_hash)?,
                ],
            ),
            function: "markDisputed(uint256,bytes32)".to_string(),
        })
    }
}

impl BaseEscrowRelease {
    pub fn validate_split(&self, total: &Money) -> Result<(), ChainBaseError> {
        let sum: i64 = self
            .recipients
            .iter()
            .filter(|recipient| recipient.amount.currency == total.currency)
            .map(|recipient| recipient.amount.amount)
            .sum();
        if sum != total.amount {
            return Err(ChainBaseError::InvalidReleaseSplit);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BaseEscrowEventKind {
    Created,
    Released,
    Refunded,
    Disputed,
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseEscrowEvent {
    pub id: Id,
    pub log_key: String,
    pub tx_hash: String,
    pub block_number: u64,
    pub onchain_escrow_id: u128,
    pub bounty_id: Id,
    pub kind: BaseEscrowEventKind,
    pub status: EscrowStatus,
    pub token: Option<String>,
    pub amount: Option<Money>,
    pub terms_hash: Option<String>,
    pub proof_hash: Option<String>,
    pub reason_hash: Option<String>,
    pub dispute_hash: Option<String>,
    pub occurred_at: DateTime<Utc>,
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
pub struct BaseEscrowLogQuery {
    pub escrow_contract: String,
    pub from_block: u64,
    pub to_block: Option<u64>,
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
pub struct EthGetLogsFilter {
    #[serde(rename = "fromBlock")]
    pub from_block: String,
    #[serde(rename = "toBlock")]
    pub to_block: String,
    pub address: String,
    pub topics: Vec<Vec<String>>,
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
            .map_err(|error| ChainBaseError::RpcTransport(error.to_string()))?;
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

pub async fn fetch_base_escrow_logs(
    rpc_url: &str,
    query: &BaseEscrowLogQuery,
    request_id: u64,
) -> Result<EthGetLogsResponse, ChainBaseError> {
    fetch_base_escrow_logs_with_transport(
        rpc_url,
        query,
        request_id,
        &ReqwestJsonRpcTransport::default(),
    )
    .await
}

pub async fn fetch_base_escrow_logs_with_transport<T>(
    rpc_url: &str,
    query: &BaseEscrowLogQuery,
    request_id: u64,
    transport: &T,
) -> Result<EthGetLogsResponse, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let request = query.rpc_request(request_id);
    let request_value = serde_json::to_value(&request)
        .map_err(|error| ChainBaseError::InvalidRpcResponse(error.to_string()))?;
    parse_eth_get_logs_response(transport.post_json_value(rpc_url, &request_value).await?)
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

impl BaseEscrowLogQuery {
    pub fn new(
        escrow_contract: impl Into<String>,
        from_block: u64,
        to_block: Option<u64>,
    ) -> Result<Self, ChainBaseError> {
        if let Some(to_block) = to_block {
            if from_block > to_block {
                return Err(ChainBaseError::InvalidBlockRange {
                    from_block,
                    to_block,
                });
            }
        }
        Ok(Self {
            escrow_contract: normalize_address(escrow_contract.into())?,
            from_block,
            to_block,
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
                address: self.escrow_contract.clone(),
                topics: vec![base_escrow_event_topics()],
            }],
        }
    }

    pub fn next_from_block(last_indexed_block: Option<u64>) -> u64 {
        last_indexed_block
            .and_then(|block| block.checked_add(1))
            .unwrap_or(0)
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

pub fn base_escrow_event_topics() -> Vec<String> {
    vec![
        event_topic("EscrowCreated(uint256,bytes32,address,address,uint256,bytes32)"),
        event_topic("EscrowReleased(uint256,bytes32)"),
        event_topic("EscrowRefunded(uint256,bytes32)"),
        event_topic("EscrowDisputed(uint256,bytes32)"),
    ]
}

#[derive(Debug, Clone)]
pub struct BaseEscrowLogDecoder {
    currency: String,
    escrow_bounties: HashMap<u128, Id>,
}

impl Default for BaseEscrowLogDecoder {
    fn default() -> Self {
        Self::new("usdc")
    }
}

impl BaseEscrowLogDecoder {
    pub fn new(currency: impl Into<String>) -> Self {
        Self {
            currency: currency.into().to_lowercase(),
            escrow_bounties: HashMap::new(),
        }
    }

    pub fn remember_event(&mut self, event: &BaseEscrowEvent) {
        if event.kind == BaseEscrowEventKind::Created {
            self.escrow_bounties
                .insert(event.onchain_escrow_id, event.bounty_id);
        }
    }

    pub fn decode(&mut self, log: EvmLog) -> Result<BaseEscrowEvent, ChainBaseError> {
        let topic0 = log
            .topics
            .first()
            .ok_or_else(|| ChainBaseError::InvalidLogTopics("missing topic0".to_string()))?;
        let signature = event_signature(topic0)
            .ok_or_else(|| ChainBaseError::UnknownEventTopic(topic0.clone()))?;
        match signature {
            EscrowEventSignature::Created => self.decode_created(log),
            EscrowEventSignature::Released => {
                self.decode_terminal(log, BaseEscrowEventKind::Released, EscrowStatus::Released)
            }
            EscrowEventSignature::Refunded => {
                self.decode_terminal(log, BaseEscrowEventKind::Refunded, EscrowStatus::Refunded)
            }
            EscrowEventSignature::Disputed => {
                self.decode_terminal(log, BaseEscrowEventKind::Disputed, EscrowStatus::Disputed)
            }
        }
    }

    fn decode_created(&mut self, log: EvmLog) -> Result<BaseEscrowEvent, ChainBaseError> {
        if log.topics.len() != 4 {
            return Err(ChainBaseError::InvalidLogTopics(
                "EscrowCreated".to_string(),
            ));
        }
        let escrow_id = word_to_u128(parse_bytes32(&log.topics[1])?)
            .map_err(|_| ChainBaseError::InvalidEscrowId)?;
        if escrow_id == 0 {
            return Err(ChainBaseError::InvalidEscrowId);
        }
        let bounty_id = uuid_from_bytes32(parse_bytes32(&log.topics[2])?);
        let words = decode_words(&log.data, 3, "EscrowCreated")?;
        let token = address_from_word(words[0]);
        let amount =
            i64::try_from(word_to_u128(words[1])?).map_err(|_| ChainBaseError::InvalidAmount)?;
        let terms_hash = word_hex(words[2]);
        self.escrow_bounties.insert(escrow_id, bounty_id);

        Ok(BaseEscrowEvent {
            id: deterministic_log_id(&log),
            log_key: log_key(&log),
            tx_hash: log.tx_hash,
            block_number: log.block_number,
            onchain_escrow_id: escrow_id,
            bounty_id,
            kind: BaseEscrowEventKind::Created,
            status: EscrowStatus::Funded,
            token: Some(token),
            amount: Some(
                Money::new(amount, self.currency.clone())
                    .map_err(|_| ChainBaseError::InvalidAmount)?,
            ),
            terms_hash: Some(terms_hash),
            proof_hash: None,
            reason_hash: None,
            dispute_hash: None,
            occurred_at: log.occurred_at.unwrap_or_else(Utc::now),
        })
    }

    fn decode_terminal(
        &mut self,
        log: EvmLog,
        kind: BaseEscrowEventKind,
        status: EscrowStatus,
    ) -> Result<BaseEscrowEvent, ChainBaseError> {
        if log.topics.len() != 2 {
            return Err(ChainBaseError::InvalidLogTopics(format!("{kind:?}")));
        }
        let escrow_id = word_to_u128(parse_bytes32(&log.topics[1])?)
            .map_err(|_| ChainBaseError::InvalidEscrowId)?;
        if escrow_id == 0 {
            return Err(ChainBaseError::InvalidEscrowId);
        }
        let bounty_id = *self
            .escrow_bounties
            .get(&escrow_id)
            .ok_or(ChainBaseError::UnknownEscrowForTerminalLog)?;
        let words = decode_words(&log.data, 1, &format!("{kind:?}"))?;
        let hash = word_hex(words[0]);
        let (proof_hash, reason_hash, dispute_hash) = match kind {
            BaseEscrowEventKind::Released => (Some(hash), None, None),
            BaseEscrowEventKind::Refunded => (None, Some(hash), None),
            BaseEscrowEventKind::Disputed => (None, None, Some(hash)),
            BaseEscrowEventKind::Created | BaseEscrowEventKind::Paused => (None, None, None),
        };

        Ok(BaseEscrowEvent {
            id: deterministic_log_id(&log),
            log_key: log_key(&log),
            tx_hash: log.tx_hash,
            block_number: log.block_number,
            onchain_escrow_id: escrow_id,
            bounty_id,
            kind,
            status,
            token: None,
            amount: None,
            terms_hash: None,
            proof_hash,
            reason_hash,
            dispute_hash,
            occurred_at: log.occurred_at.unwrap_or_else(Utc::now),
        })
    }
}

#[derive(Debug, Default)]
pub struct ChainEventIndexer {
    seen_log_keys: HashSet<String>,
    events: Vec<BaseEscrowEvent>,
}

impl ChainEventIndexer {
    pub fn from_events(
        events: impl IntoIterator<Item = BaseEscrowEvent>,
    ) -> Result<Self, ChainBaseError> {
        let mut indexer = Self::default();
        for event in events {
            indexer.ingest(event)?;
        }
        Ok(indexer)
    }

    pub fn has_seen_log_key(&self, log_key: &str) -> bool {
        self.seen_log_keys.contains(log_key)
    }

    pub fn ingest(&mut self, event: BaseEscrowEvent) -> Result<(), ChainBaseError> {
        if self.seen_log_keys.contains(&event.log_key) {
            return Err(ChainBaseError::DuplicateLog);
        }
        self.seen_log_keys.insert(event.log_key.clone());
        self.events.push(event);
        Ok(())
    }

    pub fn events(&self) -> &[BaseEscrowEvent] {
        &self.events
    }
}

pub trait WalletPolicy {
    fn can_sign_release(
        &self,
        bounty_id: Id,
        amount: &Money,
        recipients: &[EscrowRecipient],
    ) -> bool;
}

#[derive(Debug, Clone)]
pub struct LowValuePolicy {
    pub max_amount: i64,
    pub currency: String,
}

impl WalletPolicy for LowValuePolicy {
    fn can_sign_release(
        &self,
        _bounty_id: Id,
        amount: &Money,
        recipients: &[EscrowRecipient],
    ) -> bool {
        amount.currency == self.currency
            && amount.amount <= self.max_amount
            && !recipients.is_empty()
            && recipients
                .iter()
                .all(|recipient| recipient.amount.amount > 0)
    }
}

pub fn simulated_created_event(
    bounty_id: Id,
    onchain_escrow_id: u128,
    token: impl Into<String>,
    amount: Money,
    terms_hash: impl Into<String>,
) -> BaseEscrowEvent {
    BaseEscrowEvent {
        id: Uuid::new_v4(),
        log_key: format!("base:{onchain_escrow_id}:created"),
        tx_hash: format!("0x{}", Uuid::new_v4().simple()),
        block_number: 1,
        onchain_escrow_id,
        bounty_id,
        kind: BaseEscrowEventKind::Created,
        status: EscrowStatus::Funded,
        token: Some(token.into()),
        amount: Some(amount),
        terms_hash: Some(terms_hash.into()),
        proof_hash: None,
        reason_hash: None,
        dispute_hash: None,
        occurred_at: Utc::now(),
    }
}

pub fn simulated_released_event(
    bounty_id: Id,
    onchain_escrow_id: u128,
    proof_hash: impl Into<String>,
) -> BaseEscrowEvent {
    BaseEscrowEvent {
        id: Uuid::new_v4(),
        log_key: format!("base:{onchain_escrow_id}:released"),
        tx_hash: format!("0x{}", Uuid::new_v4().simple()),
        block_number: 2,
        onchain_escrow_id,
        bounty_id,
        kind: BaseEscrowEventKind::Released,
        status: EscrowStatus::Released,
        token: None,
        amount: None,
        terms_hash: None,
        proof_hash: Some(proof_hash.into()),
        reason_hash: None,
        dispute_hash: None,
        occurred_at: Utc::now(),
    }
}

pub fn simulated_refunded_event(
    bounty_id: Id,
    onchain_escrow_id: u128,
    reason_hash: impl Into<String>,
) -> BaseEscrowEvent {
    BaseEscrowEvent {
        id: Uuid::new_v4(),
        log_key: format!("base:{onchain_escrow_id}:refunded"),
        tx_hash: format!("0x{}", Uuid::new_v4().simple()),
        block_number: 2,
        onchain_escrow_id,
        bounty_id,
        kind: BaseEscrowEventKind::Refunded,
        status: EscrowStatus::Refunded,
        token: None,
        amount: None,
        terms_hash: None,
        proof_hash: None,
        reason_hash: Some(reason_hash.into()),
        dispute_hash: None,
        occurred_at: Utc::now(),
    }
}

pub fn simulated_disputed_event(
    bounty_id: Id,
    onchain_escrow_id: u128,
    dispute_hash: impl Into<String>,
) -> BaseEscrowEvent {
    BaseEscrowEvent {
        id: Uuid::new_v4(),
        log_key: format!("base:{onchain_escrow_id}:disputed"),
        tx_hash: format!("0x{}", Uuid::new_v4().simple()),
        block_number: 2,
        onchain_escrow_id,
        bounty_id,
        kind: BaseEscrowEventKind::Disputed,
        status: EscrowStatus::Disputed,
        token: None,
        amount: None,
        terms_hash: None,
        proof_hash: None,
        reason_hash: None,
        dispute_hash: Some(dispute_hash.into()),
        occurred_at: Utc::now(),
    }
}

pub fn base_usdc_rail() -> PaymentRail {
    PaymentRail::BaseUsdc
}

pub fn evm_event_topic(signature: &str) -> String {
    event_topic(signature)
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
enum EscrowEventSignature {
    Created,
    Released,
    Refunded,
    Disputed,
}

fn event_signature(topic: &str) -> Option<EscrowEventSignature> {
    let normalized = normalize_topic(topic).ok()?;
    if normalized == event_topic("EscrowCreated(uint256,bytes32,address,address,uint256,bytes32)") {
        Some(EscrowEventSignature::Created)
    } else if normalized == event_topic("EscrowReleased(uint256,bytes32)") {
        Some(EscrowEventSignature::Released)
    } else if normalized == event_topic("EscrowRefunded(uint256,bytes32)") {
        Some(EscrowEventSignature::Refunded)
    } else if normalized == event_topic("EscrowDisputed(uint256,bytes32)") {
        Some(EscrowEventSignature::Disputed)
    } else {
        None
    }
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

fn uuid_from_bytes32(word: [u8; 32]) -> Id {
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&word[16..]);
    Uuid::from_bytes(bytes)
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

fn validate_release_recipients(recipients: &[EscrowRecipient]) -> Result<(), ChainBaseError> {
    if recipients.is_empty() {
        return Err(ChainBaseError::EmptyRecipients);
    }
    let currency = &recipients[0].amount.currency;
    if recipients
        .iter()
        .any(|recipient| recipient.amount.currency != *currency)
    {
        return Err(ChainBaseError::MixedRecipientCurrencies);
    }
    Ok(())
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

fn bytes32_from_uuid(id: Id) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[16..].copy_from_slice(id.as_bytes());
    word
}

fn money_to_uint256(amount: &Money) -> Result<u128, ChainBaseError> {
    u128::try_from(amount.amount).map_err(|_| ChainBaseError::InvalidAmount)
}

fn encode_call(signature: &str, words: Vec<[u8; 32]>) -> String {
    let mut bytes = selector(signature).to_vec();
    for word in words {
        bytes.extend_from_slice(&word);
    }
    format!("0x{}", hex::encode(bytes))
}

fn encode_dynamic_call(
    signature: &str,
    escrow_id: [u8; 32],
    recipients: Vec<[u8; 32]>,
    amounts: Vec<[u8; 32]>,
    proof_hash: [u8; 32],
) -> String {
    let recipients_offset = 32u128 * 4;
    let amounts_offset = recipients_offset + 32 + (recipients.len() as u128 * 32);

    let mut bytes = selector(signature).to_vec();
    bytes.extend_from_slice(&escrow_id);
    bytes.extend_from_slice(&encode_uint256(recipients_offset).expect("constant fits"));
    bytes.extend_from_slice(&encode_uint256(amounts_offset).expect("constant fits"));
    bytes.extend_from_slice(&proof_hash);
    bytes.extend_from_slice(&encode_uint256(recipients.len() as u128).expect("length fits"));
    for recipient in recipients {
        bytes.extend_from_slice(&recipient);
    }
    bytes.extend_from_slice(&encode_uint256(amounts.len() as u128).expect("length fits"));
    for amount in amounts {
        bytes.extend_from_slice(&amount);
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

#[cfg(test)]
mod tests {
    use super::*;
    use domain::Money;
    use std::sync::{Arc, Mutex};

    #[test]
    fn duplicate_chain_logs_are_rejected() {
        let bounty_id = Uuid::new_v4();
        let event = simulated_created_event(
            bounty_id,
            1,
            "0x3333333333333333333333333333333333333333",
            Money::new(1_000_000, "usdc").unwrap(),
            format!("0x{}", "ab".repeat(32)),
        );
        let mut indexer = ChainEventIndexer::default();

        indexer.ingest(event.clone()).unwrap();
        assert_eq!(
            indexer.ingest(event).unwrap_err(),
            ChainBaseError::DuplicateLog
        );
    }

    #[test]
    fn release_split_must_match_escrow_total() {
        let release = BaseEscrowRelease {
            escrow_id: Uuid::new_v4(),
            proof_hash: "proof".to_string(),
            recipients: vec![
                EscrowRecipient {
                    address: "0xsolver".to_string(),
                    amount: Money::new(90, "usdc").unwrap(),
                },
                EscrowRecipient {
                    address: "0xplatform".to_string(),
                    amount: Money::new(10, "usdc").unwrap(),
                },
            ],
        };

        release
            .validate_split(&Money::new(100, "usdc").unwrap())
            .unwrap();
        assert_eq!(
            release
                .validate_split(&Money::new(101, "usdc").unwrap())
                .unwrap_err(),
            ChainBaseError::InvalidReleaseSplit
        );
    }

    #[test]
    fn function_selectors_match_solidity_contract() {
        assert_eq!(
            hex::encode(selector("createEscrow(bytes32,address,uint256,bytes32)")),
            "64a20554"
        );
        assert_eq!(
            hex::encode(selector("release(uint256,address[],uint256[],bytes32)")),
            "bfc95334"
        );
        assert_eq!(hex::encode(selector("refund(uint256,bytes32)")), "71eedb88");
        assert_eq!(
            hex::encode(selector("markDisputed(uint256,bytes32)")),
            "4dcc33b8"
        );
        assert_eq!(
            hex::encode(selector("approve(address,uint256)")),
            "095ea7b3"
        );
    }

    #[test]
    fn plans_funding_transactions_for_base_escrow() {
        let planner =
            BaseEscrowTxPlanner::new("0x1111111111111111111111111111111111111111").unwrap();
        let create = BaseEscrowCreate {
            bounty_id: Uuid::from_u128(42),
            payer: "0x2222222222222222222222222222222222222222".to_string(),
            token: "0x3333333333333333333333333333333333333333".to_string(),
            amount: Money::new(1_000_000, "usdc").unwrap(),
            terms_hash: format!("0x{}", "ab".repeat(32)),
        };

        let plan = planner.plan_funding(&create).unwrap();

        assert_eq!(plan.network.name, "Base Sepolia");
        assert_eq!(plan.network.chain_id, 84_532);
        assert_eq!(
            plan.network.native_usdc_token_address,
            BASE_SEPOLIA_USDC_TOKEN_ADDRESS
        );
        assert_eq!(plan.approve.from, Some(create.payer.clone()));
        assert_eq!(plan.approve.to, create.token);
        assert!(plan.approve.data.starts_with("0x095ea7b3"));
        assert_eq!(plan.create_escrow.from, Some(create.payer));
        assert_eq!(plan.create_escrow.to, planner.escrow_contract);
        assert!(plan.create_escrow.data.starts_with("0x64a20554"));
        assert!(plan.create_escrow.data.contains(&"ab".repeat(32)));
    }

    #[test]
    fn funding_plan_can_target_base_mainnet() {
        let planner =
            BaseEscrowTxPlanner::new("0x1111111111111111111111111111111111111111").unwrap();
        let create = BaseEscrowCreate {
            bounty_id: Uuid::from_u128(42),
            payer: "0x2222222222222222222222222222222222222222".to_string(),
            token: "0x3333333333333333333333333333333333333333".to_string(),
            amount: Money::new(1_000_000, "usdc").unwrap(),
            terms_hash: format!("0x{}", "ab".repeat(32)),
        };

        let plan = planner
            .plan_funding_for_network("base-mainnet", &create)
            .unwrap();

        assert_eq!(plan.network.name, "Base");
        assert_eq!(plan.network.chain_id, 8_453);
        assert_eq!(plan.network.rpc_url_env, "BASE_MAINNET_RPC_URL");
        assert_eq!(
            plan.network.native_usdc_token_address,
            BASE_MAINNET_USDC_TOKEN_ADDRESS
        );
        assert_eq!(plan.approve.to, create.token);
        assert_eq!(plan.create_escrow.to, planner.escrow_contract);
    }

    #[test]
    fn plans_release_with_dynamic_arrays() {
        let planner =
            BaseEscrowTxPlanner::new("0x1111111111111111111111111111111111111111").unwrap();
        let release = BaseEscrowReleaseCall {
            onchain_escrow_id: 7,
            proof_hash: format!("0x{}", "cd".repeat(32)),
            recipients: vec![
                EscrowRecipient {
                    address: "0x2222222222222222222222222222222222222222".to_string(),
                    amount: Money::new(900, "usdc").unwrap(),
                },
                EscrowRecipient {
                    address: "0x3333333333333333333333333333333333333333".to_string(),
                    amount: Money::new(100, "usdc").unwrap(),
                },
            ],
        };

        let tx = planner.release(&release).unwrap();

        assert_eq!(tx.to, planner.escrow_contract);
        assert!(tx.data.starts_with("0xbfc95334"));
        assert!(tx.data.contains(&"cd".repeat(32)));
        assert!(tx
            .data
            .contains("0000000000000000000000000000000000000000000000000000000000000080"));
        assert!(tx
            .data
            .contains("00000000000000000000000000000000000000000000000000000000000000e0"));
    }

    #[test]
    fn rejects_invalid_transaction_inputs() {
        assert_eq!(
            BaseEscrowTxPlanner::new("0x123").unwrap_err(),
            ChainBaseError::InvalidAddress("0x123".to_string())
        );

        let planner =
            BaseEscrowTxPlanner::new("0x1111111111111111111111111111111111111111").unwrap();
        let err = planner
            .refund(1, "not-a-bytes32")
            .expect_err("bad hash should fail");
        assert_eq!(
            err,
            ChainBaseError::InvalidBytes32("not-a-bytes32".to_string())
        );
        let err = planner
            .release(&BaseEscrowReleaseCall {
                onchain_escrow_id: 1,
                recipients: vec![],
                proof_hash: format!("0x{}", "00".repeat(32)),
            })
            .expect_err("empty recipients should fail");
        assert_eq!(err, ChainBaseError::EmptyRecipients);
    }

    #[test]
    fn plans_eth_get_logs_query_for_all_escrow_topics() {
        let query =
            BaseEscrowLogQuery::new("0x1111111111111111111111111111111111111111", 123, None)
                .unwrap();

        let request = query.rpc_request(42);
        let filter = &request.params[0];

        assert_eq!(request.jsonrpc, "2.0");
        assert_eq!(request.id, 42);
        assert_eq!(request.method, "eth_getLogs");
        assert_eq!(filter.from_block, "0x7b");
        assert_eq!(filter.to_block, "latest");
        assert_eq!(filter.address, "0x1111111111111111111111111111111111111111");
        assert_eq!(filter.topics.len(), 1);
        assert_eq!(filter.topics[0], base_escrow_event_topics());
        assert_eq!(filter.topics[0].len(), 4);
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
        let error =
            BaseEscrowLogQuery::new("0x1111111111111111111111111111111111111111", 200, Some(199))
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

    #[tokio::test]
    async fn fetches_base_logs_through_mock_transport() {
        let query =
            BaseEscrowLogQuery::new("0x1111111111111111111111111111111111111111", 10, Some(12))
                .unwrap();
        let seen_request = Arc::new(Mutex::new(None));
        let transport = MockTransport {
            seen_request: seen_request.clone(),
            response: serde_json::json!({
                "jsonrpc": "2.0",
                "id": 99,
                "result": [{
                    "address": "0x1111111111111111111111111111111111111111",
                    "topics": [event_topic("EscrowReleased(uint256,bytes32)")],
                    "data": format!("0x{}", "22".repeat(32)),
                    "transactionHash": format!("0x{}", "ab".repeat(32)),
                    "blockNumber": "0xc",
                    "logIndex": "0x1"
                }]
            }),
        };

        let response =
            fetch_base_escrow_logs_with_transport("https://rpc.example", &query, 99, &transport)
                .await
                .unwrap();

        assert_eq!(response.id, 99);
        assert_eq!(response.result.len(), 1);
        assert_eq!(response.result[0].block_number, "0xc");
        let seen = seen_request.lock().unwrap();
        let request = seen.as_ref().expect("request was captured");
        assert_eq!(request["id"], 99);
        assert_eq!(request["method"], "eth_getLogs");
        assert_eq!(request["params"][0]["fromBlock"], "0xa");
        assert_eq!(request["params"][0]["toBlock"], "0xc");
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

    #[test]
    fn decodes_created_and_released_evm_logs() {
        let bounty_id = Uuid::from_u128(42);
        let terms_hash = parse_bytes32(&format!("0x{}", "11".repeat(32))).unwrap();
        let proof_hash = parse_bytes32(&format!("0x{}", "22".repeat(32))).unwrap();
        let terms_hash_hex = word_hex(terms_hash);
        let proof_hash_hex = word_hex(proof_hash);
        let mut decoder = BaseEscrowLogDecoder::default();

        let created = decoder
            .decode(EvmLog {
                address: "0x1111111111111111111111111111111111111111".to_string(),
                topics: vec![
                    event_topic("EscrowCreated(uint256,bytes32,address,address,uint256,bytes32)"),
                    word_hex(encode_uint256(7).unwrap()),
                    word_hex(bytes32_from_uuid(bounty_id)),
                    word_hex(encode_address("0x2222222222222222222222222222222222222222").unwrap()),
                ],
                data: words_data(vec![
                    encode_address("0x3333333333333333333333333333333333333333").unwrap(),
                    encode_uint256(1_000_000).unwrap(),
                    terms_hash,
                ]),
                tx_hash: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
                block_number: 10,
                log_index: 0,
                occurred_at: None,
            })
            .unwrap();

        assert_eq!(created.kind, BaseEscrowEventKind::Created);
        assert_eq!(created.onchain_escrow_id, 7);
        assert_eq!(created.bounty_id, bounty_id);
        assert_eq!(
            created.token.as_deref(),
            Some("0x3333333333333333333333333333333333333333")
        );
        assert_eq!(created.amount, Some(Money::new(1_000_000, "usdc").unwrap()));
        assert_eq!(created.terms_hash.as_deref(), Some(terms_hash_hex.as_str()));

        let released = decoder
            .decode(EvmLog {
                address: "0x1111111111111111111111111111111111111111".to_string(),
                topics: vec![
                    event_topic("EscrowReleased(uint256,bytes32)"),
                    word_hex(encode_uint256(7).unwrap()),
                ],
                data: words_data(vec![proof_hash]),
                tx_hash: "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    .to_string(),
                block_number: 11,
                log_index: 0,
                occurred_at: None,
            })
            .unwrap();

        assert_eq!(released.kind, BaseEscrowEventKind::Released);
        assert_eq!(released.status, EscrowStatus::Released);
        assert_eq!(released.bounty_id, bounty_id);
        assert_eq!(
            released.proof_hash.as_deref(),
            Some(proof_hash_hex.as_str())
        );
    }

    #[test]
    fn terminal_log_requires_prior_created_log() {
        let proof_hash = parse_bytes32(&format!("0x{}", "22".repeat(32))).unwrap();
        let mut decoder = BaseEscrowLogDecoder::default();

        let err = decoder
            .decode(EvmLog {
                address: "0x1111111111111111111111111111111111111111".to_string(),
                topics: vec![
                    event_topic("EscrowReleased(uint256,bytes32)"),
                    word_hex(encode_uint256(7).unwrap()),
                ],
                data: words_data(vec![proof_hash]),
                tx_hash: "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    .to_string(),
                block_number: 11,
                log_index: 0,
                occurred_at: None,
            })
            .unwrap_err();

        assert_eq!(err, ChainBaseError::UnknownEscrowForTerminalLog);
    }

    fn words_data(words: Vec<[u8; 32]>) -> String {
        format!(
            "0x{}",
            words
                .into_iter()
                .map(hex::encode)
                .collect::<Vec<_>>()
                .join("")
        )
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
