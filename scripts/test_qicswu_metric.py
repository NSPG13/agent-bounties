import copy
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from scripts import qicswu_metric as metric
from scripts import qicswu_verify_baseline as baseline_verifier


ROOT = Path(__file__).resolve().parents[1]
POLICY_PATH = ROOT / "ops/metrics/qicswu-policy-v1.json"
BASELINE_INPUT_PATH = (
    ROOT / "ops/metrics/qicswu-baseline-input-2026-08-04-2026-09-01-v1.json"
)
BASELINE_RESULT_PATH = (
    ROOT / "ops/metrics/qicswu-baseline-result-2026-08-04-2026-09-01-v1.json"
)
PRINCIPALS_PATH = ROOT / "ops/metrics/qicswu-principal-registry-v1.json"
ROOT_MAP_PATH = ROOT / "ops/metrics/qicswu-root-work-map-v1.json"

SOLVER = "0x1111111111111111111111111111111111111111"
FUNDER = "0x2222222222222222222222222222222222222222"
SECOND_SOLVER = "0x3333333333333333333333333333333333333333"
OPERATOR = "0x884834e884d6e93462655a2820140ad03e6747bc"
BLOCK_HASH = "0x" + "ab" * 32
USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"


def candidate(ordinal=1, *, occurred_at="2026-08-05T12:00:00Z"):
    row = {
        "network": "base-mainnet",
        "chain_id": 8453,
        "protocol": "autonomous",
        "protocol_version": "agent-bounties/autonomous-v1",
        "factory_contract": "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9",
        "event_kind": "bounty_settled",
        "bounty_contract": "0x" + f"{100 + ordinal:040x}",
        "bounty_id": "0x" + f"{200 + ordinal:064x}",
        "round": 1,
        "transaction_hash": "0x" + f"{300 + ordinal:064x}",
        "log_index": ordinal,
        "block_number": 1_000 + ordinal,
        "block_hash": BLOCK_HASH,
        "occurred_at": occurred_at,
        "solver_wallet": SOLVER,
        "solver_payout_base_units": 100,
        "solver_payout_reconciliation": {
            "settlement_event": {
                "network": "base-mainnet",
                "chain_id": 8453,
                "protocol": "autonomous",
                "protocol_version": "agent-bounties/autonomous-v1",
                "factory_contract": "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9",
                "event_kind": "bounty_settled",
                "bounty_contract": "0x" + f"{100 + ordinal:040x}",
                "bounty_id": "0x" + f"{200 + ordinal:064x}",
                "round": 1,
                "transaction_hash": "0x" + f"{300 + ordinal:064x}",
                "log_index": ordinal,
                "block_number": 1_000 + ordinal,
                "block_hash": BLOCK_HASH,
                "solver_wallet": SOLVER,
                "solver_reward_base_units": 100,
                "completion_or_timeout_bonus_base_units": 0,
                "returned_bond_base_units": 10,
                "raw_evidence_refs": [f"fixture:payout-event:{ordinal}"],
            },
            "receipt_transfer": {
                "network": "base-mainnet",
                "chain_id": 8453,
                "asset_contract": USDC,
                "transaction_hash": "0x" + f"{300 + ordinal:064x}",
                "transfer_log_index": 10_000 + ordinal,
                "block_number": 1_000 + ordinal,
                "block_hash": BLOCK_HASH,
                "from_address": "0x" + f"{100 + ordinal:040x}",
                "to_address": SOLVER,
                "amount_base_units": 110,
                "returned_bond_base_units": 10,
                "receipt_status": 1,
                "raw_evidence_refs": [f"fixture:payout-receipt:{ordinal}"],
            },
            "contract_state": {
                "network": "base-mainnet",
                "chain_id": 8453,
                "protocol": "autonomous",
                "protocol_version": "agent-bounties/autonomous-v1",
                "factory_contract": "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9",
                "bounty_contract": "0x" + f"{100 + ordinal:040x}",
                "bounty_id": "0x" + f"{200 + ordinal:064x}",
                "round": 1,
                "solver_wallet": SOLVER,
                "observed_at_block_number": 1_000 + ordinal,
                "observed_at_block_hash": BLOCK_HASH,
                "solver_payout_base_units": 100,
                "rpc_block_observations": [
                    {
                        "provider_id": "primary",
                        "block_number": 1_000 + ordinal,
                        "block_hash": BLOCK_HASH,
                    },
                    {
                        "provider_id": "shadow",
                        "block_number": 1_000 + ordinal,
                        "block_hash": BLOCK_HASH,
                    },
                ],
                "raw_evidence_refs": [f"fixture:payout-state:{ordinal}"],
            },
        },
        "promised_reward_principal_base_units": 100,
        "settled_gmv_base_units": 110,
        "participation_event": {
            "kind": "bounty_claimed",
            "network": "base-mainnet",
            "chain_id": 8453,
            "protocol": "autonomous",
            "factory_contract": "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9",
            "bounty_contract": "0x" + f"{100 + ordinal:040x}",
            "bounty_id": "0x" + f"{200 + ordinal:064x}",
            "round": 1,
            "participant_wallet": SOLVER,
            "transaction_hash": "0x" + f"{400 + ordinal:064x}",
            "block_number": 900 + ordinal,
            "block_hash": BLOCK_HASH,
            "log_index": ordinal,
            "canonical_event_reconciled": True,
            "first_participation_event_reconciled": True,
            "receipt_status": 1,
            "event_matches_receipt": True,
            "removed": False,
            "reorg_detected": False,
            "raw_evidence_refs": [f"fixture:participation:{ordinal}"],
        },
        "funding_events": [
            {
                "kind": "funding_added",
                "network": "base-mainnet",
                "chain_id": 8453,
                "protocol": "autonomous",
                "factory_contract": "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9",
                "bounty_contract": "0x" + f"{100 + ordinal:040x}",
                "bounty_id": "0x" + f"{200 + ordinal:064x}",
                "round": 1,
                "transaction_hash": "0x" + f"{500 + ordinal:064x}",
                "contributor_wallet": FUNDER,
                "amount_base_units": 100,
                "block_number": 800 + ordinal,
                "block_hash": BLOCK_HASH,
                "log_index": ordinal,
                "canonical_event_reconciled": True,
                "receipt_status": 1,
                "event_matches_receipt": True,
                "removed": False,
                "reorg_detected": False,
                "raw_evidence_refs": [f"fixture:funding:{ordinal}"],
            }
        ],
        "funding_reconciled_to_contract": True,
        "operator_subsidies": {
            "status": "complete",
            "entries": [],
        },
        "canonicality": {
            "finality_status": "safe",
            "receipt_status": 1,
            "settlement_event_matches_receipt": True,
            "contract_state_reconciled": True,
            "removed": False,
            "reorg_detected": False,
            "rpc_block_observations": [
                {
                    "provider_id": "primary",
                    "block_number": 1_000 + ordinal,
                    "block_hash": BLOCK_HASH,
                },
                {
                    "provider_id": "shadow",
                    "block_number": 1_000 + ordinal,
                    "block_hash": BLOCK_HASH,
                },
            ],
            "raw_evidence_refs": [f"fixture:settlement:{ordinal}"],
        },
    }
    for view in row["solver_payout_reconciliation"].values():
        view["view_payload_sha256"] = metric._view_payload_hash(view)
    return row


def set_candidate_protocol(
    row,
    *,
    protocol,
    protocol_version,
    factory_contract,
    settlement_event_kind,
    participation_event_kind,
    round_value,
):
    row.update(
        protocol=protocol,
        protocol_version=protocol_version,
        factory_contract=factory_contract,
        event_kind=settlement_event_kind,
        round=round_value,
    )
    reconciliation = row["solver_payout_reconciliation"]
    reconciliation["settlement_event"].update(
        protocol=protocol,
        protocol_version=protocol_version,
        factory_contract=factory_contract,
        event_kind=settlement_event_kind,
        round=round_value,
    )
    reconciliation["contract_state"].update(
        protocol=protocol,
        protocol_version=protocol_version,
        factory_contract=factory_contract,
        round=round_value,
    )
    row["participation_event"].update(
        kind=participation_event_kind,
        protocol=protocol,
        factory_contract=factory_contract,
        round=round_value,
    )
    for funding in row["funding_events"]:
        funding.update(
            protocol=protocol,
            factory_contract=factory_contract,
            round=round_value,
        )
    reanchor_payout_views(row)
    return row


def reanchor_payout_views(row):
    for view in row["solver_payout_reconciliation"].values():
        view["view_payload_sha256"] = metric._view_payload_hash(view)


def set_reconciled_solver_payout(row, payout, *, base_reward=None, returned_bond=10):
    """Keep all independent payout evidence views aligned in a positive fixture."""

    if base_reward is None:
        base_reward = row["promised_reward_principal_base_units"]
    if payout < base_reward:
        raise ValueError("payout cannot be lower than its base reward")
    row["solver_payout_base_units"] = payout
    row["promised_reward_principal_base_units"] = base_reward
    reconciliation = row["solver_payout_reconciliation"]
    reconciliation["settlement_event"]["solver_reward_base_units"] = base_reward
    reconciliation["settlement_event"][
        "completion_or_timeout_bonus_base_units"
    ] = payout - base_reward
    reconciliation["settlement_event"][
        "returned_bond_base_units"
    ] = returned_bond
    reconciliation["receipt_transfer"]["amount_base_units"] = payout + returned_bond
    reconciliation["receipt_transfer"]["returned_bond_base_units"] = returned_bond
    reconciliation["contract_state"]["solver_payout_base_units"] = payout
    reanchor_payout_views(row)


