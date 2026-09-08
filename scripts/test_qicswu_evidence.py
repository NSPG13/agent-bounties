import copy
import json
import tempfile
import unittest
from pathlib import Path

from scripts import qicswu_evidence as evidence


BOUNTY = "0x" + "11" * 20
SOLVER = "0x" + "22" * 20
FUNDER = "0x" + "33" * 20
TX = "0x" + "44" * 32
BLOCK_HASH = "0x" + "55" * 32
PARENT_HASH = "0x" + "66" * 32
STATE_ROOT = "0x" + "77" * 32
TX_ROOT = "0x" + "88" * 32
RECEIPT_ROOT = "0x" + "99" * 32
BOUNTY_ID = "0x" + "aa" * 32
SETTLEMENT_TOPIC = evidence.SETTLEMENT_TOPICS["autonomous"]


def word(value):
    if isinstance(value, str) and value.startswith("0x"):
        value = int(value, 16)
    return f"{value:064x}"


def address_topic(value):
    return "0x" + "0" * 24 + value[2:]


def registry(*, approved=False):
    value = {
        "schema_version": evidence.REGISTRY_SCHEMA,
        "registry_id": "test-registry",
        "network": evidence.NETWORK,
        "chain_id": evidence.CHAIN_ID,
        "status": "candidate",
        "providers": [
            {
                "provider_id": "alpha",
                "endpoint_authority": "alpha.example",
                "operator_name": "Alpha",
                "control_principal_id": "organization:alpha",
                "archive_reads_supported": True,
                "operator_evidence_refs": ["https://alpha.example/operator"],
            },
            {
                "provider_id": "bravo",
                "endpoint_authority": "bravo.example",
                "operator_name": "Bravo",
                "control_principal_id": "organization:bravo",
                "archive_reads_supported": True,
                "operator_evidence_refs": ["https://bravo.example/operator"],
            },
        ],
        "approval": {
            "status": "pending",
            "registry_body_hash": None,
            "required_roles": ["data_owner", "security_privacy"],
            "approvals": [],
        },
    }
    if approved:
        value["status"] = "approved"
        value["approval"] = {
            "status": "approved",
            "registry_body_hash": evidence.registry_body_hash(value),
            "required_roles": ["data_owner", "security_privacy"],
            "approvals": [
                {
                    "role": "data_owner",
                    "reviewer_id": "reviewer:data",
                    "decision": "approved",
                    "approved_at": "2026-09-03T00:00:00Z",
                    "evidence_refs": ["https://approval.example/data"],
                },
                {
                    "role": "security_privacy",
                    "reviewer_id": "reviewer:security",
                    "decision": "approved",
                    "approved_at": "2026-09-03T00:00:00Z",
                    "evidence_refs": ["https://approval.example/security"],
                },
            ],
        }
    return value


def candidate_input():
    return {
        "schema_version": "agent-bounties/qicswu-evaluation-input-v1",
        "input_id": "test-input",
        "policy_id": "qicswu-base-mainnet-v1.2.0",
        "network": evidence.NETWORK,
        "chain_id": evidence.CHAIN_ID,
        "frozen_at": "2026-09-03T00:00:00Z",
        "window": {
            "started_at": "2026-08-06T00:00:00Z",
            "ended_at": "2026-09-03T00:00:00Z",
            "boundary": "[started_at,ended_at)",
            "complete_utc_days": 28,
        },
        "candidates": [
            {
                "network": evidence.NETWORK,
                "chain_id": evidence.CHAIN_ID,
                "protocol": "autonomous",
                "protocol_version": "agent-bounties/autonomous-v1",
                "factory_contract": "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9",
                "event_kind": "bounty_settled",
                "bounty_contract": BOUNTY,
                "bounty_id": BOUNTY_ID,
                "round": 1,
                "transaction_hash": TX,
                "log_index": 7,
                "block_number": 100,
                "block_hash": BLOCK_HASH,
                "occurred_at": "2026-08-07T00:00:00Z",
                "solver_wallet": SOLVER,
                "solver_payout_base_units": 100,
                "promised_reward_principal_base_units": 100,
                "settled_gmv_base_units": 110,
            }
        ],
    }


