#!/usr/bin/env python3
"""Fail-closed preregistration and production-window controls for QICSWU rollout.

This module does not query production, deploy software, authorize a release, or
change the QICSWU metric.  It binds an existing metric result to one frozen
deployment timestamp, builds auditable daily snapshots from later metric
results, and evaluates the two registered 28-day production windows.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import tempfile
from collections import Counter, defaultdict
from copy import deepcopy
from datetime import datetime, timedelta, timezone
from decimal import Decimal
from pathlib import Path
from typing import Any, Iterable

try:
    from scripts import qicswu_metric as metric
except ImportError:  # Direct invocation adds scripts/, rather than the repo, to sys.path.
    import qicswu_metric as metric


PREREGISTRATION_SCHEMA = "agent-bounties/qicswu-rollout-preregistration-v1"
AUTHORIZATION_SCHEMA = "agent-bounties/qicswu-deployment-authorization-v1"
MONITORING_SCHEMA = "agent-bounties/qicswu-daily-monitoring-input-v1"
DAILY_SNAPSHOT_SCHEMA = "agent-bounties/qicswu-daily-production-snapshot-v1"
EVALUATION_SCHEMA = "agent-bounties/qicswu-rollout-evaluation-v1"
RELEASE_SCHEMA = "agent-bounties/release-package-v1"

REQUIRED_SLICE_ORDER = (
    "qicswu-metric-v1",
    "truthful-ready-to-earn-v1",
    "pinned-regression-post-fund-handoff-v1",
)
SLICE_APPROVAL_ROLES = {
    "qicswu-metric-v1": (
        "metric_data_owner",
        "protocol_payment_reviewer",
        "security_privacy_reviewer",
        "maintainer",
    ),
    "truthful-ready-to-earn-v1": (
        "api_owner",
        "protocol_owner",
        "agent_experience_owner",
        "maintainer",
    ),
    "pinned-regression-post-fund-handoff-v1": (
        "mcp_api_owner",
        "verifier_security_owner",
        "frontend_owner",
        "payment_protocol_reviewer",
        "maintainer",
    ),
}
DISPUTE_KINDS = (
    "duplicate",
    "identity",
    "reimbursement",
    "finality",
    "payout_reconciliation",
)

PARTICIPATION_PATH_BY_PROTOCOL = {
    "autonomous": "exclusive_claim",
    "open_competition_v1": "open_competition",
    "open_competition_v2": "open_competition",
}
SHA256_RE = __import__("re").compile(r"^sha256:[0-9a-f]{64}$")
COMMIT_RE = __import__("re").compile(r"^[0-9a-f]{40}$")
HEX32_RE = __import__("re").compile(r"^0x[0-9a-f]{64}$")


class RolloutControlError(ValueError):
    """A rollout artifact is malformed or violates a frozen decision rule."""


def canonical_json_bytes(value: Any) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        allow_nan=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def sha256_json(value: Any) -> str:
    return "sha256:" + hashlib.sha256(canonical_json_bytes(value)).hexdigest()


def load_json(path: str | Path) -> Any:
    with Path(path).open("r", encoding="utf-8") as handle:
        return json.load(handle)


def _write_json(value: Any, path: str | None) -> None:
    rendered = json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    if not path:
        sys.stdout.write(rendered)
        return
    destination = Path(path)
    encoded = rendered.encode("utf-8")
    if destination.exists():
        _require(
            destination.read_bytes() == encoded,
            f"refusing to overwrite non-identical immutable artifact: {destination}",
        )
        return
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary: str | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="wb", dir=destination.parent, prefix=f".{destination.name}.", delete=False
        ) as handle:
            temporary = handle.name
            handle.write(encoded)
            handle.flush()
            os.fsync(handle.fileno())
        try:
            os.link(temporary, destination)
        except FileExistsError:
            _require(
                destination.read_bytes() == encoded,
                f"refusing to overwrite non-identical immutable artifact: {destination}",
            )
    finally:
        if temporary is not None:
            Path(temporary).unlink(missing_ok=True)


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise RolloutControlError(message)


def _parse_utc(value: Any, label: str) -> datetime:
    _require(isinstance(value, str), f"{label} must be an RFC3339 UTC string")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise RolloutControlError(f"{label} is not valid RFC3339") from error
    _require(
        parsed.tzinfo is not None and parsed.utcoffset() == timedelta(0),
        f"{label} must use UTC",
    )
    return parsed.astimezone(timezone.utc)


def _utc(value: datetime) -> str:
    return value.astimezone(timezone.utc).isoformat().replace("+00:00", "Z")


def _utc_midnight(value: datetime) -> datetime:
    value = value.astimezone(timezone.utc)
    return value.replace(hour=0, minute=0, second=0, microsecond=0)


def _whole_nonnegative(value: Any, label: str) -> int:
    _require(
        isinstance(value, int) and not isinstance(value, bool) and value >= 0,
        f"{label} must be a nonnegative integer",
    )
    return value


def _positive(value: Any, label: str) -> int:
    parsed = _whole_nonnegative(value, label)
    _require(parsed > 0, f"{label} must be positive")
    return parsed


def _decimal(value: Any, label: str) -> Decimal:
    _require(isinstance(value, (str, int)) and not isinstance(value, bool), f"{label} must be decimal text or an integer")
    try:
        parsed = Decimal(str(value))
    except Exception as error:
        raise RolloutControlError(f"{label} is not a decimal") from error
    _require(parsed.is_finite() and parsed >= 0, f"{label} must be finite and nonnegative")
    return parsed


def _seal(document: dict[str, Any]) -> dict[str, Any]:
    result = deepcopy(document)
    result.pop("artifact_hash", None)
    result["artifact_hash"] = sha256_json(result)
    return result


def _verify_seal(document: dict[str, Any], label: str) -> None:
    actual = document.get("artifact_hash")
    _require(isinstance(actual, str) and SHA256_RE.fullmatch(actual) is not None, f"{label}.artifact_hash is invalid")
    body = deepcopy(document)
    body.pop("artifact_hash", None)
    _require(actual == sha256_json(body), f"{label}.artifact_hash does not match the artifact body")


def _verify_metric_result(result: dict[str, Any], expected_days: int) -> None:
    _require(isinstance(result, dict), "metric result must be an object")
    _require(result.get("schema_version") == metric.RESULT_SCHEMA, "metric result schema is not QICSWU v1")
    body = deepcopy(result)
    actual_hash = body.pop("result_hash", None)
    _require(isinstance(actual_hash, str) and actual_hash == metric.sha256_json(body), "metric result_hash does not match its body")
    window = result.get("window")
    _require(isinstance(window, dict), "metric result window is missing")
    start = _parse_utc(window.get("started_at"), "metric.window.started_at")
    end = _parse_utc(window.get("ended_at"), "metric.window.ended_at")
    _require(start == _utc_midnight(start) and end == _utc_midnight(end), "metric result must use complete UTC-day boundaries")
    _require(end - start == timedelta(days=expected_days), f"metric result must cover exactly {expected_days} complete UTC days")
    _require(window.get("complete_utc_days") == expected_days, "metric result complete_utc_days is inconsistent")
    _require(window.get("boundary") == "[started_at,ended_at)", "metric result window boundary must be half-open")
    _require(result.get("status") in {"available", "unavailable"}, "metric result status is invalid")


def _slice_order(manifest: dict[str, Any]) -> tuple[str, ...]:
    _require(manifest.get("schema_version") == RELEASE_SCHEMA, "release manifest schema is invalid")
    rows = manifest.get("slices")
    _require(isinstance(rows, list), "release manifest slices must be an array")
    ordered = sorted(rows, key=lambda row: row.get("order", 0))
    result = tuple(row.get("id") for row in ordered)
    _require(result == REQUIRED_SLICE_ORDER, "release manifest slice order differs from the approved experiment order")
    return result


def build_preregistration(
    manifest: dict[str, Any],
    baseline: dict[str, Any],
    *,
    deployment_at: str,
    registered_at: str,
    release_commit: str,
    source_tree_sha256: str,
    slice_id: str,
    payout_floor_base_units: str | None = None,
    preceding_evaluation: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Build a sealed preregistration or an explicit blocked preflight record."""

    order = _slice_order(manifest)
    _require(slice_id in order, "slice_id is not part of the frozen release")
    _require(COMMIT_RE.fullmatch(release_commit) is not None, "release_commit must be a full lowercase Git SHA")
    _require(SHA256_RE.fullmatch(source_tree_sha256) is not None, "source_tree_sha256 is invalid")
    _verify_metric_result(baseline, 28)

    deploy = _parse_utc(deployment_at, "deployment_at")
    registered = _parse_utc(registered_at, "registered_at")
    baseline_end = _utc_midnight(deploy)
    baseline_start = baseline_end - timedelta(days=28)
    first_exposure_day = baseline_end if deploy == baseline_end else baseline_end + timedelta(days=1)
    window = baseline["window"]
    observed_start = _parse_utc(window["started_at"], "baseline.window.started_at")
    observed_end = _parse_utc(window["ended_at"], "baseline.window.ended_at")

    blockers: list[str] = []
    if registered >= deploy:
        blockers.append("registration_not_before_deployment")
    if observed_start != baseline_start or observed_end != baseline_end:
        blockers.append("baseline_not_preceding_deployment")
    if baseline.get("status") != "available":
        blockers.append("baseline_metric_unavailable")

    north_star = baseline.get("north_star") if isinstance(baseline.get("north_star"), dict) else {}
    secondary = baseline.get("secondary_metrics") if isinstance(baseline.get("secondary_metrics"), dict) else {}
    baseline_count = north_star.get("total_root_work_units")
    baseline_gmv = secondary.get("qualifying_settled_gmv_base_units")
    baseline_median = secondary.get("median_qualifying_solver_payout_base_units")
    target_count: int | None = None
    gmv_floor: int | None = None
    payout_floor: str | None = None
    payout_floor_source: str | None = None
    if baseline.get("status") == "available":
        try:
            baseline_count = _whole_nonnegative(baseline_count, "baseline count")
            baseline_gmv = _whole_nonnegative(baseline_gmv, "baseline qualifying GMV")
            target_count = max(10, 2 * baseline_count)
            gmv_floor = baseline_gmv
            if payout_floor_base_units is not None:
                payout_floor = str(_decimal(payout_floor_base_units, "payout floor"))
                payout_floor_source = "explicit_preregistered_floor"
            elif baseline_median is not None:
                payout_floor = str(_decimal(baseline_median, "baseline median payout"))
                payout_floor_source = "available_baseline_median_non_regression_floor"
            else:
                payout_floor_source = None
                blockers.append("payout_size_guardrail_not_defined")
        except RolloutControlError:
            blockers.append("available_baseline_metrics_invalid")
    elif payout_floor_base_units is not None:
        _decimal(payout_floor_base_units, "payout floor")
        payout_floor_source = "explicit_preregistered_floor"
    else:
        payout_floor_source = None

    release_north_star = manifest.get("north_star")
    if not isinstance(release_north_star, dict) or release_north_star.get("policy_id") != baseline.get("policy_id"):
        blockers.append("release_and_baseline_policy_mismatch")

    slice_index = order.index(slice_id)
    preceding_summary: dict[str, Any] | None = None
    if slice_index == 0:
        if preceding_evaluation is not None:
            blockers.append("unexpected_preceding_slice_evaluation")
    elif preceding_evaluation is None:
        blockers.append("preceding_slice_effect_not_evaluated")
    else:
        try:
            _require(preceding_evaluation.get("schema_version") == EVALUATION_SCHEMA, "preceding evaluation schema is invalid")
            _verify_seal(preceding_evaluation, "preceding evaluation")
            expected_preceding_slice = order[slice_index - 1]
            _require(preceding_evaluation.get("slice_id") == expected_preceding_slice, "preceding evaluation is for the wrong slice")
            _require(preceding_evaluation.get("status") == "passed" and preceding_evaluation.get("completion_eligible") is True, "preceding slice did not pass its registered evaluation")
            preceding_summary = {
                "slice_id": expected_preceding_slice,
                "rollout_id": preceding_evaluation.get("rollout_id"),
                "evaluation_hash": preceding_evaluation.get("artifact_hash"),
                "status": "passed",
            }
        except RolloutControlError:
            blockers.append("preceding_slice_effect_not_evaluated")

    status = "blocked" if blockers else "ready_for_approval"
    document = {
        "schema_version": PREREGISTRATION_SCHEMA,
        "rollout_id": f"{manifest.get('release_id')}:{slice_id}:{deployment_at}",
        "status": status,
        "registered_at": registered_at,
        "deployment": {
            "frozen_at": deployment_at,
            "first_complete_exposure_day": first_exposure_day.date().isoformat(),
            "release_id": manifest.get("release_id"),
            "release_commit": release_commit,
            "source_tree_sha256": source_tree_sha256,
            "slice_id": slice_id,
            "slice_order": slice_index + 1,
        },
        "baseline": {
            "window": deepcopy(window),
            "status": baseline.get("status"),
            "result_hash": baseline.get("result_hash"),
            "policy_id": baseline.get("policy_id"),
            "policy_hash": baseline.get("policy_hash"),
            "total_root_work_units": baseline_count if baseline.get("status") == "available" else None,
            "qualifying_settled_gmv_base_units": baseline_gmv if baseline.get("status") == "available" else None,
            "median_qualifying_solver_payout_base_units": baseline_median if baseline.get("status") == "available" else None,
        },
        "registered_decision": {
            "target_count_per_28_day_window": target_count,
            "target_method": "provisional_ambition_max_10_or_twice_baseline_count",
            "target_is_statistically_derived": False,
            "qualifying_settled_gmv_floor_base_units_per_28_day_window": gmv_floor,
            "median_qualifying_solver_payout_floor_base_units": payout_floor,
            "median_payout_floor_source": payout_floor_source,
            "guardrail_method": "non_regression_floor_equal_to_available_baseline_unless_explicit_payout_floor_is_registered",
            "rationale": "Seek a material count increase while preventing count inflation through lower aggregate qualified GMV or smaller typical solver payouts.",
        },
        "experiment_order": list(order),
        "preceding_slice_evaluation": preceding_summary,
        "evaluation_contract": {
            "window_days": 28,
            "required_consecutive_non_overlapping_windows": 2,
            "minimum_independent_funding_principals_across_windows": 5,
            "minimum_solver_principals_across_windows": 5,
            "minimum_repeat_funding_principals_across_windows": 2,
            "repeat_funder_definition": "principal independently funds at least two distinct counted root work units across the combined windows",
            "maximum_funding_principal_share_of_count": "0.500000",
            "maximum_funding_principal_share_of_gmv": "0.500000",
            "count_attribution": "fractional_by_independent_reward_principal_funding_within_each_counted_root",
            "gmv_attribution": "unit_settled_gmv_fractional_by_independent_reward_principal_funding_within_each_counted_root",
            "required_unresolved_disputes": 0,
            "required_direct_settlement_proof_for_every_unit": True,
            "required_completed_eligibility_record_for_every_unit": True,
        },
        "approval_boundary": {
            "production_deployment_authorized": False,
            "authorization_must_reference_this_artifact_hash": True,
            "required_roles": list(SLICE_APPROVAL_ROLES[slice_id]),
            "external_outreach_authorized": False,
            "incentive_or_spend_authorized": False,
        },
        "blockers": sorted(set(blockers)),
        "evidence_boundary": "This artifact pre-registers a decision but never authorizes deployment, outreach, incentives, spend, funding, verification, or settlement. A blocked artifact cannot be used for deployment.",
    }
    return _seal(document)


