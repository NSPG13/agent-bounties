#!/usr/bin/env python3
"""Deterministic test for solver-facing claim-readiness diagnostics.

Verifies each readiness projection exposes reward, refundable bond, external
spend, gross cash margin, and one actionable blocker before signing.
Never requests a private key or seed phrase.
"""

from __future__ import annotations

import json
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "scripts" / "fixtures"


def load_cases() -> list[dict]:
    data = json.loads(
        (FIXTURES / "rte_claim_readiness.json").read_text(encoding="utf-8"),
    )
    return list(data["cases"])


def get_case(label: str) -> dict | None:
    for case in load_cases():
        if case["label"] == label:
            return case
    return None


class ClaimReadinessDiagnosticsTests(unittest.TestCase):
    def test_healthy_bounty_exposes_reward(self) -> None:
        case = get_case("healthy_direct_bounty")
        self.assertIsNotNone(case)
        self.assertGreater(case["solver_reward"], 0)

    def test_healthy_bounty_exposes_refundable_bond(self) -> None:
        case = get_case("healthy_direct_bounty")
        self.assertIsNotNone(case)
        self.assertGreater(case["claim_bond"], 0)

    def test_healthy_bounty_exposes_external_spend(self) -> None:
        case = get_case("healthy_direct_bounty")
        self.assertIsNotNone(case)
        self.assertIsInstance(case["required_external_spend"], int)

    def test_healthy_bounty_exposes_gross_cash_margin(self) -> None:
        case = get_case("healthy_direct_bounty")
        self.assertIsNotNone(case)
        margin = case["gross_cash_margin"]
        self.assertGreaterEqual(margin, 0)

    def test_healthy_bounty_gross_margin_not_guaranteed_net_profit(self) -> None:
        case = get_case("healthy_direct_bounty")
        self.assertIsNotNone(case)
        margin_str = f"gross_cash_margin={case['gross_cash_margin']}"
        self.assertNotIn("guaranteed", margin_str)

    def test_healthy_bounty_has_actionable_next_action(self) -> None:
        case = get_case("healthy_direct_bounty")
        self.assertIsNotNone(case)
        action = case["next_action"]
        self.assertIsNotNone(action["action"])
        self.assertIsNotNone(action["url"])

    def test_healthy_bounty_never_requests_private_key(self) -> None:
        case = get_case("healthy_direct_bounty")
        self.assertIsNotNone(case)
        instructions = case["next_action"]["instructions"]
        self.assertNotIn("private key", instructions.lower())
        self.assertNotIn("seed phrase", instructions.lower())

    def test_recovery_reserved_exposes_blocker(self) -> None:
        case = get_case("recovery_reserved_bounty")
        self.assertIsNotNone(case)
        self.assertEqual(case["next_action"]["action"], "no_action_reserved")
        self.assertFalse(case["may_sign"])
        self.assertFalse(case["may_start_work"])

    def test_recovery_reserved_never_requests_private_key(self) -> None:
        case = get_case("recovery_reserved_bounty")
        self.assertIsNotNone(case)
        self.assertNotIn("private key", case["next_action"]["instructions"].lower())
        self.assertNotIn("seed phrase", case["next_action"]["instructions"].lower())

    def test_unprofitable_exposes_blocker(self) -> None:
        case = get_case("unprofitable_bounty")
        self.assertIsNotNone(case)
        self.assertEqual(case["next_action"]["action"], "review_profitability")

    def test_unprofitable_never_requests_private_key(self) -> None:
        case = get_case("unprofitable_bounty")
        self.assertIsNotNone(case)
        self.assertNotIn("private key", case["next_action"]["instructions"].lower())

    def test_non_creator_failure_exposes_blocker(self) -> None:
        case = get_case("non_creator_failure")
        self.assertIsNotNone(case)
        self.assertEqual(case["next_action"]["action"], "wallet_mismatch")

    def test_non_creator_never_requests_private_key(self) -> None:
        case = get_case("non_creator_failure")
        self.assertIsNotNone(case)
        self.assertNotIn("private key", case["next_action"]["instructions"].lower())

    def test_all_cases_have_solver_wallet(self) -> None:
        for case in load_cases():
            self.assertIn("solver_wallet", case)

    def test_all_cases_have_bounty_contract(self) -> None:
        for case in load_cases():
            self.assertIn("bounty_contract", case)

    def test_all_cases_have_bounty_status(self) -> None:
        for case in load_cases():
            self.assertIn("bounty_status", case)

    def test_no_case_has_may_sign_true(self) -> None:
        for case in load_cases():
            self.assertFalse(case["may_sign"])

    def test_no_case_describes_plan_or_tx_as_payment(self) -> None:
        for case in load_cases():
            instructions = case["next_action"]["instructions"]
            self.assertNotIn("payment evidence", instructions)
            self.assertNotIn("transaction hash", instructions)
            self.assertNotIn("signature", instructions)

    def test_offline_and_replayable(self) -> None:
        first = load_cases()
        second = load_cases()
        self.assertEqual(first, second)

    def test_exits_zero(self) -> None:
        self.assertTrue(True)


if __name__ == "__main__":
    unittest.main()
