#!/usr/bin/env python3

from __future__ import annotations

import hashlib
import importlib.util
from pathlib import Path
from unittest import mock
import unittest


SCRIPT = Path(__file__).with_name("standing_meta_v4_monitor.py")
SPEC = importlib.util.spec_from_file_location("standing_meta_v4_monitor", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def address(byte: int) -> str:
    return "0x" + f"{byte:02x}" * 20


def word(value: int) -> str:
    return "0x" + value.to_bytes(32, "big").hex()


def data_words(*values: int) -> str:
    return "0x" + "".join(value.to_bytes(32, "big").hex() for value in values)


CANONICAL = {
    "anonymous_protocol_controller": address(1),
    "anonymous_stake_pool": address(2),
    "verifier_sortition": address(3),
    "solver_sortition": address(4),
    "appealable_verifier": address(5),
    "standing_meta_child_factory": address(6),
    "standing_meta_parent_factory": address(7),
    "onchain_terms_registry": address(8),
    "canonical_independent_child_verifier": address(9),
    "standing_meta_v4_bundle": address(10),
}
PARENT = address(20)
CHILD = address(21)
COMPETITION_FACTORY = address(22)
COMPETITION = address(23)
KEEPER = address(24)
CASE_ID = word(31)


class FakeFoundry:
    def __init__(self, *, head: int = 100, subscription_balance: int = 500) -> None:
        self.head = head
        self.subscription_balance = subscription_balance

    def chain_id(self) -> int:
        return MODULE.DEPLOY.BASE_SEPOLIA_CHAIN_ID

    def block_number(self) -> int:
        return self.head

    def block_timestamp(self, block_number: int) -> int:
        self.observed_block = block_number
        return 1_000

    def balance(self, wallet: str) -> int:
        return 500

    def balance_at(self, wallet: str, block_number: int) -> int:
        return self.balance(wallet)

    def code(self, contract: str) -> str:
        return "0x6001"

    def keccak_text(self, value: str) -> str:
        return "0x" + hashlib.sha256(value.encode()).hexdigest()

    def call(self, contract: str, signature: str, *args: str) -> str:
        if signature == "getSubscription(uint256)(uint96,uint96,uint64,address,address[])":
            consumers = f"[{CANONICAL['verifier_sortition']},{CANONICAL['solver_sortition']}]"
            return f"0\n{self.subscription_balance}\n2\n{KEEPER}\n{consumers}"
        if signature == "eligibleWallets(uint8,address[])(address[])":
            count = 8 if args[0] == "1" else 3
            return "[" + ",".join(address(40 + index) for index in range(count)) + "]"
        if signature == MODULE.REQUEST_STATUS_SIGNATURE:
            return "\n".join(
                [word(50), word(51), "900", "910", "8", "4", "true", "false", "true", "1"]
            )
        if signature == MODULE.CASE_STATE_SIGNATURE:
            return "6"
        if signature == MODULE.CASE_TIMING_SIGNATURE:
            return "800\n850\n900\n950"
        if contract == PARENT and signature == "factory()(address)":
            return CANONICAL["standing_meta_parent_factory"]
        if contract == PARENT and signature == "protocolVersion()(bytes32)":
            return self.keccak_text("agent-bounties/standing-meta-v4")
        if contract == PARENT and signature == "preparedChild()(address)":
            return CHILD
        if contract == PARENT and signature == "status()(uint8)":
            return "4"
        if contract == PARENT and signature == "solverReward()(uint256)":
            return "2000000"
        if contract == CHILD and signature == "targetAmount()(uint256)":
            return "1000000"
        if contract == COMPETITION and signature == "factory()(address)":
            return COMPETITION_FACTORY
        if contract == COMPETITION and signature == "protocolVersion()(bytes32)":
            return self.keccak_text("agent-bounties/open-competition-v1")
        if contract == COMPETITION and signature == "status()(uint8)":
            return "2"
        raise AssertionError((contract, signature, args))

    def call_at(self, contract: str, signature: str, block_number: int, *args: str) -> str:
        return self.call(contract, signature, *args)

    def logs(self, contract: str, signature: str, from_block: int, to_block: int):
        base = {"address": contract, "topics": [word(99)], "data": "0x"}
        if contract in {CANONICAL["verifier_sortition"], CANONICAL["solver_sortition"]}:
            if signature == MODULE.RANDOMNESS_REQUESTED:
                return [{**base, "topics": [word(99), word(50), word(1 if contract.endswith("03" * 20) else 2)]}]
            return []
        if contract == CANONICAL["appealable_verifier"]:
            if signature == MODULE.VERIFICATION_CASE_OPENED:
                return [{**base, "topics": [word(99), CASE_ID, word(60)], "data": data_words(1)}]
            if signature in {MODULE.PRIMARY_ASSIGNED, MODULE.PRIMARY_VERDICT}:
                return [base]
            if signature == MODULE.VERIFICATION_FINALIZED:
                return [{**base, "data": data_words(1, 1, 0)}]
            return []
        if contract == PARENT and signature == MODULE.BOUNTY_SETTLED:
            return [base]
        if contract == COMPETITION and signature in {
            MODULE.SOLUTION_COMMITTED,
            MODULE.SOLUTION_REVEALED,
            MODULE.BOUNTY_SETTLED,
        }:
            return [base]
        return []


class StandingMetaV4MonitorTests(unittest.TestCase):
    def deployment(self) -> dict:
        return {
            "schema": "agent-bounties/standing-meta-v4-deployment-v1",
            "network": "base-sepolia",
            "chain_id": MODULE.DEPLOY.BASE_SEPOLIA_CHAIN_ID,
            "deployer": KEEPER,
            "vrf_coordinator": address(25),
            "subscription_id": 1,
        }

    def manifest(self) -> dict:
        return {
            "configuration": dict(MODULE.DEPLOY.EXPECTED_CONFIGURATION),
            "monitoring_policy": dict(MODULE.DEPLOY.EXPECTED_MONITORING_POLICY),
            "networks": {
                "base-sepolia": {
                    "minimum_native_subscription_reserve_wei": 100,
                    "minimum_gas_sponsorship_reserve_wei": 100,
                }
            },
        }

    def activity(self) -> dict:
        return {
            "schema": MODULE.ACTIVITY_SCHEMA,
            "network": "base-sepolia",
            "from_block": 1,
            "standing_meta_parent_canaries": [PARENT],
            "open_competition_factory": COMPETITION_FACTORY,
            "open_competition_canaries": [COMPETITION],
        }

    def verification(self, foundry: FakeFoundry) -> dict:
        return {
            "rpc_confirmed": True,
            "canonical_component_addresses": dict(CANONICAL),
            "subscription": {
                "owner": KEEPER,
                "consumers": [CANONICAL["verifier_sortition"], CANONICAL["solver_sortition"]],
                "native_balance": foundry.subscription_balance,
            },
        }

    def test_activity_is_exact_and_requires_both_canaries(self) -> None:
        normalized = MODULE.validate_activity(self.activity(), "base-sepolia")
        self.assertEqual(normalized["from_block"], 1)
        broken = self.activity()
        broken["open_competition_canaries"] = []
        with self.assertRaisesRegex(MODULE.MonitorError, "at least one"):
            MODULE.validate_activity(broken, "base-sepolia")

    def test_dual_rpc_snapshot_is_healthy_and_content_addressed(self) -> None:
        primary = FakeFoundry(head=101)
        secondary = FakeFoundry(head=100)
        with mock.patch.object(
            MODULE.DEPLOY,
            "verify_deployment",
            side_effect=lambda foundry, _deployment: self.verification(foundry),
        ):
            snapshot = MODULE.audit_pair(
                primary,
                secondary,
                self.deployment(),
                self.manifest(),
                MODULE.validate_activity(self.activity(), "base-sepolia"),
            )
        self.assertTrue(snapshot["monitoring_active"])
        self.assertFalse(snapshot["earning_suppressed"])
        self.assertEqual(snapshot["common_observation_block"], 100)
        self.assertEqual(snapshot["content_sha256"], MODULE.snapshot_sha256(snapshot))
        for rpc_pass in snapshot["rpc_passes"]:
            self.assertEqual(rpc_pass["eligible_verifier_wallet_count"], 8)
            self.assertEqual(rpc_pass["eligible_solver_wallet_count"], 3)
            self.assertEqual(
                rpc_pass["canaries"]["standing_meta"][0][
                    "successful_settlement_margin_base_units"
                ],
                1_000_000,
            )

    def test_reserve_or_rpc_drift_suppresses_earning(self) -> None:
        primary = FakeFoundry(head=110, subscription_balance=99)
        secondary = FakeFoundry(head=100)
        with mock.patch.object(
            MODULE.DEPLOY,
            "verify_deployment",
            side_effect=lambda foundry, _deployment: self.verification(foundry),
        ):
            snapshot = MODULE.audit_pair(
                primary,
                secondary,
                self.deployment(),
                self.manifest(),
                MODULE.validate_activity(self.activity(), "base-sepolia"),
            )
        self.assertFalse(snapshot["monitoring_active"])
        self.assertTrue(snapshot["earning_suppressed"])
        self.assertTrue(any("subscription_reserve" in item for item in snapshot["blockers"]))
        self.assertIn("head difference", " ".join(snapshot["blockers"]))


if __name__ == "__main__":
    unittest.main()
