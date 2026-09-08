#!/usr/bin/env python3
"""Deterministic tests for the QICSWU rollout control plane."""

from __future__ import annotations

import copy
import json
import tempfile
import unittest
from datetime import date, datetime, timedelta, timezone
from pathlib import Path

from scripts import qicswu_metric as metric
from scripts import qicswu_rollout_control as rollout


ROOT = Path(__file__).resolve().parents[1]
MANIFEST = json.loads(
    (ROOT / "ops/releases/qicswu-growth-release-v1/manifest.json").read_text(
        encoding="utf-8"
    )
)
POLICY_HASH = "sha256:" + "1" * 64
SOURCE_HASH = "sha256:" + "2" * 64
RELEASE_COMMIT = "0" * 40
DEPLOYMENT_AT = "2026-09-01T00:00:00Z"
REGISTERED_AT = "2026-08-31T20:00:00Z"


def metric_result(
    *,
    started_at: str,
    days: int,
    units: list[dict] | None,
    unavailable_reasons: list[str] | None = None,
) -> dict:
    start = datetime.fromisoformat(started_at.replace("Z", "+00:00"))
    end = start + timedelta(days=days)
    available = units is not None
    rows = units or []
    payouts = sorted(row["solver_payout_base_units"] for row in rows)
    if payouts:
        middle = len(payouts) // 2
        median = (
            str(payouts[middle])
            if len(payouts) % 2
            else str((payouts[middle - 1] + payouts[middle]) / 2).rstrip("0").rstrip(".")
        )
    else:
        median = None
    ledger = []
    for row in rows:
        ledger.append(
            {
                "candidate_key": f"candidate:{row['root_work_id']}",
                "canonical_event_identity": row["canonical_event_identity"],
                "protocol": row["protocol"],
                "status": "eligible",
                "counted": True,
                "reason_codes": [],
                "informational_codes": [],
                "root_work_id": row["root_work_id"],
                "transaction_hash": row["transaction_hash"],
                "log_index": row["log_index"],
                "block_number": row["block_number"],
                "block_hash": row["block_hash"],
                "occurred_at": row["occurred_at"],
                "solver_principal_id": row["solver_principal_id"],
                "solver_payout_base_units": row["solver_payout_base_units"],
                "settled_gmv_base_units": row["settled_gmv_base_units"],
                "independent_funding_by_principal": copy.deepcopy(
                    row["independent_funding_by_principal"]
                ),
            }
        )
    body = {
        "schema_version": metric.RESULT_SCHEMA,
        "policy_id": MANIFEST["north_star"]["policy_id"],
        "policy_hash": POLICY_HASH,
        "input_hash": "sha256:" + "3" * 64,
        "principal_registry_hash": "sha256:" + "4" * 64,
        "root_work_map_hash": "sha256:" + "5" * 64,
        "window": {
            "started_at": start.astimezone(timezone.utc).isoformat().replace(
                "+00:00", "Z"
            ),
            "ended_at": end.astimezone(timezone.utc).isoformat().replace(
                "+00:00", "Z"
            ),
            "boundary": "[started_at,ended_at)",
            "complete_utc_days": days,
        },
        "status": "available" if available else "unavailable",
        "unavailable_reason_codes": [] if available else (unavailable_reasons or ["fixture_unavailable"]),
        "north_star": {
            "name": "policy_qualified_independently_funded_canonical_settled_root_work_units_per_day",
            "unit": "root_work_units_per_complete_utc_day",
            "total_root_work_units": len(rows) if available else None,
            "complete_utc_days": days,
            "per_day": str(len(rows) / days) if available else None,
            "daily": [],
        },
        "secondary_metrics": {
            "qualifying_settled_gmv_base_units": (
                sum(row["settled_gmv_base_units"] for row in rows)
                if available
                else None
            ),
            "qualifying_settled_gmv_per_complete_utc_day": None,
            "qualifying_solver_payout_base_units": (
                sum(row["solver_payout_base_units"] for row in rows)
                if available
                else None
            ),
            "median_qualifying_solver_payout_base_units": median if available else None,
            "independent_preparticipation_funding_base_units": None,
            "largest_independent_funding_principal_share": None,
            "largest_solver_principal_share_of_solver_payout": None,
            "operator_subsidy_guardrail": {},
        },
        "diagnostics": {},
        "ledger": ledger if available else None,
        "evidence_boundary": "fixture",
    }
    body["result_hash"] = metric.sha256_json(body)
    return body


