#!/usr/bin/env python3
"""Unit tests for bounded-wallet liquidity and authority reporting (Issue #871)."""

from __future__ import annotations

import unittest
from typing import Any

from scripts.bounded_wallet_liquidity import (
    BOUNDED_AGENT_WALLET_ABI_SELECTORS,
    ERC20_BALANCE_OF_SELECTOR,
    NATIVE_BASE_USDC,
     BoundedWalletLiquidityReporter,
    generate_bounded_wallet_liquidity_report,
)


WALLET = "0x1eaa1c68772cf76bc5f4e4174766076e33ace662"
OWNER = "0x884834e884d6e93462655a2820140ad03e6747bc"
DELEGATE = "0xc26a630e85134ed30968735c8e7de4576cfa5dbc"
FACTORY = "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9"
DET_MOD = "0x380c1af742593dd88b6f20387e9ee693a0536731"
POLICY_HASH = "0xac8c86836e1cab2b4707420057d414b0dfa675006bda6584df7fb9118caabe88"
QUORUM_HASH = "0x2c5a10915ca1fb99d4a11e2222b4f32b986b4e0f5599f55d70e9c8f9725a28cd"
AI_HASH = "0x" + "00" * 32


def _word_uint(val: int) -> str:
    return f"{val:064x}"


def _word_addr(addr: str) -> str:
    return "0" * 24 + addr.lower().removeprefix("0x")


def _word_bytes32(b32: str) -> str:
    return b32.lower().removeprefix("0x")


def _encode_policy(
    *,
    delegate: str = DELEGATE,
    valid_after: int = 1_700_000_000,
    valid_until: int = 1_900_000_000,
    period_seconds: int = 86_400,
    max_per_action: int = 2_000_000,
    max_per_period: int = 10_000_000,
    max_lifetime_spend: int = 25_000_000,
    max_bounty_target: int = 2_000_000,
    allowed_actions: int = 15,
    allowed_modes: int = 7,
) -> str:
    return "0x" + "".join(
        [
            _word_addr(delegate),
            _word_uint(valid_after),
            _word_uint(valid_until),
            _word_uint(period_seconds),
            _word_uint(max_per_action),
            _word_uint(max_per_period),
            _word_uint(max_lifetime_spend),
            _word_uint(max_bounty_target),
            _word_uint(allowed_actions),
            _word_uint(allowed_modes),
            _word_addr(DET_MOD),
            _word_bytes32(QUORUM_HASH),
            _word_bytes32(AI_HASH),
        ]
    )


def _base_config() -> dict[str, Any]:
    return {
        "chain_id": 8453,
        "wallet": WALLET,
        "owner": OWNER,
        "delegate": DELEGATE,
        "canonical_factory": FACTORY,
        "settlement_token": NATIVE_BASE_USDC,
        "wallet_policy_hash": POLICY_HASH,
        "wallet_policy_version": 5,
    }