def validate_preregistration(document: dict[str, Any], *, require_ready: bool = True) -> None:
    _require(document.get("schema_version") == PREREGISTRATION_SCHEMA, "preregistration schema is invalid")
    _verify_seal(document, "preregistration")
    _require(document.get("experiment_order") == list(REQUIRED_SLICE_ORDER), "preregistration experiment order drifted")
    deployment = document.get("deployment")
    baseline = document.get("baseline")
    decision = document.get("registered_decision")
    contract = document.get("evaluation_contract")
    approval = document.get("approval_boundary")
    _require(all(isinstance(row, dict) for row in (deployment, baseline, decision, contract, approval)), "preregistration sections are missing")
    _parse_utc(deployment.get("frozen_at"), "preregistration.deployment.frozen_at")
    deploy_at = _parse_utc(deployment.get("frozen_at"), "preregistration.deployment.frozen_at")
    registered_at = _parse_utc(document.get("registered_at"), "preregistration.registered_at")
    baseline_end = _utc_midnight(deploy_at)
    baseline_start = baseline_end - timedelta(days=28)
    baseline_window = baseline.get("window")
    _require(isinstance(baseline_window, dict), "preregistration baseline window is missing")
    _require(
        _parse_utc(baseline_window.get("started_at"), "preregistration baseline start") == baseline_start
        and _parse_utc(baseline_window.get("ended_at"), "preregistration baseline end") == baseline_end,
        "preregistration baseline is not the preceding 28 complete UTC days",
    )
    expected_first_day = baseline_end if deploy_at == baseline_end else baseline_end + timedelta(days=1)
    _require(deployment.get("first_complete_exposure_day") == expected_first_day.date().isoformat(), "first complete exposure day is inconsistent")
    _require(COMMIT_RE.fullmatch(str(deployment.get("release_commit"))) is not None, "preregistration release commit is invalid")
    _require(SHA256_RE.fullmatch(str(deployment.get("source_tree_sha256"))) is not None, "preregistration source hash is invalid")
    _require(approval.get("production_deployment_authorized") is False, "preregistration must not self-authorize production")
    _require(contract.get("window_days") == 28 and contract.get("required_consecutive_non_overlapping_windows") == 2, "evaluation window contract drifted")
    _require(contract.get("required_unresolved_disputes") == 0, "dispute guardrail drifted")
    if require_ready:
        _require(document.get("status") == "ready_for_approval", "preregistration is not ready for approval")
        _require(document.get("blockers") == [], "ready preregistration has blockers")
        _require(registered_at < deploy_at, "preregistration was not frozen before deployment")
        _require(baseline.get("status") == "available", "ready preregistration baseline is unavailable")
        baseline_count = _whole_nonnegative(baseline.get("total_root_work_units"), "baseline count")
        target_count = _positive(decision.get("target_count_per_28_day_window"), "registered target count")
        _require(target_count == max(10, 2 * baseline_count), "registered target does not use the frozen provisional formula")
        baseline_gmv = _whole_nonnegative(baseline.get("qualifying_settled_gmv_base_units"), "baseline GMV")
        _require(decision.get("qualifying_settled_gmv_floor_base_units_per_28_day_window") == baseline_gmv, "registered GMV floor does not equal the available baseline")
        payout_floor = _decimal(decision.get("median_qualifying_solver_payout_floor_base_units"), "registered payout floor")
        payout_source = decision.get("median_payout_floor_source")
        _require(payout_source in {"explicit_preregistered_floor", "available_baseline_median_non_regression_floor"}, "registered payout floor source is invalid")
        if payout_source == "available_baseline_median_non_regression_floor":
            _require(payout_floor == _decimal(baseline.get("median_qualifying_solver_payout_base_units"), "baseline median payout"), "registered payout floor does not equal the available baseline median")
        slice_order = deployment.get("slice_order")
        _require(slice_order in {1, 2, 3}, "registered slice order is invalid")
        preceding = document.get("preceding_slice_evaluation")
        if slice_order == 1:
            _require(preceding is None, "first slice must not depend on a preceding evaluation")
        else:
            _require(isinstance(preceding, dict), "later slice lacks a preceding evaluation")
            _require(preceding.get("slice_id") == REQUIRED_SLICE_ORDER[slice_order - 2], "preceding slice evaluation order drifted")
            _require(preceding.get("status") == "passed", "preceding slice evaluation did not pass")
            _require(SHA256_RE.fullmatch(str(preceding.get("evaluation_hash"))) is not None, "preceding slice evaluation hash is invalid")