def baseline(*, available: bool = True, count: int = 5) -> dict:
    if not available:
        return metric_result(
            started_at="2026-08-04T00:00:00Z",
            days=28,
            units=None,
        )
    units = [unit(index, date(2026, 8, 4) + timedelta(days=index)) for index in range(count)]
    return metric_result(
        started_at="2026-08-04T00:00:00Z", days=28, units=units
    )


def unit(
    index: int,
    day: date,
    *,
    funder: int | None = None,
    protocol: str = "open_competition_v2",
) -> dict:
    funder = index % 5 if funder is None else funder
    ordinal = index + 1
    return {
        "root_work_id": f"root:{day.isoformat()}:{ordinal}",
        "protocol": protocol,
        "canonical_event_identity": f"base-mainnet:8453:0x{ordinal:064x}:{ordinal}",
        "transaction_hash": "0x" + f"{ordinal:064x}",
        "log_index": ordinal,
        "block_number": 1_000 + ordinal,
        "block_hash": "0x" + f"{10_000 + ordinal:064x}",
        "occurred_at": f"{day.isoformat()}T12:00:00Z",
        "solver_principal_id": f"principal:solver-{index % 5}",
        "solver_payout_base_units": 100,
        "settled_gmv_base_units": 110,
        "independent_funding_by_principal": [
            {"principal_id": f"principal:funder-{funder}", "amount_base_units": 100}
        ],
    }


def preregistration() -> dict:
    return rollout.build_preregistration(
        MANIFEST,
        baseline(),
        deployment_at=DEPLOYMENT_AT,
        registered_at=REGISTERED_AT,
        release_commit=RELEASE_COMMIT,
        source_tree_sha256=SOURCE_HASH,
        slice_id="qicswu-metric-v1",
    )


def authorization(prereg: dict) -> dict:
    document = {
        "schema_version": rollout.AUTHORIZATION_SCHEMA,
        "authorization_id": "fixture-authorization",
        "preregistration_hash": prereg["artifact_hash"],
        "deployment_at": prereg["deployment"]["frozen_at"],
        "release_commit": prereg["deployment"]["release_commit"],
        "slice_id": prereg["deployment"]["slice_id"],
        "scope": "production_deployment_of_exact_registered_slice_only",
        "approvals": [
            {
                "role": role,
                "approved_by": f"fixture:{role}",
                "approved_at": REGISTERED_AT,
                "decision": "approved",
            }
            for role in prereg["approval_boundary"]["required_roles"]
        ],
        "external_outreach_authorized": False,
        "incentive_or_spend_authorized": False,
    }
    return rollout._seal(document)


def monitoring(unit_count: int) -> dict:
    return {
        "schema_version": rollout.MONITORING_SCHEMA,
        "funnel_transitions": [
            {
                "name": "ready_item_to_action_started",
                "numerator_count": 0,
                "denominator_count": 0,
                "join_coverage_status": "complete",
                "source_refs": ["fixture:funnel"],
            }
        ],
        "canonical_settlement_proofs": {
            "observed_count": unit_count,
            "direct_proof_count": unit_count,
            "exceptions": [],
        },
        "payment_correctness": {
            "reconciled_count": unit_count,
            "disputed_count": 0,
            "exceptions": [],
        },
        "inventory_correctness": {
            "status": "correct",
            "ready_to_earn_count": 7,
            "exceptions": [],
        },
        "safety_incidents": {"open_count": 0, "incidents": []},
        "unresolved_disputes": {kind: [] for kind in rollout.DISPUTE_KINDS},
    }


def daily_snapshots(prereg: dict, auth: dict) -> list[dict]:
    snapshots = []
    first = date.fromisoformat(prereg["deployment"]["first_complete_exposure_day"])
    global_ordinal = 100
    for index in range(56):
        day = first + timedelta(days=index)
        rows = []
        if index % 28 < 10:
            rows.append(unit(global_ordinal, day, funder=(index % 28) % 5))
            global_ordinal += 1
        result = metric_result(
            started_at=f"{day.isoformat()}T00:00:00Z", days=1, units=rows
        )
        snapshots.append(
            rollout.build_daily_snapshot(
                prereg,
                auth,
                result,
                monitoring(len(rows)),
                captured_at=f"{(day + timedelta(days=1)).isoformat()}T01:00:00Z",
            )
        )
    return snapshots


