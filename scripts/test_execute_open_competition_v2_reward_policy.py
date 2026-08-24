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
import execute_open_competition_v2_reward_policy as MODULE  # noqa: E402


class RewardPolicyExecutionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.cohort = json.loads(
            (
                ROOT / "ops" / "open-competition-v2-forward-gmv-reward-cohort-v1.json"
            ).read_text()
        )
        profile = cls.cohort["profile_release"]
        cls.reviewed_state = {
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
            cls.reviewed_state,
            datetime(2026, 8, 24, 10, 15, tzinfo=timezone.utc),
        )
        cls.delegate = cls.bundle["next_policy"]["delegate"]
        cls.confirmation = {
            "status": "confirmed",
            "revoke_transaction_hash": "0x" + "ab" * 32,
            "configure_transaction_hash": "0x" + "cd" * 32,
            "transaction_block": 50_000_100,
            "safe_block": 50_000_200,
            "policy_version": cls.bundle["next_policy"]["version"],
            "policy_hash": cls.bundle["next_policy"]["hash"],
            "lifetime_spent": BUILDER.EXPECTED_LIFETIME_SPENT,
            "reserve_balance": BUILDER.EXPECTED_RESERVE_BALANCE,
            "usdc_moved_by_confirmation": 0,
        }

    def test_exact_reviewed_inputs_are_accepted(self) -> None:
        MODULE.validate_inputs(
            copy.deepcopy(self.cohort),
            copy.deepcopy(self.reviewed_state),
            copy.deepcopy(self.bundle),
            copy.deepcopy(self.confirmation),
            self.delegate,
        )

    def test_bundle_or_confirmation_mutation_fails_closed(self) -> None:
        cases = []
        changed_bundle = copy.deepcopy(self.bundle)
        changed_bundle["creations"][0]["delegate_transaction"]["data"] = "0x1234"
        cases.append((changed_bundle, copy.deepcopy(self.confirmation)))
        changed_confirmation = copy.deepcopy(self.confirmation)
        changed_confirmation["reserve_balance"] -= 1
        cases.append((copy.deepcopy(self.bundle), changed_confirmation))
        changed_hash = copy.deepcopy(self.confirmation)
        changed_hash["configure_transaction_hash"] = "0x1234"
        cases.append((copy.deepcopy(self.bundle), changed_hash))
        for bundle, confirmation in cases:
            with self.subTest(), self.assertRaises(MODULE.RewardExecutionError):
                MODULE.validate_inputs(
                    copy.deepcopy(self.cohort),
                    copy.deepcopy(self.reviewed_state),
                    bundle,
                    confirmation,
                    self.delegate,
                )

    def test_selection_keeps_reviewed_order_and_skips_used(self) -> None:
        state = {
            "creations": [
                {
                    "candidate_id": creation["candidate_id"],
                    "used": index in {1, 3},
                }
                for index, creation in enumerate(self.bundle["creations"])
            ]
        }
        selected = MODULE.selected_creations(state, self.bundle)
        self.assertEqual(
            [item["candidate_id"] for item in selected],
            [self.bundle["creations"][index]["candidate_id"] for index in (0, 2, 4)],
        )

    def test_accounting_accepts_a_utc_period_rollover(self) -> None:
        policy = copy.deepcopy(self.bundle["next_policy"])
        remaining = MODULE.validate_reward_accounting(
            policy,
            5,
            0,
            BUILDER.EXPECTED_LIFETIME_SPENT + 5 * 6_040_000,
            BUILDER.EXPECTED_RESERVE_BALANCE - 5 * 6_040_000,
        )
        self.assertEqual(remaining, 0)

    def test_accounting_rejects_impossible_current_period_spend(self) -> None:
        policy = copy.deepcopy(self.bundle["next_policy"])
        with self.assertRaises(MODULE.RewardExecutionError):
            MODULE.validate_reward_accounting(
                policy,
                2,
                6_040_001,
                BUILDER.EXPECTED_LIFETIME_SPENT + 2 * 6_040_000,
                BUILDER.EXPECTED_RESERVE_BALANCE - 2 * 6_040_000,
            )


if __name__ == "__main__":
    unittest.main()
