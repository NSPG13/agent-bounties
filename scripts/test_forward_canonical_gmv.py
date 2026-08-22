from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import unittest


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from build_forward_canonical_gmv_fixture import OUTPUT, build
from forward_canonical_gmv import (
    attestation_digest,
    recover_signer,
    snapshot_hash,
    verification_policy_hash,
)


class ForwardCanonicalGmvTests(unittest.TestCase):
    def fixture(self) -> dict:
        return build()

    def wire(self, value: list[int]) -> str:
        return "0x" + bytes(value).hex()

    def campaign_and_snapshot(self, fixture: dict) -> tuple[dict, dict]:
        campaign = fixture["campaign"]
        campaign_wire = {
            **campaign,
            "epoch_id": self.wire(campaign["epoch_id"]),
            "excluded_wallets": [self.wire(value) for value in campaign["excluded_wallets"]],
            "excluded_bounty_contracts": [
                self.wire(value) for value in campaign["excluded_bounty_contracts"]
            ],
            "snapshot_attesters": [self.wire(value) for value in campaign["snapshot_attesters"]],
        }
        snapshot = fixture["snapshot"]
        snapshot_wire = {
            **snapshot,
            "end_block_hash": self.wire(snapshot["end_block_hash"]),
            "settlements": [],
        }
        for settlement in snapshot["settlements"]:
            snapshot_wire["settlements"].append(
                {
                    **settlement,
                    "bounty_contract": self.wire(settlement["bounty_contract"]),
                    "bounty_id": self.wire(settlement["bounty_id"]),
                    "creator": self.wire(settlement["creator"]),
                    "solver": self.wire(settlement["solver"]),
                    "transaction_hash": self.wire(settlement["transaction_hash"]),
                    "funding": [
                        {**item, "contributor": self.wire(item["contributor"])}
                        for item in settlement["funding"]
                    ],
                }
            )
        return campaign_wire, snapshot_wire

    def test_checked_fixture_is_reproducible(self) -> None:
        self.assertEqual(build(), json.loads(OUTPUT.read_text(encoding="utf-8")))

    def test_two_attesters_bind_the_exact_snapshot(self) -> None:
        fixture = self.fixture()
        campaign, snapshot = self.campaign_and_snapshot(fixture)
        policy = verification_policy_hash(campaign)
        frozen = snapshot_hash(campaign, snapshot)
        digest = attestation_digest(policy, frozen, snapshot["end_block_hash"])
        recovered = [
            recover_signer(digest, self.wire(item["signature"]))
            for item in fixture["snapshot"]["attestations"]
        ]
        expected = [self.wire(value) for value in fixture["campaign"]["snapshot_attesters"]]
        self.assertEqual(recovered, expected)

        snapshot["settlements"][0]["gmv_base_units"] += 1
        changed = attestation_digest(
            policy, snapshot_hash(campaign, snapshot), snapshot["end_block_hash"]
        )
        self.assertNotEqual(
            recover_signer(changed, self.wire(fixture["snapshot"]["attestations"][0]["signature"])),
            expected[0],
        )

    def test_policy_is_known_before_the_snapshot(self) -> None:
        fixture = self.fixture()
        campaign, snapshot = self.campaign_and_snapshot(fixture)
        baseline = verification_policy_hash(campaign)
        snapshot["end_safe_block"] += 1
        snapshot["end_block_hash"] = "0x" + "99" * 32
        self.assertEqual(verification_policy_hash(campaign), baseline)
        campaign["ends_at"] += 1
        self.assertNotEqual(verification_policy_hash(campaign), baseline)


if __name__ == "__main__":
    unittest.main()