def validate_authorization(authorization: dict[str, Any], preregistration: dict[str, Any]) -> None:
    validate_preregistration(preregistration)
    _require(authorization.get("schema_version") == AUTHORIZATION_SCHEMA, "authorization schema is invalid")
    _verify_seal(authorization, "authorization")
    _require(authorization.get("preregistration_hash") == preregistration.get("artifact_hash"), "authorization does not bind the preregistration")
    deployment = preregistration["deployment"]
    _require(authorization.get("deployment_at") == deployment.get("frozen_at"), "authorization deployment timestamp drifted")
    _require(authorization.get("release_commit") == deployment.get("release_commit"), "authorization release commit drifted")
    _require(authorization.get("slice_id") == deployment.get("slice_id"), "authorization slice drifted")
    _require(authorization.get("scope") == "production_deployment_of_exact_registered_slice_only", "authorization scope is invalid")
    approvals = authorization.get("approvals")
    _require(isinstance(approvals, list), "authorization approvals must be an array")
    by_role = {row.get("role"): row for row in approvals if isinstance(row, dict)}
    _require(len(by_role) == len(approvals), "authorization approval roles must be unique objects")
    deploy_at = _parse_utc(deployment["frozen_at"], "deployment.frozen_at")
    for role in preregistration["approval_boundary"]["required_roles"]:
        row = by_role.get(role)
        _require(isinstance(row, dict), f"authorization is missing {role} approval")
        _require(isinstance(row.get("approved_by"), str) and row["approved_by"].strip(), f"{role} approved_by is missing")
        approved_at = _parse_utc(row.get("approved_at"), f"{role}.approved_at")
        _require(approved_at <= deploy_at, f"{role} approval occurs after the frozen deployment timestamp")
        _require(row.get("decision") == "approved", f"{role} decision is not approved")
    _require(authorization.get("external_outreach_authorized") is False, "deployment authorization must not silently authorize outreach")
    _require(authorization.get("incentive_or_spend_authorized") is False, "deployment authorization must not silently authorize incentives or spend")


