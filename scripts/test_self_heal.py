#!/usr/bin/env python3

from __future__ import annotations

import copy
import importlib.util
import sys
import unittest
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent
SPEC = importlib.util.spec_from_file_location("self_heal", SCRIPT_DIR / "self_heal.py")
assert SPEC and SPEC.loader
self_heal = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = self_heal
SPEC.loader.exec_module(self_heal)


class SelfHealingContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.policy = self_heal.load_json(REPO_ROOT / "ops/self-healing-policy.json")
        cls.fixtures = REPO_ROOT / "ops/fixtures/recovery-cases.json"

    def test_policy_and_recovery_bench_pass(self) -> None:
        self_heal.validate_policy(self.policy)
        report = self_heal.run_bench(self.policy, self.fixtures)

        self.assertTrue(report["gate"])
        self.assertEqual(report["failed"], 0)
        self.assertGreaterEqual(report["cases"], 12)
        self.assertEqual(report["score"], 1.0)

    def test_prohibited_action_cannot_be_allowlisted(self) -> None:
        policy = copy.deepcopy(self.policy)
        policy["prohibited_automatic_actions"].append("retry_probe")

        with self.assertRaisesRegex(self_heal.ContractError, "both automatic and prohibited"):
            self_heal.validate_policy(policy)

    def test_automatic_action_risk_cannot_exceed_r2(self) -> None:
        policy = copy.deepcopy(self.policy)
        policy["automatic_actions"]["retry_probe"]["risk_class"] = "R3"

        with self.assertRaisesRegex(self_heal.ContractError, "must be R0, R1, or R2"):
            self_heal.validate_policy(policy)

    def test_integrity_failure_emits_no_automatic_action(self) -> None:
        fixture = self_heal.load_json(self.fixtures)
        snapshot = self_heal.deep_merge(
            fixture["base_snapshot"],
            {"invariants": {"contract_code_hashes_match": False}},
        )

        plan = self_heal.evaluate(self.policy, snapshot)

        self.assertEqual(plan["decision"], "contained")
        self.assertEqual(plan["automatic_actions"], [])
        self.assertIn(
            "platform:freeze_value_movement",
            {action["action_id"] for action in plan["manual_actions"]},
        )

    def test_remote_plain_http_probe_is_rejected(self) -> None:
        with self.assertRaisesRegex(self_heal.ContractError, "require HTTPS"):
            self_heal.normalize_base_url("http://agent-bounties.example.com")

        self.assertEqual(
            self_heal.normalize_base_url("http://127.0.0.1:8080"),
            "http://127.0.0.1:8080",
        )

    def test_snapshot_rejects_negative_failure_count(self) -> None:
        fixture = self_heal.load_json(self.fixtures)
        snapshot = self_heal.deep_merge(
            fixture["base_snapshot"],
            {"components": {"api": {"consecutive_failures": -1}}},
        )

        with self.assertRaisesRegex(self_heal.ContractError, "non-negative integer"):
            self_heal.validate_snapshot(snapshot)


if __name__ == "__main__":
    unittest.main()
