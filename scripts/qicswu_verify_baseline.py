#!/usr/bin/env python3
"""Verify the frozen QICSWU baseline package and its retained evidence anchors."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit

try:
    from scripts import qicswu_metric as metric
except ImportError:  # Direct invocation adds scripts/, rather than the repo, to sys.path.
    import qicswu_metric as metric


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_INPUT = ROOT / "ops/metrics/qicswu-baseline-input-2026-08-04-2026-09-01-v1.json"
DEFAULT_POLICY = ROOT / "ops/metrics/qicswu-policy-v1.json"
DEFAULT_PRINCIPALS = ROOT / "ops/metrics/qicswu-principal-registry-v1.json"
DEFAULT_ROOT_MAP = ROOT / "ops/metrics/qicswu-root-work-map-v1.json"
DEFAULT_SOURCE = ROOT / "ops/metrics/qicswu-baseline-source-observations-v1.json"
DEFAULT_RESULT = ROOT / "ops/metrics/qicswu-baseline-result-2026-08-04-2026-09-01-v1.json"
NETWORK = "base-mainnet"
CHAIN_ID = 8453
HISTORICAL_FACTORY_CREATION_TOPIC = (
    "0xb6d089829e0ec8f7303d462b985fafd142515173bc8a1e150edac9e27005e583"
)


class BaselineVerificationError(ValueError):
    pass


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise BaselineVerificationError(message)


def _sha256_file(path: Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def _normalized_https_authority(value: Any) -> str | None:
    parsed = urlsplit(value if isinstance(value, str) else "")
    hostname = parsed.hostname.lower().rstrip(".") if parsed.hostname else None
    try:
        port = parsed.port
    except ValueError:
        return None
    if (
        parsed.scheme.lower() != "https"
        or not hostname
        or parsed.username is not None
        or parsed.password is not None
    ):
        return None
    # A different port on the same host does not establish independently
    # controlled infrastructure. Provider-organization independence remains
    # blocked pending the approved registry; this retained check is only a
    # minimum anti-aliasing control.
    return hostname


def _parse_utc(value: Any, label: str) -> datetime:
    _require(isinstance(value, str), f"{label} must be an RFC3339 UTC string")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise BaselineVerificationError(f"{label} is not valid RFC3339") from error
    _require(
        parsed.tzinfo is not None and parsed.utcoffset() == timezone.utc.utcoffset(parsed),
        f"{label} must use UTC",
    )
    return parsed.astimezone(timezone.utc)


def _retained_artifact_path(repo_root: Path, relative: Any, label: str) -> Path:
    _require(isinstance(relative, str) and relative, f"{label} is missing")
    root = repo_root.resolve()
    path = (root / relative).resolve()
    _require(path.is_relative_to(root), f"{label} must remain inside the repository")
    _require(path.is_file(), f"{label} does not exist: {relative}")
    return path


def _raw_events(document: Any, protocol: str) -> list[dict[str, Any]]:
    rows = (
        document
        if isinstance(document, list)
        else document.get("events")
        if isinstance(document, dict)
        else None
    )
    _require(isinstance(rows, list), f"retained response for {protocol} has no event array")
    _require(
        all(isinstance(row, dict) for row in rows),
        f"retained response for {protocol} contains a non-object event",
    )
    return rows


def _gmv(row: dict[str, Any]) -> int:
    data = row["data"]
    return sum(
        int(data.get(field, 0))
        for field in (
            "solver_reward",
            "verifier_reward",
            "keeper_reward",
            "timeout_bond_bonus",
        )
    )


def _snapshot_identity(row: dict[str, Any]) -> tuple[str, str, int]:
    return "open_competition_v2", row["transaction_hash"], int(row["log_index"])


def _public_candidate(
    stream: dict[str, Any], row: dict[str, Any], stream_index: int, row_index: int
) -> dict[str, Any]:
    protocol = stream["protocol"]
    data = row["data"]
    solver_reward = int(data["solver_reward"])
    completion_bonus = int(data.get("timeout_bond_bonus", 0))
    return {
        "network": NETWORK,
        "chain_id": CHAIN_ID,
        "protocol": protocol,
        "protocol_version": stream["protocol_version"],
        "factory_contract": stream["factory_contract"],
        "event_kind": row["kind"],
        "bounty_contract": row["contract_address"],
        "bounty_id": row["bounty_id"],
        "round": data.get("round") if protocol == "autonomous" else None,
        "transaction_hash": row["tx_hash"],
        "log_index": int(row["log_index"]),
        "block_number": int(row["block_number"]),
        "block_hash": None,
        "occurred_at": row["occurred_at"],
        "solver_wallet": data["solver"],
        "solver_payout_base_units": solver_reward + completion_bonus,
        "promised_reward_principal_base_units": solver_reward,
        "settled_gmv_base_units": _gmv(row),
        "participation_event": None,
        "funding_events": [],
        "funding_reconciled_to_contract": False,
        "canonicality": {
            "finality_status": None,
            "receipt_status": None,
            "settlement_event_matches_receipt": None,
            "contract_state_reconciled": None,
            "removed": None,
            "reorg_detected": None,
            "rpc_block_observations": [],
            "raw_evidence_refs": [
                "ops/metrics/qicswu-baseline-source-observations-v1.json"
                f"#public_event_stream_observations[{stream_index}]"
                f".selected_settlement_rows[{row_index}]"
            ],
        },
    }


def _historical_candidate(
    row: dict[str, Any], row_index: int, block_hash: str, factory: str
) -> dict[str, Any]:
    occurred_at = datetime.fromtimestamp(
        int(row["settled_at"]), tz=timezone.utc
    ).isoformat().replace("+00:00", "Z")
    return {
        "network": NETWORK,
        "chain_id": CHAIN_ID,
        "protocol": "open_competition_v2",
        "protocol_version": "agent-bounties/open-competition-v2-beta3",
        "factory_contract": factory,
        "event_kind": "competition_settled",
        "bounty_contract": row["bounty_contract"],
        "bounty_id": row["bounty_id"],
        "round": None,
        "transaction_hash": row["transaction_hash"],
        "log_index": int(row["log_index"]),
        "block_number": int(row["block_number"]),
        "block_hash": block_hash,
        "occurred_at": occurred_at,
        "solver_wallet": row["solver"],
        "solver_payout_base_units": None,
        "promised_reward_principal_base_units": None,
        "settled_gmv_base_units": int(row["gmv_base_units"]),
        "participation_event": None,
        "funding_events": [],
        "funding_reconciled_to_contract": False,
        "canonicality": {
            "finality_status": "safe",
            "receipt_status": None,
            "settlement_event_matches_receipt": True,
            "contract_state_reconciled": None,
            "removed": False,
            "reorg_detected": False,
            "rpc_block_observations": [
                {
                    "provider_id": "primary_rpc",
                    "block_number": int(row["block_number"]),
                    "block_hash": block_hash,
                },
                {
                    "provider_id": "shadow_rpc",
                    "block_number": int(row["block_number"]),
                    "block_hash": block_hash,
                },
            ],
            "raw_evidence_refs": [
                "ops/metrics/qicswu-baseline-source-observations-v1.json"
                "#historical_factory_reconciliation"
                f".historical_factory_settlement_rows_retained_in_snapshot[{row_index}]",
                "site/generated/gmv-snapshots/external-gmv-sprint-20260820-v1.json",
            ],
        },
    }


def _rpc_quantity(value: Any, label: str) -> int:
    _require(isinstance(value, str) and value.startswith("0x"), f"{label} is invalid")
    try:
        return int(value, 16)
    except ValueError as error:
        raise BaselineVerificationError(f"{label} is invalid") from error


def _topic_address(value: Any, label: str) -> str:
    _require(
        isinstance(value, str) and len(value) == 66 and value.startswith("0x"),
        f"{label} is invalid",
    )
    _require(value[2:26] == "0" * 24, f"{label} is not an encoded address")
    return "0x" + value[-40:].lower()


def _verify_historical_factory_membership(
    *, repo_root: Path, historical: dict[str, Any], snapshot: dict[str, Any]
) -> set[tuple[str, str]]:
    evidence_path = _retained_artifact_path(
        repo_root,
        historical.get("factory_membership_evidence_path"),
        "historical factory membership evidence",
    )
    _require(
        _sha256_file(evidence_path)
        == historical.get("factory_membership_evidence_file_sha256"),
        "historical factory membership evidence file hash mismatch",
    )
    evidence = metric.load_json(evidence_path)
    factory = historical["historical_factory_contract"]
    _require(
        evidence.get("schema_version")
        == "agent-bounties/qicswu-historical-factory-membership-evidence-v1",
        "unexpected historical factory membership evidence schema",
    )
    _require(
        evidence.get("network") == NETWORK
        and evidence.get("chain_id") == CHAIN_ID
        and evidence.get("factory_contract") == factory,
        "historical factory membership evidence scope mismatch",
    )
    _require(
        evidence.get("event_topic0") == HISTORICAL_FACTORY_CREATION_TOPIC,
        "historical factory creation topic mismatch",
    )
    _require(
        evidence.get("snapshot_path") == historical["retained_snapshot_path"]
        and evidence.get("snapshot_file_sha256")
        == historical["retained_snapshot_file_sha256"],
        "historical factory membership evidence references a different snapshot",
    )
    _require(
        evidence.get("snapshot_epoch_end_safe_block")
        == snapshot["campaign"]["end_safe_block"],
        "historical factory membership evidence safe-block mismatch",
    )

    observations = evidence.get("provider_observations")
    _require(
        isinstance(observations, list) and len(observations) >= 2,
        "historical factory membership requires at least two provider observations",
    )
    provider_ids: set[str] = set()
    provider_authorities: set[str] = set()
    encoded_logs: bytes | None = None
    creation_pairs: set[tuple[str, str]] = set()
    creation_records: dict[tuple[str, str], dict[str, Any]] = {}
    deployment_block = evidence.get("factory_deployment_block")
    safe_block = evidence.get("snapshot_epoch_end_safe_block")
    _require(
        isinstance(deployment_block, int)
        and not isinstance(deployment_block, bool)
        and isinstance(safe_block, int)
        and not isinstance(safe_block, bool)
        and deployment_block <= safe_block,
        "historical factory query block range is invalid",
    )
    _require(
        evidence.get("query_boundary")
        == "[factory_deployment_block,snapshot_epoch_end_safe_block]",
        "historical factory query boundary is invalid",
    )
    for observation in observations:
        _require(isinstance(observation, dict), "invalid factory provider observation")
        provider_id = observation.get("provider_id")
        _require(
            isinstance(provider_id, str) and provider_id and provider_id not in provider_ids,
            "historical factory provider IDs must be non-empty and unique",
        )
        provider_ids.add(provider_id)
        endpoint = observation.get("endpoint")
        authority = _normalized_https_authority(endpoint)
        _require(
            authority is not None and authority not in provider_authorities,
            "historical factory provider endpoint authorities must be valid and unique",
        )
        assert authority is not None
        provider_authorities.add(authority)
        logs = observation.get("logs")
        _require(isinstance(logs, list), "historical factory provider logs are missing")
        _require(
            observation.get("log_count") == len(logs)
            and observation.get("canonical_logs_sha256") == metric.sha256_json(logs),
            f"historical factory provider log anchor mismatch for {provider_id}",
        )
        encoded = metric.canonical_json_bytes(logs)
        if encoded_logs is None:
            encoded_logs = encoded
        else:
            _require(encoded == encoded_logs, "historical factory providers disagree")
        observed_pairs: set[tuple[str, str]] = set()
        observed_records: dict[tuple[str, str], dict[str, Any]] = {}
        for log in logs:
            _require(isinstance(log, dict), "historical factory log is not an object")
            topics = log.get("topics")
            _require(
                isinstance(topics, list)
                and len(topics) >= 3
                and topics[0] == HISTORICAL_FACTORY_CREATION_TOPIC,
                "historical factory log topic mismatch",
            )
            _require(
                str(log.get("address", "")).lower() == factory
                and log.get("removed") is False,
                "historical factory log address/removal mismatch",
            )
            bounty_id = str(topics[1]).lower()
            _require(
                metric._normalize_hex32(bounty_id) is not None,
                "historical factory bounty ID is invalid",
            )
            bounty_contract = _topic_address(topics[2], "historical factory bounty contract")
            block_number = _rpc_quantity(
                log.get("blockNumber"), "historical factory block number"
            )
            log_index = _rpc_quantity(
                log.get("logIndex"), "historical factory log index"
            )
            block_hash = metric._normalize_hex32(log.get("blockHash"))
            transaction_hash = metric._normalize_hex32(log.get("transactionHash"))
            _require(
                deployment_block <= block_number <= safe_block
                and block_hash is not None
                and transaction_hash is not None,
                "historical factory log identity or block range is invalid",
            )
            pair = (bounty_id, bounty_contract)
            _require(
                pair not in observed_records,
                "historical factory provider returned a duplicate creation pair",
            )
            observed_pairs.add(pair)
            observed_records[pair] = {
                "bounty_contract": bounty_contract,
                "bounty_id": bounty_id,
                "factory_creation_block_hash": block_hash,
                "factory_creation_block_number": block_number,
                "factory_creation_log_index": log_index,
                "factory_creation_transaction_hash": transaction_hash,
            }
        if not creation_pairs:
            creation_pairs = observed_pairs
            creation_records = observed_records
        else:
            _require(observed_pairs == creation_pairs, "historical factory pair sets disagree")
            _require(
                metric.canonical_json_bytes(
                    [observed_records[pair] for pair in sorted(observed_records)]
                )
                == metric.canonical_json_bytes(
                    [creation_records[pair] for pair in sorted(creation_records)]
                ),
                "historical factory provider creation identities disagree",
            )

    memberships = evidence.get("snapshot_v2_factory_membership")
    _require(isinstance(memberships, list), "historical snapshot membership list is missing")
    membership_pairs = {
        (str(row.get("bounty_id", "")).lower(), str(row.get("bounty_contract", "")).lower())
        for row in memberships
        if isinstance(row, dict)
    }
    snapshot_pairs = {
        (str(row["bounty_id"]).lower(), str(row["bounty_contract"]).lower())
        for row in snapshot["settlements"]
        if row.get("protocol") == "open_competition_v2"
    }
    _require(
        evidence.get("snapshot_v2_row_count") == len(snapshot_pairs)
        and len(memberships) == len(snapshot_pairs)
        and membership_pairs == snapshot_pairs
        and snapshot_pairs.issubset(creation_pairs),
        "historical V2 snapshot rows are not all proven factory members",
    )
    membership_by_pair: dict[tuple[str, str], dict[str, Any]] = {}
    for row in memberships:
        _require(isinstance(row, dict), "historical membership row is invalid")
        pair = (
            str(row.get("bounty_id", "")).lower(),
            str(row.get("bounty_contract", "")).lower(),
        )
        _require(
            pair not in membership_by_pair
            and pair in creation_records
            and metric.canonical_json_bytes(row)
            == metric.canonical_json_bytes(creation_records[pair]),
            "historical membership row does not match its retained creation log",
        )
        membership_by_pair[pair] = row
    snapshot_by_pair = {
        (str(row["bounty_id"]).lower(), str(row["bounty_contract"]).lower()): row
        for row in snapshot["settlements"]
        if row.get("protocol") == "open_competition_v2"
    }
    for pair in snapshot_pairs:
        creation_position = (
            metric._nonnegative_int(
                membership_by_pair[pair].get("factory_creation_block_number")
            ),
            metric._nonnegative_int(
                membership_by_pair[pair].get("factory_creation_log_index")
            ),
        )
        settlement_position = (
            metric._nonnegative_int(snapshot_by_pair[pair].get("block_number")),
            metric._nonnegative_int(snapshot_by_pair[pair].get("log_index")),
        )
        _require(
            None not in creation_position
            and None not in settlement_position
            and creation_position < settlement_position,
            "historical factory creation must precede its retained settlement",
        )
    return membership_pairs


def verify_package(
    *,
    repo_root: Path,
    input_path: Path,
    policy_path: Path,
    principals_path: Path,
    root_map_path: Path,
    source_path: Path,
    result_path: Path,
) -> dict[str, Any]:
    input_document = metric.load_json(input_path)
    policy = metric.load_json(policy_path)
    principals = metric.load_json(principals_path)
    root_map = metric.load_json(root_map_path)
    source = metric.load_json(source_path)
    expected_result = metric.load_json(result_path)

    _require(
        source.get("schema_version")
        == "agent-bounties/qicswu-baseline-source-observations-v1",
        "unexpected source observation schema",
    )
    _require(
        input_document["source_reconciliation"]["source_observations_file_sha256"]
        == _sha256_file(source_path),
        "source observation file hash does not match frozen input",
    )
    package_frozen_at = _parse_utc(input_document.get("frozen_at"), "input.frozen_at")
    _require(
        package_frozen_at >= _parse_utc(policy.get("effective_at"), "policy.effective_at")
        and package_frozen_at
        >= _parse_utc(
            source.get("evidence_package_extended_at"),
            "source.evidence_package_extended_at",
        ),
        "baseline package freeze predates its policy or retained evidence",
    )
    selection_window = source.get("selection_window", {})
    _require(
        selection_window.get("boundary") == "[started_at,ended_at)",
        "source selection window must use a half-open boundary",
    )
    _require(
        selection_window.get("started_at") == input_document["window"]["started_at"]
        and selection_window.get("ended_at") == input_document["window"]["ended_at"],
        "source selection window differs from the frozen evaluation window",
    )
    selection_start = _parse_utc(selection_window.get("started_at"), "selection_window.started_at")
    selection_end = _parse_utc(selection_window.get("ended_at"), "selection_window.ended_at")

    historical = source["historical_factory_reconciliation"]
    snapshot_path = _retained_artifact_path(
        repo_root,
        historical.get("retained_snapshot_path"),
        "retained historical-factory snapshot",
    )
    _require(
        historical["retained_snapshot_file_sha256"] == _sha256_file(snapshot_path),
        "historical-factory snapshot hash mismatch",
    )
    snapshot = metric.load_json(snapshot_path)
    snapshot_old_v2 = [
        row for row in snapshot["settlements"] if row["protocol"] == "open_competition_v2"
    ]
    _require(
        metric.canonical_json_bytes(snapshot_old_v2)
        == metric.canonical_json_bytes(
            historical["historical_factory_settlement_rows_retained_in_snapshot"]
        ),
        "historical-factory rows differ from the retained snapshot",
    )
    _verify_historical_factory_membership(
        repo_root=repo_root, historical=historical, snapshot=snapshot
    )

    settlement_kind = {
        "autonomous": "bounty_settled",
        "open_competition_v1": "bounty_settled",
        "open_competition_v2": "competition_settled",
    }
    public_rows: list[tuple[str, dict[str, Any]]] = []
    expected_candidates: list[dict[str, Any]] = []
    for stream_index, stream in enumerate(source["public_event_stream_observations"]):
        protocol = stream.get("protocol")
        _require(protocol in settlement_kind, f"unexpected public stream protocol: {protocol}")
        _require(
            stream.get("full_response_retained") is True,
            f"full public response not retained for {protocol}",
        )
        raw_path = _retained_artifact_path(
            repo_root,
            stream.get("full_response_artifact"),
            f"full_response_artifact for {protocol}",
        )
        raw_document = metric.load_json(raw_path)
        _require(
            metric.sha256_json(raw_document)
            == stream.get("response_canonical_json_sha256_at_capture"),
            f"retained public response hash mismatch for {protocol}",
        )
        raw_rows = _raw_events(raw_document, protocol)
        if isinstance(raw_document, dict):
            _require(
                raw_document.get("protocol_version") == stream.get("protocol_version")
                and raw_document.get("factory_contract") == stream.get("factory_contract")
                and raw_document.get("network") == NETWORK,
                f"retained public response scope mismatch for {protocol}",
            )
        _require(
            len(raw_rows) == stream.get("full_response_event_count_at_capture"),
            f"retained public response event count mismatch for {protocol}",
        )
        derived_rows = [
            row
            for row in raw_rows
            if row.get("kind") == settlement_kind[protocol]
            and selection_start
            <= _parse_utc(row.get("occurred_at"), f"{protocol} settlement occurred_at")
            < selection_end
        ]
        _require(
            stream.get("selected_settlement_rows_retained_verbatim") is True,
            f"selected settlement rows not retained for {protocol}",
        )
        _require(
            metric.canonical_json_bytes(derived_rows)
            == metric.canonical_json_bytes(stream["selected_settlement_rows"]),
            f"selected settlement rows are not reproducible from retained response for {protocol}",
        )
        public_rows.extend(
            (protocol, row) for row in derived_rows
        )
        expected_candidates.extend(
            _public_candidate(stream, row, stream_index, row_index)
            for row_index, row in enumerate(derived_rows)
        )
    public_count = len(public_rows)
    public_gmv = sum(_gmv(row) for _, row in public_rows)
    reconciliation = source["reconciliation"]
    _require(public_count == 21, f"expected 21 public-stream settlements, got {public_count}")
    _require(public_gmv == 21_265_000, f"expected 21265000 public-stream GMV, got {public_gmv}")
    _require(
        reconciliation["complete_utc_window_public_stream_settlements"] == public_count
        and reconciliation["complete_utc_window_public_stream_gmv_base_units"] == public_gmv,
        "public-stream reconciliation summary mismatch",
    )

    historical_rows = historical["historical_factory_settlement_rows_retained_in_snapshot"]
    canary_rows = historical["declared_canary_rows_excluded_by_existing_platform_metric"]
    recovered_rows = historical["aggregate_gap_recovery_rows"]
    _require(len(historical_rows) == 7, "expected seven retained historical-factory settlements")
    _require(len(canary_rows) == 2, "expected two retained declared canary settlements")
    _require(len(recovered_rows) == 5, "expected five historical-factory gap rows")
    excluded_contracts = {
        str(contract).lower() for contract in policy.get("excluded_bounty_contracts", [])
    }
    expected_canary_rows = [
        row
        for row in historical_rows
        if str(row.get("bounty_contract", "")).lower() in excluded_contracts
    ]
    expected_recovered_rows = [
        row
        for row in historical_rows
        if str(row.get("bounty_contract", "")).lower() not in excluded_contracts
    ]
    _require(
        metric.canonical_json_bytes(canary_rows)
        == metric.canonical_json_bytes(expected_canary_rows)
        and metric.canonical_json_bytes(recovered_rows)
        == metric.canonical_json_bytes(expected_recovered_rows),
        "historical canary and recovery rows are not the exact policy partition",
    )
    recovered_gmv = sum(int(row["gmv_base_units"]) for row in recovered_rows)
    _require(recovered_gmv == 15_200_000, "historical-factory gap must equal 15.2 USDC")
    _require(
        public_count + len(recovered_rows) == 26
        and public_gmv + recovered_gmv == 36_465_000,
        "historical factory does not reconcile the 26 / 36.465 aggregate",
    )
    _require(
        reconciliation["count_and_amount_match"] is True
        and reconciliation["reconciled_settlement_count"] == 26
        and reconciliation["reconciled_gmv_base_units"] == 36_465_000,
        "frozen aggregate reconciliation is not exact",
    )

    canonical_block_hashes = {
        (row["protocol"], row["transaction_hash"], int(row["log_index"])): row[
            "block_hash"
        ]
        for row in snapshot["canonical_evidence"]
    }
    for row_index, row in enumerate(historical_rows):
        identity = _snapshot_identity(row)
        _require(identity in canonical_block_hashes, "historical block evidence is missing")
        expected_candidates.append(
            _historical_candidate(
                row,
                row_index,
                canonical_block_hashes[identity],
                historical["historical_factory_contract"],
            )
        )

    v2_policy = next(
        row
        for row in policy["canonical_settlement_protocols"]
        if row["protocol"] == "open_competition_v2"
    )
    _require(
        {
            historical["historical_factory_contract"],
            historical["current_public_factory_contract"],
        }.issubset(set(v2_policy["accepted_factory_contracts"])),
        "policy does not retain both historical and current V2 factory scopes",
    )

    candidates = input_document.get("candidates")
    _require(isinstance(candidates, list), "evaluation candidates must be an array")
    candidate_identities = [
        (
            row["network"],
            int(row["chain_id"]),
            row["transaction_hash"],
            int(row["log_index"]),
        )
        for row in candidates
    ]
    _require(len(expected_candidates) == 28, "retained raw candidate set must be 28")
    _require(
        len(candidates) == len(expected_candidates)
        and len(candidate_identities) == len(set(candidate_identities)),
        "evaluation candidate cardinality or canonical identity uniqueness mismatch",
    )
    _require(
        metric.canonical_json_bytes(candidates)
        == metric.canonical_json_bytes(expected_candidates),
        "evaluation candidates are not exactly reproducible from retained records",
    )

    evaluated = metric.evaluate(input_document, policy, principals, root_map)
    _require(
        metric.canonical_json_bytes(evaluated)
        == metric.canonical_json_bytes(expected_result),
        "frozen result is not reproducible from frozen input and policy",
    )
    _require(evaluated["status"] == "unavailable", "baseline must remain unavailable")
    _require(
        evaluated["north_star"]["total_root_work_units"] is None,
        "unavailable baseline must not publish zero or a point estimate",
    )
    _require(
        evaluated["diagnostics"]["excluded_records"] is None
        and evaluated["diagnostics"]["unknown_records"] is None
        and evaluated["diagnostics"]["reason_code_counts"] is None
        and evaluated["diagnostics"]["input_candidate_records"] is None
        and evaluated["diagnostics"]["unique_canonical_event_identities"] is None
        and evaluated["diagnostics"]["ledger_records"] is None,
        "blocked publication must withhold every candidate cardinality",
    )
    _require(
        evaluated["ledger"] is None,
        "blocked publication must withhold the evaluator-derived ledger",
    )
    return {
        "status": "verified_unavailable",
        "raw_candidate_settlements": 28,
        "declared_canary_exclusions": 2,
        "aggregate_aligned_unresolved_candidates": 26,
        "aggregate_aligned_gmv_base_units": 36_465_000,
        "north_star": None,
        "result_hash": evaluated["result_hash"],
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, default=DEFAULT_INPUT)
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    parser.add_argument("--principals", type=Path, default=DEFAULT_PRINCIPALS)
    parser.add_argument("--root-map", type=Path, default=DEFAULT_ROOT_MAP)
    parser.add_argument("--source", type=Path, default=DEFAULT_SOURCE)
    parser.add_argument("--result", type=Path, default=DEFAULT_RESULT)
    args = parser.parse_args(argv)
    try:
        summary = verify_package(
            repo_root=ROOT,
            input_path=args.input,
            policy_path=args.policy,
            principals_path=args.principals,
            root_map_path=args.root_map,
            source_path=args.source,
            result_path=args.result,
        )
    except (BaselineVerificationError, metric.MetricContractError, OSError, KeyError) as error:
        sys.stderr.write(f"QICSWU baseline verification failed closed: {error}\n")
        return 1
    sys.stdout.write(json.dumps(summary, indent=2, sort_keys=True) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
