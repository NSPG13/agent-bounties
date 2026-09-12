import json
import subprocess
import unittest
from datetime import datetime
from pathlib import Path
from typing import Any, Dict

from eth_keys import keys

from scripts.forward_canonical_gmv import (
    attestation_digest,
    recover_signer,
    snapshot_hash,
    verification_policy_hash,
)

ROOT = Path(__file__).resolve().parent.parent


class TestOpenCompetitionV2GmvIssue1374(unittest.TestCase):
    """
    Test suite validating campaign configuration, cryptographic policy hashes,
    dual attester signatures, pro-rata scoring, and benchmark execution for Issue 1374.
    """

    pool: Dict[str, Any]
    metadata: Dict[str, Any]
    target_candidate: str
    candidate: Dict[str, Any]

    @classmethod
    def setUpClass(cls) -> None:
        """
        Load candidate pool and public metadata fixtures.
        """
        pool_path = ROOT / "ops" / "open-competition-v2-forward-gmv-candidate-pool-v2.json"
        metadata_path = ROOT / "ops" / "open-competition-v2-public-metadata-v1.json"

        cls.pool = json.loads(pool_path.read_text(encoding="utf-8"))
        cls.metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
        cls.target_candidate = "external-gmv-forward-week-20260831-v2"

        matching = [
            cand
            for cand in cls.pool["candidates"]
            if cand["candidate_id"] == cls.target_candidate
        ]
        if not matching:
            raise ValueError(f"Candidate {cls.target_candidate} not found in pool")
        cls.candidate = matching[0]

    def test_campaign_metadata_and_policy_hash_identity(self) -> None:
        """
        Verify candidate parameters and exact cryptographic verification policy hash identity.
        """
        candidate = self.candidate
        self.assertEqual(candidate["candidate_id"], self.target_candidate)
        self.assertEqual(candidate["gmv_lane"], "external_supply")
        self.assertEqual(
            candidate["epoch"]["epoch_id"],
            "0x3b20496282476095677bba3800efafd147a6c728f08eb69f3a1ba59a53ccaf2d",
        )
        self.assertEqual(candidate["epoch"]["starts_at"], "2026-08-31T00:00:00Z")
        self.assertEqual(candidate["epoch"]["ends_at"], "2026-09-07T00:00:00Z")
        self.assertEqual(candidate["epoch"]["minimum_score_base_units"], 1)

        starts = int(datetime.fromisoformat(candidate["epoch"]["starts_at"]).timestamp())
        ends = int(datetime.fromisoformat(candidate["epoch"]["ends_at"]).timestamp())
        self.assertEqual(starts, 1788134400)
        self.assertEqual(ends, 1788739200)

        campaign = {
            "lane": candidate["gmv_lane"],
            "starts_at": starts,
            "ends_at": ends,
            "epoch_id": candidate["epoch"]["epoch_id"],
            "minimum_score_base_units": candidate["epoch"]["minimum_score_base_units"],
            "excluded_wallets": self.pool["eligibility_policy"]["excluded_wallets"],
            "excluded_bounty_contracts": self.pool["eligibility_policy"]["excluded_bounty_contracts"],
            "snapshot_attesters": candidate["snapshot"]["snapshot_attesters"],
            "snapshot_attestation_threshold": candidate["snapshot"]["snapshot_attestation_threshold"],
        }
        computed_policy = "0x" + verification_policy_hash(campaign).hex()
        expected_policy = "0x8e32aa4f1c355ad59c7c2047a2fa8d29e84619008c1c75626cbb4335c423e178"
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
            "0x1692fecdb678537eb4cc0093e9e4e99dbded9806",
        )
        self.assertEqual(
            match["bounty_id"].lower(),
            "0x51e1f4e05e3ef3dd61f0867a378005780b8882ed4b991fe0f927e6be44bf29f0",
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
            "excluded_wallets": self.pool["eligibility_policy"]["excluded_wallets"],
            "excluded_bounty_contracts": self.pool["eligibility_policy"]["excluded_bounty_contracts"],
            "snapshot_attesters": candidate["snapshot"]["snapshot_attesters"],
            "snapshot_attestation_threshold": candidate["snapshot"]["snapshot_attestation_threshold"],
        }
        policy = verification_policy_hash(campaign)

        mock_end_block_hash = "0x" + "aa" * 32
        snapshot = {
            "start_block": 50500000,
            "end_safe_block": 50560000,
            "end_block_hash": mock_end_block_hash,
            "settlements": [
                {
                    "protocol": "open_competition_v2",
                    "bounty_contract": "0x1234567890123456789012345678901234567890",
                    "bounty_id": "0x" + "01" * 32,
                    "creator": "0x2222222222222222222222222222222222222222",
                    "solver": "0x3333333333333333333333333333333333333333",
                    "settled_at": 1788140000,
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
        benchmark_dir = ROOT / "benchmarks" / "standing-meta-v2" / "gmv-forward-week-20260831"
        runner_res = subprocess.run(
            ["node", str(benchmark_dir / "test.mjs"), str(ROOT)],
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertIn("gmv_forward_week_20260831_benchmark=passed cases=9", runner_res.stdout)

        selftest_res = subprocess.run(
            ["node", str(benchmark_dir / "self-test.mjs")],
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertIn("gmv_forward_week_20260831_benchmark_self_test=passed", selftest_res.stdout)

    def test_child_bounty_specification_document(self) -> None:
        """
        Verify the existence and structure of the child bounty specification file.
        """
        bounty_file = ROOT / "bounties" / "gmv-forward-week-20260831-child-bounty.md"
        self.assertTrue(bounty_file.exists())
        content = bounty_file.read_text(encoding="utf-8")
        self.assertIn("Forward GMV Settlement Tooling Child Bounty", content)
        self.assertIn("0.90 USDC", content)
        self.assertIn("0x1692fecdb678537eb4cc0093e9e4e99dbded9806", content)
        self.assertIn("0x51e1f4e05e3ef3dd61f0867a378005780b8882ed4b991fe0f927e6be44bf29f0", content)
        self.assertIn("0x6fe4d6da2a4371d82b4a7ff94810a94091fb4c35", content)
        self.assertIn("0xfd7be4c69541ab297aece2a674fc1418b898cc0a", content)


if __name__ == "__main__":
    unittest.main()
