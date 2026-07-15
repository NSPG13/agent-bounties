use super::{
    base_network_descriptor, encode_address, encode_call, fetch_block_number_with_transport,
    fetch_contract_word, parse_bytes32, parse_rpc_quantity, rpc_result, ChainBaseError,
    JsonRpcTransport, ReqwestJsonRpcTransport, AUTONOMOUS_BOUNTY_PROTOCOL_HASH,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;

pub const AGENT_WALLET_READINESS_SCHEMA: &str = "agent-bounties/agent-wallet-readiness-v1";
const CLAIMABLE_BOUNTY_STATUS: u8 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentWalletPolicyDeclaration {
    #[serde(default)]
    pub allowed_chain_ids: Vec<u64>,
    #[serde(default)]
    pub allowed_contracts: Vec<String>,
    pub per_transaction_usdc_base_units: Option<String>,
    pub rolling_24h_usdc_base_units: Option<String>,
    pub human_approval_policy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareAgentToEarnInput {
    pub network: String,
    pub wallet_address: String,
    pub bounty_contract: String,
    #[serde(default)]
    pub claim_bond_base_units: Option<String>,
    #[serde(default)]
    pub signing_capabilities: Vec<String>,
    pub wallet_profile: Option<String>,
    #[serde(default)]
    pub policy: AgentWalletPolicyDeclaration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWalletReadinessCheck {
    pub name: String,
    pub status: String,
    pub evidence: String,
    pub required: String,
    pub next_action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWalletProfileRecognition {
    pub requested: String,
    pub recognized: bool,
    pub profile: String,
    pub label: String,
    pub guidance: Vec<String>,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWalletReadinessReport {
    pub schema_version: String,
    pub ready: bool,
    pub status: String,
    pub network: super::BaseNetworkDescriptor,
    pub observed_block_number: u64,
    pub wallet_address: String,
    pub bounty_contract: String,
    pub canonical_factory: String,
    pub creator_wallet: String,
    pub onchain_bounty_status: String,
    pub native_usdc_token: String,
    pub claim_bond_base_units: String,
    pub requested_claim_bond_base_units: Option<String>,
    pub observed_usdc_balance_base_units: String,
    pub recommended_claim_path: Option<String>,
    pub wallet_profile: AgentWalletProfileRecognition,
    pub checks: Vec<AgentWalletReadinessCheck>,
    pub warnings: Vec<String>,
    pub next_actions: Vec<String>,
    pub evidence_boundary: String,
}

pub async fn prepare_agent_to_earn(
    rpc_url: &str,
    canonical_factory: &str,
    input: &PrepareAgentToEarnInput,
) -> Result<AgentWalletReadinessReport, ChainBaseError> {
    prepare_agent_to_earn_with_transport(
        rpc_url,
        canonical_factory,
        input,
        &ReqwestJsonRpcTransport::default(),
    )
    .await
}

pub async fn prepare_agent_to_earn_with_transport<T>(
    rpc_url: &str,
    canonical_factory: &str,
    input: &PrepareAgentToEarnInput,
    transport: &T,
) -> Result<AgentWalletReadinessReport, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let network = base_network_descriptor(&input.network)?;
    let wallet_address = super::normalize_evm_address(&input.wallet_address)?;
    let bounty_contract = super::normalize_evm_address(&input.bounty_contract)?;
    let canonical_factory = super::normalize_evm_address(canonical_factory)?;
    let requested_claim_bond = input
        .claim_bond_base_units
        .as_deref()
        .map(positive_amount)
        .transpose()?;

    let observed_chain_id = fetch_chain_id(rpc_url, transport).await?;
    if observed_chain_id != network.chain_id {
        return Err(ChainBaseError::RelayerChainMismatch {
            expected: network.chain_id,
            observed: observed_chain_id,
        });
    }
    let observed_block_number = fetch_block_number_with_transport(rpc_url, 2, transport).await?;
    let block_tag = format!("0x{observed_block_number:x}");
    let observed_balance = fetch_erc20_balance_at_with_transport(
        rpc_url,
        &network.native_usdc_token_address,
        &wallet_address,
        &block_tag,
        3,
        transport,
    )
    .await?;
    let canonical_word = fetch_contract_word(
        rpc_url,
        &canonical_factory,
        &encode_call(
            "isCanonicalBounty(address)",
            vec![encode_address(&bounty_contract)?],
        ),
        &block_tag,
        4,
        transport,
    )
    .await?;
    if parse_word_u128(&canonical_word)? != 1 {
        return Err(ChainBaseError::InvalidAddress(
            "bounty contract is not registered by the configured canonical factory".to_string(),
        ));
    }

    let bounty_factory = fetch_address_getter(
        rpc_url,
        &bounty_contract,
        "factory()",
        &block_tag,
        5,
        transport,
    )
    .await?;
    let settlement_token = fetch_address_getter(
        rpc_url,
        &bounty_contract,
        "settlementToken()",
        &block_tag,
        6,
        transport,
    )
    .await?;
    let creator_wallet = fetch_address_getter(
        rpc_url,
        &bounty_contract,
        "creator()",
        &block_tag,
        7,
        transport,
    )
    .await?;
    let claim_bond = fetch_u128_getter(
        rpc_url,
        &bounty_contract,
        "verifierReward()",
        &block_tag,
        8,
        transport,
    )
    .await?;
    if claim_bond == 0 {
        return Err(ChainBaseError::InvalidAmount);
    }
    let bounty_status = fetch_u128_getter(
        rpc_url,
        &bounty_contract,
        "status()",
        &block_tag,
        9,
        transport,
    )
    .await?;
    let bounty_status = u8::try_from(bounty_status).map_err(|_| {
        ChainBaseError::InvalidRpcResponse("bounty status does not fit uint8".to_string())
    })?;
    let protocol_hash = fetch_contract_word(
        rpc_url,
        &bounty_contract,
        &encode_call("protocolVersion()", Vec::new()),
        &block_tag,
        10,
        transport,
    )
    .await?
    .to_ascii_lowercase();

    let mut checks = Vec::new();
    checks.push(check(
        "base_network",
        observed_chain_id == network.chain_id,
        format!("RPC eth_chainId returned {observed_chain_id}"),
        format!("chain ID {}", network.chain_id),
        "Use the RPC endpoint for the requested Base network.",
    ));
    checks.push(check(
        "base_compatible_address",
        true,
        format!("{wallet_address} is a normalized 20-byte EVM address"),
        "one valid Base-compatible EVM address",
        "Provide a public Base wallet address, never a private key or seed phrase.",
    ));
    checks.push(check(
        "canonical_bounty",
        true,
        format!(
            "factory {canonical_factory} registered {bounty_contract} at block {observed_block_number}"
        ),
        "the configured factory must register the bounty contract",
        "Choose a bounty from the canonical earning feed.",
    ));
    checks.push(check(
        "protocol_version",
        protocol_hash.eq_ignore_ascii_case(AUTONOMOUS_BOUNTY_PROTOCOL_HASH),
        format!("bounty protocolVersion() returned {protocol_hash}"),
        format!("protocol hash {AUTONOMOUS_BOUNTY_PROTOCOL_HASH}"),
        "Choose an autonomous-v1 canonical bounty.",
    ));
    checks.push(check(
        "factory_binding",
        bounty_factory == canonical_factory,
        format!("bounty factory() returned {bounty_factory}"),
        format!("factory {canonical_factory}"),
        "Reject the contract and choose one whose immutable factory matches discovery.",
    ));
    checks.push(check(
        "settlement_token",
        settlement_token.eq_ignore_ascii_case(&network.native_usdc_token_address),
        format!("bounty settlementToken() returned {settlement_token}"),
        format!(
            "canonical native USDC {}",
            network.native_usdc_token_address
        ),
        "Reject the contract and choose a native-USDC bounty from canonical inventory.",
    ));
    checks.push(check(
        "bounty_claimable",
        bounty_status == CLAIMABLE_BOUNTY_STATUS,
        format!(
            "bounty status() returned {}",
            bounty_status_label(bounty_status)
        ),
        "Claimable (status 1)",
        "Refresh earning inventory and choose a currently claimable bounty.",
    ));
    checks.push(check(
        "solver_not_creator",
        wallet_address != creator_wallet,
        format!("bounty creator() returned {creator_wallet}"),
        "solver wallet must differ from the creator wallet",
        "Use an independently controlled solver wallet; the creator cannot claim its own bounty.",
    ));
    checks.push(check(
        "native_usdc_receive_compatibility",
        settlement_token.eq_ignore_ascii_case(&network.native_usdc_token_address),
        format!(
            "canonical native USDC {} exposes balanceOf({wallet_address}) at block {observed_block_number}; no transfer was attempted",
            network.native_usdc_token_address,
        ),
        "a valid address on the requested Base network using canonical native USDC",
        "Switch the wallet and policy to the requested Base network.",
    ));
    if let Some(requested) = requested_claim_bond {
        checks.push(check(
            "requested_bond_matches_chain",
            requested == claim_bond,
            format!("caller expected {requested}; bounty verifierReward() returned {claim_bond}"),
            "the optional expected bond must match the on-chain verifier reward",
            "Discard stale inventory and use the bond derived by this report.",
        ));
    }
    checks.push(check(
        "claim_bond_balance",
        observed_balance >= claim_bond,
        format!("observed {observed_balance} USDC base units"),
        format!("at least {claim_bond} USDC base units"),
        "Fund the solver wallet with the exact shortfall or request capped bond sponsorship before signing a claim.",
    ));

    let capabilities = input
        .signing_capabilities
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>();
    let agent_native = capabilities.contains("eip712_typed_data")
        && capabilities.contains("eip3009_receive_with_authorization");
    let direct_wallet =
        capabilities.contains("wallet_send_calls") || capabilities.contains("send_transaction");
    let recommended_claim_path = if agent_native {
        Some("agent_native_claim".to_string())
    } else if direct_wallet {
        Some("direct_wallet_claim_plan".to_string())
    } else {
        None
    };
    checks.push(check(
        "required_signing_capability",
        recommended_claim_path.is_some(),
        if capabilities.is_empty() {
            "no signing capabilities declared".to_string()
        } else {
            format!("declared capabilities: {}", capabilities.iter().cloned().collect::<Vec<_>>().join(", "))
        },
        "EIP-712 plus EIP-3009 for agent_native_claim, or transaction signing for the direct-wallet fallback",
        "Configure a wallet that can sign the exact typed authorization, or use a transaction-signing wallet with the direct claim plan.",
    ));

    let per_transaction = optional_amount(input.policy.per_transaction_usdc_base_units.as_deref());
    let rolling_24h = optional_amount(input.policy.rolling_24h_usdc_base_units.as_deref());
    let limits_sufficient = per_transaction.is_some_and(|amount| amount >= claim_bond)
        && rolling_24h.is_some_and(|amount| amount >= claim_bond);
    checks.push(check(
        "spend_limits",
        limits_sufficient,
        format!(
            "per-transaction={}, rolling-24h={}",
            display_amount(per_transaction),
            display_amount(rolling_24h)
        ),
        format!("both declared caps must allow the {claim_bond}-base-unit claim bond"),
        "Set explicit per-transaction and rolling-24-hour USDC caps at or above this bond, while keeping them no broader than the owner intends.",
    ));

    let allowed_contracts = input
        .policy
        .allowed_contracts
        .iter()
        .filter_map(|value| super::normalize_evm_address(value).ok())
        .collect::<BTreeSet<_>>();
    let usdc_allowed =
        allowed_contracts.contains(&network.native_usdc_token_address.to_ascii_lowercase());
    let bounty_allowed = allowed_contracts.contains(&bounty_contract);
    checks.push(check(
        "contract_allowlist",
        usdc_allowed && bounty_allowed,
        format!("{} valid contract addresses declared", allowed_contracts.len()),
        format!(
            "allow canonical native USDC {} and bounty contract {bounty_contract}",
            network.native_usdc_token_address
        ),
        "Add only the canonical USDC token and intended bounty contract to the wallet's contract or protocol allowlist.",
    ));

    let chain_allowed = input.policy.allowed_chain_ids.contains(&network.chain_id);
    checks.push(check(
        "chain_allowlist",
        chain_allowed,
        format!("declared chain IDs: {:?}", input.policy.allowed_chain_ids),
        format!("include Base chain ID {}", network.chain_id),
        "Add the requested Base chain ID to the wallet policy.",
    ));

    let approval_policy = input
        .policy
        .human_approval_policy
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase);
    let approval_explicit = approval_policy
        .as_deref()
        .is_some_and(|value| matches!(value, "always" | "out_of_policy" | "never"));
    checks.push(check(
        "human_approval_policy",
        approval_explicit,
        approval_policy
            .clone()
            .unwrap_or_else(|| "not declared".to_string()),
        "one explicit policy: always, out_of_policy, or never",
        "Declare when a human must approve. out_of_policy is the recommended bounded-autonomy setting.",
    ));

    let profile = recognize_wallet_profile(input.wallet_profile.as_deref());
    let mut warnings = Vec::new();
    if !profile.recognized {
        warnings.push(
            "The declared wallet profile is unknown; compatibility is evaluated from capabilities and policy, not provider identity."
                .to_string(),
        );
    }
    match approval_policy.as_deref() {
        Some("always") => warnings.push(
            "This wallet is compatible but every claim may pause for a human; use out_of_policy only when the owner wants bounded autonomy."
                .to_string(),
        ),
        Some("never") => warnings.push(
            "No human escalation is configured. Keep caps and allowlists narrow and use a separate low-value agent wallet."
                .to_string(),
        ),
        _ => {}
    }
    if !agent_native && direct_wallet {
        warnings.push(
            "The wallet can use the direct claim fallback, but the one-signature agent-native path requires EIP-712 and EIP-3009 support."
                .to_string(),
        );
    }

    let next_actions = checks
        .iter()
        .filter(|item| item.status == "fail")
        .filter_map(|item| item.next_action.clone())
        .collect::<Vec<_>>();
    let ready = checks.iter().all(|item| item.status == "pass");
    Ok(AgentWalletReadinessReport {
        schema_version: AGENT_WALLET_READINESS_SCHEMA.to_string(),
        ready,
        status: if ready { "ready" } else { "blocked" }.to_string(),
        network: network.clone(),
        observed_block_number,
        wallet_address,
        bounty_contract,
        canonical_factory,
        creator_wallet,
        onchain_bounty_status: bounty_status_label(bounty_status),
        native_usdc_token: network.native_usdc_token_address,
        claim_bond_base_units: claim_bond.to_string(),
        requested_claim_bond_base_units: requested_claim_bond.map(|value| value.to_string()),
        observed_usdc_balance_base_units: observed_balance.to_string(),
        recommended_claim_path,
        wallet_profile: profile,
        checks,
        warnings,
        next_actions,
        evidence_boundary: format!("This report proves chain identity, canonical factory registration, immutable bounty bindings, claimable status, creator exclusion, the on-chain claim bond, and the wallet's canonical native-USDC balance at Base block {observed_block_number}. Signing capabilities, spend limits, allowlists, provider profile, and approval policy are caller declarations; this endpoint never requests a signature, private key, seed phrase, transfer, approval, or claim. State may change after the observed block. Only confirmed canonical BountyClaimed owns a round, and only BountySettled proves payout."),
    })
}

async fn fetch_chain_id<T>(rpc_url: &str, transport: &T) -> Result<u64, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let result = rpc_result(
        transport
            .post_json_value(
                rpc_url,
                &json!({"jsonrpc": "2.0", "id": 1, "method": "eth_chainId", "params": []}),
            )
            .await?,
        1,
        "eth_chainId",
    )?;
    parse_rpc_quantity(result.as_str().ok_or_else(|| {
        ChainBaseError::InvalidRpcResponse("eth_chainId result is not a quantity".to_string())
    })?)
}

pub async fn fetch_erc20_balance(
    rpc_url: &str,
    token: &str,
    wallet: &str,
) -> Result<u128, ChainBaseError> {
    fetch_erc20_balance_at_with_transport(
        rpc_url,
        token,
        wallet,
        "latest",
        2,
        &ReqwestJsonRpcTransport::default(),
    )
    .await
}

async fn fetch_erc20_balance_at_with_transport<T>(
    rpc_url: &str,
    token: &str,
    wallet: &str,
    block_tag: &str,
    request_id: u64,
    transport: &T,
) -> Result<u128, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let token = super::normalize_evm_address(token)?;
    let wallet = super::normalize_evm_address(wallet)?;
    let word = fetch_contract_word(
        rpc_url,
        &token,
        &encode_call("balanceOf(address)", vec![encode_address(&wallet)?]),
        block_tag,
        request_id,
        transport,
    )
    .await?;
    parse_word_u128(&word)
}

async fn fetch_address_getter<T>(
    rpc_url: &str,
    contract: &str,
    function: &str,
    block_tag: &str,
    request_id: u64,
    transport: &T,
) -> Result<String, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let word = fetch_contract_word(
        rpc_url,
        contract,
        &encode_call(function, Vec::new()),
        block_tag,
        request_id,
        transport,
    )
    .await?;
    let bytes = parse_bytes32(&word)?;
    if bytes[..12].iter().any(|byte| *byte != 0) {
        return Err(ChainBaseError::InvalidRpcResponse(format!(
            "{function} did not return an ABI address"
        )));
    }
    super::normalize_evm_address(format!("0x{}", hex::encode(&bytes[12..])))
}

