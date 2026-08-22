#!/usr/bin/env python3
"""Tests for the exact bounded Base Sepolia reserve rebalance."""

from __future__ import annotations

from pathlib import Path
import unittest

import yaml

import rebalance_open_competition_v2_beta3_sepolia_usdc as subject


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "rebalance-open-competition-v2-beta3-sepolia-usdc.yml"


class SepoliaRebalanceTests(unittest.TestCase):
    def test_exact_observed_deficit_is_allowed(self) -> None:
        self.assertEqual(
            subject.planned_transfer(deployer_usdc=877_500, broker_usdc=122_500),
            22_500,
        )

    def test_is_idempotent_after_reconciliation(self) -> None:
        self.assertEqual(
            subject.planned_transfer(deployer_usdc=900_000, broker_usdc=100_000),
            0,
        )

    def test_fails_closed_on_larger_deficit_or_broker_floor(self) -> None:
        with self.assertRaises(subject.SepoliaRebalanceError):
            subject.planned_transfer(deployer_usdc=877_499, broker_usdc=122_501)
        with self.assertRaises(subject.SepoliaRebalanceError):
            subject.planned_transfer(deployer_usdc=877_500, broker_usdc=122_499)

    def test_workflow_is_manual_protected_and_has_no_inputs(self) -> None:
        value = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))
        triggers = value.get("on", value.get(True))
        self.assertEqual(triggers, {"workflow_dispatch": None})
        job = value["jobs"]["rebalance"]
        self.assertEqual(job["environment"], "v2-beta2-sepolia")
        self.assertEqual(job["runs-on"], "ubuntu-24.04")
        text = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("OPEN_COMPETITION_V2_BROKER_PRIVATE_KEY", text)
        self.assertNotIn("BASE_MAINNET", text)
        self.assertNotIn("wallet", text.lower().replace("requirements-wallet", "requirements"))

    def test_contract_is_exactly_pinned(self) -> None:
        self.assertEqual(subject.CHAIN_ID, 84_532)
        self.assertEqual(subject.DEPLOYER_TARGET_USDC, 900_000)
        self.assertEqual(subject.MAX_TRANSFER_USDC, 22_500)
        self.assertEqual(subject.MINIMUM_BROKER_USDC_AFTER, 100_000)
        self.assertEqual(subject.BROKER, "0x176f486a724720c4fdfc920d7c17dd1004c2bfb4")
        self.assertEqual(subject.DEPLOYER, "0xfd7be4c69541ab297aece2a674fc1418b898cc0a")


if __name__ == "__main__":
    unittest.main()
