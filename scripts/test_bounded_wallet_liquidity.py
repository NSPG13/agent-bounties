#!/usr/bin/env python3
"""Deterministic tests for the bounded-wallet liquidity report (#871).

Covers the five required fixture families: healthy, empty, cap-exhausted,
policy-drift, and RPC-unavailable, plus wrong-chain fail-closed behavior.
"""

from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))

from bounded_wallet_liquidity import (
    BASE_CHAIN_ID,
    LiquidityReportError,
    WalletPolicy,
    build_liquidity_report,
    snapshot_from_dict,
)

FIXTURES = SCRIPTS / "fixtures" / "bounded-wallet-liquidity"


def load_snapshot(name: str) -> dict:
    return json.loads((FIXTURES / f"{name}.json").read_text(encoding="utf-8"))


class LiquidityReportTest(unittest.TestCase):
    def test_healthy_fixture_reports_all_quantities(self) -> None:
        snapshot = snapshot_from_dict(load_snapshot("healthy"))
        report = build_liquidity_report(snapshot)
        self.assertEqual(report["status"], "ok")
        q = report["quantities"]
        self.assertEqual(q["liquid_usdc_balance"], 1200.0)
        self.assertEqual(q["remaining_lifetime_authority"], 4800.0)
        self.assertEqual(q["remaining_period_authority"], 300.0)
        self.assertEqual(report["observed_block"], 28123456)
        # Escrow is deliberately a separate, unreported quantity.
        self.assertIsNone(q["claimable_escrow"])

    def test_empty_snapshot_reports_zero_balance(self) -> None:
        snapshot = snapshot_from_dict(load_snapshot("empty"))
        report = build_liquidity_report(snapshot)
        self.assertEqual(report["status"], "ok")
        self.assertEqual(report["quantities"]["liquid_usdc_balance"], 0.0)
        self.assertEqual(report["quantities"]["remaining_lifetime_authority"], 5000.0)

    def test_cap_exhausted_reports_zero_authority(self) -> None:
        snapshot = snapshot_from_dict(load_snapshot("cap-exhausted"))
        report = build_liquidity_report(snapshot)
        self.assertEqual(report["status"], "ok")
        self.assertEqual(report["quantities"]["remaining_period_authority"], 0.0)
        self.assertEqual(report["quantities"]["remaining_lifetime_authority"], 0.0)

    def test_policy_drift_fails_closed(self) -> None:
        snapshot = snapshot_from_dict(load_snapshot("policy-drift"))
        policy = WalletPolicy(
            policy_version="v1",
            policy_hash="0x" + "ab" * 32,
            expected_owner_address=snapshot.owner_address,
        )
        with self.assertRaises(LiquidityReportError) as ctx:
            build_liquidity_report(snapshot, policy)
        self.assertIn("policy drift", str(ctx.exception))

    def test_policy_owner_mismatch_fails_closed(self) -> None:
        snapshot = snapshot_from_dict(load_snapshot("healthy"))
        policy = WalletPolicy(
            policy_version=snapshot.policy_version,
            policy_hash=snapshot.policy_hash,
            expected_owner_address="0x0000000000000000000000000000000000000000",
        )
        with self.assertRaises(LiquidityReportError) as ctx:
            build_liquidity_report(snapshot, policy)
        self.assertIn("wrong owner", str(ctx.exception))

    def test_wrong_chain_fails_closed(self) -> None:
        snapshot = snapshot_from_dict(load_snapshot("healthy"))
        snapshot = snapshot_from_dict({**snapshot.__dict__, "chain_id": 1})
        with self.assertRaises(LiquidityReportError) as ctx:
            build_liquidity_report(snapshot)
        self.assertIn("wrong chain", str(ctx.exception))
        self.assertIn(str(BASE_CHAIN_ID), str(ctx.exception))

    def test_rpc_unavailable_fails_closed(self) -> None:
        snapshot = snapshot_from_dict(load_snapshot("rpc-unavailable"))
        with self.assertRaises(LiquidityReportError) as ctx:
            build_liquidity_report(snapshot)
        self.assertIn("rpc_unavailable", str(ctx.exception))

    def test_wrong_token_fails_closed(self) -> None:
        snapshot = snapshot_from_dict(load_snapshot("healthy"))
        snapshot = snapshot_from_dict(
            {**snapshot.__dict__, "token_address": "0x" + "11" * 20}
        )
        with self.assertRaises(LiquidityReportError) as ctx:
            build_liquidity_report(snapshot)
        self.assertIn("wrong token", str(ctx.exception))

    def test_wrong_bytecode_fails_closed(self) -> None:
        snapshot = snapshot_from_dict(load_snapshot("healthy"))
        snapshot = snapshot_from_dict(
            {**snapshot.__dict__, "bytecode_hash": "0x" + "22" * 32}
        )
        with self.assertRaises(LiquidityReportError) as ctx:
            build_liquidity_report(snapshot)
        self.assertIn("wrong bytecode", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
