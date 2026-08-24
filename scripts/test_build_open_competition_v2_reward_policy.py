from __future__ import annotations

import copy
from datetime import datetime, timezone
import json
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import build_open_competition_v2_reward_policy as MODULE  # noqa: E402


class RewardPolicyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.cohort = json.loads(
            (
                ROOT / "ops" / "open-competition-v2-forward-gmv-reward-cohort-v1.json"
            ).read_text()
        )
        profile = cls.cohort["profile_release"]
        cls.state = {
            "schema_version": MODULE.STATE_SCHEMA,
            "network": "base-mainnet",
            "chain_id": 8453,
            "block_tag": "safe",
            "safe_block": 50_000_000,
            "reserve_wallet": cls.cohort["reserve_wallet"],
            "owner": MODULE.OWNER,
            "settlement_token": MODULE.USDC,
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
        cls.now = datetime(2026, 8, 24, 10, 15, tzinfo=timezone.utc)

    def build(self, state_changes: dict | None = None) -> dict:
        state = copy.deepcopy(self.state)
        if state_changes:
            state.update(state_changes)
        return MODULE.build_rotation(copy.deepcopy(self.cohort), state, self.now)

    def test_builds_two_zero_value_owner_steps_and_five_exact_creations(
        self,
    ) -> None:
        bundle = self.build()
        self.assertEqual(bundle["schema_version"], MODULE.SCHEMA)
        self.assertEqual(bundle["owner_transactions"]["revoke"]["from"], MODULE.OWNER)
        self.assertEqual(bundle["owner_transactions"]["revoke"]["value_wei"], 0)
        self.assertEqual(bundle["owner_transactions"]["configure"]["value_wei"], 0)
        self.assertTrue(
            bundle["owner_transactions"]["configure"]["data"].startswith("0x")
        )
        self.assertEqual(bundle["next_policy"]["version"], 2)
        self.assertEqual(bundle["next_policy"]["solver_reward"], 6_000_000)
        self.assertEqual(bundle["next_policy"]["max_per_period"], 30_400_000)
        self.assertEqual(bundle["next_policy"]["max_lifetime_spend"], 77_668_098)
        self.assertEqual(len(bundle["approved_creation_commitments"]), 5)
        self.assertEqual(
            len({item["predicted_competition"] for item in bundle["creations"]}), 5
        )
        self.assertFalse(bundle["execution_boundary"]["policy_change_moves_usdc"])
        self.assertTrue(
            bundle["execution_boundary"][
                "policy_reconfiguration_does_not_create_a_fresh_period"
            ]
        )
        self.assertTrue(
            bundle["execution_boundary"]["elapsed_period_sync_resets_spend"]
        )
        self.assertEqual(
            bundle["execution_boundary"]["effective_period_spent_after_configuration"],
            0,
        )
        self.assertEqual(
            bundle["execution_boundary"]["earliest_treatment_spend_at"],
            int(self.now.timestamp()),
        )
        self.assertEqual(
            bundle["execution_boundary"]["treatment_total_base_units"], 30_200_000
        )
        self.assertEqual(
            bundle["execution_boundary"]["reserve_after_treatment_base_units"],
            17_068_098,
        )
        self.assertEqual(
            bundle["execution_boundary"]["later_floor_reserve_base_units"], 15_200_000
        )

    def test_fails_closed_on_state_drift_or_insufficient_floor_reserve(self) -> None:
        for changes in (
            {"owner": "0x" + "11" * 20},
            {"policy_version": 2},
            {"reserve_balance": 47_000_000},
            {"lifetime_spent": 31_000_000},
            {"block_tag": "latest"},
        ):
            with self.subTest(changes=changes), self.assertRaises(MODULE.RotationError):
                self.build(changes)

    def test_fails_closed_when_confirmation_is_too_close_to_first_window(self) -> None:
        with self.assertRaises(MODULE.RotationError):
            MODULE.build_rotation(
                copy.deepcopy(self.cohort),
                copy.deepcopy(self.state),
                datetime(2026, 8, 24, 23, 50, tzinfo=timezone.utc),
            )

    def test_same_period_does_not_reset_spend_and_waits_for_next_utc_bucket(
        self,
    ) -> None:
        state = copy.deepcopy(self.state)
        state["period_bucket"] = int(self.now.timestamp()) // MODULE.PERIOD_SECONDS
        bundle = MODULE.build_rotation(copy.deepcopy(self.cohort), state, self.now)
        self.assertFalse(
            bundle["execution_boundary"]["elapsed_period_sync_resets_spend"]
        )
        self.assertEqual(
            bundle["execution_boundary"]["earliest_treatment_spend_at"],
            (state["period_bucket"] + 1) * MODULE.PERIOD_SECONDS,
        )


if __name__ == "__main__":
    unittest.main()