class MockBaseRpc:
    def __init__(
        self,
        *,
        chain_id: int = 8453,
        block_number: int = 22_000_123,
        wallet_code: str = "0x363d3d373d3d3d363d7340d3e16082cf71ece0129ca3044e1b8233e29db85af43d82803e903d91602b57fd5bf3",
        token_code: str = "0x6080604052",
        onchain_token: str = NATIVE_BASE_USDC,
        onchain_owner: str = OWNER,
        onchain_factory: str = FACTORY,
        policy_version: int = 5,
        policy_hash: str = POLICY_HASH,
        revoked: bool = False,
        period_bucket: int = 20_500,
        period_spent: int = 2_200_000,
        lifetime_spent: int = 6_600_000,
        usdc_balance: int = 8_500_000,
        max_per_period: int = 10_000_000,
        max_lifetime_spend: int = 25_000_000,
        fail_unavailable: bool = False,
    ) -> None:
        self.chain_id = chain_id
        self.block_number = block_number
        self.wallet_code = wallet_code
        self.token_code = token_code
        self.onchain_token = onchain_token
        self.onchain_owner = onchain_owner
        self.onchain_factory = onchain_factory
        self.policy_version = policy_version
        self.policy_hash = policy_hash
        self.revoked = revoked
        self.period_bucket = period_bucket
        self.period_spent = period_spent
        self.lifetime_spent = lifetime_spent
        self.usdc_balance = usdc_balance
        self.max_per_period = max_per_period
        self.max_lifetime_spend = max_lifetime_spend
        self.fail_unavailable = fail_unavailable
        self.observed_block_tags: list[str] = []

    def __call__(self, method: str, params: list[Any]) -> Any:
        if self.fail_unavailable:
            raise ConnectionError("RPC endpoint unavailable")
        if method == "eth_chainId":
            return hex(self.chain_id)
        if method == "eth_blockNumber":
            return hex(self.block_number)
        if method == "eth_getCode":
            target, block_tag = params[0].lower(), params[1]
            self.observed_block_tags.append(block_tag)
            if target == WALLET:
                return self.wallet_code
            if target == NATIVE_BASE_USDC:
                return self.token_code
            return "0x"
        if method == "eth_call":
            call_obj, block_tag = params[0], params[1]
            self.observed_block_tags.append(block_tag)
            target = call_obj["to"].lower()
            data = call_obj["data"].lower()
            if target == WALLET:
                if data == BOUNDED_AGENT_WALLET_ABI_SELECTORS["settlementToken()"]:
                    return "0x" + _word_addr(self.onchain_token)
                if data == BOUNDED_AGENT_WALLET_ABI_SELECTORS["owner()"]:
                    return "0x" + _word_addr(self.onchain_owner)
                if data == BOUNDED_AGENT_WALLET_ABI_SELECTORS["factory()"]:
                    return "0x" + _word_addr(self.onchain_factory)
                if data == BOUNDED_AGENT_WALLET_ABI_SELECTORS["policyVersion()"]:
                    return "0x" + _word_uint(self.policy_version)
                if data == BOUNDED_AGENT_WALLET_ABI_SELECTORS["policyHash()"]:
                    return "0x" + _word_bytes32(self.policy_hash)
                if data == BOUNDED_AGENT_WALLET_ABI_SELECTORS["revoked()"]:
                    return "0x" + _word_uint(1 if self.revoked else 0)
                if data == BOUNDED_AGENT_WALLET_ABI_SELECTORS["policy()"]:
                    return _encode_policy(
                        max_per_period=self.max_per_period,
                        max_lifetime_spend=self.max_lifetime_spend,
                    )
                if data == BOUNDED_AGENT_WALLET_ABI_SELECTORS["periodSpent()"]:
                    return "0x" + _word_uint(self.period_spent)
                if data == BOUNDED_AGENT_WALLET_ABI_SELECTORS["lifetimeSpent()"]:
                    return "0x" + _word_uint(self.lifetime_spent)
                if data == BOUNDED_AGENT_WALLET_ABI_SELECTORS["periodBucket()"]:
                    return "0x" + _word_uint(self.period_bucket)
            if target == NATIVE_BASE_USDC and data.startswith(ERC20_BALANCE_OF_SELECTOR):
                return "0x" + _word_uint(self.usdc_balance)
        raise AssertionError(f"Unexpected RPC call: {method} {params}")


