from __future__ import annotations

import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path

from eth_utils import keccak


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import serve_open_competition_v2_reserve_recovery as MODULE  # noqa: E402


class ReserveRecoveryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.manifest = {
            "schema": "agent-bounties/bounded-open-competition-v2-wallet-deployment-v1",
            "network": "base-mainnet",
            "chain_id": 8453,
            "canonical": {
                "protocol_version": "agent-bounties/open-competition-v2-beta3",
                "competition_factory": MODULE.PRODUCTION_COMPETITION_FACTORY,
                "settlement_token": MODULE.USDC,
                "release_hash": MODULE.PRODUCTION_RELEASE_HASH,
            },
            "reserve_factory": {
                "address": MODULE.PRODUCTION_RESERVE_FACTORY,
                "implementation": MODULE.PRODUCTION_RESERVE_IMPLEMENTATION,
                "clone_runtime_code_hash": MODULE.PRODUCTION_RESERVE_CLONE_HASH,
            },
        }
        cls.evidence = {
            "schema_version": "agent-bounties/bounded-open-competition-v2-wallet-deployment-evidence-v1",
            "network": "base-mainnet",
            "chain_id": 8453,
            "complete": True,
            "manifest_hash": MODULE.deployment_manifest_hash(cls.manifest),
            "release_hash": MODULE.PRODUCTION_RELEASE_HASH,
            "competition_factory": MODULE.PRODUCTION_COMPETITION_FACTORY,
            "reserve_factory": MODULE.PRODUCTION_RESERVE_FACTORY,
            "reserve_implementation": MODULE.PRODUCTION_RESERVE_IMPLEMENTATION,
            "runtime_hashes": {
                MODULE.PRODUCTION_COMPETITION_FACTORY: MODULE.PRODUCTION_COMPETITION_FACTORY_HASH,
                MODULE.PRODUCTION_RESERVE_FACTORY: MODULE.PRODUCTION_RESERVE_FACTORY_HASH,
                MODULE.PRODUCTION_RESERVE_IMPLEMENTATION: MODULE.PRODUCTION_RESERVE_IMPLEMENTATION_HASH,
            },
        }
        cls.state = {
            "network": "base-mainnet",
            "chain_id": 8453,
            "safe_block": 50_617_823,
            "safe_block_hash": "0x" + "12" * 32,
            "reserve_wallet": MODULE.RESERVE,
            "reserve_runtime_code_hash": cls.manifest["reserve_factory"]["clone_runtime_code_hash"],
            "owner": MODULE.OWNER,
            "owner_balance": 0,
            "settlement_token": cls.manifest["canonical"]["settlement_token"],
            "competition_factory": cls.manifest["canonical"]["competition_factory"],
            "competition_factory_runtime_code_hash": MODULE.PRODUCTION_COMPETITION_FACTORY_HASH,
            "deployment_factory": cls.manifest["reserve_factory"]["address"],
            "deployment_factory_runtime_code_hash": MODULE.PRODUCTION_RESERVE_FACTORY_HASH,
            "deployment_implementation": MODULE.PRODUCTION_RESERVE_IMPLEMENTATION,
            "deployment_implementation_runtime_code_hash": MODULE.PRODUCTION_RESERVE_IMPLEMENTATION_HASH,
            "policy_version": 2,
            "active_policy_hash": "0x" + "34" * 32,
            "lifetime_spent": 60_600_000,
            "reserve_balance": 17_068_098,
            "revoked": False,
        }

    def plan(self) -> dict[str, object]:
        return MODULE.build_plan(
            copy.deepcopy(self.state),
            self.manifest,
            self.evidence,
            17_068_098,
            60_600_000,
        )

    def test_exact_plan_binds_chain_owner_destination_value_and_selectors(self) -> None:
        plan = self.plan()
        MODULE.validate_plan(plan)
        self.assertEqual(plan["display"]["reserve_balance"], "17.068098 USDC")
        self.assertEqual(plan["transactions"]["revoke"]["data"], "0x9eba3667")
        self.assertEqual(plan["transactions"]["recover"]["data"], "0xb0e11ec4")
        self.assertEqual(plan["transactions"]["recover"]["value_wei"], 0)

    def test_wrong_live_deployment_or_expected_amount_fails_closed(self) -> None:
        mutations = (
            ("owner", "0x" + "11" * 20),
            ("reserve_wallet", "0x" + "22" * 20),
            ("reserve_runtime_code_hash", "0x" + "33" * 32),
            ("reserve_balance", 17_068_097),
            ("lifetime_spent", 60_599_999),
            ("revoked", True),
        )
        for field, value in mutations:
            state = copy.deepcopy(self.state)
            state[field] = value
            with self.subTest(field=field), self.assertRaises(MODULE.RecoveryError):
                MODULE.build_plan(
                    state,
                    self.manifest,
                    self.evidence,
                    17_068_098,
                    60_600_000,
                )

    def test_manifest_and_evidence_are_bound_to_the_reviewed_production_release(self) -> None:
        MODULE.validate_deployment_evidence(self.evidence, self.manifest)
        changed = copy.deepcopy(self.evidence)
        changed["manifest_hash"] = "0x" + "00" * 32
        with self.assertRaises(MODULE.RecoveryError):
            MODULE.validate_deployment_evidence(changed, self.manifest)

    def test_plan_integrity_detects_transaction_tampering(self) -> None:
        plan = self.plan()
        for field, value in (("from", "0x" + "11" * 20), ("to", "0x" + "22" * 20), ("value_wei", 1), ("data", "0xdeadbeef")):
            changed = copy.deepcopy(plan)
            changed["transactions"]["recover"][field] = value
            with self.subTest(field=field), self.assertRaises(MODULE.RecoveryError):
                MODULE.validate_plan(changed)

    def test_confirmed_transaction_must_match_all_exact_fields(self) -> None:
        plan = self.plan()
        for stage in ("revoke", "recover"):
            expected = plan["transactions"][stage]
            transaction = {"from": expected["from"], "to": expected["to"], "value": 0, "input": expected["data"]}
            MODULE.transaction_matches(plan, stage, transaction)
            transaction["input"] = "0x12345678"
            with self.assertRaises(MODULE.RecoveryError):
                MODULE.transaction_matches(plan, stage, transaction)

    def test_recovery_requires_one_canonical_usdc_transfer_to_owner(self) -> None:
        plan = self.plan()
        topic_address = lambda address: "0x" + "0" * 24 + address[2:]
        receipt = {
            "logs": [{
                "address": MODULE.USDC,
                "topics": [MODULE.TRANSFER_TOPIC, topic_address(MODULE.RESERVE), topic_address(MODULE.OWNER)],
                "data": "0x" + (17_068_098).to_bytes(32, "big").hex(),
            }]
        }
        self.assertEqual(MODULE.extract_recovery_transfer(receipt, plan), 17_068_098)
        receipt["logs"][0]["topics"][2] = topic_address("0x" + "44" * 20)
        with self.assertRaises(MODULE.RecoveryError):
            MODULE.extract_recovery_transfer(receipt, plan)

    def test_safe_block_evidence_preserves_lifetime_and_empties_reserve(self) -> None:
        plan = self.plan()
        revoked = copy.deepcopy(self.state)
        revoked["revoked"] = True
        MODULE.validate_revoke_evidence(plan, revoked)
        revoked["reserve_balance"] += 1
        MODULE.validate_revoke_evidence(plan, revoked)
        revoked["reserve_balance"] -= 2
        with self.assertRaises(MODULE.RecoveryError):
            MODULE.validate_revoke_evidence(plan, revoked)
        revoked["reserve_balance"] += 1
        recovered = copy.deepcopy(revoked)
        recovered["reserve_balance"] = 0
        MODULE.validate_recovery_evidence(plan, recovered, 17_068_098)
        recovered["lifetime_spent"] += 1
        with self.assertRaises(MODULE.RecoveryError):
            MODULE.validate_recovery_evidence(plan, recovered, 17_068_098)

    def test_submitted_hash_is_restart_safe_and_cannot_be_replaced(self) -> None:
        plan = self.plan()
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "recovery.json"
            first = MODULE.store_submission(output, plan, "revoke", "0x" + "ab" * 32)
            self.assertEqual(MODULE.load_submission(output, plan, "revoke"), first)
            with self.assertRaises(MODULE.RecoveryError):
                MODULE.store_submission(output, plan, "revoke", "0x" + "cd" * 32)

    def test_page_states_money_and_active_escrow_boundaries(self) -> None:
        self.assertIn("two explicit confirmations", MODULE.HTML)
        self.assertIn("0 ETH and 0 USDC moved", MODULE.HTML)
        self.assertIn("cannot cancel, settle, enter, or withdraw", MODULE.HTML)
        self.assertNotIn("private key", MODULE.HTML.lower())
        self.assertEqual(MODULE.TRANSFER_TOPIC, "0x" + keccak(text="Transfer(address,address,uint256)").hex())


if __name__ == "__main__":
    unittest.main()
