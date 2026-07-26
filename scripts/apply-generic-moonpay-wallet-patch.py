#!/usr/bin/env python3
"""Generalize the MoonPay checkout context from an existing bounty to wallet onboarding.

A user must be able to acquire Base USDC before a bounty contract exists. The
checkout therefore carries an optional bounty contract while preserving legacy
funding evidence fields and adding action-neutral evidence fields. The patch is
idempotent even when a new replacement intentionally contains its old suffix.
"""

from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PATH = ROOT / "crates" / "mcp-server" / "src" / "moonpay.rs"

REPLACEMENTS = (
    (
        '''    #[serde(default)]
    intent_id: Option<Uuid>,
    bounty_contract: String,
}''',
        '''    #[serde(default)]
    intent_id: Option<Uuid>,
    #[serde(default)]
    bounty_contract: Option<String>,
}''',
        "checkout request optional bounty",
    ),
    (
        '''    base_currency_amount: String,
    bounty_contract: String,
    intent_id: Option<Uuid>,
    external_transaction_id: String,
    checkout_url: String,
    bounty_funded: bool,
    canonical_funding_event: Option<String>,''',
        '''    base_currency_amount: String,
    bounty_contract: Option<String>,
    intent_id: Option<Uuid>,
    external_transaction_id: String,
    checkout_url: String,
    protocol_action_completed: bool,
    canonical_event: Option<String>,
    bounty_funded: bool,
    canonical_funding_event: Option<String>,''',
        "checkout plan generic evidence",
    ),
    (
        '''    return_url: String,
    intent_id: Option<Uuid>,
    bounty_contract: String,
}''',
        '''    return_url: String,
    intent_id: Option<Uuid>,
    bounty_contract: Option<String>,
}''',
        "validated request optional bounty",
    ),
    (
        '''                "next_action": self.next_action,
                "bounty_funded": false,
                "canonical_funding_event": null,''',
        '''                "next_action": self.next_action,
                "protocol_action_completed": false,
                "canonical_event": null,
                "bounty_funded": false,
                "canonical_funding_event": null,''',
        "error response generic evidence",
    ),
    (
        '''    let wallet_address = normalize_evm_address(&request.wallet_address, "wallet_address")?;
    let bounty_contract = normalize_evm_address(&request.bounty_contract, "bounty_contract")?;
    let (amount_minor, base_currency_amount) = parse_fiat_amount(&request.base_currency_amount)?;''',
        '''    let wallet_address = normalize_evm_address(&request.wallet_address, "wallet_address")?;
    let bounty_contract = request
        .bounty_contract
        .as_deref()
        .map(|value| normalize_evm_address(value, "bounty_contract"))
        .transpose()?;
    let (amount_minor, base_currency_amount) = parse_fiat_amount(&request.base_currency_amount)?;''',
        "optional bounty validation",
    ),
    (
        '''        external_transaction_id,
        checkout_url: checkout.to_string(),
        bounty_funded: false,
        canonical_funding_event: None,
        next_action: "Complete the MoonPay checkout, return to Agent Bounties, verify the wallet balance on Base, and separately approve the exact canonical bounty contribution.",
        evidence_boundary: "MoonPay checkout status and wallet top-up evidence are not bounty-funding evidence. Only the matching indexed canonical FundingAdded event changes the bounty's funded state.",''',
        '''        external_transaction_id,
        checkout_url: checkout.to_string(),
        protocol_action_completed: false,
        canonical_event: None,
        bounty_funded: false,
        canonical_funding_event: None,
        next_action: "Complete the MoonPay checkout, return to Agent Bounties, verify the Base USDC balance, and separately approve the exact original bounty action.",
        evidence_boundary: "MoonPay checkout status and wallet top-up evidence do not complete any Agent Bounties action. Only the matching indexed canonical protocol event changes bounty state.",''',
        "generic checkout evidence boundary",
    ),
    (
        '''            intent_id: Some(Uuid::parse_str("9e5c6d19-ae7a-4b4c-a49f-36f322fd4532").unwrap()),
            bounty_contract: "0x1111111111111111111111111111111111111111".to_string(),''',
        '''            intent_id: Some(Uuid::parse_str("9e5c6d19-ae7a-4b4c-a49f-36f322fd4532").unwrap()),
            bounty_contract: Some("0x1111111111111111111111111111111111111111".to_string()),''',
        "test request optional bounty",
    ),
    (
        '''        assert!(!plan.bounty_funded);
        assert!(plan.canonical_funding_event.is_none());
        assert_eq!(plan.state, "checkout_ready_wallet_not_yet_topped_up");''',
        '''        assert!(!plan.protocol_action_completed);
        assert!(plan.canonical_event.is_none());
        assert!(!plan.bounty_funded);
        assert!(plan.canonical_funding_event.is_none());
        assert_eq!(plan.state, "checkout_ready_wallet_not_yet_topped_up");''',
        "test generic evidence",
    ),
)

