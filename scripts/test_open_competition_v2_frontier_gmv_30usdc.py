from __future__ import annotations

import json
from pathlib import Path
import sys
import unittest

from eth_abi import encode
from eth_utils import keccak, to_checksum_address


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from forward_canonical_gmv import verification_policy_hash  # noqa: E402


PARAM_TYPE = "(uint256,uint256,uint64,uint64,uint8,uint8,int256,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32)"


def raw(value: str) -> bytes:
    return bytes.fromhex(value[2:])


def clone_address(factory: str, implementation: str, salt: bytes) -> str:
    init = (
        bytes.fromhex("3d602d80600a3d3981f3")
        + bytes.fromhex("363d3d373d3d3d363d73")
        + bytes.fromhex(implementation[2:])
        + bytes.fromhex("5af43d82803e903d91602b57fd5bf3")
    )
    return "0x" + keccak(
        b"\xff" + bytes.fromhex(factory[2:]) + salt + keccak(init)
    )[12:].hex()


class FrontierGmvThirtyUsdcTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.spec = json.loads(
            (
                ROOT
                / "ops"
                / "open-competition-v2-frontier-gmv-30usdc-v1.json"
            ).read_text(encoding="utf-8")
        )

    def test_campaign_and_creation_identity_are_reproducible(self) -> None:
        value = self.spec
        campaign = dict(value["campaign"])
        campaign.pop("starts_at_iso")
        campaign.pop("ends_at_iso")
        expected_policy = "0x" + verification_policy_hash(campaign).hex()
        self.assertEqual(
            expected_policy, value["creation_params"]["verification_policy_hash"]
        )

        seed_payload = {
            "schema_version": "agent-bounties/open-competition-v2-creation-nonce-v1",
            "seed": value["creation_nonce_seed"],
        }
        canonical = json.dumps(
            seed_payload,
            sort_keys=True,
            separators=(",", ":"),
            ensure_ascii=False,
        ).encode("utf-8")
        nonce = keccak(canonical)
        self.assertEqual("0x" + nonce.hex(), value["creation_nonce"])

        params = value["creation_params"]
        proof_system = keccak(text="sp1-plonk")
        abi_params = (
            int(params["solver_reward"]),
            int(params["keeper_reward"]),
            params["funding_deadline"],
            params["proof_window_seconds"],
            1,
            0,
            int(params["score_threshold"]),
            proof_system,
            raw(params["program_vkey"]),
            raw(params["source_hash"]),
            raw(params["elf_hash"]),
            raw(params["journal_schema_hash"]),
            raw(params["metric_program_hash"]),
            raw(params["execution_policy_hash"]),
            raw(params["verification_policy_hash"]),
            raw(params["settlement_policy_hash"]),
            raw(params["beta_risk_hash"]),
        )
        bounty_id = keccak(
            encode(
                ["uint256", "address", "address", "bytes32", PARAM_TYPE],
                [
                    8453,
                    to_checksum_address(value["factory_contract"]),
                    to_checksum_address(value["creator"]),
                    nonce,
                    abi_params,
                ],
            )
        )
        self.assertEqual("0x" + bounty_id.hex(), value["bounty_id"])
        self.assertEqual(
            clone_address(
                value["factory_contract"], value["implementation_contract"], bounty_id
            ),
            value["predicted_competition"],
        )

    def test_economics_and_public_metadata_are_exact(self) -> None:
        economics = self.spec["economics"]
        self.assertEqual(economics["solver_reward_base_units"], 30_000_000)
        self.assertEqual(economics["keeper_reward_base_units"], 40_000)
        self.assertEqual(economics["funding_target_base_units"], 30_040_000)
        self.assertEqual(
            economics["hosted_net_prize_if_win_base_units"], 29_890_000
        )

        registry = json.loads(
            (
                ROOT / "ops" / "open-competition-v2-public-metadata-v1.json"
            ).read_text(encoding="utf-8")
        )
        matches = [
            item
            for item in registry["competitions"]
            if item["competition"] == self.spec["predicted_competition"]
        ]
        self.assertEqual(len(matches), 1)
        self.assertEqual(matches[0]["bounty_id"], self.spec["bounty_id"])
        self.assertEqual(matches[0]["epoch_starts_at"], "2026-08-31T00:00:00Z")
        self.assertEqual(matches[0]["epoch_ends_at"], "2026-09-07T00:00:00Z")


if __name__ == "__main__":
    unittest.main()
