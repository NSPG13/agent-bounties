#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).with_name("activate_standing_meta_v3_replacements.py")
SPEC = importlib.util.spec_from_file_location("activate_standing_meta_v3_replacements", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ActivateStandingMetaV3Tests(unittest.TestCase):
    def test_migration_economics_are_exact(self) -> None:
        self.assertEqual(MODULE.TARGET, 2_010_000)
        self.assertEqual(MODULE.TOTAL, 8_040_000)
        self.assertEqual(sorted(MODULE.ISSUES), [333, 334, 335, 336])

    def test_create_payload_uses_published_commitments_and_v3(self) -> None:
        document = {
            "contract_terms": {
                "creator_wallet": MODULE.WALLET,
                "solver_reward": {"amount": 2_000_000, "currency": "usdc"},
                "verifier_reward": {"amount": 10_000, "currency": "usdc"},
                "initial_funding": {"amount": 2_010_000, "currency": "usdc"},
                "funding_deadline": 1_791_676_800,
                "claim_window_seconds": 1_209_600,
                "verification_window_seconds": 1_209_600,
                "creation_nonce": "0x" + "11" * 32,
            },
            "verification_policy": {
                "verifier_module": MODULE.V3,
                "verifier_reward_recipient": MODULE.KEEPER,
            },
        }
        published = {
            "terms_hash": "0x" + "21" * 32,
            "policy_hash": "0x" + "22" * 32,
            "acceptance_criteria_hash": "0x" + "23" * 32,
            "benchmark_hash": "0x" + "24" * 32,
            "evidence_schema_hash": "0x" + "25" * 32,
        }
        value = MODULE.create_payload(document, published)
        self.assertEqual(value["creator"], MODULE.WALLET)
        self.assertEqual(value["verifier_module"], MODULE.V3)
        self.assertEqual(value["initial_funding"]["amount"], MODULE.TARGET)
        self.assertEqual(value["terms_hash"], published["terms_hash"])
        self.assertEqual(value["threshold"], 1)
        self.assertEqual(value["verifiers"], [])

    def test_issue_body_advertises_profit_and_evidence_boundary(self) -> None:
        body = MODULE.issue_body(
            333,
            "cli",
            MODULE.ISSUES[333]["old"],
            {"contract": "0x" + "12" * 20, "transaction_hash": "0x" + "13" * 32},
        )
        self.assertIn("2.00 USDC", body)
        self.assertIn("0.01 USDC", body)
        self.assertIn("1.00 USDC gross profit", body)
        self.assertIn("Only canonical `BountySettled` proves earnings", body)
        self.assertIn(MODULE.ISSUES[333]["old"], body)

    def test_policy_constants_are_minimally_bounded(self) -> None:
        self.assertEqual(MODULE.POLICY_SECONDS, 7_200)
        self.assertEqual(MODULE.KEEPER, "0xc26a630e85134ed30968735c8e7de4576cfa5dbc")
        self.assertEqual(MODULE.V3, "0x8e3d799d3d2cf52112e5be4ce48f105379462077")
        self.assertEqual(MODULE.ZERO_HASH, "0x" + "00" * 32)


if __name__ == "__main__":
    unittest.main()