def _validate_monitoring(monitoring: dict[str, Any]) -> None:
    _require(monitoring.get("schema_version") == MONITORING_SCHEMA, "monitoring input schema is invalid")
    transitions = monitoring.get("funnel_transitions")
    _require(isinstance(transitions, list) and transitions, "funnel_transitions must be a non-empty array")
    for index, row in enumerate(transitions):
        _require(isinstance(row, dict), f"funnel transition {index} must be an object")
        _require(isinstance(row.get("name"), str) and row["name"], f"funnel transition {index} name is missing")
        numerator = _whole_nonnegative(row.get("numerator_count"), f"funnel transition {index} numerator")
        denominator = _whole_nonnegative(row.get("denominator_count"), f"funnel transition {index} denominator")
        _require(numerator <= denominator, f"funnel transition {index} numerator exceeds denominator")
        _require(row.get("join_coverage_status") in {"complete", "partial", "unavailable"}, f"funnel transition {index} join coverage status is invalid")
        source_refs = row.get("source_refs")
        _require(isinstance(source_refs, list) and source_refs and all(isinstance(ref, str) and ref for ref in source_refs), f"funnel transition {index} source_refs must be a non-empty string array")
    for section in ("canonical_settlement_proofs", "payment_correctness", "inventory_correctness", "safety_incidents"):
        _require(isinstance(monitoring.get(section), dict), f"monitoring.{section} is missing")
    for section in ("canonical_settlement_proofs", "payment_correctness", "inventory_correctness"):
        _require(isinstance(monitoring[section].get("exceptions"), list), f"monitoring.{section}.exceptions must be an array")
    _whole_nonnegative(monitoring["canonical_settlement_proofs"].get("observed_count"), "canonical proof observed_count")
    _whole_nonnegative(monitoring["canonical_settlement_proofs"].get("direct_proof_count"), "canonical proof direct_proof_count")
    _whole_nonnegative(monitoring["payment_correctness"].get("reconciled_count"), "payment reconciled_count")
    _whole_nonnegative(monitoring["payment_correctness"].get("disputed_count"), "payment disputed_count")
    _whole_nonnegative(monitoring["inventory_correctness"].get("ready_to_earn_count"), "ready_to_earn_count")
    _require(monitoring["inventory_correctness"].get("status") in {"correct", "incorrect", "unavailable"}, "inventory correctness status is invalid")
    incidents = monitoring["safety_incidents"].get("incidents")
    _require(isinstance(incidents, list), "safety incidents must be an array")
    open_count = sum(isinstance(row, dict) and row.get("status") == "open" for row in incidents)
    _require(monitoring["safety_incidents"].get("open_count") == open_count, "safety open_count does not match incidents")
    disputes = monitoring.get("unresolved_disputes")
    _require(isinstance(disputes, dict) and set(disputes) == set(DISPUTE_KINDS), "unresolved dispute categories are incomplete")
    for kind in DISPUTE_KINDS:
        _require(isinstance(disputes[kind], list), f"unresolved {kind} disputes must be an array")


