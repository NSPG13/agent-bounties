#!/usr/bin/env python3
"""Deterministic evaluator for policy-qualified independent settlements.

The evaluator is intentionally pure and fail closed.  It does not query RPCs,
infer beneficial ownership from wallet addresses, or turn missing evidence into
zero.  Acquisition and adjudication of evidence happen upstream; this module
only evaluates a frozen input against versioned policy and registries.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from collections import Counter, defaultdict
from copy import deepcopy
from datetime import datetime, timedelta, timezone
from decimal import Decimal, ROUND_HALF_UP
from pathlib import Path
from typing import Any, Iterable


INPUT_SCHEMA = "agent-bounties/qicswu-evaluation-input-v1"
POLICY_SCHEMA = "agent-bounties/qicswu-policy-v1"
PRINCIPAL_SCHEMA = "agent-bounties/qicswu-principal-registry-v1"
ROOT_MAP_SCHEMA = "agent-bounties/qicswu-root-work-map-v1"
RESULT_SCHEMA = "agent-bounties/qicswu-evaluation-result-v1"

HEX_32_RE = re.compile(r"^0x[0-9a-f]{64}$")
EVM_ADDRESS_RE = re.compile(r"^0x[0-9a-f]{40}$")


class MetricContractError(ValueError):
    """The caller supplied a structurally invalid versioned document."""


def canonical_json_bytes(value: Any) -> bytes:
    """Return the only JSON encoding used for policy, input, and result hashes."""

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


def _require_schema(document: dict[str, Any], expected: str, label: str) -> None:
    actual = document.get("schema_version")
    if actual != expected:
        raise MetricContractError(
            f"{label} schema_version must be {expected!r}, got {actual!r}"
        )


def _parse_utc(value: Any, label: str) -> datetime:
    if not isinstance(value, str):
        raise MetricContractError(f"{label} must be an RFC3339 UTC string")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise MetricContractError(f"{label} is not valid RFC3339: {value!r}") from error
    if parsed.tzinfo is None or parsed.utcoffset() != timedelta(0):
        raise MetricContractError(f"{label} must use UTC")
    return parsed.astimezone(timezone.utc)


def _normalize_address(value: Any) -> str | None:
    if not isinstance(value, str):
        return None
    normalized = value.lower()
    return normalized if EVM_ADDRESS_RE.fullmatch(normalized) else None


def _normalize_hex32(value: Any) -> str | None:
    if not isinstance(value, str):
        return None
    normalized = value.lower()
    return normalized if HEX_32_RE.fullmatch(normalized) else None


def _positive_int(value: Any) -> int | None:
    # bool is an int in Python and must not be accepted as money or a block.
    if isinstance(value, bool):
        return None
    if isinstance(value, int) and value > 0:
        return value
    if isinstance(value, str) and value.isdigit() and int(value) > 0:
        return int(value)
    return None


def _nonnegative_int(value: Any) -> int | None:
    if isinstance(value, bool):
        return None
    if isinstance(value, int) and value >= 0:
        return value
    if isinstance(value, str) and value.isdigit():
        return int(value)
    return None


def _round_matches(record_round: Any, candidate_round: Any) -> bool:
    """Compare protocol rounds without letting two malformed values match."""

    if candidate_round is None:
        return record_round is None
    normalized_candidate = _nonnegative_int(candidate_round)
    normalized_record = _nonnegative_int(record_round)
    return (
        normalized_candidate is not None
        and normalized_record is not None
        and normalized_record == normalized_candidate
    )


def _event_position(value: Any) -> tuple[int, int] | None:
    if not isinstance(value, dict):
        return None
    block = _nonnegative_int(value.get("block_number"))
    log_index = _nonnegative_int(value.get("log_index"))
    if block is None or log_index is None:
        return None
    return block, log_index


def _evidence_refs(value: Any) -> list[str] | None:
    """Accept only a non-empty array of non-empty evidence reference strings."""

    if not isinstance(value, list) or not value:
        return None
    if not all(isinstance(ref, str) and ref.strip() for ref in value):
        return None
    return value


def _view_payload_hash(view: dict[str, Any]) -> str:
    """Hash a decoded evidence view independently from its source references."""

    payload = {
        key: value
        for key, value in view.items()
        if key not in {"raw_evidence_refs", "view_payload_sha256"}
    }
    return sha256_json(payload)


def _candidate_key(candidate: dict[str, Any]) -> str | None:
    network = candidate.get("network")
    protocol = candidate.get("protocol")
    contract = _normalize_address(candidate.get("bounty_contract"))
    bounty_id = _normalize_hex32(candidate.get("bounty_id"))
    tx_hash = _normalize_hex32(candidate.get("transaction_hash"))
    log_index = _nonnegative_int(candidate.get("log_index"))
    if not all(isinstance(value, str) and value for value in (network, protocol)):
        return None
    if contract is None or bounty_id is None or tx_hash is None or log_index is None:
        return None
    return f"{network}:{protocol}:{contract}:{bounty_id}:{tx_hash}:{log_index}"


def _event_identity(candidate: dict[str, Any], ordinal: int) -> str:
    network = candidate.get("network")
    chain_id = _nonnegative_int(candidate.get("chain_id"))
    tx_hash = _normalize_hex32(candidate.get("transaction_hash"))
    log_index = _nonnegative_int(candidate.get("log_index"))
    if (
        isinstance(network, str)
        and network
        and chain_id is not None
        and tx_hash is not None
        and log_index is not None
    ):
        return f"{network}:{chain_id}:{tx_hash}:{log_index}"
    # Invalid records must remain distinct so two malformed rows cannot collapse
    # into an apparently trustworthy duplicate.
    return f"invalid:{ordinal}:{sha256_json(candidate)}"


def _funding_event_identity(funding: Any, ordinal: int) -> str:
    """Identify one canonical funding log inside a bounty/root candidate.

    Canonical EVM logs are unique by network, chain, transaction hash, and log
    index. Protocol labels are decoded attributes, not part of log identity.
    Malformed rows remain distinct so they cannot be collapsed into apparently
    valid evidence.
    """

    if isinstance(funding, dict):
        network = funding.get("network")
        chain_id = _nonnegative_int(funding.get("chain_id"))
        transaction_hash = _normalize_hex32(funding.get("transaction_hash"))
        log_index = _nonnegative_int(funding.get("log_index"))
        if (
            isinstance(network, str)
            and network
            and chain_id is not None
            and transaction_hash is not None
            and log_index is not None
        ):
            return f"{network}:{chain_id}:{transaction_hash}:{log_index}"
    return f"invalid:{ordinal}:{sha256_json(funding)}"


def _receipt_transfer_identity(receipt: Any, ordinal: int) -> str:
    if isinstance(receipt, dict):
        network = receipt.get("network")
        chain_id = _nonnegative_int(receipt.get("chain_id"))
        transaction_hash = _normalize_hex32(receipt.get("transaction_hash"))
        log_index = _nonnegative_int(receipt.get("transfer_log_index"))
        if (
            isinstance(network, str)
            and network
            and chain_id is not None
            and transaction_hash is not None
            and log_index is not None
        ):
            return f"{network}:{chain_id}:{transaction_hash}:{log_index}"
    return f"invalid-receipt:{ordinal}:{sha256_json(receipt)}"


def _global_canonical_log_conflicts(candidates: list[dict[str, Any]]) -> bool:
    """Reject contradictory reuse of one canonical log across work units."""

    observations: dict[str, set[tuple[str, str]]] = defaultdict(set)
    ordinal = 0

    def record(
        role: str,
        identity: str,
        value: Any,
        candidate: dict[str, Any],
        settlement_identity: str,
    ) -> None:
        scope = {
            "network": candidate.get("network"),
            "chain_id": candidate.get("chain_id"),
            "protocol": candidate.get("protocol"),
            "factory_contract": candidate.get("factory_contract"),
            "bounty_contract": candidate.get("bounty_contract"),
            "bounty_id": candidate.get("bounty_id"),
            "round": candidate.get("round"),
        }
        observations[identity].add(
            (
                role,
                sha256_json(
                    {
                        "outer_settlement_identity": settlement_identity,
                        "candidate_scope": scope,
                        "evidence": value,
                    }
                ),
            )
        )

    for candidate in candidates:
        settlement_identity = _event_identity(candidate, ordinal)
        record(
            "settlement",
            settlement_identity,
            candidate,
            candidate,
            settlement_identity,
        )
        ordinal += 1
        participation = candidate.get("participation_event")
        if isinstance(participation, dict):
            record(
                "participation",
                _funding_event_identity(participation, ordinal),
                participation,
                candidate,
                settlement_identity,
            )
            ordinal += 1
        funding_rows = candidate.get("funding_events")
        if isinstance(funding_rows, list):
            for funding in funding_rows:
                record(
                    "funding",
                    _funding_event_identity(funding, ordinal),
                    funding,
                    candidate,
                    settlement_identity,
                )
                ordinal += 1
        payout = candidate.get("solver_payout_reconciliation")
        receipt = payout.get("receipt_transfer") if isinstance(payout, dict) else None
        if isinstance(receipt, dict):
            record(
                "payout_transfer",
                _receipt_transfer_identity(receipt, ordinal),
                receipt,
                candidate,
                settlement_identity,
            )
            ordinal += 1
    return any(len(values) > 1 for values in observations.values())


def _deduplicate_funding_events(rows: list[Any]) -> tuple[list[Any], int, bool]:
    seen: dict[str, tuple[bytes, Any]] = {}
    order: list[str] = []
    conflicts: set[str] = set()
    duplicate_count = 0
    for ordinal, row in enumerate(rows):
        identity = _funding_event_identity(row, ordinal)
        encoded = canonical_json_bytes(row)
        prior = seen.get(identity)
        if prior is None:
            seen[identity] = (encoded, row)
            order.append(identity)
        elif prior[0] == encoded:
            duplicate_count += 1
        else:
            conflicts.add(identity)
    return (
        [seen[identity][1] for identity in order if identity not in conflicts],
        duplicate_count,
        bool(conflicts),
    )


def _window(input_document: dict[str, Any]) -> tuple[datetime, datetime, int]:
    window = input_document.get("window")
    if not isinstance(window, dict):
        raise MetricContractError("input.window must be an object")
    start = _parse_utc(window.get("started_at"), "window.started_at")
    end = _parse_utc(window.get("ended_at"), "window.ended_at")
    if start.time() != datetime.min.time() or end.time() != datetime.min.time():
        raise MetricContractError("metric windows must begin and end at 00:00:00 UTC")
    if end <= start:
        raise MetricContractError("window.ended_at must be after window.started_at")
    seconds = (end - start).total_seconds()
    if seconds % 86_400:
        raise MetricContractError("metric window must contain complete UTC days")
    days = int(seconds // 86_400)
    declared_days = window.get("complete_utc_days")
    if (
        isinstance(declared_days, bool)
        or not isinstance(declared_days, int)
        or declared_days <= 0
        or declared_days != days
    ):
        raise MetricContractError(
            f"window.complete_utc_days must be {days}, got {declared_days!r}"
        )
    if window.get("boundary") != "[started_at,ended_at)":
        raise MetricContractError("window.boundary must be '[started_at,ended_at)'")
    return start, end, days


def _protocol_policy(policy: dict[str, Any]) -> dict[str, dict[str, Any]]:
    rows = policy.get("canonical_settlement_protocols")
    if not isinstance(rows, list) or not rows:
        raise MetricContractError("policy.canonical_settlement_protocols must be non-empty")
    result: dict[str, dict[str, Any]] = {}
    for row in rows:
        if not isinstance(row, dict) or not isinstance(row.get("protocol"), str):
            raise MetricContractError("every protocol policy requires protocol")
        protocol = row["protocol"]
        if protocol in result:
            raise MetricContractError(f"duplicate protocol policy: {protocol}")
        result[protocol] = row
    return result


def _policy_positive_integer(value: Any, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
        raise MetricContractError(f"{label} must be a positive integer")
    return value


def _canonicality_rpc_minimum(policy: dict[str, Any]) -> int:
    canonicality = policy.get("canonicality")
    if not isinstance(canonicality, dict):
        raise MetricContractError("policy.canonicality must be an object")
    return _policy_positive_integer(
        canonicality.get("minimum_independent_rpc_observations"),
        "policy.canonicality.minimum_independent_rpc_observations",
    )


def _payout_state_rpc_minimum(policy: dict[str, Any]) -> int:
    qualification = policy.get("qualification")
    reconciliation = (
        qualification.get("solver_payout_reconciliation")
        if isinstance(qualification, dict)
        else None
    )
    if not isinstance(reconciliation, dict):
        raise MetricContractError(
            "policy.qualification.solver_payout_reconciliation must be an object"
        )
    return _policy_positive_integer(
        reconciliation.get("contract_state_minimum_independent_rpc_observations"),
        "policy.qualification.solver_payout_reconciliation."
        "contract_state_minimum_independent_rpc_observations",
    )


def _publication_gate_reasons(policy: dict[str, Any]) -> set[str]:
    gate = policy.get("publication_gate")
    if not isinstance(gate, dict) or gate.get("required_for_available_metric") is not True:
        raise MetricContractError(
            "policy.publication_gate must explicitly gate metric availability"
        )
    status = gate.get("status")
    reason_codes = gate.get("reason_codes")
    if not isinstance(reason_codes, list) or not all(
        isinstance(reason, str) and reason for reason in reason_codes
    ):
        raise MetricContractError("policy.publication_gate.reason_codes must be an array")
    if status == "approved":
        if reason_codes:
            raise MetricContractError(
                "an approved policy.publication_gate cannot retain blocker reason codes"
            )
        return set()
    if status != "blocked" or not reason_codes:
        raise MetricContractError(
            "policy.publication_gate must be approved or carry explicit blocker reasons"
        )
    return set(reason_codes)


def _principal_index(
    registry: dict[str, Any],
) -> tuple[dict[str, dict[str, Any]], set[str]]:
    by_wallet: dict[str, dict[str, Any]] = {}
    conflicts: set[str] = set()
    principal_ids: set[str] = set()
    principals = registry.get("principals")
    if not isinstance(principals, list):
        raise MetricContractError("principal registry principals must be an array")
    for principal in principals:
        if not isinstance(principal, dict) or not isinstance(
            principal.get("principal_id"), str
        ):
            raise MetricContractError("principal records require principal_id")
        principal_id = principal["principal_id"]
        if not principal_id:
            raise MetricContractError("principal_id must not be empty")
        if principal_id in principal_ids:
            raise MetricContractError(f"duplicate principal_id: {principal_id}")
        principal_ids.add(principal_id)
        if not isinstance(principal.get("operator_or_affiliate"), bool):
            raise MetricContractError(
                f"principal {principal_id} requires explicit operator_or_affiliate boolean"
            )
        if _evidence_refs(principal.get("beneficial_controller_evidence_refs")) is None:
            raise MetricContractError(
                f"principal {principal_id} requires beneficial-controller evidence refs"
            )
        wallets = principal.get("wallets")
        if not isinstance(wallets, list):
            raise MetricContractError("principal wallets must be an array")
        for raw_wallet in wallets:
            wallet = _normalize_address(raw_wallet)
            if wallet is None:
                raise MetricContractError(f"invalid wallet in principal registry: {raw_wallet!r}")
            if wallet in by_wallet and by_wallet[wallet].get("principal_id") != principal_id:
                conflicts.add(wallet)
            else:
                by_wallet[wallet] = principal
    return by_wallet, conflicts


def _root_index(
    root_map: dict[str, Any],
) -> tuple[dict[str, dict[str, Any]], set[str]]:
    rows = root_map.get("records")
    if not isinstance(rows, list):
        raise MetricContractError("root-work map records must be an array")
    result: dict[str, dict[str, Any]] = {}
    decisions: dict[str, tuple[Any, ...]] = {}
    conflicting_roots: set[str] = set()
    for row in rows:
        if not isinstance(row, dict) or not isinstance(row.get("candidate_key"), str):
            raise MetricContractError("root-work records require candidate_key")
        key = row["candidate_key"]
        if key in result and result[key] != row:
            raise MetricContractError(f"conflicting root-work record: {key}")
        result[key] = row
        root_work_id = row.get("root_work_id")
        if not isinstance(root_work_id, str) or not root_work_id:
            continue
        evidence_refs = _evidence_refs(row.get("evidence_refs"))
        history_refs = _evidence_refs(row.get("root_history_evidence_refs"))
        decision = (
            row.get("standalone_value_classification"),
            tuple(sorted(evidence_refs or [])),
            row.get("root_history_reconciled"),
            row.get("earliest_qualifying_candidate_key"),
            tuple(sorted(history_refs or [])),
        )
        prior = decisions.get(root_work_id)
        if prior is None:
            decisions[root_work_id] = decision
        elif prior != decision:
            conflicting_roots.add(root_work_id)
    return result, conflicting_roots


def _relationship_index(registry: dict[str, Any]) -> dict[tuple[str, str, str], dict[str, Any]]:
    rows = registry.get("reimbursement_relationships")
    if not isinstance(rows, list):
        raise MetricContractError(
            "principal registry reimbursement_relationships must be an array"
        )
    result: dict[tuple[str, str, str], dict[str, Any]] = {}
    for row in rows:
        if not isinstance(row, dict):
            raise MetricContractError("reimbursement relationship must be an object")
        key = (
            row.get("funder_principal_id"),
            row.get("solver_principal_id"),
            row.get("root_work_id"),
        )
        if not all(isinstance(value, str) and value for value in key):
            raise MetricContractError("reimbursement relationship key is incomplete")
        if key in result and result[key] != row:
            raise MetricContractError(f"conflicting reimbursement relationship: {key}")
        result[key] = row
    return result


def _global_availability_reasons(
    input_document: dict[str, Any],
    policy: dict[str, Any],
    principal_registry: dict[str, Any],
    root_map: dict[str, Any],
    start: datetime,
    end: datetime,
) -> set[str]:
    reasons = _publication_gate_reasons(policy)
    source = input_document.get("source_reconciliation")
    if not isinstance(source, dict):
        reasons.add("source_reconciliation_missing")
        return reasons
    if source.get("status") != "reconciled":
        reasons.add("source_streams_unreconciled")
    declared_source_reasons = source.get("reason_codes", [])
    if isinstance(declared_source_reasons, list):
        reasons.update(
            reason
            for reason in declared_source_reasons
            if isinstance(reason, str) and reason
        )
    if source.get("raw_evidence_retained") is not True:
        reasons.add("raw_evidence_incomplete")

    protocol_policy = _protocol_policy(policy)
    required = set(protocol_policy)
    required_factory_pairs: set[tuple[str, str]] = set()
    for protocol, rule in protocol_policy.items():
        factories = rule.get("accepted_factory_contracts")
        if not isinstance(factories, list) or not factories:
            raise MetricContractError(
                f"policy protocol {protocol} requires accepted_factory_contracts"
            )
        normalized_factories = [_normalize_address(value) for value in factories]
        if any(value is None for value in normalized_factories):
            raise MetricContractError(
                f"policy protocol {protocol} has an invalid accepted factory"
            )
        required_factory_pairs.update(
            (protocol, factory)
            for factory in normalized_factories
            if factory is not None
        )
    rows = source.get("protocols")
    seen: set[str] = set()
    seen_factory_pairs: set[tuple[str, str]] = set()
    complete_factory_pairs: set[tuple[str, str]] = set()
    if not isinstance(rows, list):
        reasons.add("protocol_source_coverage_missing")
        rows = []
    minimum_providers = _canonicality_rpc_minimum(policy)
    for row in rows:
        if not isinstance(row, dict) or not isinstance(row.get("protocol"), str):
            reasons.add("protocol_source_coverage_invalid")
            continue
        protocol = row["protocol"]
        seen.add(protocol)
        factory_contracts = row.get("factory_contracts")
        if not isinstance(factory_contracts, list) or not factory_contracts:
            reasons.add("protocol_source_factory_coverage_invalid")
            normalized_factories: list[str] = []
        else:
            normalized_factories = []
            for value in factory_contracts:
                normalized = _normalize_address(value)
                if normalized is None:
                    reasons.add("protocol_source_factory_coverage_invalid")
                    continue
                normalized_factories.append(normalized)
                seen_factory_pairs.add((protocol, normalized))
            if len(set(normalized_factories)) != len(normalized_factories):
                reasons.add("protocol_source_factory_coverage_invalid")
        row_complete = bool(normalized_factories)
        try:
            covered_start = _parse_utc(
                row.get("coverage_started_at"), f"source.{protocol}.coverage_started_at"
            )
            covered_end = _parse_utc(
                row.get("coverage_ended_at"), f"source.{protocol}.coverage_ended_at"
            )
        except MetricContractError:
            reasons.add("protocol_source_coverage_invalid")
            continue
        if covered_start > start or covered_end < end:
            reasons.add("protocol_source_window_incomplete")
            row_complete = False
        if row.get("raw_response_retained") is not True:
            reasons.add("raw_evidence_incomplete")
            row_complete = False
        provider_count = _nonnegative_int(row.get("independent_rpc_observation_count"))
        if provider_count is None or provider_count < minimum_providers:
            reasons.add("protocol_finality_evidence_incomplete")
            row_complete = False
        if _normalize_hex32(row.get("safe_block_hash")) is None:
            reasons.add("protocol_safe_block_hash_missing")
            row_complete = False
        if row_complete:
            complete_factory_pairs.update(
                (protocol, factory) for factory in normalized_factories
            )
    if not required.issubset(seen):
        reasons.add("required_protocol_stream_missing")
    if not required_factory_pairs.issubset(seen_factory_pairs):
        reasons.add("required_factory_stream_missing")
    if not required_factory_pairs.issubset(complete_factory_pairs):
        reasons.add("required_factory_stream_incomplete")

    if principal_registry.get("status") != "complete" or principal_registry.get(
        "coverage", {}
    ).get("all_candidate_wallets_mapped") is not True:
        reasons.add("beneficial_principal_coverage_incomplete")
    if principal_registry.get("coverage", {}).get(
        "all_candidate_reimbursement_relationships_reviewed"
    ) is not True:
        reasons.add("reimbursement_coverage_incomplete")
    if root_map.get("status") != "complete" or root_map.get("coverage", {}).get(
        "all_candidate_settlements_mapped"
    ) is not True:
        reasons.add("root_work_coverage_incomplete")
    if root_map.get("coverage", {}).get("all_roots_standalone_value_reviewed") is not True:
        reasons.add("standalone_value_coverage_incomplete")
    if root_map.get("coverage", {}).get("all_root_histories_reconciled") is not True:
        reasons.add("root_work_history_coverage_incomplete")
    return reasons


def _canonicality_reasons(
    candidate: dict[str, Any], policy: dict[str, Any]
) -> tuple[list[str], list[str]]:
    excluded: list[str] = []
    unknown: list[str] = []
    evidence = candidate.get("canonicality")
    if not isinstance(evidence, dict):
        return excluded, ["canonicality_evidence_missing"]
    if evidence.get("removed") is not False or evidence.get("reorg_detected") is not False:
        unknown.append("reorg_or_removed_log_detected")
    receipt_status = evidence.get("receipt_status")
    if isinstance(receipt_status, bool) or not isinstance(receipt_status, int) or receipt_status != 1:
        unknown.append("successful_receipt_unreconciled")
    if evidence.get("settlement_event_matches_receipt") is not True:
        unknown.append("settlement_event_receipt_mismatch")
    if evidence.get("contract_state_reconciled") is not True:
        unknown.append("settlement_state_unreconciled")
    accepted = set(policy.get("canonicality", {}).get("accepted_finality_statuses", []))
    if evidence.get("finality_status") not in accepted:
        unknown.append("finality_not_established")
    block_hash = _normalize_hex32(candidate.get("block_hash"))
    if block_hash is None:
        unknown.append("settlement_block_hash_missing")
    observations = evidence.get("rpc_block_observations")
    minimum = _canonicality_rpc_minimum(policy)
    if not isinstance(observations, list):
        unknown.append("rpc_block_observations_missing")
    else:
        providers: set[str] = set()
        observed_hashes: set[str] = set()
        observed_blocks: set[int] = set()
        for observation in observations:
            if not isinstance(observation, dict):
                continue
            provider_id = observation.get("provider_id")
            observed_hash = _normalize_hex32(observation.get("block_hash"))
            observed_block = _nonnegative_int(observation.get("block_number"))
            if isinstance(provider_id, str) and provider_id and observed_hash and observed_block is not None:
                providers.add(provider_id)
                observed_hashes.add(observed_hash)
                observed_blocks.add(observed_block)
        if len(providers) < minimum:
            unknown.append("insufficient_independent_rpc_observations")
        candidate_block = _nonnegative_int(candidate.get("block_number"))
        if (
            len(observed_hashes) != 1
            or block_hash not in observed_hashes
            or len(observed_blocks) != 1
            or candidate_block not in observed_blocks
        ):
            unknown.append("rpc_block_identity_disagreement")
    if _evidence_refs(evidence.get("raw_evidence_refs")) is None:
        unknown.append("settlement_raw_evidence_missing")
    return excluded, unknown


def _candidate_scope_matches(record: dict[str, Any], candidate: dict[str, Any]) -> bool:
    return (
        record.get("network") == candidate.get("network")
        and record.get("chain_id") == candidate.get("chain_id")
        and record.get("protocol") == candidate.get("protocol")
        and record.get("protocol_version") == candidate.get("protocol_version")
        and _normalize_address(record.get("factory_contract"))
        == _normalize_address(candidate.get("factory_contract"))
        and _normalize_address(record.get("bounty_contract"))
        == _normalize_address(candidate.get("bounty_contract"))
        and _normalize_hex32(record.get("bounty_id"))
        == _normalize_hex32(candidate.get("bounty_id"))
        and _round_matches(record.get("round"), candidate.get("round"))
    )


def _solver_payout_reconciliation_reasons(
    candidate: dict[str, Any], payout: int, policy: dict[str, Any]
) -> list[str]:
    """Require typed, separately anchored event, receipt, and state views."""

    evidence = candidate.get("solver_payout_reconciliation")
    if not isinstance(evidence, dict):
        return ["solver_payout_reconciliation_missing"]
    event = evidence.get("settlement_event")
    receipt = evidence.get("receipt_transfer")
    state = evidence.get("contract_state")
    if not all(isinstance(view, dict) for view in (event, receipt, state)):
        return ["solver_payout_reconciliation_malformed"]
    assert isinstance(event, dict) and isinstance(receipt, dict) and isinstance(state, dict)

    reasons: list[str] = []
    candidate_transaction = _normalize_hex32(candidate.get("transaction_hash"))
    candidate_log_index = _nonnegative_int(candidate.get("log_index"))
    candidate_block = _nonnegative_int(candidate.get("block_number"))
    candidate_block_hash = _normalize_hex32(candidate.get("block_hash"))
    solver = _normalize_address(candidate.get("solver_wallet"))
    if (
        not _candidate_scope_matches(event, candidate)
        or event.get("event_kind") != candidate.get("event_kind")
        or _normalize_hex32(event.get("transaction_hash")) != candidate_transaction
        or _nonnegative_int(event.get("log_index")) != candidate_log_index
        or _nonnegative_int(event.get("block_number")) != candidate_block
        or _normalize_hex32(event.get("block_hash")) != candidate_block_hash
        or _normalize_address(event.get("solver_wallet")) != solver
    ):
        reasons.append("solver_payout_event_identity_mismatch")

    settlement_asset = _normalize_address(
        policy.get("settlement_asset", {}).get("contract")
    )
    transfer_log_index = _nonnegative_int(receipt.get("transfer_log_index"))
    receipt_status = receipt.get("receipt_status")
    if (
        _normalize_address(receipt.get("asset_contract")) != settlement_asset
        or receipt.get("network") != candidate.get("network")
        or receipt.get("chain_id") != candidate.get("chain_id")
        or _normalize_hex32(receipt.get("transaction_hash")) != candidate_transaction
        or transfer_log_index is None
        or transfer_log_index == candidate_log_index
        or _nonnegative_int(receipt.get("block_number")) != candidate_block
        or _normalize_hex32(receipt.get("block_hash")) != candidate_block_hash
        or _normalize_address(receipt.get("from_address"))
        != _normalize_address(candidate.get("bounty_contract"))
        or _normalize_address(receipt.get("to_address")) != solver
        or isinstance(receipt_status, bool)
        or receipt_status != 1
    ):
        reasons.append("solver_payout_receipt_identity_mismatch")

    observed_block = _nonnegative_int(state.get("observed_at_block_number"))
    observed_hash = _normalize_hex32(state.get("observed_at_block_hash"))
    state_observations = state.get("rpc_block_observations")
    minimum_providers = _payout_state_rpc_minimum(policy)
    state_provider_ids: set[str] = set()
    state_blocks: set[int] = set()
    state_hashes: set[str] = set()
    if isinstance(state_observations, list):
        for observation in state_observations:
            if not isinstance(observation, dict):
                continue
            provider_id = observation.get("provider_id")
            observation_block = _nonnegative_int(observation.get("block_number"))
            observation_hash = _normalize_hex32(observation.get("block_hash"))
            if (
                isinstance(provider_id, str)
                and provider_id
                and observation_block is not None
                and observation_hash is not None
            ):
                state_provider_ids.add(provider_id)
                state_blocks.add(observation_block)
                state_hashes.add(observation_hash)
    if (
        not _candidate_scope_matches(state, candidate)
        or _normalize_address(state.get("solver_wallet")) != solver
        or candidate_block is None
        or observed_block is None
        or observed_block != candidate_block
        or observed_hash != candidate_block_hash
        or len(state_provider_ids) < minimum_providers
        or state_blocks != {observed_block}
        or state_hashes != {observed_hash}
    ):
        reasons.append("solver_payout_state_identity_mismatch")

    event_reward = _nonnegative_int(event.get("solver_reward_base_units"))
    event_bonus = _nonnegative_int(
        event.get("completion_or_timeout_bonus_base_units")
    )
    event_returned_bond = _nonnegative_int(event.get("returned_bond_base_units"))
    receipt_transfer = _nonnegative_int(receipt.get("amount_base_units"))
    returned_bond = _nonnegative_int(receipt.get("returned_bond_base_units"))
    state_payout = _nonnegative_int(state.get("solver_payout_base_units"))
    promised_principal = _positive_int(
        candidate.get("promised_reward_principal_base_units")
    )
    if any(
        amount is None
        for amount in (
            event_reward,
            event_bonus,
            event_returned_bond,
            receipt_transfer,
            returned_bond,
            state_payout,
            promised_principal,
        )
    ):
        reasons.append("solver_payout_reconciliation_malformed")
    else:
        assert event_reward is not None and event_bonus is not None
        assert event_returned_bond is not None
        assert receipt_transfer is not None and returned_bond is not None
        assert state_payout is not None and promised_principal is not None
        protocol_rule = _protocol_policy(policy).get(candidate.get("protocol"), {})
        returned_bond_semantics = protocol_rule.get("returned_bond_semantics")
        if (
            receipt_transfer < returned_bond
            or event_reward != promised_principal
            or event_returned_bond != returned_bond
            or (
                returned_bond_semantics == "must_be_zero"
                and event_returned_bond != 0
            )
            or returned_bond_semantics
            not in {"event_reported", "must_be_zero"}
            or payout != event_reward + event_bonus
            or payout != receipt_transfer - returned_bond
            or payout != state_payout
        ):
            reasons.append("solver_payout_reconciliation_mismatch")

    reference_sets: list[set[str]] = []
    for label, view in (("event", event), ("receipt", receipt), ("state", state)):
        if view.get("view_payload_sha256") != _view_payload_hash(view):
            reasons.append(f"solver_payout_{label}_view_hash_mismatch")
        refs = _evidence_refs(view.get("raw_evidence_refs"))
        if refs is None:
            reasons.append(f"solver_payout_{label}_evidence_missing")
        else:
            reference_sets.append(set(refs))
    if len(reference_sets) == 3 and any(
        left & right
        for index, left in enumerate(reference_sets)
        for right in reference_sets[index + 1 :]
    ):
        reasons.append("solver_payout_evidence_views_not_distinct")
    return reasons


def _evaluate_candidate(
    candidate: dict[str, Any],
    occurrences: int,
    input_document: dict[str, Any],
    policy: dict[str, Any],
    protocols: dict[str, dict[str, Any]],
    principals: dict[str, dict[str, Any]],
    principal_conflicts: set[str],
    roots: dict[str, dict[str, Any]],
    conflicting_roots: set[str],
    relationships: dict[tuple[str, str, str], dict[str, Any]],
    start: datetime,
    end: datetime,
) -> dict[str, Any]:
    excluded: list[str] = []
    unknown: list[str] = []
    info: list[str] = []
    key = _candidate_key(candidate)
    if key is None:
        unknown.append("candidate_identity_invalid")
        key = "invalid:" + sha256_json(candidate)
    if occurrences > 1:
        info.append("duplicate_event_collapsed")

    bounty_contract = _normalize_address(candidate.get("bounty_contract"))
    excluded_contracts = {
        _normalize_address(value) for value in policy.get("excluded_bounty_contracts", [])
    }
    excluded_contracts.discard(None)
    if bounty_contract in excluded_contracts:
        excluded.append("policy_declared_synthetic_contract")

    expected_network = input_document.get("network")
    expected_chain = input_document.get("chain_id")
    if candidate.get("network") != expected_network or candidate.get("chain_id") != expected_chain:
        excluded.append("wrong_network_or_chain")

    protocol = candidate.get("protocol")
    protocol_rule = protocols.get(protocol)
    if protocol_rule is None:
        excluded.append("unsupported_protocol")
    else:
        if candidate.get("protocol_version") not in protocol_rule.get(
            "accepted_protocol_versions", []
        ):
            excluded.append("unsupported_protocol_version")
        if candidate.get("event_kind") not in protocol_rule.get(
            "canonical_settlement_event_kinds", []
        ):
            excluded.append("wrong_settlement_event_kind")
        factory = _normalize_address(candidate.get("factory_contract"))
        accepted_factories = {
            _normalize_address(value)
            for value in protocol_rule.get("accepted_factory_contracts", [])
        }
        accepted_factories.discard(None)
        if factory is None or factory not in accepted_factories:
            excluded.append("unsupported_factory_scope")
        round_semantics = protocol_rule.get("round_semantics")
        candidate_round = candidate.get("round")
        if round_semantics == "required_positive_integer":
            if _positive_int(candidate_round) is None:
                unknown.append("candidate_round_invalid")
        elif round_semantics == "must_be_null":
            if candidate_round is not None:
                unknown.append("candidate_round_invalid")
        else:
            unknown.append("protocol_round_semantics_invalid")

    try:
        occurred_at = _parse_utc(candidate.get("occurred_at"), "candidate.occurred_at")
    except MetricContractError:
        occurred_at = None
        unknown.append("settlement_time_invalid")
    if occurred_at is not None and not (start <= occurred_at < end):
        excluded.append("outside_metric_window")

    settlement_position = _event_position(candidate)
    if settlement_position is None:
        unknown.append("settlement_position_missing")
    raw_payout = candidate.get("solver_payout_base_units")
    payout = _positive_int(raw_payout)
    if payout is None:
        if _nonnegative_int(raw_payout) == 0:
            excluded.append("solver_payout_not_positive")
        else:
            unknown.append("solver_payout_unreconciled")
    promised_principal = _positive_int(
        candidate.get("promised_reward_principal_base_units")
    )
    if promised_principal is None:
        unknown.append("promised_reward_principal_missing")
    settled_gmv = _positive_int(candidate.get("settled_gmv_base_units"))
    if settled_gmv is None:
        unknown.append("settled_gmv_unreconciled")

    _, canonical_unknown = _canonicality_reasons(candidate, policy)
    unknown.extend(canonical_unknown)
    # Payout evidence is reason-coded independently of every other canonicality
    # gate, so one missing view cannot be hidden by a different unknown fact.
    if payout is not None:
        unknown.extend(_solver_payout_reconciliation_reasons(candidate, payout, policy))

    excluded_principals = set(policy.get("excluded_operator_principal_ids", []))
    excluded_operator_wallets = {
        _normalize_address(value) for value in policy.get("excluded_operator_wallets", [])
    }
    excluded_operator_wallets.discard(None)

    solver_wallet = _normalize_address(candidate.get("solver_wallet"))
    solver_principal: dict[str, Any] | None = None
    if solver_wallet is None:
        unknown.append("solver_wallet_invalid")
    elif solver_wallet in principal_conflicts:
        unknown.append("beneficial_principal_registry_conflict")
    else:
        solver_principal = principals.get(solver_wallet)
        if solver_principal is None:
            unknown.append("solver_beneficial_principal_unknown")
        elif (
            solver_wallet in excluded_operator_wallets
            or solver_principal.get("principal_id") in excluded_principals
            or solver_principal.get("operator_or_affiliate") is True
        ):
            excluded.append("operator_solver_principal")

    root = roots.get(key)
    root_work_id: str | None = None
    if root is None:
        unknown.append("root_work_mapping_missing")
    else:
        root_work_id = root.get("root_work_id")
        if not isinstance(root_work_id, str) or not root_work_id:
            unknown.append("root_work_id_invalid")
            root_work_id = None
        elif root_work_id in conflicting_roots:
            unknown.append("root_work_decision_conflict")
        if _evidence_refs(root.get("evidence_refs")) is None:
            unknown.append("root_work_evidence_missing")
        standalone = root.get("standalone_value_classification")
        if standalone in set(policy.get("excluded_standalone_value_classifications", [])):
            excluded.append(f"standalone_value_excluded:{standalone}")
        elif standalone != policy.get("qualifying_standalone_value_classification"):
            unknown.append("standalone_value_classification_unknown")
        if root.get("root_history_reconciled") is not True:
            unknown.append("root_work_history_incomplete")
        if _evidence_refs(root.get("root_history_evidence_refs")) is None:
            unknown.append("root_work_history_evidence_missing")
        earliest = root.get("earliest_qualifying_candidate_key")
        if not isinstance(earliest, str) or not earliest:
            unknown.append("root_work_earliest_qualifying_settlement_unknown")
        elif earliest != key:
            excluded.extend(
                ["root_work_duplicate", "root_work_not_all_history_earliest"]
            )

    participation = candidate.get("participation_event")
    participation_position = _event_position(participation)
    if participation_position is None:
        unknown.append("participation_event_missing")
    else:
        if (
            protocol_rule is None
            or participation.get("kind")
            != protocol_rule.get("participation_event_kind")
        ):
            unknown.append("wrong_participation_event_kind")
        if _normalize_hex32(participation.get("transaction_hash")) is None:
            unknown.append("participation_transaction_hash_missing")
        if _normalize_hex32(participation.get("block_hash")) is None:
            unknown.append("participation_block_hash_missing")
        participation_scope_matches = (
            participation.get("network") == candidate.get("network")
            and participation.get("chain_id") == candidate.get("chain_id")
            and participation.get("protocol") == candidate.get("protocol")
            and _normalize_address(participation.get("factory_contract"))
            == _normalize_address(candidate.get("factory_contract"))
            and _normalize_address(participation.get("bounty_contract"))
            == bounty_contract
            and _normalize_hex32(participation.get("bounty_id"))
            == _normalize_hex32(candidate.get("bounty_id"))
            and _round_matches(
                participation.get("round"), candidate.get("round")
            )
            and _normalize_address(participation.get("participant_wallet"))
            == solver_wallet
        )
        if not participation_scope_matches:
            unknown.append("participation_candidate_scope_mismatch")
        if participation.get("canonical_event_reconciled") is not True:
            unknown.append("participation_event_unreconciled")
        if participation.get("first_participation_event_reconciled") is not True:
            unknown.append("first_participation_not_reconciled")
        receipt_status = participation.get("receipt_status")
        if (
            not isinstance(receipt_status, int)
            or isinstance(receipt_status, bool)
            or receipt_status != 1
        ):
            unknown.append("participation_receipt_unreconciled")
        if participation.get("event_matches_receipt") is not True:
            unknown.append("participation_event_receipt_mismatch")
        if (
            participation.get("removed") is not False
            or participation.get("reorg_detected") is not False
        ):
            unknown.append("participation_reorg_or_removal_unknown")
        if _evidence_refs(participation.get("raw_evidence_refs")) is None:
            unknown.append("participation_raw_evidence_missing")
        if settlement_position is not None and participation_position >= settlement_position:
            unknown.append("participation_not_before_settlement")

    funding_rows = candidate.get("funding_events")
    if not isinstance(funding_rows, list) or not funding_rows:
        unknown.append("canonical_funding_evidence_missing")
        funding_rows = []
    funding_rows, collapsed_funding_duplicates, conflicting_funding_duplicates = (
        _deduplicate_funding_events(funding_rows)
    )
    if collapsed_funding_duplicates:
        info.append("duplicate_funding_event_collapsed")
    if conflicting_funding_duplicates:
        unknown.append("conflicting_duplicate_funding_event")
    independent_amount = 0
    operator_subsidy_amount = 0
    independent_amounts_by_principal: Counter[str] = Counter()
    reviewed_funder_ids: set[str] = set()
    for funding in funding_rows:
        if not isinstance(funding, dict):
            unknown.append("funding_event_invalid")
            continue
        if protocol_rule is None or funding.get("kind") != protocol_rule.get(
            "funding_event_kind"
        ):
            unknown.append("wrong_funding_event_kind")
            continue
        amount = _positive_int(funding.get("amount_base_units"))
        position = _event_position(funding)
        if amount is None or position is None:
            unknown.append("funding_event_invalid")
            continue
        postparticipation = (
            participation_position is None or position >= participation_position
        )
        if funding.get("canonical_event_reconciled") is not True:
            unknown.append("funding_event_unreconciled")
            continue
        if _normalize_hex32(funding.get("transaction_hash")) is None:
            unknown.append("funding_transaction_hash_missing")
            continue
        if _normalize_hex32(funding.get("block_hash")) is None:
            unknown.append("funding_block_hash_missing")
            continue
        funding_scope_matches = (
            funding.get("network") == candidate.get("network")
            and funding.get("chain_id") == candidate.get("chain_id")
            and funding.get("protocol") == candidate.get("protocol")
            and _normalize_address(funding.get("factory_contract"))
            == _normalize_address(candidate.get("factory_contract"))
            and _normalize_address(funding.get("bounty_contract"))
            == bounty_contract
            and _normalize_hex32(funding.get("bounty_id"))
            == _normalize_hex32(candidate.get("bounty_id"))
            and _round_matches(funding.get("round"), candidate.get("round"))
        )
        if not funding_scope_matches:
            unknown.append("funding_candidate_scope_mismatch")
            continue
        funding_receipt_status = funding.get("receipt_status")
        if (
            not isinstance(funding_receipt_status, int)
            or isinstance(funding_receipt_status, bool)
            or funding_receipt_status != 1
        ):
            unknown.append("funding_receipt_unreconciled")
            continue
        if funding.get("event_matches_receipt") is not True:
            unknown.append("funding_event_receipt_mismatch")
            continue
        if funding.get("removed") is not False or funding.get("reorg_detected") is not False:
            unknown.append("funding_reorg_or_removal_unknown")
            continue
        if _evidence_refs(funding.get("raw_evidence_refs")) is None:
            unknown.append("funding_raw_evidence_missing")
            continue
        wallet = _normalize_address(funding.get("contributor_wallet"))
        if wallet is None:
            unknown.append("funder_wallet_invalid")
            continue
        if wallet in excluded_operator_wallets:
            operator_subsidy_amount += amount
            info.append("operator_wallet_funding_excluded")
            if postparticipation:
                info.append("postparticipation_funding_excluded")
            continue
        if wallet in principal_conflicts:
            unknown.append("beneficial_principal_registry_conflict")
            continue
        principal = principals.get(wallet)
        if principal is None:
            unknown.append("funder_beneficial_principal_unknown")
            continue
        principal_id = principal.get("principal_id")
        if not isinstance(principal_id, str):
            unknown.append("funder_beneficial_principal_unknown")
            continue
        if principal_id in excluded_principals or principal.get("operator_or_affiliate") is True:
            operator_subsidy_amount += amount
            info.append("operator_funding_excluded")
            if postparticipation:
                info.append("postparticipation_funding_excluded")
            continue
        if postparticipation:
            info.append("postparticipation_funding_excluded")
            continue
        if solver_principal is not None and principal_id == solver_principal.get("principal_id"):
            info.append("solver_self_funding_excluded")
            continue
        if solver_principal is None or root_work_id is None:
            # The missing evidence has already been recorded; this amount cannot
            # be promoted to independent funding.
            continue
        relationship_key = (
            principal_id,
            solver_principal.get("principal_id"),
            root_work_id,
        )
        relationship = relationships.get(relationship_key)
        if relationship is None or _evidence_refs(relationship.get("evidence_refs")) is None:
            unknown.append("reimbursement_relationship_unknown")
            continue
        status = relationship.get("status")
        if status == "no_reimbursement_found":
            independent_amount += amount
            independent_amounts_by_principal[principal_id] += amount
            reviewed_funder_ids.add(principal_id)
        elif status in {"reimbursed", "common_control", "suspected_reimbursement"}:
            # This contribution is not independent. It does not veto a work unit
            # when other qualifying contributions already cover the full promised
            # reward principal; the threshold check below remains authoritative.
            info.append("reimbursed_or_common_control_funding_excluded")
        else:
            unknown.append("reimbursement_relationship_unknown")

    if candidate.get("funding_reconciled_to_contract") is not True:
        unknown.append("funding_total_unreconciled")
    if promised_principal is not None and independent_amount < promised_principal:
        excluded.append("insufficient_independent_preparticipation_funding")

    declared_subsidies: list[dict[str, Any]] = []
    subsidy_evidence_status = "unknown"
    subsidy_document = candidate.get("operator_subsidies")
    if isinstance(subsidy_document, dict) and subsidy_document.get("status") == "complete":
        entries = subsidy_document.get("entries")
        if isinstance(entries, list):
            subsidy_evidence_status = "complete"
            for entry in entries:
                if not isinstance(entry, dict):
                    subsidy_evidence_status = "unknown"
                    break
                amount = _positive_int(entry.get("amount_base_units"))
                kind = entry.get("kind")
                asset = entry.get("asset")
                refs = entry.get("evidence_refs")
                if (
                    amount is None
                    or not isinstance(kind, str)
                    or not kind
                    or not isinstance(asset, str)
                    or not asset
                    or _evidence_refs(refs) is None
                ):
                    subsidy_evidence_status = "unknown"
                    break
                declared_subsidies.append(
                    {
                        "kind": kind,
                        "asset": asset,
                        "amount_base_units": amount,
                        "evidence_refs": sorted(str(ref) for ref in refs),
                    }
                )
    if subsidy_evidence_status != "complete":
        info.append("operator_subsidy_evidence_incomplete")
        declared_subsidies = []

    conclusive_exclusions = {
        "outside_metric_window",
        "operator_solver_principal",
        "policy_declared_synthetic_contract",
        "solver_payout_not_positive",
        "unsupported_factory_scope",
        "unsupported_protocol",
        "unsupported_protocol_version",
        "wrong_network_or_chain",
        "wrong_settlement_event_kind",
    }
    has_conclusive_exclusion = any(
        reason in conclusive_exclusions
        or reason.startswith("standalone_value_excluded:")
        for reason in excluded
    )
    if has_conclusive_exclusion:
        status = "excluded"
        counted = False
    elif unknown:
        status = "unknown"
        counted = False
    elif excluded:
        status = "excluded"
        counted = False
    else:
        status = "eligible"
        counted = True
    return {
        "candidate_key": key,
        "canonical_event_identity": _event_identity(candidate, 0),
        "status": status,
        "counted": counted,
        "reason_codes": sorted(set(unknown + excluded)),
        "informational_codes": sorted(set(info)),
        "source_record_count": occurrences,
        "protocol": protocol,
        "factory_contract": _normalize_address(candidate.get("factory_contract")),
        "bounty_id": candidate.get("bounty_id"),
        "round": candidate.get("round"),
        "transaction_hash": candidate.get("transaction_hash"),
        "log_index": candidate.get("log_index"),
        "block_number": candidate.get("block_number"),
        "block_hash": candidate.get("block_hash"),
        "occurred_at": candidate.get("occurred_at"),
        "solver_wallet": solver_wallet,
        "solver_principal_id": (
            solver_principal.get("principal_id") if solver_principal else None
        ),
        "solver_payout_base_units": payout,
        "promised_reward_principal_base_units": promised_principal,
        "settled_gmv_base_units": settled_gmv,
        "independent_preparticipation_funding_base_units": independent_amount,
        "independent_funding_by_principal": [
            {
                "principal_id": principal_id,
                "amount_base_units": amount,
            }
            for principal_id, amount in sorted(independent_amounts_by_principal.items())
        ],
        "operator_subsidy_base_units": operator_subsidy_amount,
        "operator_subsidy_evidence_status": subsidy_evidence_status,
        "declared_operator_subsidies": sorted(
            declared_subsidies,
            key=lambda row: (row["asset"], row["kind"], row["amount_base_units"]),
        ),
        "independent_funder_principal_ids": sorted(reviewed_funder_ids),
        "root_work_id": root_work_id,
    }


def _median_decimal(values: Iterable[int]) -> str | None:
    ordered = sorted(values)
    if not ordered:
        return None
    middle = len(ordered) // 2
    if len(ordered) % 2:
        return str(ordered[middle])
    return str((Decimal(ordered[middle - 1]) + Decimal(ordered[middle])) / Decimal(2))


def _ratio(numerator: int, denominator: int) -> str | None:
    if denominator == 0:
        return None
    return str(
        (Decimal(numerator) / Decimal(denominator)).quantize(
            Decimal("0.000001"), rounding=ROUND_HALF_UP
        )
    )


def evaluate(
    input_document: dict[str, Any],
    policy: dict[str, Any],
    principal_registry: dict[str, Any],
    root_map: dict[str, Any],
) -> dict[str, Any]:
    """Evaluate a frozen evidence package and return a deterministic ledger."""

    for document, schema, label in (
        (input_document, INPUT_SCHEMA, "input"),
        (policy, POLICY_SCHEMA, "policy"),
        (principal_registry, PRINCIPAL_SCHEMA, "principal registry"),
        (root_map, ROOT_MAP_SCHEMA, "root-work map"),
    ):
        if not isinstance(document, dict):
            raise MetricContractError(f"{label} must be a JSON object")
        _require_schema(document, schema, label)
    if input_document.get("policy_id") != policy.get("policy_id"):
        raise MetricContractError("input policy_id does not match policy")
    if input_document.get("network") != policy.get("network") or input_document.get(
        "chain_id"
    ) != policy.get("chain_id"):
        raise MetricContractError("input network/chain does not match policy")

    frozen_at = _parse_utc(input_document.get("frozen_at"), "input.frozen_at")
    policy_effective_at = _parse_utc(policy.get("effective_at"), "policy.effective_at")
    if frozen_at < policy_effective_at:
        raise MetricContractError(
            "input.frozen_at cannot predate the applied policy effective_at"
        )

    start, end, complete_days = _window(input_document)
    if frozen_at < end:
        raise MetricContractError("input.frozen_at cannot predate the metric window end")
    protocols = _protocol_policy(policy)
    principals, principal_conflicts = _principal_index(principal_registry)
    roots, conflicting_roots = _root_index(root_map)
    relationships = _relationship_index(principal_registry)
    unavailable = _global_availability_reasons(
        input_document, policy, principal_registry, root_map, start, end
    )
    if principal_conflicts:
        unavailable.add("beneficial_principal_registry_conflict")
    if conflicting_roots:
        unavailable.add("root_work_decision_conflict")

    candidates = input_document.get("candidates")
    if not isinstance(candidates, list):
        raise MetricContractError("input.candidates must be an array")

    by_identity: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for ordinal, candidate in enumerate(candidates):
        if not isinstance(candidate, dict):
            raise MetricContractError(f"candidate {ordinal} must be an object")
        by_identity[_event_identity(candidate, ordinal)].append(candidate)
    if _global_canonical_log_conflicts(candidates):
        unavailable.add("conflicting_reused_canonical_evidence_log")

    ledger: list[dict[str, Any]] = []
    for identity in sorted(by_identity):
        rows = by_identity[identity]
        semantic_hashes = {sha256_json(row) for row in rows}
        if len(semantic_hashes) > 1:
            unavailable.add("conflicting_duplicate_event")
            exemplar = rows[0]
            ledger.append(
                {
                    "candidate_key": _candidate_key(exemplar)
                    or "invalid:" + sha256_json(exemplar),
                    "canonical_event_identity": identity,
                    "status": "unknown",
                    "counted": False,
                    "reason_codes": ["conflicting_duplicate_event"],
                    "informational_codes": [],
                    "source_record_count": len(rows),
                    "conflicting_record_hashes": sorted(semantic_hashes),
                    "protocol": exemplar.get("protocol"),
                    "factory_contract": _normalize_address(
                        exemplar.get("factory_contract")
                    ),
                    "bounty_id": exemplar.get("bounty_id"),
                    "round": exemplar.get("round"),
                    "transaction_hash": exemplar.get("transaction_hash"),
                    "log_index": exemplar.get("log_index"),
                    "block_number": exemplar.get("block_number"),
                    "block_hash": exemplar.get("block_hash"),
                    "occurred_at": exemplar.get("occurred_at"),
                    "solver_wallet": _normalize_address(exemplar.get("solver_wallet")),
                    "solver_principal_id": None,
                    "solver_payout_base_units": _positive_int(
                        exemplar.get("solver_payout_base_units")
                    ),
                    "promised_reward_principal_base_units": _positive_int(
                        exemplar.get("promised_reward_principal_base_units")
                    ),
                    "settled_gmv_base_units": _positive_int(
                        exemplar.get("settled_gmv_base_units")
                    ),
                    "independent_preparticipation_funding_base_units": 0,
                    "independent_funding_by_principal": [],
                    "operator_subsidy_base_units": 0,
                    "operator_subsidy_evidence_status": "unknown",
                    "declared_operator_subsidies": [],
                    "independent_funder_principal_ids": [],
                    "root_work_id": None,
                }
            )
            continue
        ledger.append(
            _evaluate_candidate(
                rows[0],
                len(rows),
                input_document,
                policy,
                protocols,
                principals,
                principal_conflicts,
                roots,
                conflicting_roots,
                relationships,
                start,
                end,
            )
        )

    # Deduplicate at the economic root, not at wallet, bounty, or transaction.
    eligible_by_root: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in ledger:
        if row["status"] == "eligible" and row.get("root_work_id"):
            eligible_by_root[row["root_work_id"]].append(row)
    for root_rows in eligible_by_root.values():
        root_rows.sort(
            key=lambda row: (
                _parse_utc(row.get("occurred_at"), "occurred_at"),
                row.get("block_number") if row.get("block_number") is not None else -1,
                row.get("log_index") if row.get("log_index") is not None else -1,
                row["candidate_key"],
            )
        )
        for duplicate in root_rows[1:]:
            duplicate["status"] = "excluded"
            duplicate["counted"] = False
            duplicate["reason_codes"] = sorted(
                set(duplicate["reason_codes"] + ["root_work_duplicate"])
            )

    for row in ledger:
        if row["status"] == "unknown":
            unavailable.update(row["reason_codes"])
    ledger.sort(key=lambda row: row["candidate_key"])
    counted = [row for row in ledger if row["counted"]]
    status = "available" if not unavailable else "unavailable"
    publication_blockers = _publication_gate_reasons(policy)

    # A blocked production publication gate must not expose the provisional
    # numerator through diagnostics, candidate-level qualification fields, or
    # even an empty ledger. The retained input remains separately auditable,
    # but no evaluator-derived ledger or cardinality is published until the
    # source-trust controls are approved.
    published_ledger: list[dict[str, Any]] | None = ledger
    if publication_blockers:
        status = "unavailable"
        unavailable = set(publication_blockers)
        published_ledger = None

    daily: list[dict[str, Any]] = []
    cursor = start
    counted_by_day: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in counted:
        occurred = _parse_utc(row["occurred_at"], "ledger.occurred_at")
        counted_by_day[occurred.date().isoformat()].append(row)
    while cursor < end:
        day = cursor.date().isoformat()
        rows = counted_by_day.get(day, [])
        daily.append(
            {
                "day": day,
                "root_work_units": len(rows) if status == "available" else None,
                "solver_payout_base_units": (
                    sum(row["solver_payout_base_units"] for row in rows)
                    if status == "available"
                    else None
                ),
                "settled_gmv_base_units": (
                    sum(row["settled_gmv_base_units"] for row in rows)
                    if status == "available"
                    else None
                ),
            }
        )
        cursor += timedelta(days=1)

    payouts = [row["solver_payout_base_units"] for row in counted]
    total_payout = sum(payouts)
    total_gmv = sum(row["settled_gmv_base_units"] for row in counted)
    total_operator_subsidy = sum(row["operator_subsidy_base_units"] for row in counted)
    independent_funding_by_principal: Counter[str] = Counter()
    solver_payouts: Counter[str] = Counter()
    declared_subsidies_by_asset_and_kind: Counter[tuple[str, str]] = Counter()
    subsidy_guardrail_reasons: set[str] = set()
    for row in counted:
        solver_payouts[row["solver_principal_id"]] += row["solver_payout_base_units"]
        for funding in row["independent_funding_by_principal"]:
            independent_funding_by_principal[funding["principal_id"]] += funding[
                "amount_base_units"
            ]
        if row["operator_subsidy_evidence_status"] != "complete":
            subsidy_guardrail_reasons.add("operator_subsidy_evidence_incomplete")
        for subsidy in row["declared_operator_subsidies"]:
            declared_subsidies_by_asset_and_kind[
                (subsidy["asset"], subsidy["kind"])
            ] += subsidy["amount_base_units"]
    if publication_blockers:
        subsidy_guardrail_reasons.clear()
    total_independent_funding = sum(independent_funding_by_principal.values())
    subsidy_guardrail_available = status == "available" and not subsidy_guardrail_reasons

    result: dict[str, Any] = {
        "schema_version": RESULT_SCHEMA,
        "policy_id": policy["policy_id"],
        "policy_hash": sha256_json(policy),
        "input_hash": sha256_json(input_document),
        "principal_registry_hash": sha256_json(principal_registry),
        "root_work_map_hash": sha256_json(root_map),
        "window": deepcopy(input_document["window"]),
        "status": status,
        "unavailable_reason_codes": sorted(unavailable),
        "north_star": {
            "name": "policy_qualified_independently_funded_canonical_settled_root_work_units_per_day",
            "unit": "root_work_units_per_complete_utc_day",
            "total_root_work_units": len(counted) if status == "available" else None,
            "complete_utc_days": complete_days,
            "per_day": _ratio(len(counted), complete_days) if status == "available" else None,
            "daily": daily,
        },
        "secondary_metrics": {
            "qualifying_settled_gmv_base_units": total_gmv if status == "available" else None,
            "qualifying_settled_gmv_per_complete_utc_day": (
                _ratio(total_gmv, complete_days) if status == "available" else None
            ),
            "qualifying_solver_payout_base_units": total_payout if status == "available" else None,
            "median_qualifying_solver_payout_base_units": (
                _median_decimal(payouts) if status == "available" else None
            ),
            "independent_preparticipation_funding_base_units": (
                total_independent_funding if status == "available" else None
            ),
            "largest_independent_funding_principal_share": (
                _ratio(
                    max(independent_funding_by_principal.values(), default=0),
                    total_independent_funding,
                )
                if status == "available"
                else None
            ),
            "largest_solver_principal_share_of_solver_payout": (
                _ratio(max(solver_payouts.values(), default=0), total_payout)
                if status == "available"
                else None
            ),
            "operator_subsidy_guardrail": {
                "status": "available" if subsidy_guardrail_available else "unavailable",
                "unavailable_reason_codes": (
                    []
                    if subsidy_guardrail_available
                    else sorted(
                        subsidy_guardrail_reasons
                        or {"north_star_qualification_unavailable"}
                    )
                ),
                "operator_reward_funding_usdc_base_units": (
                    total_operator_subsidy if subsidy_guardrail_available else None
                ),
                "operator_reward_funding_share_of_qualifying_settled_gmv": (
                    _ratio(total_operator_subsidy, total_gmv)
                    if subsidy_guardrail_available
                    else None
                ),
                "declared_nonreward_subsidies_by_asset_and_kind": (
                    [
                        {
                            "asset": asset,
                            "kind": kind,
                            "amount_base_units": amount,
                        }
                        for (asset, kind), amount in sorted(
                            declared_subsidies_by_asset_and_kind.items()
                        )
                    ]
                    if subsidy_guardrail_available
                    else None
                ),
                "boundary": "Operator reward funding never qualifies as independent reward principal. Gas, onboarding, relayer, and other declared support are reported by asset and kind without cross-asset addition.",
            },
        },
        "diagnostics": {
            "input_candidate_records": (
                None if publication_blockers else len(candidates)
            ),
            "unique_canonical_event_identities": (
                None if publication_blockers else len(by_identity)
            ),
            "ledger_records": (
                None if publication_blockers else len(published_ledger or [])
            ),
            "eligible_root_work_units_before_availability_gate": (
                None if publication_blockers else len(counted)
            ),
            "excluded_records": (
                None
                if publication_blockers
                else sum(row["status"] == "excluded" for row in published_ledger)
            ),
            "unknown_records": (
                None
                if publication_blockers
                else sum(row["status"] == "unknown" for row in published_ledger)
            ),
            "reason_code_counts": (
                None
                if publication_blockers
                else dict(
                    sorted(
                        Counter(
                            reason
                            for row in published_ledger
                            for reason in row.get("reason_codes", [])
                        ).items()
                    )
                )
            ),
        },
        "ledger": published_ledger,
        "evidence_boundary": (
            "Only a fully reconciled, final canonical settlement with positive solver payout, "
            "beneficial-principal evidence, sufficient pre-participation independent funding, "
            "a reviewed no-reimbursement relationship, and a qualifying standalone root-work "
            "classification can count. Unknown or incomplete evidence makes the published metric "
            "unavailable; it never becomes zero or eligible by default."
        ),
    }
    result["result_hash"] = sha256_json(result)
    return result


def _write_result(result: dict[str, Any], output: str | None) -> None:
    rendered = json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    if output:
        Path(output).write_text(rendered, encoding="utf-8")
    else:
        sys.stdout.write(rendered)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, help="frozen evaluation input JSON")
    parser.add_argument("--policy", required=True, help="versioned QICSWU policy JSON")
    parser.add_argument("--principals", required=True, help="beneficial-principal registry JSON")
    parser.add_argument("--root-map", required=True, help="root-work classification map JSON")
    parser.add_argument("--output", help="write result to this path instead of stdout")
    parser.add_argument(
        "--check-result",
        help="fail if the deterministic result differs from this frozen JSON",
    )
    args = parser.parse_args(argv)
    try:
        result = evaluate(
            load_json(args.input),
            load_json(args.policy),
            load_json(args.principals),
            load_json(args.root_map),
        )
        if args.check_result:
            expected = load_json(args.check_result)
            if canonical_json_bytes(result) != canonical_json_bytes(expected):
                sys.stderr.write("QICSWU result differs from frozen result\n")
                return 1
        _write_result(result, args.output)
        return 0
    except (MetricContractError, OSError, json.JSONDecodeError) as error:
        sys.stderr.write(f"QICSWU evaluation failed closed: {error}\n")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
