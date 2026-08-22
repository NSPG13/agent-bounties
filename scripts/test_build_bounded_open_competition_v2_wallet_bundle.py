#!/usr/bin/env python3
"""Tests for the deterministic bounded V2 reserve deployment manifest."""

from __future__ import annotations

import re
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / "scripts"
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))

import build_bounded_open_competition_v2_wallet_bundle as BUILDER


class BoundedOpenCompetitionV2BundleTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.bundle = BUILDER.build_bundle()

    def test_pins_production_release_and_recovery_contracts(self) -> None:
        bundle = self.bundle
        self.assertEqual(
            bundle["schema"],
            "agent-bounties/bounded-open-competition-v2-wallet-deployment-v1",
        )
        self.assertEqual(bundle["network"], "base-mainnet")
        self.assertEqual(bundle["chain_id"], 8453)
        self.assertEqual(bundle["canonical"]["competition_factory"], BUILDER.COMPETITION_FACTORY)
        self.assertEqual(bundle["canonical"]["settlement_token"], BUILDER.USDC)
        self.assertEqual(bundle["canonical"]["release_hash"], BUILDER.RELEASE_HASH)
        self.assertEqual(
            set(bundle["contracts"]),
            {BUILDER.FACTORY_CONTRACT, BUILDER.WALLET_CONTRACT},
        )

    def test_addresses_hashes_and_deployment_transaction_are_exact(self) -> None:
        deployment = self.bundle["reserve_factory"]
        for field in ("address", "implementation"):
            self.assertRegex(deployment[field], r"^0x[0-9a-f]{40}$")
        for field in (
            "salt",
            "init_code_hash",
            "runtime_code_hash",
            "implementation_runtime_code_hash",
            "clone_runtime_code_hash",
        ):
            self.assertRegex(deployment[field], r"^0x[0-9a-f]{64}$")
        self.assertRegex(deployment["deployment_transaction"], r"^0x(?:[0-9a-f]{2})+$")
        self.assertTrue(deployment["deployment_transaction"].startswith(deployment["salt"]))
        self.assertLess(deployment["runtime_size_bytes"], 24_576)
        self.assertLess(deployment["implementation_runtime_size_bytes"], 24_576)

    def test_manifest_is_content_addressed_and_contains_no_secret(self) -> None:
        self.assertRegex(self.bundle["contract_source_revision"], r"^[0-9a-f]{40}$")
        serialized = str(self.bundle).lower()
        for forbidden in ("private_key", "seed phrase", "mnemonic", "bearer_token"):
            self.assertNotIn(forbidden, serialized)
        for value in self.bundle["contracts"].values():
            self.assertTrue(re.fullmatch(r"0x[0-9a-f]{64}", value["source_sha256"]))

    def test_release_binding_is_an_explicit_build_input(self) -> None:
        competition_factory = "0x" + "12" * 20
        release_hash = "0x" + "34" * 32
        rebound = BUILDER.build_bundle(competition_factory, release_hash)
        self.assertEqual(rebound["canonical"]["competition_factory"], competition_factory)
        self.assertEqual(rebound["canonical"]["release_hash"], release_hash)
        self.assertNotEqual(
            rebound["reserve_factory"]["address"],
            self.bundle["reserve_factory"]["address"],
        )

    def test_release_binding_rejects_malformed_values(self) -> None:
        with self.assertRaises(SystemExit):
            BUILDER.build_bundle("0x1234", BUILDER.RELEASE_HASH)
        with self.assertRaises(SystemExit):
            BUILDER.build_bundle(BUILDER.COMPETITION_FACTORY, "0x" + "00" * 32)


if __name__ == "__main__":
    unittest.main()
