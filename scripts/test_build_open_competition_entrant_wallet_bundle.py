#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PATH = ROOT / "scripts" / "build_open_competition_entrant_wallet_bundle.py"
SPEC = importlib.util.spec_from_file_location("build_open_competition_entrant_wallet_bundle", PATH)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class OpenCompetitionEntrantWalletBundleTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.sepolia = MODULE.build_bundle("base-sepolia")
        cls.mainnet = MODULE.build_bundle("base-mainnet", compile_contracts=False)

    def test_networks_bind_distinct_frozen_factories(self) -> None:
        self.assertEqual(self.sepolia["chain_id"], 84532)
        self.assertEqual(self.mainnet["chain_id"], 8453)
        self.assertNotEqual(
            self.sepolia["canonical"]["competition_factory"],
            self.mainnet["canonical"]["competition_factory"],
        )
        self.assertNotEqual(
            self.sepolia["entrant_wallet_factory"]["address"],
            self.mainnet["entrant_wallet_factory"]["address"],
        )

    def test_bundle_is_deterministic_and_under_code_limits(self) -> None:
        rebuilt = MODULE.build_bundle("base-sepolia", compile_contracts=False)
        self.assertEqual(
            rebuilt["entrant_wallet_factory"]["address"],
            self.sepolia["entrant_wallet_factory"]["address"],
        )
        self.assertLess(self.sepolia["entrant_wallet_factory"]["implementation_runtime_code_bytes"], 24_576)
        self.assertLess(self.sepolia["entrant_wallet_factory"]["runtime_code_bytes"], 24_576)

    def test_activation_defaults_fail_closed(self) -> None:
        self.assertTrue(all(value is False for value in self.mainnet["activation_gates"].values()))
        self.assertEqual(self.mainnet["deployment_state"], "source_only_not_ready_to_earn")


if __name__ == "__main__":
    unittest.main()
