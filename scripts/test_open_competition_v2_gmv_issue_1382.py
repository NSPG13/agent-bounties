from __future__ import annotations

from datetime import datetime
import json
from pathlib import Path
import subprocess
import sys
import unittest

from eth_keys import keys


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from forward_canonical_gmv import (
    attestation_digest,
    recover_signer,
    snapshot_hash,
    verification_policy_hash,
)


class OpenCompetitionV2GmvIssue1382Tests(unittest.TestCase):
    """
    Validation test suite for Open Competition V2 fortnight 20260907 forward GMV issue 1382.
    """

    @classmethod
    def setUpClass(cls) -> None:
        """
        Load configuration files and manifests.
        """
        cohort_path = ROOT / "ops" / "open-competition-v2-forward-gmv-reward-cohort-v1.json"
        metadata_path = ROOT / "ops" / "open-competition-v2-public-metadata-v1.json"
        cls.cohort = json.loads(cohort_path.read_text(encoding="utf-8"))
        cls.metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
        cls.target_candidate = "external-gmv-reward-6usdc-fortnight-20260907-v1"
        cls.candidate = next(
            c for c in cls.cohort["candidates"] if c["candidate_id"] == cls.target_candidate
        )
        cls.meta_entry = next(
            m for m in cls.metadata["competitions"] if m.get("seed_id") == cls.target_candidate
        )

    def test_campaign_metadata_and_policy_hash_identity(self) -> None:
        """
        Verify that the campaign identity, epoch ID, and verification policy hash match exactly.
        """
        candidate = self.candidate
        meta_entry = self.meta_entry

        self.assertEqual(
            meta_entry["competition"].lower(),
            "0x81f0dd1f7da5f53ab6317e27131f9af45392b84c",
        )
        self.assertEqual(
            meta_entry["bounty_id"].lower(),
            "0x6f4939b0e12c8dd5d84efbfec0c7933bcdfcac9fb012c1b63014c6a6be7cad04",
        )
        self.assertEqual(
            candidate["epoch"]["epoch_id"].lower(),
            "0x34750799f9f543ea8b35feaceb2b50b248ce358b3aad071f482900399c394b8c",
        )
        self.assertEqual(candidate["epoch"]["starts_at"], "2026-09-07T00:00:00Z")
        self.assertEqual(candidate["epoch"]["ends_at"], "2026-09-21T00:00:00Z")
        self.assertEqual(candidate["epoch"]["minimum_score_base_units"], 1)

        starts = int(datetime.fromisoformat(candidate["epoch"]["starts_at"]).timestamp())
        ends = int(datetime.fromisoformat(candidate["epoch"]["ends_at"]).timestamp())
        self.assertEqual(starts, 1788739200)
        self.assertEqual(ends, 1789948800)

        campaign = {
            "lane": candidate["gmv_lane"],
            "starts_at": starts,
            "ends_at": ends,
            "epoch_id": candidate["epoch"]["epoch_id"],
            "minimum_score_base_units": candidate["epoch"]["minimum_score_base_units"],
            "excluded_wallets": self.cohort["eligibility_policy"]["excluded_wallets"],
            "excluded_bounty_contracts": self.cohort["eligibility_policy"]["excluded_bounty_contracts"],
            "snapshot_attesters": candidate["snapshot"]["snapshot_attesters"],
            "snapshot_attestation_threshold": candidate["snapshot"]["snapshot_attestation_threshold"],
        }
        computed_policy = "0x" + verification_policy_hash(campaign).hex()
        expected_policy = "0x617eb9935be1dd5d7287068ec5a9ce3e15b92851a84ec8633813199863b05e73"
        self.assertEqual(computed_policy, expected_policy)
        self.assertEqual(candidate["snapshot"]["verification_policy_hash"], expected_policy)

    def test_public_metadata_registry_binding(self) -> None:
        """
        Verify that public metadata binds the candidate to the competition contract.
        """
        matches = [
            item
            for item in self.metadata["competitions"]
            if item.get("seed_id") == self.target_candidate
        ]
        self.assertEqual(len(matches), 1)
        match = matches[0]
        self.assertEqual(
            match["competition"].lower(),
            "0x81f0dd1f7da5f53ab6317e27131f9af45392b84c",
        )
        self.assertEqual(
            match["bounty_id"].lower(),
            "0x6f4939b0e12c8dd5d84efbfec0c7933bcdfcac9fb012c1b63014c6a6be7cad04",
        )

    def test_dual_attester_quorum_verification(self) -> None:
        """
        Verify the 2-of-2 dual attester quorum requirements using cryptographic signing.
        """
        candidate = self.candidate
        starts = int(datetime.fromisoformat(candidate["epoch"]["starts_at"]).timestamp())
        ends = int(datetime.fromisoformat(candidate["epoch"]["ends_at"]).timestamp())
        campaign = {
            "lane": candidate["gmv_lane"],
            "starts_at": starts,
            "ends_at": ends,
            "epoch_id": candidate["epoch"]["epoch_id"],
            "minimum_score_base_units": candidate["epoch"]["minimum_score_base_units"],
            "excluded_wallets": self.cohort["eligibility_policy"]["excluded_wallets"],
            "excluded_bounty_contracts": self.cohort["eligibility_policy"]["excluded_bounty_contracts"],
            "snapshot_attesters": candidate["snapshot"]["snapshot_attesters"],
            "snapshot_attestation_threshold": candidate["snapshot"]["snapshot_attestation_threshold"],
        }
        policy = verification_policy_hash(campaign)

        mock_end_block_hash = "0x" + "aa" * 32
        snapshot = {
            "start_block": 50500000,
            "end_safe_block": 50600000,
            "end_block_hash": mock_end_block_hash,
            "settlements": [
                {
                    "protocol": "open_competition_v2",
                    "bounty_contract": "0x1234567890123456789012345678901234567890",
                    "bounty_id": "0x" + "01" * 32,
                    "creator": "0x2222222222222222222222222222222222222222",
                    "solver": "0x3333333333333333333333333333333333333333",
                    "settled_at": 1788800000,
                    "block_number": 50550000,
                    "transaction_hash": "0x" + "02" * 32,
                    "log_index": 0,
                    "gmv_base_units": 900000,
                    "funding": [
                        {
                            "contributor": "0x2222222222222222222222222222222222222222",
                            "amount_base_units": 900000,
                        }
                    ],
                }
            ],
        }
        frozen = snapshot_hash(campaign, snapshot)
        digest = attestation_digest(policy, frozen, mock_end_block_hash)

        priv1 = keys.PrivateKey(bytes.fromhex("11" * 32))
        priv2 = keys.PrivateKey(bytes.fromhex("22" * 32))

        sig1 = "0x" + priv1.sign_msg_hash(digest).to_bytes().hex()
        sig2 = "0x" + priv2.sign_msg_hash(digest).to_bytes().hex()

        rec1 = recover_signer(digest, sig1)
        rec2 = recover_signer(digest, sig2)
        self.assertEqual(rec1, priv1.public_key.to_checksum_address().lower())
        self.assertEqual(rec2, priv2.public_key.to_checksum_address().lower())

        attesters = [a.lower() for a in candidate["snapshot"]["snapshot_attesters"]]
        self.assertEqual(len(attesters), 2)
        self.assertIn("0x6fe4d6da2a4371d82b4a7ff94810a94091fb4c35", attesters)
        self.assertIn("0xfd7be4c69541ab297aece2a674fc1418b898cc0a", attesters)
        self.assertEqual(candidate["snapshot"]["snapshot_attestation_threshold"], 2)

    def test_pro_rata_scoring_calculation(self) -> None:
        """
        Verify the mathematical scoring attribution according to sum(gmv * entrant_funding / total_funding).
        """
        settlement = {
            "gmv_base_units": 900000,
            "funding": [
                {"contributor": "0x2222222222222222222222222222222222222222", "amount": 600000},
                {"contributor": "0x3333333333333333333333333333333333333333", "amount": 300000},
            ],
        }
        total_funding = sum(item["amount"] for item in settlement["funding"])
        entrant_wallet = "0x2222222222222222222222222222222222222222"
        entrant_funding = sum(
            item["amount"]
            for item in settlement["funding"]
            if item["contributor"].lower() == entrant_wallet.lower()
        )
        attributed_gmv = (settlement["gmv_base_units"] * entrant_funding) // total_funding
        self.assertEqual(attributed_gmv, 600000)
        self.assertGreaterEqual(attributed_gmv, self.candidate["epoch"]["minimum_score_base_units"])

    def test_benchmark_runner_and_self_test(self) -> None:
        """
        Run the Node.js benchmark test suite and self-test harness.
        """
        benchmark_dir = ROOT / "benchmarks" / "standing-meta-v2" / "gmv-fortnight-20260907"
        runner_res = subprocess.run(
            ["node", str(benchmark_dir / "test.mjs"), str(ROOT)],
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertIn("gmv_fortnight_20260907_benchmark=passed cases=9", runner_res.stdout)

        selftest_res = subprocess.run(
            ["node", str(benchmark_dir / "self-test.mjs")],
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertIn("gmv_fortnight_20260907_benchmark_self_test=passed", selftest_res.stdout)

    def test_child_bounty_specification_document(self) -> None:
        """
        Verify the existence and structure of the child bounty specification file.
        """
        bounty_file = ROOT / "bounties" / "gmv-fortnight-20260907-child-bounty.md"
        self.assertTrue(bounty_file.exists())
        content = bounty_file.read_text(encoding="utf-8")
        self.assertIn("Forward GMV Settlement Tooling Child Bounty", content)
        self.assertIn("0.90 USDC", content)
        self.assertIn("0x81f0dd1f7da5f53ab6317e27131f9af45392b84c", content)
        self.assertIn("0x6f4939b0e12c8dd5d84efbfec0c7933bcdfcac9fb012c1b63014c6a6be7cad04", content)
        self.assertIn("0x6fe4d6da2a4371d82b4a7ff94810a94091fb4c35", content)
        self.assertIn("0xfd7be4c69541ab297aece2a674fc1418b898cc0a", content)


if __name__ == "__main__":
    unittest.main()