def rebind_settlement(row, *, transaction_ordinal, block_number, log_index, occurred_at):
    """Create a distinct canonical settlement identity for retry/lineage fixtures."""

    transaction_hash = "0x" + f"{transaction_ordinal:064x}"
    row["transaction_hash"] = transaction_hash
    row["block_number"] = block_number
    row["log_index"] = log_index
    row["occurred_at"] = occurred_at
    payout = row["solver_payout_reconciliation"]
    payout["settlement_event"]["transaction_hash"] = transaction_hash
    payout["settlement_event"]["log_index"] = log_index
    payout["settlement_event"]["block_number"] = block_number
    payout["receipt_transfer"]["transaction_hash"] = transaction_hash
    payout["receipt_transfer"]["block_number"] = block_number
    payout["contract_state"]["observed_at_block_number"] = block_number
    for observation in payout["contract_state"]["rpc_block_observations"]:
        observation["block_number"] = block_number
    for observation in row["canonicality"]["rpc_block_observations"]:
        observation["block_number"] = block_number
    reanchor_payout_views(row)


def complete_documents(candidates, *, root_ids=None, classifications=None):
    policy = json.loads(POLICY_PATH.read_text())
    policy["publication_gate"]["status"] = "approved"
    policy["publication_gate"]["reason_codes"] = []
    input_document = {
        "schema_version": metric.INPUT_SCHEMA,
        "input_id": "test-input",
        "policy_id": policy["policy_id"],
        "network": "base-mainnet",
        "chain_id": 8453,
        "frozen_at": "2026-09-02T17:30:00Z",
        "window": {
            "started_at": "2026-08-04T00:00:00Z",
            "ended_at": "2026-09-01T00:00:00Z",
            "boundary": "[started_at,ended_at)",
            "complete_utc_days": 28,
        },
        "source_reconciliation": {
            "status": "reconciled",
            "raw_evidence_retained": True,
            "protocols": [
                {
                    "protocol": row["protocol"],
                    "factory_contracts": list(row["accepted_factory_contracts"]),
                    "coverage_started_at": "2026-08-04T00:00:00Z",
                    "coverage_ended_at": "2026-09-01T00:00:00Z",
                    "raw_response_retained": True,
                    "independent_rpc_observation_count": 2,
                    "safe_block_hash": BLOCK_HASH,
                }
                for row in policy["canonical_settlement_protocols"]
            ],
        },
        "candidates": candidates,
    }
    principals = {
        "schema_version": metric.PRINCIPAL_SCHEMA,
        "registry_id": "test-principals",
        "as_of": "2026-09-01T00:00:00Z",
        "status": "complete",
        "principals": [
            {
                "principal_id": "principal:solver",
                "wallets": [SOLVER],
                "operator_or_affiliate": False,
                "beneficial_controller_evidence_refs": ["fixture:solver-owner"],
            },
            {
                "principal_id": "principal:second-solver",
                "wallets": [SECOND_SOLVER],
                "operator_or_affiliate": False,
                "beneficial_controller_evidence_refs": ["fixture:second-solver-owner"],
            },
            {
                "principal_id": "principal:customer",
                "wallets": [FUNDER],
                "operator_or_affiliate": False,
                "beneficial_controller_evidence_refs": ["fixture:funder-owner"],
            },
            {
                "principal_id": "principal:agent-bounties-operator",
                "wallets": [OPERATOR],
                "operator_or_affiliate": True,
                "beneficial_controller_evidence_refs": ["fixture:operator-owner"],
            },
        ],
        "reimbursement_relationships": [],
        "coverage": {
            "all_candidate_wallets_mapped": True,
            "all_candidate_reimbursement_relationships_reviewed": True,
        },
    }
    root_ids = root_ids or {
        metric._candidate_key(row): f"root:{index}"
        for index, row in enumerate(candidates, start=1)
    }
    classifications = classifications or {}
    candidate_by_key = {metric._candidate_key(row): row for row in candidates}
    earliest_by_root = {}
    for candidate_key, root_id in root_ids.items():
        row = candidate_by_key[candidate_key]
        order = (
            metric._parse_utc(row["occurred_at"], "fixture.occurred_at"),
            row["block_number"],
            row["log_index"],
            candidate_key,
        )
        prior = earliest_by_root.get(root_id)
        if prior is None or order < prior[0]:
            earliest_by_root[root_id] = (order, candidate_key)
    root_records = []
    relationship_keys = set()
    relationships = []
    for row in candidates:
        key = metric._candidate_key(row)
        root_id = root_ids[key]
        root_records.append(
            {
                "candidate_key": key,
                "root_work_id": root_id,
                "standalone_value_classification": classifications.get(
                    key, "standalone_end_customer_value"
                ),
                "evidence_refs": [f"fixture:root:{root_id}"],
                "root_history_reconciled": True,
                "earliest_qualifying_candidate_key": earliest_by_root[root_id][1],
                "root_history_evidence_refs": [f"fixture:root-history:{root_id}"],
            }
        )
        solver_id = (
            "principal:second-solver"
            if row["solver_wallet"].lower() == SECOND_SOLVER
            else "principal:solver"
        )
        for funding in row.get("funding_events", []):
            if funding.get("contributor_wallet", "").lower() != FUNDER:
                continue
            relationship_key = ("principal:customer", solver_id, root_id)
            if relationship_key in relationship_keys:
                continue
            relationship_keys.add(relationship_key)
            relationships.append(
                {
                    "funder_principal_id": relationship_key[0],
                    "solver_principal_id": relationship_key[1],
                    "root_work_id": relationship_key[2],
                    "status": "no_reimbursement_found",
                    "evidence_refs": [f"fixture:relationship:{root_id}"],
                }
            )
    principals["reimbursement_relationships"] = relationships
    root_map = {
        "schema_version": metric.ROOT_MAP_SCHEMA,
        "map_id": "test-roots",
        "as_of": "2026-09-01T00:00:00Z",
        "status": "complete",
        "records": root_records,
        "coverage": {
            "all_candidate_settlements_mapped": True,
            "all_roots_standalone_value_reviewed": True,
            "all_root_histories_reconciled": True,
        },
    }
    return input_document, policy, principals, root_map