def _qualified_units(result: dict[str, Any]) -> list[dict[str, Any]] | None:
    if result.get("status") != "available":
        return None
    ledger = result.get("ledger")
    _require(isinstance(ledger, list), "available metric result must include a ledger")
    units = []
    for row in ledger:
        if not isinstance(row, dict) or not row.get("counted"):
            continue
        _require(row.get("status") == "eligible" and row.get("reason_codes") == [], "counted ledger row is not a completed eligibility record")
        for key in ("root_work_id", "candidate_key", "canonical_event_identity", "solver_principal_id"):
            _require(isinstance(row.get(key), str) and row[key], f"counted ledger row is missing {key}")
        protocol = row.get("protocol")
        _require(protocol in PARTICIPATION_PATH_BY_PROTOCOL, "counted ledger row has an unsupported protocol")
        transaction_hash = str(row.get("transaction_hash", "")).lower()
        block_hash = str(row.get("block_hash", "")).lower()
        _require(HEX32_RE.fullmatch(transaction_hash) is not None, "counted unit transaction hash is invalid")
        _require(HEX32_RE.fullmatch(block_hash) is not None, "counted unit block hash is invalid")
        funding = row.get("independent_funding_by_principal")
        _require(isinstance(funding, list) and funding, "counted unit has no independent funding principal")
        normalized_funding = []
        for contribution in funding:
            _require(isinstance(contribution, dict), "independent funding contribution is malformed")
            principal_id = contribution.get("principal_id")
            _require(isinstance(principal_id, str) and principal_id, "independent funding principal is missing")
            normalized_funding.append({"principal_id": principal_id, "amount_base_units": _positive(contribution.get("amount_base_units"), "independent funding amount")})
        units.append(
            {
                "root_work_id": row["root_work_id"],
                "candidate_key": row["candidate_key"],
                "protocol": protocol,
                "participation_path": PARTICIPATION_PATH_BY_PROTOCOL[protocol],
                "eligibility_status": "complete",
                "eligibility_result_hash": result["result_hash"],
                "direct_settlement_proof": {
                    "status": "complete",
                    "canonical_event_identity": row["canonical_event_identity"],
                    "transaction_hash": transaction_hash,
                    "log_index": _whole_nonnegative(row.get("log_index"), "settlement log_index"),
                    "block_number": _positive(row.get("block_number"), "settlement block_number"),
                    "block_hash": block_hash,
                },
                "solver_principal_id": row["solver_principal_id"],
                "independent_funding_by_principal": normalized_funding,
                "solver_payout_base_units": _positive(row.get("solver_payout_base_units"), "solver payout"),
                "settled_gmv_base_units": _positive(row.get("settled_gmv_base_units"), "settled GMV"),
                "unresolved_disputes": [],
            }
        )
    _require(len(units) == result["north_star"].get("total_root_work_units"), "qualified-unit extraction does not match metric numerator")
    return units