def settlement_log():
    return {
        "address": BOUNTY,
        "blockHash": BLOCK_HASH,
        "blockNumber": hex(100),
        "data": "0x" + "".join(word(value) for value in [100, 5, 0, 10, 0, 0, 0, 0, 0, 0, 0]),
        "logIndex": hex(7),
        "removed": False,
        "topics": [SETTLEMENT_TOPIC, BOUNTY_ID, "0x" + word(1), address_topic(SOLVER)],
        "transactionHash": TX,
        "transactionIndex": "0x1",
    }


def transfer_log(amount=105):
    return {
        "address": evidence.USDC,
        "blockHash": BLOCK_HASH,
        "blockNumber": hex(100),
        "data": "0x" + word(amount),
        "logIndex": hex(8),
        "removed": False,
        "topics": [evidence.TRANSFER_TOPIC, address_topic(BOUNTY), address_topic(SOLVER)],
        "transactionHash": TX,
        "transactionIndex": "0x1",
    }


def receipt_result(amount=105):
    return {
        "transactionHash": TX,
        "blockHash": BLOCK_HASH,
        "blockNumber": hex(100),
        "status": "0x1",
        "from": FUNDER,
        "to": BOUNTY,
        "logs": [settlement_log(), transfer_log(amount)],
    }


def block_result(number=100, block_hash=BLOCK_HASH):
    return {
        "number": hex(number),
        "hash": block_hash,
        "parentHash": PARENT_HASH,
        "timestamp": hex(1_786_060_800 if number == 100 else 1_786_061_000),
        "stateRoot": STATE_ROOT,
        "transactionsRoot": TX_ROOT,
        "receiptsRoot": RECEIPT_ROOT,
    }


def fake_rpc_factory(
    *,
    transfer_amount=105,
    disagree=False,
    settlement_removed=False,
    transfer_transaction_hash=TX,
):
    def fake_rpc(endpoint, method, params, request_id, **kwargs):
        del request_id, kwargs
        if method == "eth_chainId":
            return hex(evidence.CHAIN_ID)
        if method == "eth_getTransactionReceipt":
            amount = transfer_amount + (1 if disagree and "bravo" in endpoint else 0)
            receipt = receipt_result(amount)
            receipt["logs"][0]["removed"] = settlement_removed
            receipt["logs"][1]["transactionHash"] = transfer_transaction_hash
            return receipt
        if method == "eth_getBlockByNumber" and params[0] == "safe":
            return block_result(200, "0x" + "cc" * 32)
        if method == "eth_getBlockByNumber":
            return block_result()
        raise AssertionError((endpoint, method, params))

    return fake_rpc


