#!/usr/bin/env python3

from __future__ import annotations

import copy
import json
import sys
import unittest
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / "scripts"
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))

import build_open_competition_v2_gmv_activation as MODULE


class OpenCompetitionV2GmvActivationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.pool = json.loads(
            (ROOT / "ops" / "open-competition-v2-gmv-candidate-pool-v1.json").read_text()
        )
        cls.reserve = json.loads(
            (
                ROOT
                / "deployments"
                / "bounded-open-competition-v2-wallet-base-mainnet.json"
            ).read_text()
        )
        profile = cls.pool["profile_release"]
        cls.release = {
            "protocol_version": MODULE.PROTOCOL,
            "network": "base-mainnet",
            "settlement_token": MODULE.USDC,
            "factory_contract": cls.pool["factory_contract"],
            "implementation_contract": "0x" + "ab" * 20,
            "release_hash": cls.pool["release_hash"],
            "beta_risk_hash": "0x" + "cd" * 32,
            "metric_programs": [
                {
                    "profile_id": MODULE.PROFILE_ID,
                    "classification": "reviewed",
                    **{
                        field: profile[field]
                        for field in (
                            "program_vkey",
                            "source_hash",
                            "elf_hash",
                            "journal_schema_hash",
                            "metric_program_hash",
                        )
                    },
                }
            ],
        }
        cls.now = datetime(2026, 8, 22, 6, 30, tzinfo=timezone.utc)

    def build(self, **changes: object) -> dict:
        release = copy.deepcopy(self.release)
        reserve = copy.deepcopy(self.reserve)
        pool = copy.deepcopy(self.pool)
        release.update(changes.pop("release", {}))
        reserve.update(changes.pop("reserve", {}))
        pool.update(changes.pop("pool", {}))
        self.assertFalse(changes)
        return MODULE.build_activation(release, reserve, pool, self.now)

    def test_bundle_is_exact_recoverable_and_v2_only(self) -> None:
        value = self.build()
        self.assertEqual(value["owner"], MODULE.OWNER)
        self.assertEqual(value["delegate"], MODULE.DELEGATE)
        self.assertEqual(value["initial_funding_base_units"], 77_668_098)
        self.assertEqual(value["policy"]["max_per_period"], 30_400_000)
        self.assertEqual(value["policy"]["max_lifetime_spend"], 77_668_098)
        self.assertEqual(len(value["approved_creation_commitments"]), 20)
        self.assertEqual(len(value["creations"]), 20)
        self.assertEqual(len({item["predicted_competition"] for item in value["creations"]}), 20)
        self.assertEqual(
            value["owner_authorization"]["typed_data"]["message"]["to"].lower(),
            value["reserve_wallet"],
        )
        self.assertEqual(
            value["owner_authorization"]["typed_data"]["message"]["value"],
            "77668098",
        )
        for creation in value["creations"]:
            self.assertEqual(creation["params"]["winner_mode"], "best_score")
            self.assertEqual(creation["params"]["score_direction"], "higher_is_better")
            self.assertEqual(
                creation["params"]["metric_program_hash"],
                self.pool["profile_release"]["metric_program_hash"],
            )
            self.assertEqual(creation["delegate_transaction"]["to"], value["reserve_wallet"])

    def test_factory_or_release_mismatch_fails_closed(self) -> None:
        with self.assertRaises(MODULE.ActivationError):
            self.build(release={"factory_contract": "0x" + "11" * 20})
        with self.assertRaises(MODULE.ActivationError):
            self.build(pool={"release_hash": "0x" + "22" * 32})

    def test_non_external_or_expired_pool_fails_closed(self) -> None:
        pool = copy.deepcopy(self.pool)
        pool["candidates"][0]["gmv_lane"] = "retention"
        with self.assertRaises(MODULE.ActivationError):
            MODULE.build_activation(self.release, self.reserve, pool, self.now)
        with self.assertRaises(MODULE.ActivationError):
            self.build(pool={"expires_at": "2026-08-22T06:00:00Z"})


if __name__ == "__main__":
    unittest.main()
