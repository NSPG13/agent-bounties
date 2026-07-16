#!/usr/bin/env python3

from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import local_delegate_wallet as wallet


DELEGATE = "0x1111111111111111111111111111111111111111"
OWNER = "0x2222222222222222222222222222222222222222"
BOUNDED_WALLET = "0x3333333333333333333333333333333333333333"
BOUNTY = "0x4444444444444444444444444444444444444444"
FACTORY = "0x5555555555555555555555555555555555555555"
SETTLEMENT_ASSET = "0x" + "66" * 20
VERIFIER = "0x7777777777777777777777777777777777777777"
POLICY_HASH = "0x" + "88" * 32


class LocalDelegateTests(unittest.TestCase):
    def test_init_never_writes_plaintext_password_or_private_key(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            state_dir = Path(temporary) / "delegate"
            with (
                patch.object(wallet, "harden_acl"),
                patch.object(wallet, "protect_secret", side_effect=lambda value: b"protected:" + value),
            ):
                result = wallet.initialize(state_dir)
            self.assertTrue(result["initialized"])
            self.assertRegex(result["delegate"], r"^0x[0-9a-f]{40}$")
            password_blob = (state_dir / wallet.DPAPI_BLOB).read_bytes()
            self.assertTrue(password_blob.startswith(b"protected:"))
            files = b"".join(path.read_bytes() for path in state_dir.iterdir())
            self.assertNotIn(password_blob.removeprefix(b"protected:"), files.replace(password_blob, b""))
            self.assertEqual(wallet.status(state_dir)["delegate"], result["delegate"])

    @unittest.skipUnless(os.name == "nt", "DPAPI is Windows-only")
    def test_dpapi_round_trip(self) -> None:
        secret = b"one-time-test-secret"
        protected = wallet.protect_secret(secret)
        self.assertNotEqual(protected, secret)
        self.assertEqual(wallet.unprotect_secret(protected), secret)

    def test_rpc_hex_adds_json_rpc_prefix(self) -> None:
        self.assertEqual(wallet.rpc_hex(bytes.fromhex("abcd")), "0xabcd")

    def setUp(self) -> None:
        self.manifest = {
            "chain_id": 8453,
            "canonical": {
                "bounty_factory": FACTORY,
                "settlement_token": SETTLEMENT_ASSET,
            },
        }
        self.binding = {
            "wallet": BOUNDED_WALLET,
            "delegate": DELEGATE,
            "owner": OWNER,
            "policy_hash": POLICY_HASH,
            "policy_version": 1,
        }
        self.policy = {
            "delegate": DELEGATE,
            "valid_until": 9_999,
            "period_seconds": 86_400,
            "max_per_action": 5_000_000,
            "max_per_period": 10_000_000,
            "max_lifetime_spend": 89_000_000,
            "max_bounty_target": 5_000_000,
            "allowed_actions": (1 << 1) | (1 << 2) | (1 << 3),
            "allowed_verification_modes": 1,
            "deterministic_verifier_module": VERIFIER,
            "signed_quorum_verifier_set_hash": "0x" + "00" * 32,
            "ai_judge_verifier_set_hash": "0x" + "00" * 32,
        }
        self.report = {
            "ready": True,
            "failures": [],
            "safe_block": {"number": 101, "hash": "0xsafe", "timestamp": 1_100},
            "state": {
                "owner": OWNER,
                "policy_hash": POLICY_HASH,
                "policy_version": 1,
                "policy": self.policy,
                "period_bucket": "0",
                "period_spent": "0",
                "lifetime_spent": "0",
                "wallet_usdc_balance": "89000000",
            },
        }
        self.observed = {
            "factory": FACTORY,
            "settlement_token": SETTLEMENT_ASSET,
            "creator": OWNER,
            "solver": "0x" + "00" * 20,
            "status": 0,
            "verification_mode": 0,
            "verifier_module": VERIFIER,
            "verifier_set_hash": "0x" + "00" * 32,
            "target_amount": 2_010_000,
            "funded_amount": 0,
            "verifier_reward": 10_000,
            "funding_deadline": 9_999,
            "claim_expires_at": 0,
        }

    def plan(self) -> dict:
        payload = "0xpayload"
        direct = "0xdirect"
        return {
            "schema": wallet.PLAN_SCHEMA,
            "network": "base-mainnet",
            "safe_block": {"number": 100, "hash": "0xplanned", "timestamp": 1_000},
            "wallet": BOUNDED_WALLET,
            "delegate": DELEGATE,
            "policy_hash": POLICY_HASH,
            "action": "fund",
            "action_code": 1,
            "bounty": BOUNTY,
            "bounty_state": self.observed,
            "action_summary": {
                "requested_amount": "2010000",
                "maximum_accepted_amount": "2010000",
            },
            "maximum_gross_spend": "2010000",
            "payload": payload,
            "payload_hash": "0xhash",
            "direct_transaction": {
                "from": DELEGATE,
                "to": BOUNDED_WALLET,
                "data": direct,
                "value": "0x0",
            },
        }

    def validate(self, plan: dict) -> dict:
        with (
            patch.object(wallet, "encode", return_value="0xpayload"),
            patch.object(wallet, "calldata", return_value="0xdirect"),
            patch.object(wallet, "keccak_hex", return_value="0xhash"),
        ):
            return wallet.validate_plan(
                plan,
                self.binding,
                self.manifest,
                self.report,
                self.observed,
                {"hash": "0xplanned"},
            )

    def test_accepts_exact_policy_bound_funding_plan(self) -> None:
        self.assertEqual(self.validate(self.plan())["to"], BOUNDED_WALLET)

    def test_rejects_arbitrary_target(self) -> None:
        plan = self.plan()
        plan["direct_transaction"]["to"] = BOUNTY
        with self.assertRaisesRegex(SystemExit, "unexpected target"):
            self.validate(plan)

    def test_rejects_arbitrary_calldata(self) -> None:
        plan = self.plan()
        plan["direct_transaction"]["data"] = "0xdeadbeef"
        with self.assertRaisesRegex(SystemExit, "unexpected target"):
            self.validate(plan)

    def test_rejects_stale_plan(self) -> None:
        plan = self.plan()
        self.report["safe_block"]["timestamp"] = 1_301
        with self.assertRaisesRegex(SystemExit, "stale"):
            self.validate(plan)

    def test_rejects_changed_bounty_state(self) -> None:
        plan = self.plan()
        plan["bounty_state"] = dict(self.observed, funded_amount=1)
        with self.assertRaisesRegex(SystemExit, "state changed"):
            self.validate(plan)

    def test_rejects_policy_rotation(self) -> None:
        plan = self.plan()
        self.report["state"]["policy_version"] = 2
        with self.assertRaisesRegex(SystemExit, "policy version changed"):
            self.validate(plan)

    def test_rejects_gross_spend_tampering(self) -> None:
        plan = self.plan()
        plan["maximum_gross_spend"] = "1"
        with self.assertRaisesRegex(SystemExit, "spend does not match"):
            self.validate(plan)


if __name__ == "__main__":
    unittest.main()