def build_daily_snapshot(
    preregistration: dict[str, Any],
    authorization: dict[str, Any],
    result: dict[str, Any],
    monitoring: dict[str, Any],
    *,
    captured_at: str,
) -> dict[str, Any]:
    validate_authorization(authorization, preregistration)
    _verify_metric_result(result, 1)
    _validate_monitoring(monitoring)
    _require(result.get("policy_id") == preregistration["baseline"].get("policy_id"), "daily metric policy id drifted")
    _require(result.get("policy_hash") == preregistration["baseline"].get("policy_hash"), "daily metric policy hash drifted")
    captured = _parse_utc(captured_at, "captured_at")
    start = _parse_utc(result["window"]["started_at"], "metric.window.started_at")
    end = _parse_utc(result["window"]["ended_at"], "metric.window.ended_at")
    _require(captured >= end, "daily snapshot cannot be captured before the UTC day closes")
    first_day = datetime.fromisoformat(preregistration["deployment"]["first_complete_exposure_day"]).replace(tzinfo=timezone.utc)
    _require(start >= first_day, "daily snapshot predates the registered complete-exposure period")
    units = _qualified_units(result)
    document = {
        "schema_version": DAILY_SNAPSHOT_SCHEMA,
        "rollout_id": preregistration["rollout_id"],
        "slice_id": preregistration["deployment"]["slice_id"],
        "day": start.date().isoformat(),
        "captured_at": captured_at,
        "preregistration_hash": preregistration["artifact_hash"],
        "authorization_hash": authorization["artifact_hash"],
        "metric": {
            "status": result["status"],
            "policy_id": result["policy_id"],
            "policy_hash": result["policy_hash"],
            "input_hash": result["input_hash"],
            "result_hash": result["result_hash"],
            "unavailable_reason_codes": deepcopy(result.get("unavailable_reason_codes", [])),
            "total_root_work_units": result["north_star"].get("total_root_work_units"),
            "qualifying_settled_gmv_base_units": result["secondary_metrics"].get("qualifying_settled_gmv_base_units"),
            "median_qualifying_solver_payout_base_units": result["secondary_metrics"].get("median_qualifying_solver_payout_base_units"),
        },
        "qualified_units": units,
        "monitoring": deepcopy(monitoring),
        "evidence_boundary": "Only qualified_units copied from an available sealed QICSWU result count. Monitoring observations do not create eligibility or payment evidence.",
    }
    return _seal(document)


def validate_daily_snapshot(snapshot: dict[str, Any], preregistration: dict[str, Any], authorization: dict[str, Any]) -> None:
    validate_authorization(authorization, preregistration)
    _require(snapshot.get("schema_version") == DAILY_SNAPSHOT_SCHEMA, "daily snapshot schema is invalid")
    _verify_seal(snapshot, "daily snapshot")
    _require(snapshot.get("rollout_id") == preregistration.get("rollout_id"), "daily snapshot rollout binding drifted")
    _require(snapshot.get("preregistration_hash") == preregistration.get("artifact_hash"), "daily snapshot preregistration hash drifted")
    _require(snapshot.get("authorization_hash") == authorization.get("artifact_hash"), "daily snapshot authorization hash drifted")
    captured_at = _parse_utc(snapshot.get("captured_at"), "daily snapshot captured_at")
    try:
        snapshot_day = datetime.fromisoformat(str(snapshot.get("day"))).date()
    except ValueError as error:
        raise RolloutControlError("daily snapshot day is invalid") from error
    _require(str(snapshot_day) == snapshot.get("day"), "daily snapshot day must be ISO calendar text")
    _require(captured_at >= datetime.combine(snapshot_day + timedelta(days=1), datetime.min.time(), tzinfo=timezone.utc), "daily snapshot was captured before its UTC day closed")
    metric_summary = snapshot.get("metric")
    _require(isinstance(metric_summary, dict), "daily snapshot metric summary is missing")
    _require(metric_summary.get("policy_id") == preregistration["baseline"].get("policy_id"), "daily snapshot policy id drifted")
    _require(metric_summary.get("policy_hash") == preregistration["baseline"].get("policy_hash"), "daily snapshot policy hash drifted")
    units = snapshot.get("qualified_units")
    if metric_summary.get("status") == "available":
        _require(isinstance(units, list), "available daily snapshot must include qualified_units")
        _require(len(units) == metric_summary.get("total_root_work_units"), "daily snapshot unit count does not match metric total")
        for unit in units:
            _validate_snapshot_unit(unit, metric_summary.get("result_hash"))
    else:
        _require(metric_summary.get("status") == "unavailable" and units is None, "unavailable daily snapshot must keep qualified_units null")
    _validate_monitoring(snapshot.get("monitoring", {}))


def _validate_snapshot_unit(unit: Any, result_hash: Any) -> None:
    _require(isinstance(unit, dict), "qualified unit must be an object")
    for key in ("root_work_id", "candidate_key", "solver_principal_id"):
        _require(isinstance(unit.get(key), str) and unit[key], f"qualified unit is missing {key}")
    protocol = unit.get("protocol")
    _require(protocol in PARTICIPATION_PATH_BY_PROTOCOL, "qualified unit protocol is invalid")
    _require(
        unit.get("participation_path") == PARTICIPATION_PATH_BY_PROTOCOL[protocol],
        "qualified unit participation path does not match its protocol",
    )
    _require(unit.get("eligibility_status") == "complete", "qualified unit eligibility is incomplete")
    _require(unit.get("eligibility_result_hash") == result_hash, "qualified unit eligibility result binding drifted")
    _require(unit.get("unresolved_disputes") == [], "qualified unit has an unresolved dispute")
    proof = unit.get("direct_settlement_proof")
    _require(isinstance(proof, dict) and proof.get("status") == "complete", "qualified unit direct settlement proof is incomplete")
    _require(isinstance(proof.get("canonical_event_identity"), str) and proof["canonical_event_identity"], "qualified unit canonical event identity is missing")
    _require(HEX32_RE.fullmatch(str(proof.get("transaction_hash", "")).lower()) is not None, "qualified unit transaction hash is invalid")
    _require(HEX32_RE.fullmatch(str(proof.get("block_hash", "")).lower()) is not None, "qualified unit block hash is invalid")
    _whole_nonnegative(proof.get("log_index"), "qualified unit settlement log index")
    _positive(proof.get("block_number"), "qualified unit settlement block number")
    _positive(unit.get("solver_payout_base_units"), "qualified unit solver payout")
    _positive(unit.get("settled_gmv_base_units"), "qualified unit settled GMV")
    funding = unit.get("independent_funding_by_principal")
    _require(isinstance(funding, list) and funding, "qualified unit has no independent funding")
    for contribution in funding:
        _require(isinstance(contribution, dict), "qualified unit funding contribution is malformed")
        _require(isinstance(contribution.get("principal_id"), str) and contribution["principal_id"], "qualified unit funding principal is missing")
        _positive(contribution.get("amount_base_units"), "qualified unit funding amount")


