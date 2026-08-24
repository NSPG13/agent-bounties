from __future__ import annotations

import copy
import json
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import build_open_competition_v2_reward_cohort as MODULE  # noqa: E402
from forward_canonical_gmv import verification_policy_hash  # noqa: E402


class RewardCohortTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.baseline = json.loads(
            (
                ROOT / "ops" / "open-competition-v2-forward-gmv-candidate-pool-v2.json"
            ).read_text()
        )
        cls.contracts = [f"0x{index:040x}" for index in range(1, 11)]

    def build(self) -> dict:
        return MODULE.build_cohort(
            copy.deepcopy(self.baseline), self.contracts, "2026-08-24T10:00:00Z"
        )

    def test_builds_five_exact_matched_reward_treatments(self) -> None:
        cohort = self.build()
        self.assertEqual(cohort["schema_version"], MODULE.SCHEMA)
        self.assertEqual(cohort["economics"]["solver_reward_base_units"], 6_000_000)
        self.assertEqual(len(cohort["candidates"]), 5)
        self.assertEqual(cohort["experiment"]["minimum_qualified_starts"], 10)
        self.assertTrue(
            set(self.contracts).issubset(
                cohort["eligibility_policy"]["excluded_bounty_contracts"]
            )
        )
        for candidate in cohort["candidates"]:
            control = candidate["matched_control"]
            self.assertEqual(control["starts_at"], candidate["epoch"]["starts_at"])
            self.assertEqual(control["ends_at"], candidate["epoch"]["ends_at"])
            self.assertEqual(control["solver_reward_base_units"], 3_000_000)
            campaign = {
                "epoch_id": candidate["epoch"]["epoch_id"],
                "starts_at": MODULE.timestamp(candidate["epoch"]["starts_at"]),
                "ends_at": MODULE.timestamp(candidate["epoch"]["ends_at"]),
                "minimum_score_base_units": candidate["epoch"][
                    "minimum_score_base_units"
                ],
                "excluded_wallets": cohort["eligibility_policy"]["excluded_wallets"],
                "excluded_bounty_contracts": cohort["eligibility_policy"][
                    "excluded_bounty_contracts"
                ],
                "snapshot_attesters": cohort["attestation_policy"]["attesters"],
                "snapshot_attestation_threshold": cohort["attestation_policy"][
                    "threshold"
                ],
            }
            self.assertEqual(
                candidate["snapshot"]["verification_policy_hash"],
                "0x" + verification_policy_hash(campaign).hex(),
            )

    def test_fails_closed_on_bad_contract_set_or_late_review(self) -> None:
        with self.assertRaises(MODULE.CohortError):
            MODULE.build_cohort(
                copy.deepcopy(self.baseline), self.contracts[:9], "2026-08-24T10:00:00Z"
            )
        with self.assertRaises(MODULE.CohortError):
            MODULE.build_cohort(
                copy.deepcopy(self.baseline), self.contracts, "2026-08-25T00:00:00Z"
            )

    def test_fails_closed_when_control_economics_change(self) -> None:
        baseline = copy.deepcopy(self.baseline)
        baseline["economics"]["solver_reward_base_units"] = 4_000_000
        with self.assertRaises(MODULE.CohortError):
            MODULE.build_cohort(baseline, self.contracts, "2026-08-24T10:00:00Z")


if __name__ == "__main__":
    unittest.main()