class BoundedWalletLiquidityTests(unittest.TestCase):
    def test_healthy_wallet_pins_all_reads_to_observed_block_and_separates_authority(self) -> None:
        rpc = MockBaseRpc()
        reporter = BoundedWalletLiquidityReporter(_base_config(), rpc)
        report = reporter.generateReport()

        self.assertTrue(report["ok"])
        self.assertEqual(report["status"], "healthy")
        self.assertEqual(report["observed_block"], 22_000_123)
        self.assertEqual(report["usdc_balance"], 8_500_000)
        self.assertEqual(report["period_spent"], 2_200_000)
        self.assertEqual(report["max_per_period"], 10_000_000)
        self.assertEqual(report["lifetime_spent"], 6_600_000)
        self.assertEqual(report["max_lifetime"], 25_000_000)
        self.assertEqual(report["policy_hash"], POLICY_HASH)
        self.assertEqual(report["policy_version"], 5)
        self.assertEqual(report["policy_authority"]["remaining_period_authority"], 7_800_000)
        self.assertEqual(report["policy_authority"]["remaining_lifetime_authority"], 18_400_000)
        self.assertEqual(report["wallet_liquidity"]["effective_spendable_usdc"], 7_800_000)
        self.assertFalse(report["claimable_escrow_separation"]["is_claimable_bounty_inventory"])
        # Verify every single eth_getCode and eth_call used the exact pinned block tag
        self.assertTrue(len(rpc.observed_block_tags) >= 11)
        self.assertEqual(set(rpc.observed_block_tags), {hex(22_000_123)})

    def test_empty_wallet_and_cap_exhausted_states(self) -> None:
        empty_rpc = MockBaseRpc(usdc_balance=0)
        empty_report = generate_bounded_wallet_liquidity_report(_base_config(), empty_rpc)
        self.assertTrue(empty_report["ok"])
        self.assertEqual(empty_report["status"], "empty")
        self.assertEqual(empty_report["usdc_balance"], 0)
        self.assertEqual(empty_report["wallet_liquidity"]["effective_spendable_usdc"], 0)
        self.assertIn("empty_wallet_balance", empty_report["blockers"])

        cap_rpc = MockBaseRpc(period_spent=10_000_000, lifetime_spent=25_000_000)
        cap_report = generate_bounded_wallet_liquidity_report(_base_config(), cap_rpc)
        self.assertTrue(cap_report["ok"])
        self.assertEqual(cap_report["status"], "exhausted")
        self.assertEqual(cap_report["policy_authority"]["remaining_period_authority"], 0)
        self.assertEqual(cap_report["policy_authority"]["remaining_lifetime_authority"], 0)
        self.assertIn("period_cap_exhausted", cap_report["blockers"])
        self.assertIn("lifetime_cap_exhausted", cap_report["blockers"])

    def test_wrong_chain_and_wrong_token_fail_closed_and_withhold_values(self) -> None:
        # Wrong chain case
        wrong_chain_rpc = MockBaseRpc(chain_id=84532)
        wrong_chain_report = generate_bounded_wallet_liquidity_report(_base_config(), wrong_chain_rpc)
        self.assertFalse(wrong_chain_report["ok"])
        self.assertIn("wrong_chain", wrong_chain_report["blockers"])
        self.assertIsNone(wrong_chain_report["usdc_balance"])
        self.assertIsNone(wrong_chain_report["lifetime_spent"])

        # Wrong token binding on-chain
        wrong_token_rpc = MockBaseRpc(onchain_token="0x" + "11" * 20)
        wrong_token_report = generate_bounded_wallet_liquidity_report(_base_config(), wrong_token_rpc)
        self.assertFalse(wrong_token_report["ok"])
        self.assertIn("wrong_token_binding", wrong_token_report["blockers"])
        self.assertIsNone(wrong_token_report["usdc_balance"])

    def test_policy_owner_and_bytecode_drift_withhold_financial_values(self) -> None:
        # Policy hash & version drift
        policy_drift_rpc = MockBaseRpc(policy_version=6, policy_hash="0x" + "ab" * 32)
        policy_report = generate_bounded_wallet_liquidity_report(_base_config(), policy_drift_rpc)
        self.assertFalse(policy_report["ok"])
        self.assertIn("policy_version_drift", policy_report["blockers"])
        self.assertIn("policy_hash_drift", policy_report["blockers"])
        self.assertIsNone(policy_report["usdc_balance"])
        self.assertIsNone(policy_report["period_spent"])
        self.assertIsNone(policy_report["wallet_liquidity"])

        # Owner drift
        owner_drift_rpc = MockBaseRpc(onchain_owner="0x" + "99" * 20)
        owner_report = generate_bounded_wallet_liquidity_report(_base_config(), owner_drift_rpc)
        self.assertFalse(owner_report["ok"])
        self.assertIn("owner_drift", owner_report["blockers"])
        self.assertIsNone(owner_report["usdc_balance"])

        # Missing wallet bytecode
        no_code_rpc = MockBaseRpc(wallet_code="0x")
        code_report = generate_bounded_wallet_liquidity_report(_base_config(), no_code_rpc)
        self.assertFalse(code_report["ok"])
        self.assertIn("missing_wallet_bytecode", code_report["blockers"])
        self.assertIsNone(code_report["usdc_balance"])

    def test_rpc_unavailable_returns_unavailable_without_zero_substitution(self) -> None:
        unavailable_rpc = MockBaseRpc(fail_unavailable=True)
        report = generate_bounded_wallet_liquidity_report(_base_config(), unavailable_rpc)
        self.assertFalse(report["ok"])
        self.assertEqual(report["status"], "unavailable")
        self.assertIn("rpc_unavailable", report["blockers"])
        self.assertIsNone(report["usdc_balance"])
        self.assertIsNone(report["period_spent"])
        self.assertIsNone(report["lifetime_spent"])


if __name__ == "__main__":
    unittest.main()
