#!/usr/bin/env python3
"""Deterministic tests for bounded-wallet liquidity reporting (#871)."""
from __future__ import annotations

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from scripts.bounded_wallet_liquidity import (  # noqa: E402
    liquidity_report, SCHEMA_VERSION, USDC_BASE_MAINNET,
)

EXPECTED = {"chain_id": 8453, "owner": "0xAbC0000000000000000000000000000000000001",
            "bytecode_hash": "0xdead", "policy_hash": "0xbeef",
            "token_address": USDC_BASE_MAINNET}


def obs(**kw):
    base = {"chain_id": 8453, "owner": EXPECTED["owner"], "bytecode_hash": "0xdead",
            "policy_hash": "0xbeef", "token_address": USDC_BASE_MAINNET,
            "usdc_balance": 250, "lifetime_spent": 40, "max_lifetime": 500,
            "period_spent": 10, "max_per_period": 100, "policy_version": "v3",
            "observed_block": 21_000_000, "claimable_escrow": 12, "rpc_available": True}
    base.update(kw)
    return base


class HealthyTests(unittest.TestCase):
    def test_healthy_report(self):
        rep = liquidity_report(obs(), EXPECTED)
        self.assertTrue(rep["ok"])
        self.assertEqual(rep["usdc_balance"], 250)
        self.assertEqual(rep["observed_block"], 21_000_000)
        self.assertEqual(rep["policy_version"], "v3")
        self.assertEqual(rep["schema"], SCHEMA_VERSION)

    def test_three_quantities_are_labelled_and_distinct(self):
        rep = liquidity_report(obs(), EXPECTED)["labels"]
        self.assertEqual(rep["liquid_balance"], 250)
        self.assertEqual(rep["remaining_lifetime_authority"], 460)
        self.assertEqual(rep["remaining_period_authority"], 90)
        self.assertEqual(rep["claimable_escrow"], 12)
        self.assertNotEqual(rep["liquid_balance"], rep["claimable_escrow"])

    def test_no_signer_secret_is_exposed(self):
        blob = repr(liquidity_report(obs(), EXPECTED)).lower()
        for word in ("private", "seed", "mnemonic", "signature"):
            self.assertNotIn(word, blob)


class EmptyWalletTests(unittest.TestCase):
    """Empty wallet: zero balance is not a failure and is not a cap."""

    def test_empty_balance_still_reports_authority(self):
        rep = liquidity_report(obs(usdc_balance=0, lifetime_spent=0, period_spent=0), EXPECTED)
        self.assertTrue(rep["ok"])
        self.assertEqual(rep["usdc_balance"], 0)
        self.assertEqual(rep["labels"]["liquid_balance"], 0)
        self.assertEqual(rep["labels"]["remaining_lifetime_authority"], 500)


class CapExhaustedTests(unittest.TestCase):
    """Cap exhausted: authority is zero but the report still succeeds."""

    def test_lifetime_cap_exhausted(self):
        rep = liquidity_report(obs(lifetime_spent=500), EXPECTED)
        self.assertEqual(rep["labels"]["remaining_lifetime_authority"], 0)

    def test_period_cap_exhausted(self):
        rep = liquidity_report(obs(period_spent=100), EXPECTED)
        self.assertEqual(rep["labels"]["remaining_period_authority"], 0)


class PolicyDriftTests(unittest.TestCase):
    """Policy drift: any change to the policy hash must fail closed."""

    def test_policy_drift_fails_closed(self):
        rep = liquidity_report(obs(policy_hash="0xfeed"), EXPECTED)
        self.assertFalse(rep["ok"])
        self.assertEqual(rep["reason"], "policy_drift")
        self.assertIsNone(rep["usdc_balance"])

    def test_owner_and_bytecode_drift_fail_closed(self):
        self.assertFalse(liquidity_report(obs(owner="0x" + "2" * 40), EXPECTED)["ok"])
        self.assertFalse(liquidity_report(obs(bytecode_hash="0x0bad"), EXPECTED)["ok"])


class WrongChainTests(unittest.TestCase):
    """Wrong chain: any chain other than Base mainnet must fail closed."""

    def test_wrong_chain_fails_closed(self):
        rep = liquidity_report(obs(chain_id=1), EXPECTED)
        self.assertFalse(rep["ok"])
        self.assertEqual(rep["reason"], "wrong_chain")

    def test_token_mismatch_fails_closed(self):
        rep = liquidity_report(obs(token_address="0x" + "9" * 40), EXPECTED)
        self.assertEqual(rep["reason"], "token_mismatch")


class RpcUnavailableTests(unittest.TestCase):
    """RPC unavailable: an unreadable snapshot must fail closed, not report zero."""

    def test_unavailable_rpc_fails_closed(self):
        rep = liquidity_report(obs(rpc_available=False), EXPECTED)
        self.assertFalse(rep["ok"])
        self.assertEqual(rep["reason"], "rpc_unavailable")
        self.assertIsNone(rep["usdc_balance"])

    def test_missing_chain_id_fails_closed(self):
        rep = liquidity_report(obs(chain_id=None), EXPECTED)
        self.assertEqual(rep["reason"], "rpc_unavailable")


class DeterminismTests(unittest.TestCase):
    def test_report_is_deterministic(self):
        self.assertEqual(liquidity_report(obs(), EXPECTED), liquidity_report(obs(), EXPECTED))


if __name__ == "__main__":
    unittest.main(verbosity=2)