async fn fetch_u128_getter<T>(
    rpc_url: &str,
    contract: &str,
    function: &str,
    block_tag: &str,
    request_id: u64,
    transport: &T,
) -> Result<u128, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let word = fetch_contract_word(
        rpc_url,
        contract,
        &encode_call(function, Vec::new()),
        block_tag,
        request_id,
        transport,
    )
    .await?;
    parse_word_u128(&word)
}

fn parse_word_u128(word: &str) -> Result<u128, ChainBaseError> {
    let bytes = parse_bytes32(word)?;
    if bytes[..16].iter().any(|byte| *byte != 0) {
        return Err(ChainBaseError::InvalidAmount);
    }
    Ok(u128::from_be_bytes(
        bytes[16..]
            .try_into()
            .map_err(|_| ChainBaseError::InvalidAmount)?,
    ))
}

fn bounty_status_label(status: u8) -> String {
    match status {
        0 => "open (0)".to_string(),
        1 => "claimable (1)".to_string(),
        2 => "claimed (2)".to_string(),
        3 => "submitted (3)".to_string(),
        4 => "settled (4)".to_string(),
        5 => "cancelled (5)".to_string(),
        _ => format!("unknown ({status})"),
    }
}

fn positive_amount(value: &str) -> Result<u128, ChainBaseError> {
    value
        .trim()
        .parse::<u128>()
        .ok()
        .filter(|amount| *amount > 0)
        .ok_or(ChainBaseError::InvalidAmount)
}