def _median(values: Iterable[int]) -> Decimal | None:
    ordered = sorted(values)
    if not ordered:
        return None
    middle = len(ordered) // 2
    if len(ordered) % 2:
        return Decimal(ordered[middle])
    return (Decimal(ordered[middle - 1]) + Decimal(ordered[middle])) / Decimal(2)


def _window_result(units: list[dict[str, Any]], decision: dict[str, Any], label: str) -> dict[str, Any]:
    reasons: list[str] = []
    count = len(units)
    gmv = sum(unit["settled_gmv_base_units"] for unit in units)
    payout_median = _median(unit["solver_payout_base_units"] for unit in units)
    protocol_counts = Counter(unit["protocol"] for unit in units)
    participation_path_counts = Counter(unit["participation_path"] for unit in units)
    if count < decision["target_count_per_28_day_window"]:
        reasons.append("target_count_not_met")
    if gmv < decision["qualifying_settled_gmv_floor_base_units_per_28_day_window"]:
        reasons.append("qualified_gmv_guardrail_not_met")
    payout_floor = _decimal(decision["median_qualifying_solver_payout_floor_base_units"], "registered payout floor")
    if payout_median is None or payout_median < payout_floor:
        reasons.append("median_solver_payout_guardrail_not_met")
    return {
        "label": label,
        "status": "passed" if not reasons else "failed",
        "qualified_root_work_units": count,
        "qualified_root_work_units_by_protocol": dict(sorted(protocol_counts.items())),
        "qualified_root_work_units_by_participation_path": dict(
            sorted(participation_path_counts.items())
        ),
        "qualifying_settled_gmv_base_units": gmv,
        "median_qualifying_solver_payout_base_units": str(payout_median) if payout_median is not None else None,
        "reason_codes": reasons,
    }