TEST_SIGNATURE = "fn wallet_onboarding_checkout_does_not_require_an_existing_bounty()"
TEST_ANCHOR = '''    #[test]
    fn sandbox_checkout_is_test_only_and_supports_gas_asset() {'''
FORMATTED_TEST_BLOCK = '''    #[test]
    fn wallet_onboarding_checkout_does_not_require_an_existing_bounty() {
        let mut onboarding = request("usdc");
        onboarding.bounty_contract = None;
        onboarding.intent_id = None;
        let plan =
            build_checkout_plan(&config(MoonpayEnvironment::Sandbox), onboarding, None).unwrap();
        assert!(plan.bounty_contract.is_none());
        assert!(!plan.protocol_action_completed);
        assert!(plan.canonical_event.is_none());
        assert!(!plan.bounty_funded);
        assert!(plan.canonical_funding_event.is_none());
    }

'''
UNFORMATTED_TEST_BLOCK = '''    #[test]
    fn wallet_onboarding_checkout_does_not_require_an_existing_bounty() {
        let mut onboarding = request("usdc");
        onboarding.bounty_contract = None;
        onboarding.intent_id = None;
        let plan = build_checkout_plan(&config(MoonpayEnvironment::Sandbox), onboarding, None)
            .unwrap();
        assert!(plan.bounty_contract.is_none());
        assert!(!plan.protocol_action_completed);
        assert!(plan.canonical_event.is_none());
        assert!(!plan.bounty_funded);
        assert!(plan.canonical_funding_event.is_none());
    }

'''


def replace_exact(source: str, old: str, new: str, label: str) -> str:
    old_count = source.count(old)
    new_count = source.count(new)
    if new_count == 1:
        return source
    if new_count == 0 and old_count == 1:
        return source.replace(old, new, 1)
    raise SystemExit(f"{label}: old={old_count}, new={new_count}; inspect moonpay.rs drift")


def ensure_wallet_onboarding_test(source: str) -> str:
    count = source.count(TEST_SIGNATURE)
    if count == 0:
        if source.count(TEST_ANCHOR) != 1:
            raise SystemExit("wallet onboarding test anchor drifted")
        source = source.replace(TEST_ANCHOR, FORMATTED_TEST_BLOCK + TEST_ANCHOR, 1)
    elif count > 1:
        while source.count(TEST_SIGNATURE) > 1 and UNFORMATTED_TEST_BLOCK in source:
            source = source.replace(UNFORMATTED_TEST_BLOCK, "", 1)
    if source.count(TEST_SIGNATURE) != 1:
        raise SystemExit(
            f"wallet onboarding test must exist exactly once; found {source.count(TEST_SIGNATURE)}"
        )
    if UNFORMATTED_TEST_BLOCK in source:
        source = source.replace(UNFORMATTED_TEST_BLOCK, FORMATTED_TEST_BLOCK, 1)
    return source


def main() -> int:
    source = PATH.read_text(encoding="utf-8")
    for old, new, label in REPLACEMENTS:
        source = replace_exact(source, old, new, label)
    source = ensure_wallet_onboarding_test(source)

    for required in (
        "bounty_contract: Option<String>",
        "protocol_action_completed: bool",
        "canonical_event: Option<String>",
        TEST_SIGNATURE,
        "do not complete any Agent Bounties action",
    ):
        if required not in source:
            raise SystemExit(f"generic MoonPay patch missing marker: {required}")

    PATH.write_text(source, encoding="utf-8")
    print("Generic MoonPay wallet-onboarding patch is applied")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
