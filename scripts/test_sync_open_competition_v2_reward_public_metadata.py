from __future__ import annotations

import copy
from datetime import datetime, timezone
import json
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import build_open_competition_v2_reward_policy as BUILDER  # noqa: E402
import sync_open_competition_v2_reward_public_metadata as MODULE  # noqa: E402


class RewardPublicMetadataTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.registry = json.loads(
            (ROOT / "ops" / "open-competition-v2-public-metadata-v1.json").read_text()
        )
        cls.registry["competitions"] = [
            item
            for item in cls.registry["competitions"]
            if item.get("source_url") != MODULE.SOURCE_URL
        ]
        cls.cohort = json.loads(
            (
                ROOT / "ops" / "open-competition-v2-forward-gmv-reward-cohort-v1.json"
            ).read_text()
        )
        profile = cls.cohort["profile_release"]
        reviewed_state = {
            "schema_version": BUILDER.STATE_SCHEMA,
            "network": "base-mainnet",
            "chain_id": 8453,
            "block_tag": "safe",
            "safe_block": 50_000_000,
            "reserve_wallet": cls.cohort["reserve_wallet"],
            "owner": BUILDER.OWNER,
            "settlement_token": BUILDER.USDC,
            "competition_factory": cls.cohort["factory_contract"],
            "competition_implementation": "0x" + "ab" * 20,
            "policy_version": 1,
            "active_policy_hash": "0x" + "12" * 32,
            "period_bucket": 20_688,
            "period_spent": 30_400_000,
            "lifetime_spent": 30_400_000,
            "reserve_balance": 47_268_098,
            "revoked": False,
            "policy": {
                "delegate": "0xb358898d34c5e907877a1cd7540b234f6851f61b",
                "valid_after": 1_787_503_420,
                "valid_until": 1_793_491_199,
                "period_seconds": 86_400,
                "solver_reward": 3_000_000,
                "keeper_reward": 40_000,
                "exact_funding_per_competition": 3_040_000,
                "max_per_period": 30_400_000,
                "max_lifetime_spend": 77_668_098,
                "beta_risk_hash": "0x" + "34" * 32,
                "gmv_metric_program_hash": profile["metric_program_hash"],
                "gmv_journal_schema_hash": profile["journal_schema_hash"],
            },
        }
        cls.bundle = BUILDER.build_rotation(
            cls.cohort,
            reviewed_state,
            datetime(2026, 8, 24, 10, 15, tzinfo=timezone.utc),
        )
        observed = [
            {
                "candidate_id": item["candidate_id"],
                "competition": item["predicted_competition"],
                "used": True,
                "approved": True,
                "status": 1,
            }
            for item in cls.bundle["creations"]
        ]
        cls.result = {
            "schema_version": MODULE.RESULT_SCHEMA,
            "status": "canonically_activated",
            "state": {
                "used_count": len(observed),
                "active_count": len(observed),
                "creations": observed,
            },
        }

    def test_exact_active_cohort_is_appended_idempotently(self) -> None:
        original_count = len(self.registry["competitions"])
        first = MODULE.synchronize(
            copy.deepcopy(self.registry),
            copy.deepcopy(self.cohort),
            copy.deepcopy(self.bundle),
            copy.deepcopy(self.result),
        )
        self.assertEqual(len(first["competitions"]), original_count + 5)
        second = MODULE.synchronize(
            first,
            copy.deepcopy(self.cohort),
            copy.deepcopy(self.bundle),
            copy.deepcopy(self.result),
        )
        self.assertEqual(second, first)
        reward = first["competitions"][-5:]
        self.assertEqual(
            [item["competition"] for item in reward],
            [item["predicted_competition"] for item in self.bundle["creations"]],
        )
        self.assertTrue(all(item["source_url"] == MODULE.SOURCE_URL for item in reward))

    def test_incomplete_or_drifted_activation_fails_closed(self) -> None:
        cases = []
        incomplete = copy.deepcopy(self.result)
        incomplete["state"]["active_count"] -= 1
        cases.append(incomplete)
        drifted = copy.deepcopy(self.result)
        drifted["state"]["creations"][0]["competition"] = "0x" + "ff" * 20
        cases.append(drifted)
        unused = copy.deepcopy(self.result)
        unused["state"]["creations"][0]["used"] = False
        cases.append(unused)
        for result in cases:
            with self.subTest(), self.assertRaises(MODULE.MetadataSyncError):
                MODULE.synchronize(
                    copy.deepcopy(self.registry),
                    copy.deepcopy(self.cohort),
                    copy.deepcopy(self.bundle),
                    result,
                )

    def test_conflicting_existing_metadata_fails_closed(self) -> None:
        registry = copy.deepcopy(self.registry)
        registry["competitions"].append(
            {
                "seed_id": self.cohort["candidates"][0]["candidate_id"],
                "competition": "0x" + "ee" * 20,
                "bounty_id": "0x" + "ee" * 32,
            }
        )
        with self.assertRaises(MODULE.MetadataSyncError):
            MODULE.synchronize(
                registry,
                copy.deepcopy(self.cohort),
                copy.deepcopy(self.bundle),
                copy.deepcopy(self.result),
            )


if __name__ == "__main__":
    unittest.main()
