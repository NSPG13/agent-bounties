#!/usr/bin/env python3
"""Regression tests for the protected gas-only V2 replenisher seed."""

from __future__ import annotations

from pathlib import Path
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "seed-open-competition-v2-replenisher-gas.yml"
DELEGATE = "0xb358898d34c5e907877a1cd7540b234f6851f61b"


class ReplenisherGasWorkflowTests(unittest.TestCase):
    def test_is_manual_protected_and_has_no_mutable_inputs(self) -> None:
        value = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))
        triggers = value.get("on", value.get(True))
        self.assertEqual(triggers, {"workflow_dispatch": None})
        job = value["jobs"]["seed"]
        self.assertEqual(job["environment"], "v2-beta2-mainnet")
        self.assertEqual(value["permissions"], {"contents": "read"})

    def test_seeds_only_exact_delegate_eth_and_zero_usdc(self) -> None:
        text = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn(f"--broker {DELEGATE}", text)
        self.assertIn("--target-usdc-base-units 0", text)
        self.assertIn("--target-eth-wei 100000000000000", text)
        self.assertIn(".funded_usdc_base_units == 0", text)
        self.assertNotIn("BASE_KEEPER_PRIVATE_KEY", text)
        self.assertNotIn("github.event.inputs", text)

    def test_requires_canonical_reconciliation_artifact(self) -> None:
        text = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn(".passed == true", text)
        self.assertIn(".final_eth_wei >= 100000000000000", text)
        self.assertIn("if-no-files-found: error", text)
        self.assertIn("retention-days: 365", text)


if __name__ == "__main__":
    unittest.main()
