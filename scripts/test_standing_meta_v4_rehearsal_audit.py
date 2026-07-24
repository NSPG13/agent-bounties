#!/usr/bin/env python3

from __future__ import annotations

import copy
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest import mock


SCRIPT = Path(__file__).with_name("standing_meta_v4_rehearsal_audit.py")
SPEC = importlib.util.spec_from_file_location("standing_meta_v4_rehearsal_audit", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def address(index: int) -> str:
    return "0x" + f"{index:040x}"


def bytes32(index: int) -> str:
    return "0x" + f"{index:064x}"


def address_topic(value: str) -> str:
    return "0x" + "00" * 12 + value[2:].lower()


def data_words(*values: int) -> str:
    return "0x" + "".join(f"{value:064x}" for value in values)


class FakeFactFoundry:
    def keccak_text(self, value: str) -> str:
        if value == MODULE.TRANSFER_EVENT_SIGNATURE:
            return bytes32(999)
        return bytes32(998)

    def call(self, address_value: str, signature: str, *args: str) -> str:
        if signature == "caseState(bytes32)(uint8)":
            return "6"
        raise AssertionError((address_value, signature, args))


class FakeRuntimeFoundry:
    def __init__(self, runtime: str) -> None:
        self.runtime = runtime

    def runtime_hash(self, source: str) -> str:
        self.source = source
        return bytes32(1)

    def code(self, address_value: str) -> str:
        return self.runtime

    def keccak_text(self, value: str) -> str:
        return value.lower()


def evidence_fixture() -> dict:
    scenarios: dict[str, dict] = {}
    transaction_index = 1_000
    for scenario_index, (name, requirements) in enumerate(MODULE.REQUIRED_SCENARIOS.items(), start=1):
        kind = sorted(requirements["subject_kinds"])[0]
        subject = address(100 + scenario_index)
        events: list[dict] = []
        transactions: list[str] = []
        for event_index, event_name in enumerate(sorted(requirements["events"])):
            transaction_index += 1
            transaction_hash = bytes32(transaction_index)
            transactions.append(transaction_hash)
            events.append(
                {
                    "name": event_name,
                    "signature": MODULE.EVENT_SIGNATURES[event_name],
                    "contract": address(10) if event_name in MODULE.VERIFIER_EVENTS else subject,
                    "transaction_hash": transaction_hash,
                    "log_index": event_index,
                    "block_number": 20_000 + transaction_index,
                }
            )
        scenario = {
            "status": "confirmed",
            "scenario_id": bytes32(scenario_index),
            "subject_kind": kind,
            "subject_contract": subject,
            "bounty_id": bytes32(100 + scenario_index),
            "actors": {"creator": address(200 + scenario_index), "solver": address(300 + scenario_index)},
            "transactions": transactions,
            "events": events,
            "facts": copy.deepcopy(requirements["facts"]),
        }
        if any(event["name"] in MODULE.VERIFIER_EVENTS for event in events):
            scenario["case_id"] = bytes32(200 + scenario_index)
        if name == "standing_meta_parent_settlement":
            scenario["linked_child_contract"] = address(500)
        scenarios[name] = scenario
    evidence = {
        "schema": MODULE.SCHEMA,
        "source_commit": "ab" * 20,
        "network": "base-sepolia",
        "chain_id": MODULE.DEPLOY.BASE_SEPOLIA_CHAIN_ID,
        "settlement_token": MODULE.DEPLOY.BASE_SEPOLIA_USDC,
        "open_competition_factory": address(20),
        "minimum_native_subscription_reserve_wei": 1,
        "minimum_gas_sponsorship_reserve_wei": 1,
        "faucet_funding": {
            "native_eth_transaction": bytes32(900),
            "test_usdc_transactions": [bytes32(901)],
            "confirmed_native_eth_wei": 10**16,
            "confirmed_test_usdc_base_units": 55_000_000,
        },
        "rpc_observations": [
            {"endpoint_label": "primary", "chain_id": MODULE.DEPLOY.BASE_SEPOLIA_CHAIN_ID, "block_number": 1},
            {"endpoint_label": "secondary", "chain_id": MODULE.DEPLOY.BASE_SEPOLIA_CHAIN_ID, "block_number": 2},
        ],
        "scenarios": scenarios,
    }
    evidence["content_sha256"] = MODULE.content_sha256(evidence)
    return evidence


class StandingMetaV4RehearsalAuditTests(unittest.TestCase):
    def test_exact_runtime_source_map_covers_every_canonical_v4_component(self) -> None:
        self.assertEqual(set(MODULE.V4_CANONICAL_SOURCES), set(MODULE.DEPLOY.EXPECTED_CANONICAL_COMPONENTS))

    def test_complete_structure_is_not_live_rehearsal_proof(self) -> None:
        evidence = evidence_fixture()
        structure = MODULE.audit_structure(evidence)
        self.assertTrue(structure["structure_complete"], structure["blockers"])
        result = MODULE.audit_rehearsal(evidence, None)
        self.assertFalse(result["complete"])
        self.assertIn("live RPC verification is missing", result["blockers"])

    def test_content_commitment_detects_post_hash_mutation(self) -> None:
        evidence = evidence_fixture()
        evidence["minimum_gas_sponsorship_reserve_wei"] = 2
        result = MODULE.audit_structure(evidence)
        self.assertFalse(result["checks"]["content_sha256"])
        self.assertFalse(result["structure_complete"])

    def test_exact_scenario_set_and_required_facts_fail_closed(self) -> None:
        evidence = evidence_fixture()
        evidence["scenarios"].pop("appeal_timeout")
        evidence["content_sha256"] = MODULE.content_sha256(evidence)
        result = MODULE.audit_structure(evidence)
        self.assertFalse(result["checks"]["exact_unique_scenario_set"])

        evidence = evidence_fixture()
        evidence["scenarios"]["solver_appeal_overturned_rejection"]["facts"]["appellant_role"] = "creator"
        evidence["content_sha256"] = MODULE.content_sha256(evidence)
        result = MODULE.audit_structure(evidence)
        self.assertFalse(
            result["scenario_checks"]["solver_appeal_overturned_rejection"]["required_facts"]
        )

    def test_event_records_cannot_be_reused_across_scenarios(self) -> None:
        evidence = evidence_fixture()
        first = evidence["scenarios"]["unappealed_acceptance"]["events"][0]
        second_scenario = evidence["scenarios"]["primary_rejection"]
        second = second_scenario["events"][0]
        second["transaction_hash"] = first["transaction_hash"]
        second["log_index"] = first["log_index"]
        second_scenario["transactions"].append(first["transaction_hash"])
        evidence["content_sha256"] = MODULE.content_sha256(evidence)
        result = MODULE.audit_structure(evidence)
        self.assertFalse(result["checks"]["globally_unique_event_records"])

    def test_event_signature_and_subject_provenance_fields_are_required(self) -> None:
        evidence = evidence_fixture()
        scenario = evidence["scenarios"]["standing_meta_parent_settlement"]
        scenario["events"][0]["signature"] = "BountySettled(bytes32)"
        scenario["linked_child_contract"] = address(0)
        evidence["content_sha256"] = MODULE.content_sha256(evidence)
        result = MODULE.audit_structure(evidence)
        checks = result["scenario_checks"]["standing_meta_parent_settlement"]
        self.assertFalse(checks["event_records"])
        self.assertFalse(checks["linked_child_contract"])

    def test_receipt_log_accepts_hex_log_index(self) -> None:
        expected = {"logIndex": "0x2", "data": "0x"}
        receipt = {"logs": [{"logIndex": "0x1"}, expected]}
        self.assertIs(MODULE._receipt_log(receipt, 2), expected)

    def test_completion_requires_two_agreeing_live_rpc_passes(self) -> None:
        evidence = evidence_fixture()
        incomplete = MODULE.audit_rehearsal(
            evidence,
            {"live_complete": True, "independent_rpc_agreement": True, "rpc_passes": [], "blockers": []},
        )
        self.assertFalse(incomplete["complete"])

        complete = MODULE.audit_rehearsal(
            evidence,
            {
                "live_complete": True,
                "independent_rpc_agreement": True,
                "rpc_passes": [{"pass_complete": True}, {"pass_complete": True}],
                "blockers": [],
            },
        )
        self.assertTrue(complete["complete"], complete["blockers"])

    def test_compiled_runtime_comparison_masks_only_declared_immutables(self) -> None:
        artifact = {
            "deployedBytecode": {
                "object": "0x0102030405060708",
                "immutableReferences": {"1": [{"start": 2, "length": 2}]},
            }
        }
        foundry = FakeRuntimeFoundry("0x0102aabb05060708")
        with mock.patch.object(MODULE, "read_object", return_value=artifact):
            self.assertTrue(
                MODULE._runtime_matches_compiled_with_immutables(foundry, address(1), "src/X.sol:X")
            )
            foundry.runtime = "0xff02aabb05060708"
            self.assertFalse(
                MODULE._runtime_matches_compiled_with_immutables(foundry, address(1), "src/X.sol:X")
            )

    def test_cli_seals_to_a_new_file_but_still_reports_missing_live_rpc(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            draft = root / "draft.json"
            sealed = root / "sealed.json"
            audit = root / "audit.json"
            evidence = evidence_fixture()
            evidence.pop("content_sha256")
            MODULE.write_object(draft, evidence)
            with mock.patch(
                "sys.argv",
                [
                    str(SCRIPT),
                    "--evidence",
                    str(draft),
                    "--seal-output",
                    str(sealed),
                    "--output",
                    str(audit),
                ],
            ):
                self.assertEqual(MODULE.main(), 0)
            sealed_evidence = MODULE.read_object(sealed)
            self.assertTrue(MODULE.audit_structure(sealed_evidence)["structure_complete"])
            self.assertFalse(MODULE.read_object(audit)["complete"])

    def test_decoded_unappealed_settlement_requires_canonical_usdc_transfer(self) -> None:
        scenario = evidence_fixture()["scenarios"]["unappealed_acceptance"]
        solver = scenario["actors"]["solver"]
        subject = scenario["subject_contract"]
        module = address(10)
        transfer = {
            "address": MODULE.DEPLOY.BASE_SEPOLIA_USDC,
            "topics": [bytes32(999), address_topic(subject), address_topic(solver)],
            "data": data_words(1_000_000),
        }
        records = [
            {
                "event": {"name": "PrimaryVerdictSubmitted", "contract": module},
                "log": {
                    "topics": [bytes32(1), scenario["case_id"], address_topic(address(88))],
                    "data": data_words(1, 123, 456),
                },
                "receipt": {"logs": []},
                "block_number": 100,
            },
            {
                "event": {"name": "VerificationFinalized", "contract": module},
                "log": {"topics": [bytes32(1), scenario["case_id"]], "data": data_words(1, 0, 0)},
                "receipt": {"logs": []},
                "block_number": 101,
            },
            {
                "event": {"name": "BountySettled", "contract": subject},
                "log": {
                    "topics": [bytes32(1), scenario["bounty_id"], bytes32(1), address_topic(solver)],
                    "data": data_words(990_000, 10_000, 0),
                },
                "receipt": {"logs": [transfer]},
                "block_number": 102,
            },
        ]
        checks = MODULE._scenario_fact_checks(
            FakeFactFoundry(), "unappealed_acceptance", scenario, records
        )
        self.assertTrue(all(checks.values()), checks)

        records[-1]["receipt"] = {"logs": []}
        checks = MODULE._scenario_fact_checks(
            FakeFactFoundry(), "unappealed_acceptance", scenario, records
        )
        self.assertFalse(checks["canonical_solver_transfer"])

    def test_decoded_open_competition_proves_one_block_ordering_and_same_winner(self) -> None:
        scenario = evidence_fixture()["scenarios"]["open_competition_first_valid_settlement"]
        solver = scenario["actors"]["solver"]
        subject = scenario["subject_contract"]
        transfer = {
            "address": MODULE.DEPLOY.BASE_SEPOLIA_USDC,
            "topics": [bytes32(999), address_topic(subject), address_topic(solver)],
            "data": data_words(1_000_000),
        }
        records = [
            {
                "event": {"name": "SolutionCommitted", "contract": subject},
                "log": {
                    "topics": [bytes32(1), scenario["bounty_id"], address_topic(solver), bytes32(1)],
                    "data": data_words(77, 50, 80, 10_000),
                },
                "receipt": {"logs": []},
                "block_number": 50,
            },
            {
                "event": {"name": "SolutionRevealed", "contract": subject, "transaction_hash": bytes32(700)},
                "log": {
                    "topics": [bytes32(1), scenario["bounty_id"], bytes32(1), address_topic(solver)],
                    "data": data_words(11, 12, 1, 13),
                },
                "receipt": {"logs": []},
                "block_number": 51,
            },
            {
                "event": {"name": "BountySettled", "contract": subject, "transaction_hash": bytes32(700)},
                "log": {
                    "topics": [bytes32(1), scenario["bounty_id"], bytes32(1), address_topic(solver)],
                    "data": data_words(990_000, 10_000, 0),
                },
                "receipt": {"logs": [transfer]},
                "block_number": 51,
            },
        ]
        checks = MODULE._scenario_fact_checks(
            FakeFactFoundry(), "open_competition_first_valid_settlement", scenario, records
        )
        self.assertTrue(all(checks.values()), checks)

        records[1]["block_number"] = 50
        checks = MODULE._scenario_fact_checks(
            FakeFactFoundry(), "open_competition_first_valid_settlement", scenario, records
        )
        self.assertFalse(checks["first_valid_ordering"])


if __name__ == "__main__":
    unittest.main()