fn optional_amount(value: Option<&str>) -> Option<u128> {
    value.and_then(|item| item.trim().parse::<u128>().ok())
}

fn display_amount(value: Option<u128>) -> String {
    value
        .map(|amount| amount.to_string())
        .unwrap_or_else(|| "missing or invalid".to_string())
}

fn check(
    name: &str,
    passed: bool,
    evidence: String,
    required: impl Into<String>,
    next_action: &str,
) -> AgentWalletReadinessCheck {
    AgentWalletReadinessCheck {
        name: name.to_string(),
        status: if passed { "pass" } else { "fail" }.to_string(),
        evidence,
        required: required.into(),
        next_action: (!passed).then(|| next_action.to_string()),
    }
}

fn recognize_wallet_profile(requested: Option<&str>) -> AgentWalletProfileRecognition {
    let requested = requested
        .unwrap_or("generic-evm")
        .trim()
        .to_ascii_lowercase();
    let (recognized, profile, label, guidance): (bool, &str, &str, Vec<&str>) = match requested.as_str() {
        "metamask-agent-wallet" | "metamask_agent_wallet" => (
            true,
            "metamask-agent-wallet",
            "MetaMask Agent Wallet",
            vec![
                "Use Base in the Agent Wallet CLI.",
                "Use Guard Mode or equivalent limits with the bounty and native USDC allowlisted.",
                "Confirm the CLI supports the exact EIP-712/EIP-3009 payload before selecting agent_native_claim.",
            ],
        ),
        "circle-agent-wallet" | "circle_agent_wallet" => (
            true,
            "circle-agent-wallet",
            "Circle Agent Wallet",
            vec![
                "Enable Base and native USDC.",
                "Set global and per-service limits plus chain and contract allowlists.",
                "Use contract execution or typed-data support exposed by the current wallet configuration.",
            ],
        ),
        "cdp-server-wallet" | "cdp_server_wallet" => (
            true,
            "cdp-server-wallet",
            "CDP Server Wallet",
            vec![
                "Pin Base chain ID and canonical contracts in wallet policy.",
                "Expose only the signing operations required by the selected claim path.",
            ],
        ),
        "privy-server-wallet" | "privy_server_wallet" => (
            true,
            "privy-server-wallet",
            "Privy Server Wallet",
            vec![
                "Use a dedicated low-value wallet and server-side policy controls.",
                "Declare actual typed-data and transaction-signing support explicitly.",
            ],
        ),
        "generic-evm" | "generic_evm" | "external-eoa" => (
            true,
            "generic-evm",
            "Generic EVM wallet",
            vec![
                "Use a dedicated Base wallet with narrow spend caps.",
                "Declare only signing capabilities the wallet actually exposes to the agent.",
            ],
        ),
        _ => (
            false,
            "generic-evm",
            "Unrecognized wallet profile",
            vec![
                "Compatibility is determined from the declared capabilities and policy, not the provider name.",
                "Use wallet_profile=generic-evm unless a documented profile matches.",
            ],
        ),
    };
    AgentWalletProfileRecognition {
        requested,
        recognized,
        profile: profile.to_string(),
        label: label.to_string(),
        guidance: guidance.into_iter().map(str::to_string).collect(),
        evidence: "Provider recognition uses the caller-declared wallet_profile only; Agent Bounties never infers custody provider from an address.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::Value;
    use std::sync::Mutex;

    const FACTORY: &str = "0x3333333333333333333333333333333333333333";
    const CREATOR: &str = "0x4444444444444444444444444444444444444444";
    const USDC: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";

    struct MockTransport {
        responses: Mutex<Vec<Value>>,
        requests: Mutex<Vec<Value>>,
    }

    #[async_trait]
    impl JsonRpcTransport for MockTransport {
        async fn post_json_value(
            &self,
            _rpc_url: &str,
            request: &Value,
        ) -> Result<Value, ChainBaseError> {
            self.requests.lock().unwrap().push(request.clone());
            Ok(self.responses.lock().unwrap().remove(0))
        }
    }

    fn input() -> PrepareAgentToEarnInput {
        PrepareAgentToEarnInput {
            network: "base-mainnet".to_string(),
            wallet_address: "0x1111111111111111111111111111111111111111".to_string(),
            bounty_contract: "0x2222222222222222222222222222222222222222".to_string(),
            claim_bond_base_units: Some("100000".to_string()),
            signing_capabilities: vec![
                "eip712_typed_data".to_string(),
                "eip3009_receive_with_authorization".to_string(),
            ],
            wallet_profile: Some("metamask-agent-wallet".to_string()),
            policy: AgentWalletPolicyDeclaration {
                allowed_chain_ids: vec![8453],
                allowed_contracts: vec![
                    USDC.to_string(),
                    "0x2222222222222222222222222222222222222222".to_string(),
                ],
                per_transaction_usdc_base_units: Some("200000".to_string()),
                rolling_24h_usdc_base_units: Some("500000".to_string()),
                human_approval_policy: Some("out_of_policy".to_string()),
            },
        }
    }

    fn address_word(address: &str) -> String {
        format!("0x{}{}", "0".repeat(24), &address[2..].to_ascii_lowercase())
    }

    fn transport(
        balance: u128,
        chain_id: u64,
        status: u8,
        creator: &str,
        canonical: bool,
        claim_bond: u128,
    ) -> MockTransport {
        MockTransport {
            responses: Mutex::new(vec![
                json!({"jsonrpc":"2.0","id":1,"result":format!("0x{chain_id:x}")}),
                json!({"jsonrpc":"2.0","id":2,"result":"0xabc"}),
                json!({"jsonrpc":"2.0","id":3,"result":format!("0x{balance:064x}")}),
                json!({"jsonrpc":"2.0","id":4,"result":format!("0x{:064x}", u8::from(canonical))}),
                json!({"jsonrpc":"2.0","id":5,"result":address_word(FACTORY)}),
                json!({"jsonrpc":"2.0","id":6,"result":address_word(USDC)}),
                json!({"jsonrpc":"2.0","id":7,"result":address_word(creator)}),
                json!({"jsonrpc":"2.0","id":8,"result":format!("0x{claim_bond:064x}")}),
                json!({"jsonrpc":"2.0","id":9,"result":format!("0x{status:064x}")}),
                json!({"jsonrpc":"2.0","id":10,"result":AUTONOMOUS_BOUNTY_PROTOCOL_HASH}),
            ]),
            requests: Mutex::new(Vec::new()),
        }
    }

    #[tokio::test]
    async fn readiness_passes_for_bounded_agent_native_wallet() {
        let transport = transport(250_000, 8453, 1, CREATOR, true, 100_000);
        let report = prepare_agent_to_earn_with_transport(
            "https://rpc.example",
            FACTORY,
            &input(),
            &transport,
        )
        .await
        .unwrap();

        assert!(report.ready);
        assert_eq!(report.status, "ready");
        assert_eq!(report.observed_block_number, 0xabc);
        assert_eq!(report.claim_bond_base_units, "100000");
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 10);
        assert_eq!(requests[0]["method"], "eth_chainId");
        assert_eq!(requests[1]["method"], "eth_blockNumber");
        for request in &requests[2..] {
            assert_eq!(request["method"], "eth_call");
            assert_eq!(request["params"][1], "0xabc");
        }
        assert_eq!(
            report.recommended_claim_path.as_deref(),
            Some("agent_native_claim")
        );
        assert!(report.checks.iter().all(|check| check.status == "pass"));
        assert!(report.evidence_boundary.contains("caller declarations"));
    }

    #[tokio::test]
    async fn readiness_fails_closed_for_low_balance_and_missing_policy() {
        let mut request = input();
        request.policy = AgentWalletPolicyDeclaration::default();
        request.signing_capabilities.clear();
        let report = prepare_agent_to_earn_with_transport(
            "https://rpc.example",
            FACTORY,
            &request,
            &transport(99_999, 8453, 1, CREATOR, true, 100_000),
        )
        .await
        .unwrap();

        assert!(!report.ready);
        assert_eq!(report.status, "blocked");
        for name in [
            "claim_bond_balance",
            "required_signing_capability",
            "spend_limits",
            "contract_allowlist",
            "chain_allowlist",
            "human_approval_policy",
        ] {
            assert!(report
                .checks
                .iter()
                .any(|check| check.name == name && check.status == "fail"));
        }
        assert!(!report.next_actions.is_empty());
    }

    #[tokio::test]
    async fn readiness_rejects_rpc_on_wrong_chain() {
        let error = prepare_agent_to_earn_with_transport(
            "https://rpc.example",
            FACTORY,
            &input(),
            &transport(250_000, 84532, 1, CREATOR, true, 100_000),
        )
        .await
        .unwrap_err();

        assert_eq!(
            error,
            ChainBaseError::RelayerChainMismatch {
                expected: 8453,
                observed: 84532,
            }
        );
    }

    #[tokio::test]
    async fn readiness_blocks_creator_nonclaimable_and_stale_bond() {
        let mut request = input();
        request.wallet_address = CREATOR.to_string();
        request.claim_bond_base_units = Some("99999".to_string());
        let report = prepare_agent_to_earn_with_transport(
            "https://rpc.example",
            FACTORY,
            &request,
            &transport(250_000, 8453, 2, CREATOR, true, 100_000),
        )
        .await
        .unwrap();

        assert!(!report.ready);
        for name in [
            "bounty_claimable",
            "solver_not_creator",
            "requested_bond_matches_chain",
        ] {
            assert!(report
                .checks
                .iter()
                .any(|check| check.name == name && check.status == "fail"));
        }
    }

    #[tokio::test]
    async fn readiness_rejects_noncanonical_contract_before_trusting_getters() {
        let error = prepare_agent_to_earn_with_transport(
            "https://rpc.example",
            FACTORY,
            &input(),
            &transport(250_000, 8453, 1, CREATOR, false, 100_000),
        )
        .await
        .unwrap_err();

        assert!(matches!(
            error,
            ChainBaseError::InvalidAddress(message) if message.contains("not registered")
        ));
    }
}
