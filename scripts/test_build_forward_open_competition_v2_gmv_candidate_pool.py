from __future__ import annotations

import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import build_forward_open_competition_v2_gmv_candidate_pool as MODULE
from forward_canonical_gmv import verification_policy_hash


class ForwardGmvCandidatePoolTests(unittest.TestCase):
    def identity(self, reproduced: bool) -> dict:
        return {
            "status": "reproduced_beta3" if reproduced else "awaiting_reproduction",
            "program_vkey": "0x" + "11" * 32 if reproduced else None,
            "source_hash": "0x" + "22" * 32 if reproduced else None,
            "elf_keccak256": "0x" + "33" * 32 if reproduced else None,
        }

    def build(self, reproduced: bool = True) -> dict:
        return MODULE.build("0x" + "44" * 20, "0x" + "55" * 32, self.identity(reproduced))

    def test_pool_contains_ten_initial_and_ten_standby_forward_competitions(self) -> None:
        pool = self.build()
        self.assertEqual(pool["profile_release"]["profile_id"], MODULE.PROFILE_ID)
        self.assertEqual(pool["profile_release"]["status"], "reviewed")
        self.assertEqual(len(pool["candidates"]), 20)
        self.assertTrue(all("role" not in item for item in pool["candidates"]))
        for candidate in pool["candidates"]:
            campaign = {
                "epoch_id": candidate["epoch"]["epoch_id"],
                "starts_at": MODULE.timestamp(candidate["epoch"]["starts_at"]),
                "ends_at": MODULE.timestamp(candidate["epoch"]["ends_at"]),
                "minimum_score_base_units": candidate["epoch"]["minimum_score_base_units"],
                "excluded_wallets": pool["eligibility_policy"]["excluded_wallets"],
                "excluded_bounty_contracts": pool["eligibility_policy"]["excluded_bounty_contracts"],
                "snapshot_attesters": pool["attestation_policy"]["attesters"],
                "snapshot_attestation_threshold": pool["attestation_policy"]["threshold"],
            }
            self.assertEqual(candidate["snapshot"]["status"], "scheduled")
            self.assertEqual(
                candidate["snapshot"]["verification_policy_hash"],
                "0x" + verification_policy_hash(campaign).hex(),
            )

    def test_pending_identity_cannot_impersonate_reviewed_release(self) -> None:
        profile = self.build(False)["profile_release"]
        self.assertEqual(profile["status"], "awaiting_reproduction")
        self.assertIsNone(profile["program_vkey"])
        self.assertIsNone(profile["source_hash"])
        self.assertIsNone(profile["elf_hash"])


if __name__ == "__main__":
    unittest.main()
