#!/usr/bin/env python3
"""Unit tests for bounded agent wallet liquidity and authority reporting."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "bounded_wallet_liquidity.py"
MANIFEST_V2 = ROOT / "deployments" / "bounded-agent-wallet-v2-base-mainnet.json"
MANIFEST_V1 = ROOT / "deployments" / "bounded-agent-wallet-base-mainnet.json"

sys.path.insert(0, str(ROOT / "scripts"))
import bounded_wallet_liquidity as bwl  # noqa: E402


class MockRpcBackend:
    """Mock JSON-RPC transport for reproducible Base state simulation."""

    def __init__(
        self,
        manifest: dict[str, Any],
        wallet: str = "0x1eaa1c68772cf76bc5f4e4174766076e33ace662",
        owner: str = "0x884834e884d6e93462655a2820140ad03e6747bc",
        delegate: str = "0xe46741de0f379bff0ab8b01bce1b79a12d892fdb",
        chain_id: int = 8453,
        safe_block_num: int = 50_000_000,
        safe_block_timestamp: int = 1_785_000_000,
        usdc_balance: int = 50_000_000,
        lifetime_spent: int = 10_000_000,
        max_lifetime_spend: int = 89_000_000,
        period_spent: int = 2_000_000,
        max_per_period: int = 10_000_000,
        max_per_action: int = 5_000_000,
        max_bounty_target: int = 5_000_000,
        valid_after: int = 1_784_000_000,
        valid_until: int = (1 << 64) - 1,
        period_seconds: int = 86_400,
        policy_version: int = 6,
        revoked: bool = False,
        is_registered: bool = True,
        corrupt_factory_code: bool = False,
        corrupt_wallet_code: bool = False,
        corrupt_implementation_code: bool = False,
        raise_on_safe_block: bool = False,
        raise_on_chain_id: bool = False,
    ) -> None:
        self.manifest = manifest
        self.wallet = wallet.lower()
        self.owner = owner.lower()
        self.delegate = delegate.lower()
        self.chain_id = chain_id
        self.safe_block_num = safe_block_num
        self.safe_block_timestamp = safe_block_timestamp
        self.usdc_balance = usdc_balance
        self.lifetime_spent = lifetime_spent
        self.max_lifetime_spend = max_lifetime_spend
        self.period_spent = period_spent
        self.max_per_period = max_per_period
        self.max_per_action = max_per_action
        self.max_bounty_target = max_bounty_target
        self.valid_after = valid_after
        self.valid_until = valid_until
        self.period_seconds = period_seconds
        self.policy_version = policy_version
        self.revoked = revoked
        self.is_registered = is_registered
        self.corrupt_factory_code = corrupt_factory_code
        self.corrupt_wallet_code = corrupt_wallet_code
        self.corrupt_implementation_code = corrupt_implementation_code
        self.raise_on_safe_block = raise_on_safe_block
        self.raise_on_chain_id = raise_on_chain_id

    def build_policy_raw(self) -> str:
        words = [
            bwl.pad_address(self.delegate),
            bwl.pad_uint(self.valid_after),
            bwl.pad_uint(self.valid_until),
            bwl.pad_uint(self.period_seconds),
            bwl.pad_uint(self.max_per_action),
            bwl.pad_uint(self.max_per_period),
            bwl.pad_uint(self.max_lifetime_spend),
            bwl.pad_uint(self.max_bounty_target),
            bwl.pad_uint(15),  # allowed_actions
            bwl.pad_uint(3),   # allowed_verification_modes
            bwl.pad_address(self.manifest["canonical"]["deterministic_verifier"]),
            self.manifest["canonical"]["signed_quorum_verifier_set_hash"].lower().removeprefix("0x"),
            bwl.ZERO_HASH.removeprefix("0x"),
        ]
        return "0x" + "".join(words)

    def handle_rpc(self, url: str, method: str, params: list, request_id: int = 1) -> Any:
        if self.raise_on_safe_block and method == "eth_getBlockByNumber":
            raise RuntimeError("RPC endpoint unavailable: connection refused")
        if self.raise_on_chain_id and method == "eth_chainId":
            raise RuntimeError("RPC endpoint unavailable: timeout")

        if method == "eth_getBlockByNumber":
            return {
                "number": hex(self.safe_block_num),
                "hash": "0x" + "aa" * 32,
                "timestamp": hex(self.safe_block_timestamp),
            }
        if method == "eth_chainId":
            return hex(self.chain_id)

        if method == "eth_getCode":
            addr = params[0].lower()
            factory_addr = self.manifest["wallet_factory"]["address"].lower()
            impl_addr = self.manifest["wallet_factory"]["implementation"].lower()
            if addr == factory_addr:
                if self.corrupt_factory_code:
                    return "0xdeadbeef"
                return "0x6080604052348015"
            elif addr == impl_addr:
                if self.corrupt_implementation_code:
                    return "0xdeadbeef"
                return "0x6080604052600436"
            elif addr == self.wallet:
                if self.corrupt_wallet_code:
                    return "0xdeadbeef"
                return "0x363d3d373d3d3d36"
            return "0x"

        if method == "eth_call":
            call_obj = params[0]
            to = call_obj.get("to", "").lower()
            data = call_obj.get("data", "").lower()
            factory_addr = self.manifest["wallet_factory"]["address"].lower()
            token_addr = self.manifest["canonical"]["settlement_token"].lower()

            # Factory calls
            if to == factory_addr:
                if data.startswith(bwl.SEL_IMPLEMENTATION):
                    return "0x" + bwl.pad_address(self.manifest["wallet_factory"]["implementation"])
                if data.startswith(bwl.SEL_BOUNTY_FACTORY):
                    return "0x" + bwl.pad_address(self.manifest["canonical"]["bounty_factory"])
                if data.startswith(bwl.SEL_SETTLEMENT_TOKEN):
                    return "0x" + bwl.pad_address(self.manifest["canonical"]["settlement_token"])
                if data.startswith(bwl.SEL_IS_FACTORY_WALLET):
                    return "0x" + bwl.pad_uint(1 if self.is_registered else 0)

            # Token calls
            if to == token_addr:
                if data.startswith(bwl.SEL_BALANCE_OF):
                    return "0x" + bwl.pad_uint(self.usdc_balance)

            # Wallet calls
            if to == self.wallet:
                if data.startswith(bwl.SEL_POLICY):
                    return self.build_policy_raw()
                if data.startswith(bwl.SEL_OWNER):
                    return "0x" + bwl.pad_address(self.owner)
                if data.startswith(bwl.SEL_DELEGATE_NONCE):
                    return "0x" + bwl.pad_uint(0)
                if data.startswith(bwl.SEL_POLICY_VERSION):
                    return "0x" + bwl.pad_uint(self.policy_version)
                if data.startswith(bwl.SEL_PERIOD_BUCKET):
                    return "0x" + bwl.pad_uint(self.safe_block_timestamp // self.period_seconds)
                if data.startswith(bwl.SEL_PERIOD_SPENT):
                    return "0x" + bwl.pad_uint(self.period_spent)
                if data.startswith(bwl.SEL_LIFETIME_SPENT):
                    return "0x" + bwl.pad_uint(self.lifetime_spent)
                if data.startswith(bwl.SEL_REVOKED):
                    return "0x" + bwl.pad_uint(1 if self.revoked else 0)

            return "0x" + "00" * 32

        raise NotImplementedError(f"Unhandled method: {method}")


class BoundedWalletLiquidityTests(unittest.TestCase):
    """Test suite ensuring accurate separation of liquidity and authority."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.manifest = json.loads(MANIFEST_V2.read_text(encoding="utf-8"))
        # Patch code hashes in manifest to match test simulation
        cls.manifest["wallet_factory"]["runtime_code_hash"] = bwl.code_hash("0x6080604052348015")
        cls.manifest["wallet_factory"]["implementation_runtime_code_hash"] = bwl.code_hash("0x6080604052600436")
        cls.manifest["wallet_factory"]["clone_runtime_code_hash"] = bwl.code_hash("0x363d3d373d3d3d36")

    def test_healthy_wallet_liquidity_report(self) -> None:
        """Test healthy wallet inspection with clear separation of balance and authority."""
        backend = MockRpcBackend(
            manifest=self.manifest,
            usdc_balance=50_000_000,
            lifetime_spent=10_000_000,
            max_lifetime_spend=89_000_000,
            period_spent=2_000_000,
            max_per_period=10_000_000,
            max_per_action=5_000_000,
        )
        report = bwl.inspect_bounded_wallet_liquidity(
            wallet=backend.wallet,
            manifest=self.manifest,
            rpc_caller=backend.handle_rpc,
        )

        self.assertTrue(report["ready"])
        self.assertEqual(report["failures"], [])
        self.assertEqual(report["usdc_balance"], 50_000_000)
        self.assertEqual(report["lifetime_spent"], 10_000_000)
        self.assertEqual(report["max_lifetime"], 89_000_000)
        self.assertEqual(report["remaining_lifetime_authority"], 79_000_000)
        self.assertEqual(report["period_spent"], 2_000_000)
        self.assertEqual(report["max_per_period"], 10_000_000)
        self.assertEqual(report["remaining_period_authority"], 8_000_000)
        self.assertEqual(report["max_per_action"], 5_000_000)
        # Spendable liquidity is bounded by remaining period authority (8,000,000)
        self.assertEqual(report["spendable_liquidity"], 8_000_000)
        self.assertEqual(report["currently_claimable_escrow"], 0)
        self.assertEqual(report["observed_block"]["number"], 50_000_000)
        self.assertEqual(report["policy_version"], 6)

    def test_empty_wallet_balance(self) -> None:
        """Test empty wallet: when usdc_balance is 0, spendable liquidity must be 0."""
        backend = MockRpcBackend(
            manifest=self.manifest,
            usdc_balance=0,  # empty wallet balance
            lifetime_spent=0,
            max_lifetime_spend=89_000_000,
            period_spent=0,
            max_per_period=10_000_000,
        )
        report = bwl.inspect_bounded_wallet_liquidity(
            wallet=backend.wallet,
            manifest=self.manifest,
            rpc_caller=backend.handle_rpc,
        )

        self.assertTrue(report["ready"])
        self.assertEqual(report["usdc_balance"], 0)
        self.assertEqual(report["spendable_liquidity"], 0)
        self.assertEqual(report["remaining_lifetime_authority"], 89_000_000)
        self.assertEqual(report["remaining_period_authority"], 10_000_000)
        self.assertIn("empty", "Testing empty balance evaluation: 0 balance yields 0 spendable")

    def test_cap_exhausted_lifetime_and_period(self) -> None:
        """Test authority cap exhaustion: lifetime or period cap spent reduces spendable liquidity."""
        # 1. Lifetime cap exhausted
        backend_lifetime_capped = MockRpcBackend(
            manifest=self.manifest,
            usdc_balance=50_000_000,
            lifetime_spent=89_000_000,
            max_lifetime_spend=89_000_000,
            period_spent=0,
            max_per_period=10_000_000,
        )
        rep1 = bwl.inspect_bounded_wallet_liquidity(
            wallet=backend_lifetime_capped.wallet,
            manifest=self.manifest,
            rpc_caller=backend_lifetime_capped.handle_rpc,
        )
        self.assertEqual(rep1["remaining_lifetime_authority"], 0)
        self.assertEqual(rep1["spendable_liquidity"], 0)
        self.assertEqual(rep1["usdc_balance"], 50_000_000)

        # 2. Period cap exhausted
        backend_period_capped = MockRpcBackend(
            manifest=self.manifest,
            usdc_balance=50_000_000,
            lifetime_spent=10_000_000,
            max_lifetime_spend=89_000_000,
            period_spent=10_000_000,
            max_per_period=10_000_000,
        )
        rep2 = bwl.inspect_bounded_wallet_liquidity(
            wallet=backend_period_capped.wallet,
            manifest=self.manifest,
            rpc_caller=backend_period_capped.handle_rpc,
        )
        self.assertEqual(rep2["remaining_period_authority"], 0)
        self.assertEqual(rep2["spendable_liquidity"], 0)
        self.assertEqual(rep2["usdc_balance"], 50_000_000)

    def test_policy_drift_and_revocation(self) -> None:
        """Test policy drift, inactive window, expired policy, or revoked policy status."""
        # 1. Revoked policy
        backend_revoked = MockRpcBackend(
            manifest=self.manifest,
            revoked=True,
        )
        rep1 = bwl.inspect_bounded_wallet_liquidity(
            wallet=backend_revoked.wallet,
            manifest=self.manifest,
            rpc_caller=backend_revoked.handle_rpc,
        )
        self.assertFalse(rep1["ready"])
        self.assertIn("policy_revoked", rep1["failures"])

        # 2. Inactive policy (before valid_after)
        backend_inactive = MockRpcBackend(
            manifest=self.manifest,
            safe_block_timestamp=1_700_000_000,
            valid_after=1_784_000_000,
        )
        rep2 = bwl.inspect_bounded_wallet_liquidity(
            wallet=backend_inactive.wallet,
            manifest=self.manifest,
            rpc_caller=backend_inactive.handle_rpc,
        )
        self.assertFalse(rep2["ready"])
        self.assertIn("policy_not_active", rep2["failures"])

        # 3. Expired policy (after valid_until)
        backend_expired = MockRpcBackend(
            manifest=self.manifest,
            safe_block_timestamp=1_900_000_000,
            valid_until=1_800_000_000,
        )
        rep3 = bwl.inspect_bounded_wallet_liquidity(
            wallet=backend_expired.wallet,
            manifest=self.manifest,
            rpc_caller=backend_expired.handle_rpc,
        )
        self.assertFalse(rep3["ready"])
        self.assertIn("policy_expired", rep3["failures"])

        # 4. Expected policy hash mismatch
        rep4 = bwl.inspect_bounded_wallet_liquidity(
            wallet=backend_revoked.wallet,
            manifest=self.manifest,
            expected_policy_hash="0x" + "ff" * 32,
            rpc_caller=backend_revoked.handle_rpc,
        )
        self.assertFalse(rep4["ready"])
        self.assertTrue(any("policy_hash_mismatch" in f for f in rep4["failures"]))

    def test_wrong_chain_fails_closed(self) -> None:
        """Test wrong chain: must fail closed when connected to wrong chain ID."""
        backend_wrong_chain = MockRpcBackend(
            manifest=self.manifest,
            chain_id=1,  # wrong chain (Ethereum mainnet instead of Base 8453)
        )
        report = bwl.inspect_bounded_wallet_liquidity(
            wallet=backend_wrong_chain.wallet,
            manifest=self.manifest,
            rpc_caller=backend_wrong_chain.handle_rpc,
        )
        self.assertFalse(report["ready"])
        self.assertTrue(any("wrong chain" in f.lower() for f in report["failures"]))

    def test_rpc_unavailable_fails_closed(self) -> None:
        """Test RPC unavailable: must raise RuntimeError and fail closed when unavailable."""
        backend_unavailable = MockRpcBackend(
            manifest=self.manifest,
            raise_on_safe_block=True,
        )
        with self.assertRaises(RuntimeError) as ctx:
            bwl.inspect_bounded_wallet_liquidity(
                wallet=backend_unavailable.wallet,
                manifest=self.manifest,
                rpc_caller=backend_unavailable.handle_rpc,
            )
        self.assertIn("unavailable", str(ctx.exception).lower())

    def test_bytecode_mismatch_fails_closed(self) -> None:
        """Test bytecode corruption fails closed."""
        backend_corrupt = MockRpcBackend(
            manifest=self.manifest,
            corrupt_wallet_code=True,
        )
        report = bwl.inspect_bounded_wallet_liquidity(
            wallet=backend_corrupt.wallet,
            manifest=self.manifest,
            rpc_caller=backend_corrupt.handle_rpc,
        )
        self.assertFalse(report["ready"])
        self.assertIn("wallet_clone_code_mismatch", report["failures"])

    def test_markdown_and_json_rendering(self) -> None:
        """Test markdown summary table and json output generation."""
        backend = MockRpcBackend(
            manifest=self.manifest,
            usdc_balance=25_000_000,
        )
        report = bwl.inspect_bounded_wallet_liquidity(
            wallet=backend.wallet,
            manifest=self.manifest,
            rpc_caller=backend.handle_rpc,
        )
        md = bwl.render_markdown_summary(report)
        self.assertIn("Bounded Wallet Liquidity & Authority Report [PASS]", md)
        self.assertIn("Native USDC Balance", md)
        self.assertIn("$25.000000", md)
        self.assertIn("Spendable Liquidity", md)


if __name__ == "__main__":
    unittest.main()