class QicswuEvidenceTests(unittest.TestCase):
    def test_pending_registry_is_valid_but_not_approved(self):
        result = evidence.validate_registry(registry())
        self.assertFalse(result["approved"])
        self.assertEqual(set(result["providers"]), {"alpha", "bravo"})

    def test_approved_registry_requires_role_separation_and_body_binding(self):
        value = registry(approved=True)
        self.assertTrue(evidence.validate_registry(value)["approved"])
        value["approval"]["approvals"][1]["reviewer_id"] = "reviewer:data"
        with self.assertRaisesRegex(evidence.EvidenceError, "distinct reviewers"):
            evidence.validate_registry(value)

    def test_registry_rejects_shared_control(self):
        value = registry()
        value["providers"][1]["control_principal_id"] = "organization:alpha"
        with self.assertRaisesRegex(evidence.EvidenceError, "control principals"):
            evidence.validate_registry(value)

    def test_capture_and_verify_retains_partial_publication_blockers(self):
        reg = registry()
        document = candidate_input()
        assignments = {
            "alpha": "https://alpha.example",
            "bravo": "https://bravo.example",
        }
        bundle = evidence.capture(
            document,
            reg,
            assignments,
            captured_at="2026-09-03T01:00:00Z",
            rpc_call=fake_rpc_factory(),
        )
        report = evidence.verify_bundle(bundle, reg, document)
        self.assertEqual(report["canonical_settlements_verified"], 1)
        self.assertEqual(report["status"], "verified_partial_blocked")
        self.assertFalse(report["publication_eligible"])
        self.assertIn("rpc_provider_registry_approval_pending", report["remaining_reason_codes"])
        self.assertIn("complete_funding_lifecycle_missing", report["remaining_reason_codes"])
        for reference in bundle["evidence_index"][0]["settlement_log_refs"]:
            self.assertEqual(evidence.resolve_ref(bundle, reference)["address"], BOUNTY)

    def test_approved_registry_alone_cannot_make_partial_capture_publishable(self):
        reg = registry(approved=True)
        document = candidate_input()
        bundle = evidence.capture(
            document,
            reg,
            {"alpha": "https://alpha.example", "bravo": "https://bravo.example"},
            captured_at="2026-09-03T01:00:00Z",
            rpc_call=fake_rpc_factory(),
        )
        report = evidence.verify_bundle(bundle, reg, document)
        self.assertFalse(report["publication_eligible"])
        self.assertNotIn("rpc_provider_registry_approval_pending", report["remaining_reason_codes"])
        self.assertIn("raw_evidence_resolution_incomplete", report["remaining_reason_codes"])

    def test_provider_disagreement_fails_closed(self):
        with self.assertRaisesRegex(evidence.EvidenceError, "solver transfer"):
            evidence.capture(
                candidate_input(),
                registry(),
                {"alpha": "https://alpha.example", "bravo": "https://bravo.example"},
                captured_at="2026-09-03T01:00:00Z",
                rpc_call=fake_rpc_factory(disagree=True),
            )

    def test_payout_mismatch_fails_closed(self):
        with self.assertRaisesRegex(evidence.EvidenceError, "does not reconcile"):
            evidence.capture(
                candidate_input(),
                registry(),
                {"alpha": "https://alpha.example", "bravo": "https://bravo.example"},
                captured_at="2026-09-03T01:00:00Z",
                rpc_call=fake_rpc_factory(transfer_amount=104),
            )

    def test_removed_settlement_log_fails_closed(self):
        with self.assertRaisesRegex(evidence.EvidenceError, "settlement log is marked removed"):
            evidence.capture(
                candidate_input(),
                registry(),
                {"alpha": "https://alpha.example", "bravo": "https://bravo.example"},
                captured_at="2026-09-03T01:00:00Z",
                rpc_call=fake_rpc_factory(settlement_removed=True),
            )

    def test_embedded_transfer_must_match_its_receipt_identity(self):
        with self.assertRaisesRegex(evidence.EvidenceError, "transaction hash does not match"):
            evidence.capture(
                candidate_input(),
                registry(),
                {"alpha": "https://alpha.example", "bravo": "https://bravo.example"},
                captured_at="2026-09-03T01:00:00Z",
                rpc_call=fake_rpc_factory(transfer_transaction_hash="0x" + "ab" * 32),
            )

    def test_verifier_recomputes_normalized_observation_hashes(self):
        reg = registry()
        document = candidate_input()
        bundle = evidence.capture(
            document,
            reg,
            {"alpha": "https://alpha.example", "bravo": "https://bravo.example"},
            captured_at="2026-09-03T01:00:00Z",
            rpc_call=fake_rpc_factory(),
        )
        bundle["observations"][0]["normalized_result_sha256"] = "sha256:" + "00" * 32
        bundle["evidence_payload_hash"] = evidence.canonical_hash(
            {
                key: copy.deepcopy(bundle[key])
                for key in (
                    "schema_version",
                    "bundle_id",
                    "network",
                    "chain_id",
                    "captured_at",
                    "evaluation_input_hash",
                    "provider_registry_body_hash",
                    "observations",
                )
            }
        )
        bundle["artifact_hash"] = evidence.artifact_hash(bundle)
        with self.assertRaisesRegex(evidence.EvidenceError, "normalized result hash"):
            evidence.verify_bundle(bundle, reg, document)

    def test_tamper_breaks_artifact_hash(self):
        reg = registry()
        document = candidate_input()
        bundle = evidence.capture(
            document,
            reg,
            {"alpha": "https://alpha.example", "bravo": "https://bravo.example"},
            captured_at="2026-09-03T01:00:00Z",
            rpc_call=fake_rpc_factory(),
        )
        receipt = next(
            row
            for row in bundle["observations"]
            if row["method"] == "eth_getTransactionReceipt"
        )
        receipt["result"]["status"] = "0x0"
        with self.assertRaisesRegex(evidence.EvidenceError, "artifact hash"):
            evidence.verify_bundle(bundle, reg, document)

    def test_immutable_writer_allows_identical_replay_only(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "evidence.json"
            document = {"ok": True}
            evidence.write_immutable_json(path, document)
            evidence.write_immutable_json(path, document)
            with self.assertRaisesRegex(evidence.EvidenceError, "refusing to overwrite"):
                evidence.write_immutable_json(path, {"ok": False})


if __name__ == "__main__":
    unittest.main()
