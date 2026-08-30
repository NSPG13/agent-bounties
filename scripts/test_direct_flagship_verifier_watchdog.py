#!/usr/bin/env python3
"""Validate the unfunded flagship watchdog precommit."""

from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from scripts.activate_direct_growth_v2 import benchmark_digest  # noqa: E402


SPEC_PATH = ROOT / "ops" / "direct-flagship-verifier-settlement-watchdog-v1.json"


class DirectFlagshipWatchdogPrecommitTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.spec = json.loads(SPEC_PATH.read_text(encoding="utf-8"))

    def test_economics_preserve_liquid_margin(self) -> None:
        recovered = self.spec["recovered_funds"]["amount_base_units"]
        economics = self.spec["economics"]
        self.assertEqual(
            economics["initial_funding_base_units"],
            economics["solver_reward_base_units"]
            + economics["verifier_reward_base_units"],
        )
        self.assertEqual(
            economics["refundable_claim_bond_base_units"],
            economics["verifier_reward_base_units"],
        )
        self.assertEqual(
            recovered - economics["initial_funding_base_units"],
            economics["owner_liquid_margin_after_funding_base_units"],
        )
        self.assertGreaterEqual(
            economics["solver_reward_base_units"],
            2 * economics["profit_gate_max_solver_compute_base_units"],
        )

    def test_benchmark_digest_is_exact(self) -> None:
        verification = self.spec["verification"]
        self.assertEqual(
            benchmark_digest(verification["benchmark_subdirectory"]),
            verification["benchmark_digest"],
        )
        self.assertEqual(verification["runner_manifest"]["command"], ["python", "/benchmark/check.py"])
        self.assertEqual(verification["runner_manifest"]["timeout_seconds"], 1200)

    def test_recovery_and_authority_are_bounded(self) -> None:
        self.assertEqual(self.spec["status"], "unfunded_precommit")
        self.assertEqual(self.spec["recovered_funds"]["reserve_balance_at_safe_block"], 0)
        self.assertEqual(self.spec["lifecycle"]["claim_window_seconds"], 96 * 60 * 60)
        self.assertEqual(self.spec["lifecycle"]["verification_window_seconds"], 6 * 60 * 60)
        self.assertEqual(self.spec["verification"]["threshold"], 1)
        self.assertEqual(len(self.spec["verification"]["verifiers"]), 1)
        self.assertNotIn("private_key", SPEC_PATH.read_text(encoding="utf-8").lower())

    def test_funding_waits_for_all_safety_gates(self) -> None:
        gates = " ".join(self.spec["funding_gates"]).lower()
        for phrase in (
            "protected main",
            "known-good and known-bad",
            "verifier address",
            "live canary",
            "cancellation",
            "metamask",
        ):
            self.assertIn(phrase, gates)


if __name__ == "__main__":
    unittest.main()
