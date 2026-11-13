#!/usr/bin/env python3
"""Deterministic tests for inventory replenishment planning (#872).

Required fixture families: sufficient inventory, insufficient balance,
period cap, stale evidence, duplicate candidates, and exact five-task
replenishment.
"""

from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))

from plan_inventory_replenishment import plan_replenishment

FIXTURES = SCRIPTS / "fixtures" / "inventory-replenishment"


def load_fixture(name: str) -> dict:
    return json.loads((FIXTURES / f"{name}.json").read_text(encoding="utf-8"))


NOW = "2026-08-17T00:00:00+00:00"


class ReplenishmentPlanTest(unittest.TestCase):
    def test_five_task_replenishment_plan(self) -> None:
        fx = load_fixture("five")
        plan = plan_replenishment(
            fx["guard_report"], fx["candidate_tasks"], fx["wallet"], now=NOW
        )
        self.assertEqual(plan["deficit"], 5)
        self.assertEqual(len(plan["selected_candidates"]), 5)
        self.assertEqual(plan["total_funding"], 5.0)
        self.assertTrue(plan["wallet"]["wallet_sufficient"])
        self.assertTrue(plan["policy"]["policy_sufficient"])
        self.assertEqual(plan["blockers"], [])
        self.assertEqual(plan["financial_action_taken"], False)

    def test_sufficient_inventory_selects_exact_deficit(self) -> None:
        fx = load_fixture("sufficient")
        plan = plan_replenishment(
            fx["guard_report"], fx["candidate_tasks"], fx["wallet"], now=NOW
        )
        self.assertEqual(plan["deficit"], 2)
        self.assertEqual(plan["selected_candidates"], ["task-01", "task-02"])
        self.assertEqual(plan["total_funding"], 2.0)

    def test_insufficient_balance_is_blocked(self) -> None:
        fx = load_fixture("insufficient")
        plan = plan_replenishment(
            fx["guard_report"], fx["candidate_tasks"], fx["wallet"], now=NOW
        )
        self.assertFalse(plan["wallet"]["wallet_sufficient"])
        self.assertTrue(any("insufficient balance" in b for b in plan["blockers"]))

    def test_period_cap_exceeded_is_blocked(self) -> None:
        fx = load_fixture("period-cap")
        plan = plan_replenishment(
            fx["guard_report"], fx["candidate_tasks"], fx["wallet"], now=NOW
        )
        self.assertFalse(plan["policy"]["policy_sufficient"])
        self.assertTrue(any("period cap exceeded" in b for b in plan["blockers"]))

    def test_stale_evidence_is_blocked(self) -> None:
        fx = load_fixture("stale")
        plan = plan_replenishment(
            fx["guard_report"], fx["candidate_tasks"], fx["wallet"], now=NOW
        )
        self.assertTrue(any("stale evidence" in b for b in plan["blockers"]))
        self.assertNotIn("task-stale", plan["selected_candidates"])

    def test_duplicate_candidates_are_blocked(self) -> None:
        fx = load_fixture("duplicate")
        plan = plan_replenishment(
            fx["guard_report"], fx["candidate_tasks"], fx["wallet"], now=NOW
        )
        duplicates = [b for b in plan["blockers"] if "duplicate task" in b]
        self.assertEqual(len(duplicates), 1)
        # The deduplicated task is still eligible.
        self.assertIn("task-dup", plan["selected_candidates"])

    def test_idempotency_same_inputs_same_plan_and_key(self) -> None:
        fx = load_fixture("five")
        first = plan_replenishment(
            fx["guard_report"], fx["candidate_tasks"], fx["wallet"], now=NOW
        )
        second = plan_replenishment(
            fx["guard_report"], fx["candidate_tasks"], fx["wallet"], now=NOW
        )
        self.assertEqual(first["idempotency_key"], second["idempotency_key"])
        self.assertEqual(first["selected_candidates"], second["selected_candidates"])
        self.assertEqual(first["per_task_funding"], second["per_task_funding"])

    def test_plan_never_takes_financial_action(self) -> None:
        for name in ("sufficient", "insufficient", "period-cap", "stale", "duplicate", "five"):
            fx = load_fixture(name)
            plan = plan_replenishment(
                fx["guard_report"], fx["candidate_tasks"], fx["wallet"], now=NOW
            )
            self.assertFalse(plan["financial_action_taken"], name)


if __name__ == "__main__":
    unittest.main()
