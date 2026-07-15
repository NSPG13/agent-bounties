use super::{
    base_network_descriptor, encode_address, encode_call, fetch_contract_word, parse_bytes32,
    parse_rpc_quantity, rpc_result, ChainBaseError, JsonRpcTransport, ReqwestJsonRpcTransport,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;

pub const AGENT_WALLET_READINESS_SCHEMA: &str = "agent-bounties/agent-wallet-readiness-v1";

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
    pub claim_bond_base_units: String,
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
    pub wallet_address: String,
    pub bounty_contract: String,
    pub native_usdc_token: String,
    pub claim_bond_base_units: String,
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
    input: &PrepareAgentToEarnInput,
) -> Result<AgentWalletReadinessReport, ChainBaseError> {
    prepare_agent_to_earn_with_transport(rpc_url, input, &ReqwestJsonRpcTransport::default()).await
}

pub async fn prepare_agent_to_earn_with_transport<T>(
    rpc_url: &str,
    input: &PrepareAgentToEarnInput,
    transport: &T,
) -> Result<AgentWalletReadinessReport, ChainBaseError>
where
    T: JsonRpcTransport + ?Sized,
{
    let network = base_network_descriptor(&input.network)?;
    let wallet_address = super::normalize_evm_address(&input.wallet_address)?;
    let bounty_contract = super::normalize_evm_address(&input.bounty_contract)?;
    let claim_bond = positive_amount(&input.claim_bond_base_units)?;

    let observed_chain_id = fetch_chain_id(rpc_url, transport).await?;
    let observed_balance = fetch_erc20_balance_with_transport(
        rpc_url,
        &network.native_usdc_token_address,
        &wallet_address,
        transport,
    )
    .await?;

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
        "native_usdc_receive_compatibility",
        observed_chain_id == network.chain_id,
        format!(
            "canonical native USDC {} exposes balanceOf({wallet_address}); no transfer was attempted",
            network.native_usdc_token_address
        ),
        "a valid address on the requested Base network using canonical native USDC",
        "Switch the wallet and policy to the requested Base network.",
    ));
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
        wallet_address,
        bounty_contract,
        native_usdc_token: network.native_usdc_token_address,
        claim_bond_base_units: claim_bond.to_string(),
        observed_usdc_balance_base_units: observed_balance.to_string(),
        recommended_claim_path,
        wallet_profile: profile,
        checks,
        warnings,
        next_actions,
        evidence_boundary: "This report proves the RPC chain identity and the wallet's current canonical native-USDC balance. Signing capabilities, spend limits, allowlists, provider profile, and approval policy are caller declarations; this endpoint never requests a signature, private key, seed phrase, transfer, approval, or claim. Readiness is not claim ownership or payment. Only confirmed canonical BountyClaimed owns a round, and only BountySettled proves payout.".to_string(),
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
    fetch_erc20_balance_with_transport(rpc_url, token, wallet, &ReqwestJsonRpcTransport::default())
        .await
}

pub async fn fetch_erc20_balance_with_transport<T>(
    rpc_url: &str,
    token: &str,
    wallet: &str,
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
        "latest",
        2,
        transport,
    )
    .await?;
    let bytes = parse_bytes32(&word)?;
    if bytes[..16].iter().any(|byte| *byte != 0) {
        return Err(ChainBaseError::InvalidAmount);
    }
    Ok(u128::from_be_bytes(
        bytes[16..]
            .try_into()
            .map_err(|_| ChainBaseError::InvalidAmount)?,
    ))
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

    struct MockTransport {
        responses: Mutex<Vec<Value>>,
    }

    #[async_trait]
    impl JsonRpcTransport for MockTransport {
        async fn post_json_value(
            &self,
            _rpc_url: &str,
            _request: &Value,
        ) -> Result<Value, ChainBaseError> {
            Ok(self.responses.lock().unwrap().remove(0))
        }
    }

    fn input() -> PrepareAgentToEarnInput {
        PrepareAgentToEarnInput {
            network: "base-mainnet".to_string(),
            wallet_address: "0x1111111111111111111111111111111111111111".to_string(),
            bounty_contract: "0x2222222222222222222222222222222222222222".to_string(),
            claim_bond_base_units: "100000".to_string(),
            signing_capabilities: vec![
                "eip712_typed_data".to_string(),
                "eip3009_receive_with_authorization".to_string(),
            ],
            wallet_profile: Some("metamask-agent-wallet".to_string()),
            policy: AgentWalletPolicyDeclaration {
                allowed_chain_ids: vec![8453],
                allowed_contracts: vec![
                    "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913".to_string(),
                    "0x2222222222222222222222222222222222222222".to_string(),
                ],
                per_transaction_usdc_base_units: Some("200000".to_string()),
                rolling_24h_usdc_base_units: Some("500000".to_string()),
                human_approval_policy: Some("out_of_policy".to_string()),
            },
        }
    }

    fn transport(balance: u128, chain_id: u64) -> MockTransport {
        MockTransport {
            responses: Mutex::new(vec![
                json!({"jsonrpc":"2.0","id":1,"result":format!("0x{chain_id:x}")}),
                json!({"jsonrpc":"2.0","id":2,"result":format!("0x{balance:064x}")}),
            ]),
        }
    }

    #[tokio::test]
    async fn readiness_passes_for_bounded_agent_native_wallet() {
        let report = prepare_agent_to_earn_with_transport(
            "https://rpc.example",
            &input(),
            &transport(250_000, 8453),
        )
        .await
        .unwrap();

        assert!(report.ready);
        assert_eq!(report.status, "ready");
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
            &request,
            &transport(99_999, 8453),
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
        let report = prepare_agent_to_earn_with_transport(
            "https://rpc.example",
            &input(),
            &transport(250_000, 84532),
        )
        .await
        .unwrap();

        assert!(!report.ready);
        assert!(report
            .checks
            .iter()
            .any(|check| check.name == "base_network" && check.status == "fail"));
    }
}
