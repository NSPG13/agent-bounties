#!/usr/bin/env python3
"""Deterministic tests for inventory replenishment planning (#872)."""
from __future__ import annotations

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from scripts.plan_inventory_replenishment import (  # noqa: E402
    financial_action_taken, idempotency_key, plan_replenishment, SCHEMA_VERSION,
)

NOW = 1_800_000_000


def guard(deficit=5, balance=100, **kw):
    policy = {"max_lifetime": 500, "max_per_period": 60,
              "lifetime_spent": 10, "period_spent": 5}
    policy.update(kw.pop("policy", {}))
    return {"deficit": deficit, "wallet_balance": balance,
            "observed_at": NOW, "max_evidence_age": 3600, "policy": policy, **kw}


def cand(tid="T1", funding=3, margin=1, ready=True, evidenced=NOW - 60):
    return {"task_id": tid, "funding_usdc": funding, "solver_margin": margin,
            "verifier_ready": ready, "evidenced_at": evidenced}


class SufficientInventoryTests(unittest.TestCase):
    def test_plan_selects_and_is_consistent(self):
        plan = plan_replenishment(guard(), [cand()])
        self.assertTrue(plan["ok"])
        self.assertEqual(plan["blockers"], [])
        self.assertTrue(plan["wallet_sufficient"])
        self.assertTrue(plan["policy_sufficient"])
        self.assertEqual(plan["financial_action_taken"], False)

    def test_financial_action_is_never_taken(self):
        for plan in (plan_replenishment(guard(), [cand()]),
                     plan_replenishment(guard(deficit=0), [cand()])):
            self.assertFalse(plan["financial_action_taken"])
        self.assertFalse(financial_action_taken)


class DeficitTests(unittest.TestCase):
    def test_non_positive_deficit_is_blocked(self):
        plan = plan_replenishment(guard(deficit=0), [cand()])
        self.assertFalse(plan["ok"])
        self.assertEqual(plan["blocker"], "non_positive_deficit")


class InsufficientTests(unittest.TestCase):
    def test_insufficient_wallet_balance_is_blocked(self):
        plan = plan_replenishment(guard(balance=1), [cand(funding=50)])
        self.assertFalse(plan["ok"])
        self.assertIn("insufficient_wallet_balance", plan["blockers"])

    def test_period_cap_is_blocked(self):
        plan = plan_replenishment(guard(policy={"max_per_period": 10, "period_spent": 8}),
                                  [cand(funding=50)])
        self.assertFalse(plan["ok"])
        self.assertIn("exceeds_period_cap", plan["blockers"])

    def test_lifetime_cap_is_blocked(self):
        plan = plan_replenishment(guard(policy={"max_lifetime": 20, "lifetime_spent": 15}),
                                  [cand(funding=50)])
        self.assertIn("exceeds_lifetime_cap", plan["blockers"])


class DuplicateTests(unittest.TestCase):
    def test_duplicate_task_ids_are_rejected(self):
        plan = plan_replenishment(guard(), [cand("T1"), cand("T1")])
        self.assertIn("duplicate_task:T1", plan["blockers"])
        self.assertEqual(len(plan["selected"]), 1)

    def test_unverifiable_task_is_rejected(self):
        plan = plan_replenishment(guard(), [cand("T9", ready=False)])
        self.assertIn("unverifiable_task:T9", plan["blockers"])

    def test_non_positive_solver_margin_is_rejected(self):
        self.assertIn("non_positive_solver_margin:T8",
                      plan_replenishment(guard(), [cand("T8", margin=0)])["blockers"])


class StaleEvidenceTests(unittest.TestCase):
    def test_stale_evidence_is_rejected(self):
        plan = plan_replenishment(guard(), [cand("T7", evidenced=NOW - 99999)])
        self.assertIn("stale_evidence:T7", plan["blockers"])

    def test_fresh_evidence_is_accepted(self):
        self.assertTrue(plan_replenishment(guard(), [cand("T6", evidenced=NOW - 10)])["ok"])


class FiveTaskTests(unittest.TestCase):
    def test_exactly_five_tasks_are_selected(self):
        cands = [cand(f"T{i}", funding=2) for i in range(7)]
        plan = plan_replenishment(guard(deficit=40), cands)
        self.assertEqual(len(plan["selected"]), 5)
        self.assertEqual(plan["total_funding_usdc"], 10)


class IdempotencyTests(unittest.TestCase):
    def test_identical_inputs_give_identical_plan_and_keys(self):
        g, c = guard(), [cand("T1"), cand("T2")]
        self.assertEqual(plan_replenishment(g, c), plan_replenishment(g, c))

    def test_key_is_stable_and_reordered_input_matches(self):
        g = guard()
        a = [cand("T1"), cand("T2")]
        self.assertEqual(idempotency_key(g, a), idempotency_key(g, list(reversed(a))))

    def test_per_task_keys_are_present(self):
        plan = plan_replenishment(guard(), [cand("T1"), cand("T2")])
        self.assertEqual(set(plan["idempotency_keys"]), {"T1", "T2"})

    def test_schema_is_versioned(self):
        self.assertEqual(plan_replenishment(guard(), [cand()])["schema"], SCHEMA_VERSION)


if __name__ == "__main__":
    unittest.main(verbosity=2)