def evaluate_rollout(
    preregistration: dict[str, Any],
    authorization: dict[str, Any],
    snapshots: list[dict[str, Any]],
    *,
    evaluated_at: str,
) -> dict[str, Any]:
    validate_authorization(authorization, preregistration)
    _parse_utc(evaluated_at, "evaluated_at")
    _require(len(snapshots) == 56, "exactly 56 daily snapshots are required")
    for snapshot in snapshots:
        validate_daily_snapshot(snapshot, preregistration, authorization)
    ordered = sorted(snapshots, key=lambda row: row["day"])
    first_day = datetime.fromisoformat(preregistration["deployment"]["first_complete_exposure_day"]).date()
    expected_days = [(first_day + timedelta(days=index)).isoformat() for index in range(56)]
    _require([row["day"] for row in ordered] == expected_days, "daily snapshots are not the exact registered 56 consecutive UTC days")

    reasons: set[str] = set()
    units_by_window: list[list[dict[str, Any]]] = [[], []]
    roots: set[str] = set()
    proofs: set[str] = set()
    for index, snapshot in enumerate(ordered):
        if snapshot["metric"]["status"] != "available":
            reasons.add("daily_metric_unavailable")
            continue
        monitoring = snapshot["monitoring"]
        unit_count = len(snapshot["qualified_units"])
        if monitoring["canonical_settlement_proofs"]["direct_proof_count"] != unit_count:
            reasons.add("canonical_settlement_proof_monitor_mismatch")
        if monitoring["canonical_settlement_proofs"]["observed_count"] < unit_count:
            reasons.add("canonical_settlement_proof_observation_incomplete")
        if monitoring["payment_correctness"]["reconciled_count"] != unit_count:
            reasons.add("payment_reconciliation_monitor_mismatch")
        if any(monitoring["unresolved_disputes"][kind] for kind in DISPUTE_KINDS):
            reasons.add("unresolved_dispute")
        if monitoring["payment_correctness"]["disputed_count"]:
            reasons.add("payment_correctness_dispute")
        if monitoring["canonical_settlement_proofs"]["exceptions"]:
            reasons.add("canonical_settlement_proof_exception")
        if monitoring["payment_correctness"]["exceptions"]:
            reasons.add("payment_correctness_exception")
        if monitoring["inventory_correctness"]["status"] != "correct":
            reasons.add("inventory_correctness_not_proven")
        if monitoring["inventory_correctness"]["exceptions"]:
            reasons.add("inventory_correctness_exception")
        if any(row["join_coverage_status"] != "complete" for row in monitoring["funnel_transitions"]):
            reasons.add("funnel_transition_coverage_incomplete")
        if monitoring["safety_incidents"]["open_count"]:
            reasons.add("open_safety_incident")
        for unit in snapshot["qualified_units"]:
            root = unit["root_work_id"]
            proof = unit["direct_settlement_proof"]["canonical_event_identity"]
            if root in roots:
                reasons.add("duplicate_root_work_unit")
            if proof in proofs:
                reasons.add("duplicate_canonical_settlement_proof")
            roots.add(root)
            proofs.add(proof)
            if unit["eligibility_status"] != "complete" or unit["unresolved_disputes"]:
                reasons.add("eligibility_record_incomplete")
            if unit["direct_settlement_proof"]["status"] != "complete":
                reasons.add("direct_settlement_proof_incomplete")
            units_by_window[index // 28].append(unit)

    decision = preregistration["registered_decision"]
    windows = [
        _window_result(units_by_window[0], decision, "window_1"),
        _window_result(units_by_window[1], decision, "window_2"),
    ]
    for window in windows:
        reasons.update(window["reason_codes"])

    funder_unit_counts: Counter[str] = Counter()
    funders: set[str] = set()
    solvers: set[str] = set()
    count_credit: defaultdict[str, Decimal] = defaultdict(Decimal)
    gmv_credit: defaultdict[str, Decimal] = defaultdict(Decimal)
    total_gmv = 0
    all_units = units_by_window[0] + units_by_window[1]
    for unit in all_units:
        solvers.add(unit["solver_principal_id"])
        contributions = unit["independent_funding_by_principal"]
        total_funding = sum(row["amount_base_units"] for row in contributions)
        _require(total_funding > 0, "counted unit independent funding total is not positive")
        for contribution in contributions:
            principal = contribution["principal_id"]
            funders.add(principal)
            funder_unit_counts[principal] += 1
            fraction = Decimal(contribution["amount_base_units"]) / Decimal(total_funding)
            count_credit[principal] += fraction
            gmv_credit[principal] += fraction * Decimal(unit["settled_gmv_base_units"])
        total_gmv += unit["settled_gmv_base_units"]
    repeat_funders = {principal for principal, count in funder_unit_counts.items() if count >= 2}
    total_count = len(all_units)
    largest_count_share = max(count_credit.values(), default=Decimal(0)) / Decimal(total_count) if total_count else Decimal(0)
    largest_gmv_share = max(gmv_credit.values(), default=Decimal(0)) / Decimal(total_gmv) if total_gmv else Decimal(0)
    contract = preregistration["evaluation_contract"]
    if len(funders) < contract["minimum_independent_funding_principals_across_windows"]:
        reasons.add("insufficient_independent_funding_principals")
    if len(solvers) < contract["minimum_solver_principals_across_windows"]:
        reasons.add("insufficient_solver_principals")
    if len(repeat_funders) < contract["minimum_repeat_funding_principals_across_windows"]:
        reasons.add("insufficient_repeat_funding_principals")
    if largest_count_share > _decimal(contract["maximum_funding_principal_share_of_count"], "maximum count concentration"):
        reasons.add("funding_count_concentration_exceeded")
    if largest_gmv_share > _decimal(contract["maximum_funding_principal_share_of_gmv"], "maximum GMV concentration"):
        reasons.add("funding_gmv_concentration_exceeded")

    document = {
        "schema_version": EVALUATION_SCHEMA,
        "rollout_id": preregistration["rollout_id"],
        "slice_id": preregistration["deployment"]["slice_id"],
        "evaluated_at": evaluated_at,
        "preregistration_hash": preregistration["artifact_hash"],
        "authorization_hash": authorization["artifact_hash"],
        "status": "passed" if not reasons else "failed",
        "windows": windows,
        "combined_diversity": {
            "independent_funding_principals": len(funders),
            "solver_principals": len(solvers),
            "repeat_funding_principals": len(repeat_funders),
            "largest_funding_principal_share_of_count": f"{largest_count_share:.6f}",
            "largest_funding_principal_share_of_gmv": f"{largest_gmv_share:.6f}",
        },
        "reason_codes": sorted(reasons),
        "completion_eligible": not reasons,
        "evidence_boundary": "Passing evaluates only the exact registered slice and frozen 56-day sequence. It cannot authorize another slice, outreach, incentives, spend, funding, verification, or settlement.",
    }
    return _seal(document)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    prereg = subparsers.add_parser("preregister", help="build a sealed preregistration or blocked preflight")
    prereg.add_argument("--manifest", required=True)
    prereg.add_argument("--baseline", required=True)
    prereg.add_argument("--deployment-at", required=True)
    prereg.add_argument("--registered-at", required=True)
    prereg.add_argument("--release-commit", required=True)
    prereg.add_argument("--source-tree-sha256", required=True)
    prereg.add_argument("--slice-id", required=True, choices=REQUIRED_SLICE_ORDER)
    prereg.add_argument("--payout-floor-base-units")
    prereg.add_argument("--preceding-evaluation")
    prereg.add_argument("--output")

    validate = subparsers.add_parser("validate-preregistration")
    validate.add_argument("--input", required=True)

    daily = subparsers.add_parser("daily-snapshot")
    daily.add_argument("--preregistration", required=True)
    daily.add_argument("--authorization", required=True)
    daily.add_argument("--metric-result", required=True)
    daily.add_argument("--monitoring", required=True)
    daily.add_argument("--captured-at", required=True)
    daily.add_argument("--output")

    evaluate = subparsers.add_parser("evaluate")
    evaluate.add_argument("--preregistration", required=True)
    evaluate.add_argument("--authorization", required=True)
    evaluate.add_argument("--snapshot", action="append", required=True)
    evaluate.add_argument("--evaluated-at", required=True)
    evaluate.add_argument("--output")

    args = parser.parse_args(argv)
    try:
        if args.command == "preregister":
            result = build_preregistration(
                load_json(args.manifest),
                load_json(args.baseline),
                deployment_at=args.deployment_at,
                registered_at=args.registered_at,
                release_commit=args.release_commit,
                source_tree_sha256=args.source_tree_sha256,
                slice_id=args.slice_id,
                payout_floor_base_units=args.payout_floor_base_units,
                preceding_evaluation=(
                    load_json(args.preceding_evaluation)
                    if args.preceding_evaluation
                    else None
                ),
            )
            _write_json(result, args.output)
            return 0 if result["status"] == "ready_for_approval" else 3
        if args.command == "validate-preregistration":
            validate_preregistration(load_json(args.input))
            return 0
        if args.command == "daily-snapshot":
            result = build_daily_snapshot(
                load_json(args.preregistration),
                load_json(args.authorization),
                load_json(args.metric_result),
                load_json(args.monitoring),
                captured_at=args.captured_at,
            )
            _write_json(result, args.output)
            return 0
        if args.command == "evaluate":
            result = evaluate_rollout(
                load_json(args.preregistration),
                load_json(args.authorization),
                [load_json(path) for path in args.snapshot],
                evaluated_at=args.evaluated_at,
            )
            _write_json(result, args.output)
            return 0 if result["status"] == "passed" else 4
        raise RolloutControlError("unknown command")
    except (OSError, json.JSONDecodeError, RolloutControlError) as error:
        sys.stderr.write(f"QICSWU rollout control failed closed: {error}\n")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