class QicswuRolloutControlTests(unittest.TestCase):
    def test_daily_snapshot_preserves_open_competition_as_a_first_class_path(self) -> None:
        prereg = preregistration()
        auth = authorization(prereg)
        day = date.fromisoformat(prereg["deployment"]["first_complete_exposure_day"])
        rows = [
            unit(0, day, protocol="open_competition_v1"),
            unit(1, day, protocol="open_competition_v2"),
            unit(2, day, protocol="autonomous"),
        ]
        result = metric_result(
            started_at=f"{day.isoformat()}T00:00:00Z", days=1, units=rows
        )
        snapshot = rollout.build_daily_snapshot(
            prereg,
            auth,
            result,
            monitoring(len(rows)),
            captured_at=f"{(day + timedelta(days=1)).isoformat()}T01:00:00Z",
        )

        self.assertEqual(
            [row["participation_path"] for row in snapshot["qualified_units"]],
            ["open_competition", "open_competition", "exclusive_claim"],
        )
        self.assertEqual(
            [row["protocol_role"] for row in snapshot["qualified_units"]],
            ["compatibility", "primary", "legacy"],
        )
        window = rollout._window_result(
            snapshot["qualified_units"], prereg["registered_decision"], "fixture"
        )
        self.assertEqual(
            window["qualified_root_work_units_by_protocol"],
            {"autonomous": 1, "open_competition_v1": 1, "open_competition_v2": 1},
        )
        self.assertEqual(
            window["qualified_root_work_units_by_protocol_role"],
            {"compatibility": 1, "legacy": 1, "primary": 1},
        )
        self.assertEqual(
            window["qualified_root_work_units_by_participation_path"],
            {"exclusive_claim": 1, "open_competition": 2},
        )

    def test_unavailable_baseline_fails_closed_without_a_numeric_target(self) -> None:
        result = rollout.build_preregistration(
            MANIFEST,
            baseline(available=False),
            deployment_at=DEPLOYMENT_AT,
            registered_at=REGISTERED_AT,
            release_commit=RELEASE_COMMIT,
            source_tree_sha256=SOURCE_HASH,
            slice_id="qicswu-metric-v1",
        )
        self.assertEqual(result["status"], "blocked")
        self.assertIn("baseline_metric_unavailable", result["blockers"])
        self.assertIsNone(
            result["registered_decision"]["target_count_per_28_day_window"]
        )

    def test_preregistration_freezes_formula_guardrails_and_order(self) -> None:
        result = preregistration()
        rollout.validate_preregistration(result)
        self.assertEqual(result["status"], "ready_for_approval")
        self.assertEqual(
            result["registered_decision"]["target_count_per_28_day_window"], 10
        )
        self.assertEqual(
            result["registered_decision"][
                "qualifying_settled_gmv_floor_base_units_per_28_day_window"
            ],
            550,
        )
        self.assertEqual(result["experiment_order"], list(rollout.REQUIRED_SLICE_ORDER))
        self.assertFalse(
            result["approval_boundary"]["production_deployment_authorized"]
        )

    def test_wrong_baseline_window_is_blocked(self) -> None:
        result = rollout.build_preregistration(
            MANIFEST,
            metric_result(
                started_at="2026-08-03T00:00:00Z",
                days=28,
                units=[unit(1, date(2026, 8, 3))],
            ),
            deployment_at=DEPLOYMENT_AT,
            registered_at=REGISTERED_AT,
            release_commit=RELEASE_COMMIT,
            source_tree_sha256=SOURCE_HASH,
            slice_id="qicswu-metric-v1",
        )
        self.assertEqual(result["status"], "blocked")
        self.assertIn("baseline_not_preceding_deployment", result["blockers"])

    def test_preregistration_hash_detects_mutation(self) -> None:
        result = preregistration()
        result["registered_decision"]["target_count_per_28_day_window"] = 9
        with self.assertRaisesRegex(rollout.RolloutControlError, "artifact_hash"):
            rollout.validate_preregistration(result)

    def test_authorization_cannot_silently_authorize_outreach(self) -> None:
        prereg = preregistration()
        auth = authorization(prereg)
        auth["external_outreach_authorized"] = True
        auth = rollout._seal(auth)
        with self.assertRaisesRegex(rollout.RolloutControlError, "outreach"):
            rollout.validate_authorization(auth, prereg)

    def test_later_slice_requires_the_preceding_slice_effect_to_pass(self) -> None:
        missing = rollout.build_preregistration(
            MANIFEST,
            baseline(),
            deployment_at=DEPLOYMENT_AT,
            registered_at=REGISTERED_AT,
            release_commit=RELEASE_COMMIT,
            source_tree_sha256=SOURCE_HASH,
            slice_id="truthful-ready-to-earn-v1",
        )
        self.assertEqual(missing["status"], "blocked")
        self.assertIn("preceding_slice_effect_not_evaluated", missing["blockers"])

        first = preregistration()
        first_auth = authorization(first)
        first_evaluation = rollout.evaluate_rollout(
            first,
            first_auth,
            daily_snapshots(first, first_auth),
            evaluated_at="2026-10-28T00:00:00Z",
        )
        later_baseline = metric_result(
            started_at="2026-09-30T00:00:00Z",
            days=28,
            units=[
                unit(index + 500, date(2026, 10, 1) + timedelta(days=index))
                for index in range(5)
            ],
        )
        ready = rollout.build_preregistration(
            MANIFEST,
            later_baseline,
            deployment_at="2026-10-28T00:00:00Z",
            registered_at="2026-10-27T20:00:00Z",
            release_commit=RELEASE_COMMIT,
            source_tree_sha256=SOURCE_HASH,
            slice_id="truthful-ready-to-earn-v1",
            preceding_evaluation=first_evaluation,
        )
        rollout.validate_preregistration(ready)
        self.assertEqual(ready["status"], "ready_for_approval")
        self.assertEqual(
            ready["preceding_slice_evaluation"]["slice_id"],
            "qicswu-metric-v1",
        )

    def test_immutable_writer_is_idempotent_and_rejects_replacement(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = str(Path(directory) / "artifact.json")
            rollout._write_json({"value": 1}, output)
            rollout._write_json({"value": 1}, output)
            with self.assertRaisesRegex(rollout.RolloutControlError, "overwrite"):
                rollout._write_json({"value": 2}, output)

    def test_two_exact_windows_pass_all_registered_requirements(self) -> None:
        prereg = preregistration()
        auth = authorization(prereg)
        result = rollout.evaluate_rollout(
            prereg,
            auth,
            daily_snapshots(prereg, auth),
            evaluated_at="2026-10-28T00:00:00Z",
        )
        self.assertEqual(result["status"], "passed")
        self.assertTrue(result["completion_eligible"])
        self.assertEqual(
            [row["qualified_root_work_units"] for row in result["windows"]],
            [10, 10],
        )
        self.assertEqual(
            result["combined_diversity"]["independent_funding_principals"], 5
        )
        self.assertEqual(result["combined_diversity"]["solver_principals"], 5)
        self.assertGreaterEqual(
            result["combined_diversity"]["repeat_funding_principals"], 2
        )

    def test_concentrated_funding_fails_even_when_volume_passes(self) -> None:
        prereg = preregistration()
        auth = authorization(prereg)
        snapshots = daily_snapshots(prereg, auth)
        for snapshot in snapshots:
            if not snapshot["qualified_units"]:
                continue
            snapshot["qualified_units"][0]["independent_funding_by_principal"][0][
                "principal_id"
            ] = "principal:dominant"
            snapshot.update(rollout._seal(snapshot))
        result = rollout.evaluate_rollout(
            prereg,
            auth,
            snapshots,
            evaluated_at="2026-10-28T00:00:00Z",
        )
        self.assertEqual(result["status"], "failed")
        self.assertIn("funding_count_concentration_exceeded", result["reason_codes"])
        self.assertIn("funding_gmv_concentration_exceeded", result["reason_codes"])


if __name__ == "__main__":
    unittest.main()