class QicswuMetricTests(unittest.TestCase):
    def evaluate(self, documents):
        return metric.evaluate(*documents)

    def test_one_fully_qualified_root_counts(self):
        result = self.evaluate(complete_documents([candidate()]))
        self.assertEqual(result["status"], "available")
        self.assertEqual(result["north_star"]["total_root_work_units"], 1)
        self.assertEqual(result["north_star"]["per_day"], "0.035714")

    def test_draft_policy_machine_blocks_numeric_publication(self):
        first = candidate(1)
        duplicate = candidate(2, occurred_at="2026-08-06T12:00:00Z")
        excluded = candidate(3, occurred_at="2026-08-07T12:00:00Z")
        root_ids = {
            metric._candidate_key(first): "root:shared",
            metric._candidate_key(duplicate): "root:shared",
            metric._candidate_key(excluded): "root:excluded",
        }
        classifications = {
            metric._candidate_key(excluded): "synthetic_canary",
        }
        documents = list(
            complete_documents(
                [first, duplicate, excluded],
                root_ids=root_ids,
                classifications=classifications,
            )
        )
        documents[1] = json.loads(POLICY_PATH.read_text())
        result = self.evaluate(documents)
        self.assertEqual(result["status"], "unavailable")
        self.assertIsNone(result["north_star"]["total_root_work_units"])
        self.assertIn(
            "raw_evidence_resolution_not_implemented",
            result["unavailable_reason_codes"],
        )
        self.assertIn(
            "rpc_provider_registry_not_approved",
            result["unavailable_reason_codes"],
        )
        self.assertIsNone(
            result["secondary_metrics"]["qualifying_settled_gmv_base_units"]
        )
        self.assertIsNone(
            result["secondary_metrics"][
                "median_qualifying_solver_payout_base_units"
            ]
        )
        self.assertIsNone(
            result["secondary_metrics"][
                "largest_independent_funding_principal_share"
            ]
        )
        self.assertIsNone(
            result["secondary_metrics"][
                "largest_solver_principal_share_of_solver_payout"
            ]
        )
        self.assertIsNone(
            result["diagnostics"][
                "eligible_root_work_units_before_availability_gate"
            ]
        )
        self.assertIsNone(result["diagnostics"]["excluded_records"])
        self.assertIsNone(result["diagnostics"]["unknown_records"])
        self.assertIsNone(result["diagnostics"]["reason_code_counts"])
        self.assertIsNone(result["diagnostics"]["input_candidate_records"])
        self.assertIsNone(
            result["diagnostics"]["unique_canonical_event_identities"]
        )
        self.assertIsNone(result["diagnostics"]["ledger_records"])
        self.assertIsNone(result["ledger"])

    def test_blocked_empty_window_does_not_publish_an_exact_zero_side_channel(self):
        documents = list(complete_documents([]))
        documents[1] = json.loads(POLICY_PATH.read_text())
        result = self.evaluate(documents)
        self.assertEqual(result["status"], "unavailable")
        self.assertIsNone(result["north_star"]["total_root_work_units"])
        self.assertIsNone(result["ledger"])
        self.assertTrue(all(value is None for value in result["diagnostics"].values()))

    def test_empty_complete_window_is_available_zero(self):
        result = self.evaluate(complete_documents([]))
        self.assertEqual(result["status"], "available")
        self.assertEqual(result["north_star"]["total_root_work_units"], 0)
        self.assertEqual(result["north_star"]["per_day"], "0.000000")
        self.assertTrue(all(row["root_work_units"] == 0 for row in result["north_star"]["daily"]))

    def test_every_policy_factory_requires_complete_source_coverage(self):
        documents = list(complete_documents([]))
        v2 = next(
            row
            for row in documents[0]["source_reconciliation"]["protocols"]
            if row["protocol"] == "open_competition_v2"
        )
        v2["factory_contracts"] = [v2["factory_contracts"][1]]
        result = self.evaluate(documents)
        self.assertEqual(result["status"], "unavailable")
        self.assertIn("required_factory_stream_missing", result["unavailable_reason_codes"])
        self.assertIn("required_factory_stream_incomplete", result["unavailable_reason_codes"])

        documents = list(complete_documents([]))
        rows = documents[0]["source_reconciliation"]["protocols"]
        v2 = next(row for row in rows if row["protocol"] == "open_competition_v2")
        historical = copy.deepcopy(v2)
        historical["factory_contracts"] = [v2["factory_contracts"][0]]
        historical["coverage_started_at"] = "2026-08-05T00:00:00Z"
        v2["factory_contracts"] = [v2["factory_contracts"][1]]
        rows.append(historical)
        result = self.evaluate(documents)
        self.assertEqual(result["status"], "unavailable")
        self.assertIn("protocol_source_window_incomplete", result["unavailable_reason_codes"])
        self.assertIn("required_factory_stream_incomplete", result["unavailable_reason_codes"])

        documents = list(complete_documents([]))
        documents[0]["source_reconciliation"]["protocols"][0].pop("factory_contracts")
        result = self.evaluate(documents)
        self.assertEqual(result["status"], "unavailable")
        self.assertIn(
            "protocol_source_factory_coverage_invalid",
            result["unavailable_reason_codes"],
        )

    def test_half_open_complete_utc_window(self):
        at_start = candidate(1, occurred_at="2026-08-04T00:00:00Z")
        at_end = candidate(2, occurred_at="2026-09-01T00:00:00Z")
        result = self.evaluate(complete_documents([at_start, at_end]))
        self.assertEqual(result["status"], "available")
        self.assertEqual(result["north_star"]["total_root_work_units"], 1)
        end_row = next(row for row in result["ledger"] if row["candidate_key"] == metric._candidate_key(at_end))
        self.assertEqual(end_row["status"], "excluded")
        self.assertIn("outside_metric_window", end_row["reason_codes"])

    def test_incomplete_window_contract_is_rejected(self):
        documents = list(complete_documents([]))
        documents[0]["window"]["started_at"] = "2026-08-04T00:00:01Z"
        with self.assertRaises(metric.MetricContractError):
            self.evaluate(documents)

        one_day = list(complete_documents([]))
        one_day[0]["window"]["ended_at"] = "2026-08-05T00:00:00Z"
        one_day[0]["window"]["complete_utc_days"] = True
        with self.assertRaises(metric.MetricContractError):
            self.evaluate(one_day)

    def test_missing_principal_is_unknown_not_zero(self):
        documents = list(complete_documents([candidate()]))
        documents[2]["principals"] = [
            row for row in documents[2]["principals"] if row["principal_id"] != "principal:solver"
        ]
        result = self.evaluate(documents)
        self.assertEqual(result["status"], "unavailable")
        self.assertIsNone(result["north_star"]["total_root_work_units"])
        self.assertIn("solver_beneficial_principal_unknown", result["unavailable_reason_codes"])

    def test_principal_registry_duplicates_and_malformed_evidence_fail_closed(self):
        documents = list(complete_documents([candidate()]))
        duplicate = copy.deepcopy(documents[2]["principals"][0])
        duplicate["operator_or_affiliate"] = True
        documents[2]["principals"].append(duplicate)
        with self.assertRaises(metric.MetricContractError):
            self.evaluate(documents)

        documents = list(complete_documents([candidate()]))
        documents[2]["principals"][0]["beneficial_controller_evidence_refs"] = (
            "not-an-evidence-array"
        )
        with self.assertRaises(metric.MetricContractError):
            self.evaluate(documents)

        row = candidate()
        row["funding_events"][0]["raw_evidence_refs"] = "not-an-evidence-array"
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "unavailable")
        self.assertIn("funding_raw_evidence_missing", result["unavailable_reason_codes"])

    def test_operator_solver_is_conclusively_excluded(self):
        row = candidate()
        row["solver_wallet"] = OPERATOR
        documents = list(complete_documents([row]))
        result = self.evaluate(documents)
        self.assertEqual(result["status"], "available")
        self.assertEqual(result["north_star"]["total_root_work_units"], 0)
        self.assertEqual(result["ledger"][0]["status"], "excluded")
        self.assertIn("operator_solver_principal", result["ledger"][0]["reason_codes"])

    def test_missing_root_and_standalone_classification_are_unknown(self):
        documents = list(complete_documents([candidate()]))
        documents[3]["records"] = []
        result = self.evaluate(documents)
        self.assertEqual(result["status"], "unavailable")
        self.assertIn("root_work_mapping_missing", result["unavailable_reason_codes"])

        documents = list(complete_documents([candidate()]))
        documents[3]["records"][0]["standalone_value_classification"] = "unknown"
        result = self.evaluate(documents)
        self.assertEqual(result["status"], "unavailable")
        self.assertIn("standalone_value_classification_unknown", result["unavailable_reason_codes"])

    def test_missing_reimbursement_review_is_unknown(self):
        documents = list(complete_documents([candidate()]))
        documents[2]["reimbursement_relationships"] = []
        result = self.evaluate(documents)
        self.assertEqual(result["status"], "unavailable")
        self.assertIn("reimbursement_relationship_unknown", result["unavailable_reason_codes"])

    def test_reimbursed_funding_is_excluded(self):
        documents = list(complete_documents([candidate()]))
        documents[2]["reimbursement_relationships"][0]["status"] = "reimbursed"
        result = self.evaluate(documents)
        self.assertEqual(result["status"], "available")
        self.assertEqual(result["north_star"]["total_root_work_units"], 0)
        self.assertIn(
            "insufficient_independent_preparticipation_funding",
            result["ledger"][0]["reason_codes"],
        )
        self.assertIn(
            "reimbursed_or_common_control_funding_excluded",
            result["ledger"][0]["informational_codes"],
        )

    def test_reimbursed_extra_does_not_veto_fully_independently_funded_work(self):
        row = candidate()
        reimbursed_extra = copy.deepcopy(row["funding_events"][0])
        reimbursed_extra.update(
            {
                "transaction_hash": "0x" + "44" * 32,
                "contributor_wallet": "0x4444444444444444444444444444444444444444",
                "amount_base_units": 25,
                "block_number": 850,
                "log_index": 2,
                "raw_evidence_refs": ["fixture:reimbursed-extra"],
            }
        )
        row["funding_events"].append(reimbursed_extra)
        documents = list(complete_documents([row]))
        documents[2]["principals"].append(
            {
                "principal_id": "principal:reimbursed-extra",
                "wallets": ["0x4444444444444444444444444444444444444444"],
                "operator_or_affiliate": False,
                "beneficial_controller_evidence_refs": ["fixture:extra-owner"],
            }
        )
        root_id = documents[3]["records"][0]["root_work_id"]
        documents[2]["reimbursement_relationships"].append(
            {
                "funder_principal_id": "principal:reimbursed-extra",
                "solver_principal_id": "principal:solver",
                "root_work_id": root_id,
                "status": "reimbursed",
                "evidence_refs": ["fixture:extra-reimbursement"],
            }
        )
        result = self.evaluate(documents)
        self.assertEqual(result["status"], "available")
        self.assertEqual(result["north_star"]["total_root_work_units"], 1)
        self.assertEqual(result["ledger"][0]["status"], "eligible")
        self.assertIn(
            "reimbursed_or_common_control_funding_excluded",
            result["ledger"][0]["informational_codes"],
        )

    def test_funding_must_precede_participation(self):
        row = candidate()
        row["funding_events"][0]["block_number"] = row["participation_event"]["block_number"]
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "available")
        self.assertEqual(result["north_star"]["total_root_work_units"], 0)
        self.assertIn(
            "insufficient_independent_preparticipation_funding",
            result["ledger"][0]["reason_codes"],
        )
        self.assertIn(
            "postparticipation_funding_excluded",
            result["ledger"][0]["informational_codes"],
        )

    def test_participation_cutoff_requires_canonical_earliest_event_evidence(self):
        cases = [
            ("transaction_hash", None, "participation_transaction_hash_missing"),
            ("block_hash", "not-a-hash", "participation_block_hash_missing"),
            ("canonical_event_reconciled", False, "participation_event_unreconciled"),
            (
                "first_participation_event_reconciled",
                False,
                "first_participation_not_reconciled",
            ),
            ("receipt_status", "1", "participation_receipt_unreconciled"),
            ("event_matches_receipt", False, "participation_event_receipt_mismatch"),
            ("removed", None, "participation_reorg_or_removal_unknown"),
            ("raw_evidence_refs", [], "participation_raw_evidence_missing"),
        ]
        for field, value, reason in cases:
            with self.subTest(field=field):
                row = candidate()
                if value is None:
                    row["participation_event"].pop(field)
                else:
                    row["participation_event"][field] = value
                result = self.evaluate(complete_documents([row]))
                self.assertEqual(result["status"], "unavailable")
                self.assertIn(reason, result["unavailable_reason_codes"])

    def test_participation_cutoff_is_bound_to_candidate_and_solver(self):
        mutations = {
            "network": "wrong-network",
            "chain_id": 1,
            "protocol": "open_competition_v1",
            "factory_contract": "0x9999999999999999999999999999999999999999",
            "bounty_contract": "0x9999999999999999999999999999999999999999",
            "bounty_id": "0x" + "99" * 32,
            "round": 2,
            "participant_wallet": SECOND_SOLVER,
        }
        for field, value in mutations.items():
            with self.subTest(field=field):
                row = candidate()
                row["participation_event"][field] = value
                result = self.evaluate(complete_documents([row]))
                self.assertEqual(result["status"], "unavailable")
                self.assertIn(
                    "participation_candidate_scope_mismatch",
                    result["unavailable_reason_codes"],
                )

    def test_late_extra_does_not_veto_sufficient_preparticipation_funding(self):
        row = candidate()
        late = copy.deepcopy(row["funding_events"][0])
        late["amount_base_units"] = 1
        late["block_number"] = row["participation_event"]["block_number"]
        late["log_index"] = 2
        late["raw_evidence_refs"] = ["fixture:late-extra"]
        row["funding_events"].append(late)
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "available")
        self.assertEqual(result["north_star"]["total_root_work_units"], 1)
        self.assertEqual(result["ledger"][0]["status"], "eligible")
        self.assertIn(
            "postparticipation_funding_excluded",
            result["ledger"][0]["informational_codes"],
        )

    def test_canonical_funding_logs_are_deduplicated_and_conflicts_fail_closed(self):
        row = candidate()
        row["funding_events"][0]["amount_base_units"] = 50
        row["funding_events"].append(copy.deepcopy(row["funding_events"][0]))
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "available")
        self.assertEqual(result["north_star"]["total_root_work_units"], 0)
        self.assertEqual(
            result["ledger"][0]["independent_preparticipation_funding_base_units"],
            50,
        )
        self.assertIn(
            "duplicate_funding_event_collapsed",
            result["ledger"][0]["informational_codes"],
        )

        conflict = candidate()
        conflicting_log = copy.deepcopy(conflict["funding_events"][0])
        conflicting_log["amount_base_units"] = 99
        conflict["funding_events"].append(conflicting_log)
        result = self.evaluate(complete_documents([conflict]))
        self.assertEqual(result["status"], "unavailable")
        self.assertIn(
            "conflicting_duplicate_funding_event",
            result["unavailable_reason_codes"],
        )

    def test_participation_and_funding_event_kinds_are_exact(self):
        row = candidate()
        row["participation_event"]["kind"] = "arbitrary_claim"
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "unavailable")
        self.assertIn("wrong_participation_event_kind", result["unavailable_reason_codes"])

        row = candidate()
        row["funding_events"][0]["kind"] = "funding_intent"
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "unavailable")
        self.assertIn("wrong_funding_event_kind", result["unavailable_reason_codes"])

    def test_funding_is_bound_to_candidate_and_canonical_receipt(self):
        scope_mutations = {
            "network": "wrong-network",
            "chain_id": 1,
            "protocol": "open_competition_v1",
            "factory_contract": "0x9999999999999999999999999999999999999999",
            "bounty_contract": "0x9999999999999999999999999999999999999999",
            "bounty_id": "0x" + "99" * 32,
            "round": 2,
        }
        for field, value in scope_mutations.items():
            with self.subTest(field=field):
                row = candidate()
                row["funding_events"][0][field] = value
                result = self.evaluate(complete_documents([row]))
                self.assertEqual(result["status"], "unavailable")
                self.assertIn(
                    "funding_candidate_scope_mismatch",
                    result["unavailable_reason_codes"],
                )

        evidence_mutations = {
            "transaction_hash": "not-a-hash",
            "block_hash": "not-a-hash",
            "receipt_status": "1",
            "event_matches_receipt": False,
            "removed": True,
        }
        expected_reasons = {
            "transaction_hash": "funding_transaction_hash_missing",
            "block_hash": "funding_block_hash_missing",
            "receipt_status": "funding_receipt_unreconciled",
            "event_matches_receipt": "funding_event_receipt_mismatch",
            "removed": "funding_reorg_or_removal_unknown",
        }
        for field, value in evidence_mutations.items():
            with self.subTest(field=field):
                row = candidate()
                row["funding_events"][0][field] = value
                result = self.evaluate(complete_documents([row]))
                self.assertEqual(result["status"], "unavailable")
                self.assertIn(expected_reasons[field], result["unavailable_reason_codes"])

    def test_solver_self_funding_does_not_qualify(self):
        row = candidate()
        row["funding_events"][0]["contributor_wallet"] = SOLVER
        documents = list(complete_documents([row]))
        documents[2]["reimbursement_relationships"] = []
        result = self.evaluate(documents)
        self.assertEqual(result["status"], "available")
        self.assertEqual(result["north_star"]["total_root_work_units"], 0)
        self.assertIn("solver_self_funding_excluded", result["ledger"][0]["informational_codes"])

    def test_operator_principal_and_wallet_funding_do_not_qualify(self):
        row = candidate()
        row["funding_events"][0]["contributor_wallet"] = OPERATOR
        documents = list(complete_documents([row]))
        documents[2]["reimbursement_relationships"] = []
        result = self.evaluate(documents)
        self.assertEqual(result["status"], "available")
        self.assertEqual(result["north_star"]["total_root_work_units"], 0)
        self.assertIn("operator_wallet_funding_excluded", result["ledger"][0]["informational_codes"])

    def test_late_operator_funding_is_excluded_but_reported_in_guardrail(self):
        row = candidate()
        late_operator = copy.deepcopy(row["funding_events"][0])
        late_operator["contributor_wallet"] = OPERATOR
        late_operator["amount_base_units"] = 25
        late_operator["block_number"] = row["participation_event"]["block_number"]
        late_operator["log_index"] = row["participation_event"]["log_index"] + 1
        late_operator["raw_evidence_refs"] = ["fixture:late-operator-funding"]
        row["funding_events"].append(late_operator)
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "available")
        self.assertEqual(result["north_star"]["total_root_work_units"], 1)
        self.assertEqual(
            result["secondary_metrics"]["operator_subsidy_guardrail"][
                "operator_reward_funding_usdc_base_units"
            ],
            25,
        )
        self.assertIn(
            "postparticipation_funding_excluded",
            result["ledger"][0]["informational_codes"],
        )
        self.assertIn(
            "operator_wallet_funding_excluded",
            result["ledger"][0]["informational_codes"],
        )

    def test_policy_declared_canary_is_excluded_without_root_mapping(self):
        row = candidate()
        row["bounty_contract"] = "0x3e052b933628b960d61654a68fca23d869d8989f"
        documents = list(complete_documents([row]))
        documents[3]["records"] = []
        result = self.evaluate(documents)
        self.assertEqual(result["status"], "available")
        self.assertEqual(result["north_star"]["total_root_work_units"], 0)
        self.assertEqual(result["ledger"][0]["status"], "excluded")
        self.assertIn("policy_declared_synthetic_contract", result["ledger"][0]["reason_codes"])

    def test_canary_maintenance_and_meta_roots_are_excluded(self):
        for classification in (
            "synthetic_canary",
            "platform_maintenance",
            "meta_self_generated_demand",
        ):
            row = candidate()
            key = metric._candidate_key(row)
            result = self.evaluate(
                complete_documents(
                    [row], classifications={key: classification}
                )
            )
            self.assertEqual(result["status"], "available")
            self.assertEqual(result["north_star"]["total_root_work_units"], 0)
            self.assertIn(
                f"standalone_value_excluded:{classification}",
                result["ledger"][0]["reason_codes"],
            )

    def test_root_work_is_deduplicated(self):
        first = candidate(1)
        second = candidate(2, occurred_at="2026-08-06T12:00:00Z")
        shared_root = {
            metric._candidate_key(first): "root:shared",
            metric._candidate_key(second): "root:shared",
        }
        result = self.evaluate(complete_documents([first, second], root_ids=shared_root))
        self.assertEqual(result["north_star"]["total_root_work_units"], 1)
        duplicate = next(row for row in result["ledger"] if "root_work_duplicate" in row["reason_codes"])
        self.assertFalse(duplicate["counted"])

    def test_ingest_duplicate_retry_round_and_material_split_count_one_root(self):
        first = candidate(1, occurred_at="2026-08-05T12:00:00Z")
        ingest_duplicate = copy.deepcopy(first)

        # A retried settlement for the same bounty/round has a distinct
        # canonical transaction/log identity but remains the same economic job.
        same_round_retry = copy.deepcopy(first)
        rebind_settlement(
            same_round_retry,
            transaction_ordinal=901,
            block_number=1_101,
            log_index=101,
            occurred_at="2026-08-06T12:00:00Z",
        )
        same_round_retry["participation_event"]["transaction_hash"] = (
            "0x" + f"{905:064x}"
        )
        same_round_retry["participation_event"]["block_number"] = 902
        same_round_retry["participation_event"]["log_index"] = 105
        same_round_retry["funding_events"][0]["transaction_hash"] = (
            "0x" + f"{906:064x}"
        )
        same_round_retry["funding_events"][0]["block_number"] = 802
        same_round_retry["funding_events"][0]["log_index"] = 106

        # A later protocol round for the same bounty is also not a new root.
        later_round = copy.deepcopy(first)
        later_round["round"] = 2
        later_round["participation_event"]["round"] = 2
        later_round["participation_event"]["transaction_hash"] = "0x" + f"{903:064x}"
        later_round["participation_event"]["block_number"] = 1_002
        later_round["participation_event"]["log_index"] = 103
        later_round["funding_events"][0]["round"] = 2
        later_round["funding_events"][0]["transaction_hash"] = "0x" + f"{904:064x}"
        later_round["funding_events"][0]["block_number"] = 1_001
        later_round["funding_events"][0]["log_index"] = 104
        later_round["solver_payout_reconciliation"]["settlement_event"]["round"] = 2
        later_round["solver_payout_reconciliation"]["contract_state"]["round"] = 2
        rebind_settlement(
            later_round,
            transaction_ordinal=902,
            block_number=1_102,
            log_index=102,
            occurred_at="2026-08-07T12:00:00Z",
        )

        # A materially split version uses a different bounty id and contract;
        # retained lineage, rather than those surface identifiers, controls it.
        material_split = candidate(4, occurred_at="2026-08-08T12:00:00Z")
        candidates = [
            first,
            ingest_duplicate,
            same_round_retry,
            later_round,
            material_split,
        ]
        root_ids = {
            metric._candidate_key(row): "root:retried-and-split"
            for row in candidates
        }

        result = self.evaluate(complete_documents(candidates, root_ids=root_ids))

        self.assertEqual(result["status"], "available")
        self.assertEqual(result["north_star"]["total_root_work_units"], 1)
        self.assertEqual(result["diagnostics"]["ledger_records"], 4)
        first_ledger = next(
            row
            for row in result["ledger"]
            if row["candidate_key"] == metric._candidate_key(first)
        )
        self.assertEqual(first_ledger["source_record_count"], 2)
        self.assertIn("duplicate_event_collapsed", first_ledger["informational_codes"])
        later_ledgers = [
            row
            for row in result["ledger"]
            if row["candidate_key"] != metric._candidate_key(first)
        ]
        self.assertEqual(len(later_ledgers), 3)
        self.assertTrue(
            all("root_work_duplicate" in row["reason_codes"] for row in later_ledgers)
        )
        self.assertTrue(all(row["counted"] is False for row in later_ledgers))

    def test_root_work_is_not_recounted_across_windows(self):
        row = candidate()
        documents = list(complete_documents([row]))
        prior = candidate(99, occurred_at="2026-07-01T00:00:00Z")
        documents[3]["records"][0]["earliest_qualifying_candidate_key"] = (
            metric._candidate_key(prior)
        )
        result = self.evaluate(documents)
        self.assertEqual(result["status"], "available")
        self.assertEqual(result["north_star"]["total_root_work_units"], 0)
        self.assertIn(
            "root_work_not_all_history_earliest",
            result["ledger"][0]["reason_codes"],
        )

    def test_same_root_conflicting_value_decisions_fail_closed(self):
        first = candidate(1)
        second = candidate(2, occurred_at="2026-08-06T12:00:00Z")
        first_key = metric._candidate_key(first)
        second_key = metric._candidate_key(second)
        documents = complete_documents(
            [first, second],
            root_ids={first_key: "root:conflict", second_key: "root:conflict"},
            classifications={
                first_key: "standalone_end_customer_value",
                second_key: "synthetic_canary",
            },
        )
        result = self.evaluate(documents)
        self.assertEqual(result["status"], "unavailable")
        self.assertIn("root_work_decision_conflict", result["unavailable_reason_codes"])

    def test_root_dedup_orders_parsed_instants_not_timestamp_text(self):
        earlier = candidate(1, occurred_at="2026-08-05T00:00:00Z")
        later = candidate(2, occurred_at="2026-08-05T00:00:00.100000Z")
        set_reconciled_solver_payout(later, 200)
        later["settled_gmv_base_units"] = 210
        shared_root = {
            metric._candidate_key(earlier): "root:fractional-time",
            metric._candidate_key(later): "root:fractional-time",
        }
        result = self.evaluate(
            complete_documents([later, earlier], root_ids=shared_root)
        )
        self.assertEqual(result["north_star"]["total_root_work_units"], 1)
        self.assertEqual(
            result["secondary_metrics"]["median_qualifying_solver_payout_base_units"],
            "100",
        )
        earlier_ledger = next(
            row
            for row in result["ledger"]
            if row["candidate_key"] == metric._candidate_key(earlier)
        )
        self.assertTrue(earlier_ledger["counted"])

    def test_exact_duplicate_collapses_but_conflict_fails_closed(self):
        row = candidate()
        result = self.evaluate(complete_documents([row, copy.deepcopy(row)]))
        self.assertEqual(result["status"], "available")
        self.assertEqual(result["diagnostics"]["ledger_records"], 1)
        self.assertEqual(result["ledger"][0]["source_record_count"], 2)
        self.assertIn("duplicate_event_collapsed", result["ledger"][0]["informational_codes"])

        conflict = copy.deepcopy(row)
        conflict["settled_gmv_base_units"] = 111
        result = self.evaluate(complete_documents([row, conflict]))
        self.assertEqual(result["status"], "unavailable")
        self.assertIn("conflicting_duplicate_event", result["unavailable_reason_codes"])

    def test_canonical_identity_is_chain_transaction_and_log_not_protocol(self):
        first = candidate(1)
        conflicting_protocol_label = copy.deepcopy(first)
        conflicting_protocol_label["protocol"] = "open_competition_v1"
        result = self.evaluate(
            complete_documents([first, conflicting_protocol_label])
        )
        self.assertEqual(result["status"], "unavailable")
        self.assertEqual(result["diagnostics"]["unique_canonical_event_identities"], 1)
        self.assertEqual(result["diagnostics"]["ledger_records"], 1)
        self.assertIn("conflicting_duplicate_event", result["unavailable_reason_codes"])

    def test_cross_candidate_canonical_funding_log_conflict_fails_closed(self):
        first = candidate(1)
        second = candidate(2)
        second["funding_events"][0] = copy.deepcopy(first["funding_events"][0])
        result = self.evaluate(complete_documents([first, second]))
        self.assertEqual(result["status"], "unavailable")
        self.assertIn(
            "conflicting_reused_canonical_evidence_log",
            result["unavailable_reason_codes"],
        )

    def test_same_scope_different_settlements_cannot_reuse_canonical_evidence(self):
        first = candidate(1)
        second = copy.deepcopy(first)
        second["log_index"] = 9_999
        second["solver_payout_reconciliation"]["settlement_event"][
            "log_index"
        ] = 9_999
        reanchor_payout_views(second)

        result = self.evaluate(complete_documents([first, second]))

        self.assertEqual(result["status"], "unavailable")
        self.assertIn(
            "conflicting_reused_canonical_evidence_log",
            result["unavailable_reason_codes"],
        )

    def test_reorg_and_block_disagreement_fail_closed(self):
        row = candidate()
        row["canonicality"]["reorg_detected"] = True
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "unavailable")
        self.assertIn("reorg_or_removed_log_detected", result["unavailable_reason_codes"])

        row = candidate()
        row["canonicality"]["rpc_block_observations"][1]["block_hash"] = "0x" + "cd" * 32
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "unavailable")
        self.assertIn("rpc_block_identity_disagreement", result["unavailable_reason_codes"])

        row = candidate()
        row["canonicality"]["raw_evidence_refs"] = ["   "]
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "unavailable")
        self.assertIn("settlement_raw_evidence_missing", result["unavailable_reason_codes"])

    def test_unaccepted_finality_status_fails_closed_in_isolation(self):
        accepted = candidate()
        accepted["canonicality"]["finality_status"] = "finalized"
        accepted_result = self.evaluate(complete_documents([accepted]))
        self.assertEqual(accepted_result["status"], "available")

        for finality_status in ("latest", "unsafe", None, 123):
            with self.subTest(finality_status=finality_status):
                row = candidate()
                row["canonicality"]["finality_status"] = finality_status
                result = self.evaluate(complete_documents([row]))
                self.assertEqual(result["status"], "unavailable")
                self.assertEqual(result["ledger"][0]["status"], "unknown")
                self.assertIn(
                    "finality_not_established", result["unavailable_reason_codes"]
                )

    def test_canonicality_boolean_and_receipt_types_are_strict(self):
        for field, value, reason in (
            ("removed", None, "reorg_or_removed_log_detected"),
            ("reorg_detected", "false", "reorg_or_removed_log_detected"),
            ("receipt_status", True, "successful_receipt_unreconciled"),
        ):
            with self.subTest(field=field, value=value):
                row = candidate()
                if value is None:
                    row["canonicality"].pop(field)
                else:
                    row["canonicality"][field] = value
                result = self.evaluate(complete_documents([row]))
                self.assertEqual(result["status"], "unavailable")
                self.assertIn(reason, result["unavailable_reason_codes"])

    def test_receipt_state_payout_and_funding_reconciliation_fail_closed(self):
        mutations = (
            ("successful_receipt_unreconciled", lambda row: row["canonicality"].update(receipt_status=0)),
            (
                "settlement_event_receipt_mismatch",
                lambda row: row["canonicality"].update(settlement_event_matches_receipt=False),
            ),
            (
                "settlement_state_unreconciled",
                lambda row: row["canonicality"].update(contract_state_reconciled=False),
            ),
            ("funding_total_unreconciled", lambda row: row.update(funding_reconciled_to_contract=False)),
            ("solver_payout_unreconciled", lambda row: row.update(solver_payout_base_units=None)),
        )
        for reason, mutate in mutations:
            with self.subTest(reason=reason):
                row = candidate()
                mutate(row)
                result = self.evaluate(complete_documents([row]))
                self.assertEqual(result["status"], "unavailable")
                self.assertIn(reason, result["unavailable_reason_codes"])

    def test_solver_payout_reconciles_event_receipt_bond_and_contract_state(self):
        reconciled = self.evaluate(complete_documents([candidate()]))
        self.assertEqual(reconciled["status"], "available")
        self.assertEqual(reconciled["ledger"][0]["solver_payout_base_units"], 100)

        mismatches = (
            ("settlement_event", "solver_reward_base_units", 99),
            ("receipt_transfer", "amount_base_units", 109),
            ("receipt_transfer", "returned_bond_base_units", 11),
            ("contract_state", "solver_payout_base_units", 99),
        )
        for view, field, value in mismatches:
            with self.subTest(view=view, field=field):
                row = candidate()
                row["solver_payout_reconciliation"][view][field] = value
                result = self.evaluate(complete_documents([row]))
                self.assertEqual(result["status"], "unavailable")
                self.assertEqual(result["ledger"][0]["status"], "unknown")
                self.assertIn(
                    "solver_payout_reconciliation_mismatch",
                    result["unavailable_reason_codes"],
                )

        row = candidate()
        row.pop("solver_payout_reconciliation")
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "unavailable")
        self.assertIn(
            "solver_payout_reconciliation_missing",
            result["unavailable_reason_codes"],
        )

        row = candidate()
        row["solver_payout_reconciliation"]["settlement_event"][
            "transaction_hash"
        ] = "0x" + "ff" * 32
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "unavailable")
        self.assertIn(
            "solver_payout_event_identity_mismatch",
            result["unavailable_reason_codes"],
        )

        identity_mismatches = (
            ("settlement_event", "event_kind", "wrong_event",
             "solver_payout_event_identity_mismatch"),
            ("settlement_event", "protocol_version", "wrong-version",
             "solver_payout_event_identity_mismatch"),
            ("receipt_transfer", "asset_contract", "0x" + "ff" * 20,
             "solver_payout_receipt_identity_mismatch"),
            ("receipt_transfer", "to_address", FUNDER,
             "solver_payout_receipt_identity_mismatch"),
            ("contract_state", "observed_at_block_number", 1,
             "solver_payout_state_identity_mismatch"),
        )
        for view, field, value, reason in identity_mismatches:
            with self.subTest(view=view, field=field):
                row = candidate()
                row["solver_payout_reconciliation"][view][field] = value
                result = self.evaluate(complete_documents([row]))
                self.assertEqual(result["status"], "unavailable")
                self.assertIn(reason, result["unavailable_reason_codes"])

        row = candidate()
        row["round"] = None
        row["participation_event"]["round"] = None
        row["funding_events"][0]["round"] = None
        row["solver_payout_reconciliation"]["settlement_event"]["round"] = "invalid"
        row["solver_payout_reconciliation"]["contract_state"]["round"] = None
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "unavailable")
        self.assertIn(
            "solver_payout_event_identity_mismatch",
            result["unavailable_reason_codes"],
        )

        row = candidate()
        shared_ref = row["solver_payout_reconciliation"]["settlement_event"][
            "raw_evidence_refs"
        ]
        row["solver_payout_reconciliation"]["receipt_transfer"][
            "raw_evidence_refs"
        ] = shared_ref
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "unavailable")
        self.assertIn(
            "solver_payout_evidence_views_not_distinct",
            result["unavailable_reason_codes"],
        )

        row = candidate()
        row["solver_payout_reconciliation"]["contract_state"][
            "observed_at_block_hash"
        ] = "0x" + "ff" * 32
        for observation in row["solver_payout_reconciliation"]["contract_state"][
            "rpc_block_observations"
        ]:
            observation["block_hash"] = "0x" + "ff" * 32
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "unavailable")
        self.assertIn(
            "solver_payout_state_identity_mismatch",
            result["unavailable_reason_codes"],
        )

        row = candidate()
        row["solver_payout_reconciliation"]["contract_state"][
            "rpc_block_observations"
        ] = row["solver_payout_reconciliation"]["contract_state"][
            "rpc_block_observations"
        ][:1]
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "unavailable")
        self.assertIn(
            "solver_payout_state_identity_mismatch",
            result["unavailable_reason_codes"],
        )

        row = candidate()
        row["solver_payout_reconciliation"]["settlement_event"][
            "solver_reward_base_units"
        ] = 120
        row["solver_payout_reconciliation"]["settlement_event"][
            "completion_or_timeout_bonus_base_units"
        ] = 0
        row["solver_payout_base_units"] = 120
        row["solver_payout_reconciliation"]["receipt_transfer"][
            "amount_base_units"
        ] = 130
        row["solver_payout_reconciliation"]["contract_state"][
            "solver_payout_base_units"
        ] = 120
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "unavailable")
        self.assertIn(
            "solver_payout_reconciliation_mismatch",
            result["unavailable_reason_codes"],
        )

    def test_protocol_specific_returned_bond_semantics(self):
        autonomous = candidate()
        autonomous["solver_payout_reconciliation"]["settlement_event"][
            "returned_bond_base_units"
        ] = 50
        autonomous["solver_payout_reconciliation"]["receipt_transfer"][
            "returned_bond_base_units"
        ] = 50
        autonomous["solver_payout_reconciliation"]["receipt_transfer"][
            "amount_base_units"
        ] = 150
        reanchor_payout_views(autonomous)
        autonomous_result = self.evaluate(complete_documents([autonomous]))
        self.assertEqual(autonomous_result["status"], "available")
        self.assertEqual(
            autonomous_result["north_star"]["total_root_work_units"], 1
        )

        autonomous = candidate()
        autonomous["solver_payout_reconciliation"]["receipt_transfer"][
            "returned_bond_base_units"
        ] = 50
        autonomous["solver_payout_reconciliation"]["receipt_transfer"][
            "amount_base_units"
        ] = 150
        autonomous_result = self.evaluate(complete_documents([autonomous]))
        self.assertEqual(autonomous_result["status"], "unavailable")
        self.assertIn(
            "solver_payout_reconciliation_mismatch",
            autonomous_result["unavailable_reason_codes"],
        )

        v1 = set_candidate_protocol(
            candidate(1),
            protocol="open_competition_v1",
            protocol_version="agent-bounties/open-competition-v1",
            factory_contract="0x9e9382beb8b1a45b737d484b5eafa7b8779d4ca5",
            settlement_event_kind="bounty_settled",
            participation_event_kind="solution_committed",
            round_value=None,
        )
        v1_result = self.evaluate(complete_documents([v1]))
        self.assertEqual(v1_result["status"], "available")
        self.assertEqual(v1_result["north_star"]["total_root_work_units"], 1)

        v2 = set_candidate_protocol(
            candidate(2),
            protocol="open_competition_v2",
            protocol_version="agent-bounties/open-competition-v2-beta3",
            factory_contract="0x29d0e39e0c03797c690633535722e6b34a69a78a",
            settlement_event_kind="competition_settled",
            participation_event_kind="entry_qualified",
            round_value=None,
        )
        positive_bond_result = self.evaluate(complete_documents([v2]))
        self.assertEqual(positive_bond_result["status"], "unavailable")
        self.assertIn(
            "solver_payout_reconciliation_mismatch",
            positive_bond_result["unavailable_reason_codes"],
        )

        set_reconciled_solver_payout(v2, 100, returned_bond=0)
        zero_bond_result = self.evaluate(complete_documents([v2]))
        self.assertEqual(zero_bond_result["status"], "available")
        self.assertEqual(zero_bond_result["north_star"]["total_root_work_units"], 1)

    def test_rpc_minimums_apply_to_their_declared_layers(self):
        documents = list(complete_documents([candidate()]))
        documents[1]["canonicality"]["minimum_independent_rpc_observations"] = 2
        documents[1]["qualification"]["solver_payout_reconciliation"][
            "contract_state_minimum_independent_rpc_observations"
        ] = 3
        for source in documents[0]["source_reconciliation"]["protocols"]:
            source["independent_rpc_observation_count"] = 3
        payout_limited = self.evaluate(documents)
        self.assertEqual(payout_limited["status"], "unavailable")
        self.assertIn(
            "solver_payout_state_identity_mismatch",
            payout_limited["unavailable_reason_codes"],
        )
        self.assertNotIn(
            "protocol_finality_evidence_incomplete",
            payout_limited["unavailable_reason_codes"],
        )

        documents = list(complete_documents([candidate()]))
        documents[1]["canonicality"]["minimum_independent_rpc_observations"] = 3
        documents[1]["qualification"]["solver_payout_reconciliation"][
            "contract_state_minimum_independent_rpc_observations"
        ] = 2
        source_limited = self.evaluate(documents)
        self.assertEqual(source_limited["status"], "unavailable")
        self.assertIn(
            "protocol_finality_evidence_incomplete",
            source_limited["unavailable_reason_codes"],
        )
        self.assertNotIn(
            "solver_payout_state_identity_mismatch",
            source_limited["unavailable_reason_codes"],
        )

        for path in ("canonicality", "payout"):
            with self.subTest(invalid_policy_minimum=path):
                documents = list(complete_documents([candidate()]))
                if path == "canonicality":
                    documents[1]["canonicality"][
                        "minimum_independent_rpc_observations"
                    ] = 0
                else:
                    documents[1]["qualification"]["solver_payout_reconciliation"][
                        "contract_state_minimum_independent_rpc_observations"
                    ] = True
                with self.assertRaisesRegex(
                    metric.MetricContractError, "must be a positive integer"
                ):
                    self.evaluate(documents)

    def test_zero_payout_and_unsupported_factory_are_excluded(self):
        row = candidate()
        row["solver_payout_base_units"] = 0
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "available")
        self.assertIn("solver_payout_not_positive", result["ledger"][0]["reason_codes"])

        row = candidate()
        row["factory_contract"] = "0x9999999999999999999999999999999999999999"
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "available")
        self.assertIn("unsupported_factory_scope", result["ledger"][0]["reason_codes"])

        row = candidate()
        row["event_kind"] = "submission_rejected"
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "available")
        self.assertIn("wrong_settlement_event_kind", result["ledger"][0]["reason_codes"])

    def test_funding_threshold_uses_promised_principal_not_bonus_in_payout(self):
        row = candidate()
        set_reconciled_solver_payout(row, 120)
        row["promised_reward_principal_base_units"] = 100
        row["funding_events"][0]["amount_base_units"] = 100
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "available")
        self.assertEqual(result["north_star"]["total_root_work_units"], 1)

    def test_protocol_round_semantics_and_strict_evidence_rounds(self):
        row = candidate()
        row["round"] = None
        row["participation_event"]["round"] = None
        row["funding_events"][0]["round"] = None
        row["solver_payout_reconciliation"]["settlement_event"]["round"] = None
        row["solver_payout_reconciliation"]["contract_state"]["round"] = None
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "unavailable")
        self.assertIn("candidate_round_invalid", result["unavailable_reason_codes"])

        row = candidate()
        row["round"] = None
        row["solver_payout_reconciliation"]["settlement_event"]["round"] = None
        row["solver_payout_reconciliation"]["contract_state"]["round"] = None
        row["participation_event"]["round"] = "not-a-round"
        row["funding_events"][0]["round"] = "not-a-round"
        result = self.evaluate(complete_documents([row]))
        self.assertIn(
            "participation_candidate_scope_mismatch",
            result["unavailable_reason_codes"],
        )
        self.assertIn(
            "funding_candidate_scope_mismatch",
            result["unavailable_reason_codes"],
        )

    def test_operator_subsidy_is_separate_guardrail(self):
        row = candidate()
        operator_funding = copy.deepcopy(row["funding_events"][0])
        operator_funding.update(
            {
                "transaction_hash": "0x" + "55" * 32,
                "contributor_wallet": OPERATOR,
                "amount_base_units": 20,
                "block_number": 850,
                "log_index": 1,
                "raw_evidence_refs": ["fixture:operator-funding"],
            }
        )
        row["funding_events"].append(operator_funding)
        row["operator_subsidies"] = {
            "status": "complete",
            "entries": [
                {
                    "kind": "gas",
                    "asset": "ETH",
                    "amount_base_units": 7,
                    "evidence_refs": ["fixture:gas-subsidy"],
                }
            ],
        }
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["north_star"]["total_root_work_units"], 1)
        guardrail = result["secondary_metrics"]["operator_subsidy_guardrail"]
        self.assertEqual(guardrail["status"], "available")
        self.assertEqual(guardrail["operator_reward_funding_usdc_base_units"], 20)
        self.assertEqual(
            guardrail["operator_reward_funding_share_of_qualifying_settled_gmv"],
            "0.181818",
        )
        self.assertEqual(
            guardrail["declared_nonreward_subsidies_by_asset_and_kind"],
            [{"asset": "ETH", "kind": "gas", "amount_base_units": 7}],
        )

    def test_funder_concentration_uses_actual_funding_amounts(self):
        row = candidate()
        row["funding_events"][0]["amount_base_units"] = 150
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(
            result["secondary_metrics"]["independent_preparticipation_funding_base_units"],
            150,
        )
        self.assertEqual(
            result["secondary_metrics"]["largest_independent_funding_principal_share"],
            "1.000000",
        )

    def test_missing_operator_subsidy_evidence_only_withholds_guardrail(self):
        row = candidate()
        row.pop("operator_subsidies")
        result = self.evaluate(complete_documents([row]))
        self.assertEqual(result["status"], "available")
        self.assertEqual(result["north_star"]["total_root_work_units"], 1)
        guardrail = result["secondary_metrics"]["operator_subsidy_guardrail"]
        self.assertEqual(guardrail["status"], "unavailable")
        self.assertIn(
            "operator_subsidy_evidence_incomplete",
            guardrail["unavailable_reason_codes"],
        )

    def test_frozen_baseline_is_reproducible_and_unavailable_not_zero(self):
        documents = (
            json.loads(BASELINE_INPUT_PATH.read_text()),
            json.loads(POLICY_PATH.read_text()),
            json.loads(PRINCIPALS_PATH.read_text()),
            json.loads(ROOT_MAP_PATH.read_text()),
        )
        first = self.evaluate(documents)
        second = self.evaluate(copy.deepcopy(documents))
        frozen = json.loads(BASELINE_RESULT_PATH.read_text())
        self.assertEqual(first, second)
        self.assertEqual(first, frozen)
        self.assertEqual(first["status"], "unavailable")
        self.assertIsNone(first["north_star"]["total_root_work_units"])
        self.assertTrue(all(value is None for value in first["diagnostics"].values()))
        self.assertIsNone(first["ledger"])

    def test_policy_change_changes_hash_without_rewriting_input(self):
        documents = list(complete_documents([candidate()]))
        original = self.evaluate(documents)
        changed_policy = copy.deepcopy(documents[1])
        changed_policy["metric_contract_version"] = "1.1.1-test"
        changed = metric.evaluate(documents[0], changed_policy, documents[2], documents[3])
        self.assertNotEqual(original["policy_hash"], changed["policy_hash"])
        self.assertNotEqual(original["result_hash"], changed["result_hash"])

    def test_input_freeze_cannot_predate_policy(self):
        documents = list(complete_documents([candidate()]))
        documents[0]["frozen_at"] = "2026-09-01T00:00:00Z"
        with self.assertRaisesRegex(
            metric.MetricContractError, "cannot predate the applied policy"
        ):
            self.evaluate(documents)

    def test_cli_check_result(self):
        completed = subprocess.run(
            [
                sys.executable,
                str(ROOT / "scripts/qicswu_metric.py"),
                "--input",
                str(BASELINE_INPUT_PATH),
                "--policy",
                str(POLICY_PATH),
                "--principals",
                str(PRINCIPALS_PATH),
                "--root-map",
                str(ROOT_MAP_PATH),
                "--check-result",
                str(BASELINE_RESULT_PATH),
            ],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_baseline_package_verifier_checks_evidence_anchors(self):
        summary = baseline_verifier.verify_package(
            repo_root=ROOT,
            input_path=BASELINE_INPUT_PATH,
            policy_path=POLICY_PATH,
            principals_path=PRINCIPALS_PATH,
            root_map_path=ROOT_MAP_PATH,
            source_path=ROOT / "ops/metrics/qicswu-baseline-source-observations-v1.json",
            result_path=BASELINE_RESULT_PATH,
        )
        self.assertEqual(summary["status"], "verified_unavailable")
        self.assertEqual(summary["raw_candidate_settlements"], 28)
        self.assertEqual(summary["aggregate_aligned_unresolved_candidates"], 26)

    def test_baseline_package_verifier_rejects_tampered_source(self):
        source = json.loads(
            (ROOT / "ops/metrics/qicswu-baseline-source-observations-v1.json").read_text()
        )
        source["reconciliation"]["reconciled_settlement_count"] = 25
        with tempfile.TemporaryDirectory() as directory:
            tampered = Path(directory) / "source.json"
            tampered.write_text(json.dumps(source))
            with self.assertRaises(baseline_verifier.BaselineVerificationError):
                baseline_verifier.verify_package(
                    repo_root=ROOT,
                    input_path=BASELINE_INPUT_PATH,
                    policy_path=POLICY_PATH,
                    principals_path=PRINCIPALS_PATH,
                    root_map_path=ROOT_MAP_PATH,
                    source_path=tampered,
                    result_path=BASELINE_RESULT_PATH,
                )

    def test_baseline_verifier_requires_exact_historical_policy_partition(self):
        source = json.loads(
            (ROOT / "ops/metrics/qicswu-baseline-source-observations-v1.json").read_text()
        )
        historical = source["historical_factory_reconciliation"]
        rows = historical["historical_factory_settlement_rows_retained_in_snapshot"]
        historical["declared_canary_rows_excluded_by_existing_platform_metric"] = copy.deepcopy(
            rows[2:4]
        )
        historical["aggregate_gap_recovery_rows"] = copy.deepcopy(
            [rows[0], rows[1], *rows[4:]]
        )
        with tempfile.TemporaryDirectory() as directory:
            source_path = Path(directory) / "source.json"
            input_path = Path(directory) / "input.json"
            result_path = Path(directory) / "result.json"
            source_path.write_text(json.dumps(source))
            baseline_input = json.loads(BASELINE_INPUT_PATH.read_text())
            baseline_input["source_reconciliation"][
                "source_observations_file_sha256"
            ] = baseline_verifier._sha256_file(source_path)
            input_path.write_text(json.dumps(baseline_input))
            tampered_result = metric.evaluate(
                baseline_input,
                json.loads(POLICY_PATH.read_text()),
                json.loads(PRINCIPALS_PATH.read_text()),
                json.loads(ROOT_MAP_PATH.read_text()),
            )
            result_path.write_text(json.dumps(tampered_result))
            with self.assertRaisesRegex(
                baseline_verifier.BaselineVerificationError,
                "exact policy partition",
            ):
                baseline_verifier.verify_package(
                    repo_root=ROOT,
                    input_path=input_path,
                    policy_path=POLICY_PATH,
                    principals_path=PRINCIPALS_PATH,
                    root_map_path=ROOT_MAP_PATH,
                    source_path=source_path,
                    result_path=result_path,
                )

    def test_baseline_verifier_rejects_coordinated_candidate_value_tamper(self):
        baseline_input = json.loads(BASELINE_INPUT_PATH.read_text())
        baseline_input["candidates"][0]["solver_payout_base_units"] = 124_446_789
        tampered_result = metric.evaluate(
            baseline_input,
            json.loads(POLICY_PATH.read_text()),
            json.loads(PRINCIPALS_PATH.read_text()),
            json.loads(ROOT_MAP_PATH.read_text()),
        )
        with tempfile.TemporaryDirectory() as directory:
            input_path = Path(directory) / "input.json"
            result_path = Path(directory) / "result.json"
            input_path.write_text(json.dumps(baseline_input))
            result_path.write_text(json.dumps(tampered_result))
            with self.assertRaisesRegex(
                baseline_verifier.BaselineVerificationError,
                "not exactly reproducible from retained records",
            ):
                baseline_verifier.verify_package(
                    repo_root=ROOT,
                    input_path=input_path,
                    policy_path=POLICY_PATH,
                    principals_path=PRINCIPALS_PATH,
                    root_map_path=ROOT_MAP_PATH,
                    source_path=(
                        ROOT / "ops/metrics/qicswu-baseline-source-observations-v1.json"
                    ),
                    result_path=result_path,
                )

    def test_baseline_verifier_rejects_duplicate_candidate(self):
        baseline_input = json.loads(BASELINE_INPUT_PATH.read_text())
        baseline_input["candidates"].append(copy.deepcopy(baseline_input["candidates"][0]))
        tampered_result = metric.evaluate(
            baseline_input,
            json.loads(POLICY_PATH.read_text()),
            json.loads(PRINCIPALS_PATH.read_text()),
            json.loads(ROOT_MAP_PATH.read_text()),
        )
        with tempfile.TemporaryDirectory() as directory:
            input_path = Path(directory) / "input.json"
            result_path = Path(directory) / "result.json"
            input_path.write_text(json.dumps(baseline_input))
            result_path.write_text(json.dumps(tampered_result))
            with self.assertRaisesRegex(
                baseline_verifier.BaselineVerificationError,
                "cardinality or canonical identity uniqueness mismatch",
            ):
                baseline_verifier.verify_package(
                    repo_root=ROOT,
                    input_path=input_path,
                    policy_path=POLICY_PATH,
                    principals_path=PRINCIPALS_PATH,
                    root_map_path=ROOT_MAP_PATH,
                    source_path=(
                        ROOT / "ops/metrics/qicswu-baseline-source-observations-v1.json"
                    ),
                    result_path=result_path,
                )

    def test_historical_factory_membership_rejects_coordinated_provenance_tamper(self):
        source = json.loads(
            (
                ROOT / "ops/metrics/qicswu-baseline-source-observations-v1.json"
            ).read_text()
        )
        snapshot = json.loads(
            (
                ROOT
                / "site/generated/gmv-snapshots/external-gmv-sprint-20260820-v1.json"
            ).read_text()
        )
        evidence = json.loads(
            (
                ROOT
                / "ops/metrics/qicswu-baseline-historical-v2-factory-membership-2026-09-02-v1.json"
            ).read_text()
        )
        replacement_bounty_id = "0x" + "ff" * 32
        for observation in evidence["provider_observations"]:
            observation["logs"][0]["topics"][1] = replacement_bounty_id
            observation["canonical_logs_sha256"] = metric.sha256_json(
                observation["logs"]
            )

        with tempfile.TemporaryDirectory() as directory:
            temporary_root = Path(directory)
            evidence_path = temporary_root / "membership.json"
            evidence_path.write_text(json.dumps(evidence))
            historical = copy.deepcopy(source["historical_factory_reconciliation"])
            historical["factory_membership_evidence_path"] = "membership.json"
            historical[
                "factory_membership_evidence_file_sha256"
            ] = baseline_verifier._sha256_file(evidence_path)
            with self.assertRaisesRegex(
                baseline_verifier.BaselineVerificationError,
                "not all proven factory members",
            ):
                baseline_verifier._verify_historical_factory_membership(
                    repo_root=temporary_root,
                    historical=historical,
                    snapshot=snapshot,
                )

    def test_historical_factory_membership_requires_distinct_provider_authorities(self):
        source = json.loads(
            (ROOT / "ops/metrics/qicswu-baseline-source-observations-v1.json").read_text()
        )
        snapshot = json.loads(
            (
                ROOT
                / "site/generated/gmv-snapshots/external-gmv-sprint-20260820-v1.json"
            ).read_text()
        )
        evidence = json.loads(
            (
                ROOT
                / "ops/metrics/qicswu-baseline-historical-v2-factory-membership-2026-09-02-v1.json"
            ).read_text()
        )
        evidence["provider_observations"][1]["endpoint"] = evidence[
            "provider_observations"
        ][0]["endpoint"]
        with tempfile.TemporaryDirectory() as directory:
            temporary_root = Path(directory)
            evidence_path = temporary_root / "membership.json"
            evidence_path.write_text(json.dumps(evidence))
            historical = copy.deepcopy(source["historical_factory_reconciliation"])
            historical["factory_membership_evidence_path"] = "membership.json"
            historical[
                "factory_membership_evidence_file_sha256"
            ] = baseline_verifier._sha256_file(evidence_path)
            with self.assertRaisesRegex(
                baseline_verifier.BaselineVerificationError,
                "endpoint authorities",
            ):
                baseline_verifier._verify_historical_factory_membership(
                    repo_root=temporary_root,
                    historical=historical,
                    snapshot=snapshot,
                )

    def test_historical_factory_provider_authority_rejects_same_host_on_any_port(self):
        source = json.loads(
            (ROOT / "ops/metrics/qicswu-baseline-source-observations-v1.json").read_text()
        )
        snapshot = json.loads(
            (
                ROOT
                / "site/generated/gmv-snapshots/external-gmv-sprint-20260820-v1.json"
            ).read_text()
        )
        evidence = json.loads(
            (
                ROOT
                / "ops/metrics/qicswu-baseline-historical-v2-factory-membership-2026-09-02-v1.json"
            ).read_text()
        )
        first_endpoint = evidence["provider_observations"][0]["endpoint"].rstrip("/")
        for port in (443, 444):
            with self.subTest(port=port), tempfile.TemporaryDirectory() as directory:
                modified = copy.deepcopy(evidence)
                modified["provider_observations"][1]["endpoint"] = (
                    f"{first_endpoint}:{port}"
                )
                temporary_root = Path(directory)
                evidence_path = temporary_root / "membership.json"
                evidence_path.write_text(json.dumps(modified))
                historical = copy.deepcopy(source["historical_factory_reconciliation"])
                historical["factory_membership_evidence_path"] = "membership.json"
                historical[
                    "factory_membership_evidence_file_sha256"
                ] = baseline_verifier._sha256_file(evidence_path)
                with self.assertRaisesRegex(
                    baseline_verifier.BaselineVerificationError,
                    "endpoint authorities",
                ):
                    baseline_verifier._verify_historical_factory_membership(
                        repo_root=temporary_root,
                        historical=historical,
                        snapshot=snapshot,
                    )

    def test_retained_historical_snapshot_cannot_escape_repository(self):
        with tempfile.TemporaryDirectory() as directory:
            external = Path(directory) / "snapshot.json"
            external.write_text("{}")
            for escaped in (str(external), str(Path("..") / external.name)):
                with self.subTest(path=escaped), self.assertRaisesRegex(
                    baseline_verifier.BaselineVerificationError,
                    "must remain inside the repository",
                ):
                    baseline_verifier._retained_artifact_path(
                        ROOT,
                        escaped,
                        "retained historical-factory snapshot",
                    )

    def test_historical_membership_rows_match_retained_creation_logs_exactly(self):
        source = json.loads(
            (ROOT / "ops/metrics/qicswu-baseline-source-observations-v1.json").read_text()
        )
        snapshot = json.loads(
            (
                ROOT
                / "site/generated/gmv-snapshots/external-gmv-sprint-20260820-v1.json"
            ).read_text()
        )
        evidence = json.loads(
            (
                ROOT
                / "ops/metrics/qicswu-baseline-historical-v2-factory-membership-2026-09-02-v1.json"
            ).read_text()
        )
        evidence["snapshot_v2_factory_membership"][0][
            "factory_creation_transaction_hash"
        ] = "0x" + "ff" * 32
        with tempfile.TemporaryDirectory() as directory:
            temporary_root = Path(directory)
            evidence_path = temporary_root / "membership.json"
            evidence_path.write_text(json.dumps(evidence))
            historical = copy.deepcopy(source["historical_factory_reconciliation"])
            historical["factory_membership_evidence_path"] = "membership.json"
            historical[
                "factory_membership_evidence_file_sha256"
            ] = baseline_verifier._sha256_file(evidence_path)
            with self.assertRaisesRegex(
                baseline_verifier.BaselineVerificationError,
                "retained creation log",
            ):
                baseline_verifier._verify_historical_factory_membership(
                    repo_root=temporary_root,
                    historical=historical,
                    snapshot=snapshot,
                )

    def test_historical_factory_creation_must_precede_settlement_within_block(self):
        source = json.loads(
            (ROOT / "ops/metrics/qicswu-baseline-source-observations-v1.json").read_text()
        )
        snapshot = json.loads(
            (
                ROOT
                / "site/generated/gmv-snapshots/external-gmv-sprint-20260820-v1.json"
            ).read_text()
        )
        evidence = json.loads(
            (
                ROOT
                / "ops/metrics/qicswu-baseline-historical-v2-factory-membership-2026-09-02-v1.json"
            ).read_text()
        )
        membership = evidence["snapshot_v2_factory_membership"][0]
        settlement = next(
            row
            for row in snapshot["settlements"]
            if row["bounty_id"].lower() == membership["bounty_id"].lower()
        )
        settlement["block_number"] = membership["factory_creation_block_number"]
        settlement["log_index"] = membership["factory_creation_log_index"] - 1

        with tempfile.TemporaryDirectory() as directory:
            temporary_root = Path(directory)
            evidence_path = temporary_root / "membership.json"
            evidence_path.write_text(json.dumps(evidence))
            historical = copy.deepcopy(source["historical_factory_reconciliation"])
            historical["factory_membership_evidence_path"] = "membership.json"
            historical[
                "factory_membership_evidence_file_sha256"
            ] = baseline_verifier._sha256_file(evidence_path)
            with self.assertRaisesRegex(
                baseline_verifier.BaselineVerificationError,
                "must precede",
            ):
                baseline_verifier._verify_historical_factory_membership(
                    repo_root=temporary_root,
                    historical=historical,
                    snapshot=snapshot,
                )

    def test_baseline_package_verifier_rejects_tampered_raw_response(self):
        retained_paths = [
            "ops/metrics/qicswu-baseline-source-observations-v1.json",
            "ops/metrics/qicswu-baseline-raw-autonomous-events-2026-09-01-v1.json",
            "ops/metrics/qicswu-baseline-raw-open-competition-v1-events-2026-09-01-v1.json",
            "ops/metrics/qicswu-baseline-raw-open-competition-v2-beta3-events-2026-09-01-v1.json",
            "ops/metrics/qicswu-baseline-historical-v2-factory-membership-2026-09-02-v1.json",
            "site/generated/gmv-snapshots/external-gmv-sprint-20260820-v1.json",
        ]
        with tempfile.TemporaryDirectory() as directory:
            temporary_root = Path(directory)
            for relative in retained_paths:
                destination = temporary_root / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT / relative, destination)

            raw_path = (
                temporary_root
                / "ops/metrics/qicswu-baseline-raw-autonomous-events-2026-09-01-v1.json"
            )
            raw = json.loads(raw_path.read_text())
            raw[0]["id"] = "tampered"
            raw_path.write_text(json.dumps(raw))

            with self.assertRaisesRegex(
                baseline_verifier.BaselineVerificationError,
                "retained public response hash mismatch",
            ):
                baseline_verifier.verify_package(
                    repo_root=temporary_root,
                    input_path=BASELINE_INPUT_PATH,
                    policy_path=POLICY_PATH,
                    principals_path=PRINCIPALS_PATH,
                    root_map_path=ROOT_MAP_PATH,
                    source_path=(
                        temporary_root
                        / "ops/metrics/qicswu-baseline-source-observations-v1.json"
                    ),
                    result_path=BASELINE_RESULT_PATH,
                )


if __name__ == "__main__":
    unittest.main()
