#!/usr/bin/env python3
"""Capture and verify production QICSWU canonical-settlement evidence.

This module deliberately covers the on-chain settlement/finality layer only.
It never infers beneficial ownership, reimbursement, root-work identity, or
complete protocol-stream coverage.  A technically valid capture therefore
remains publication-ineligible until those separate controls and the RPC
provider registry are approved.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from copy import deepcopy
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any, Callable
from urllib.parse import urlsplit

try:
    from scripts import qicswu_metric as metric
    from scripts._shared.rpc import rpc
except ImportError:  # Direct invocation adds scripts/ to sys.path.
    import qicswu_metric as metric
    from _shared.rpc import rpc


ROOT = Path(__file__).resolve().parents[1]
REGISTRY_SCHEMA = "agent-bounties/qicswu-rpc-provider-registry-v1"
BUNDLE_SCHEMA = "agent-bounties/qicswu-canonical-evidence-bundle-v1"
REPORT_SCHEMA = "agent-bounties/qicswu-canonical-evidence-resolution-v1"
NETWORK = "base-mainnet"
CHAIN_ID = 8453
USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
TRANSFER_TOPIC = (
    "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
)
SETTLEMENT_TOPICS = {
    # keccak256 of the exact reviewed Solidity event signatures. Keeping the
    # topics literal makes this production-evidence checker stdlib-only.
    "autonomous": "0x2ee7b1323824324868017119c9922527f2e2c8eb20a8044c3dd574bdbfb2dd71",
    "open_competition_v1": "0x2ee7b1323824324868017119c9922527f2e2c8eb20a8044c3dd574bdbfb2dd71",
    "open_competition_v2": "0xc20e8d588b6d7bc5f86fd5c1d73eb149e0318efd5d22c8904f0dbf75f7a22c2d",
}
HEX_32_RE = re.compile(r"^0x[0-9a-f]{64}$")
ADDRESS_RE = re.compile(r"^0x[0-9a-f]{40}$")
PROVIDER_ID_RE = re.compile(r"^[a-z][a-z0-9_-]{1,63}$")
OBSERVATION_ID_RE = re.compile(r"^[a-z][a-z0-9_.:-]{1,191}$")
EVIDENCE_REF_RE = re.compile(
    r"^qicswu-evidence://sha256/([0-9a-f]{64})/([a-z][a-z0-9_.:-]{1,191})"
    r"(#(?:/[^#]*)?)?$"
)
REQUIRED_APPROVAL_ROLES = frozenset({"data_owner", "security_privacy"})
READ_ONLY_METHODS = frozenset(
    {
        "eth_chainId",
        "eth_getBlockByNumber",
        "eth_getTransactionReceipt",
    }
)


class EvidenceError(ValueError):
    """Evidence or provider configuration failed a fail-closed check."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise EvidenceError(message)


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def parse_utc(value: Any, label: str) -> datetime:
    require(isinstance(value, str), f"{label} must be an RFC3339 UTC timestamp")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise EvidenceError(f"{label} is not valid RFC3339") from error
    require(
        parsed.tzinfo is not None and parsed.utcoffset() == timedelta(0),
        f"{label} must use UTC",
    )
    return parsed.astimezone(timezone.utc)


def quantity(value: Any, label: str) -> int:
    require(
        isinstance(value, str) and value.startswith("0x"),
        f"{label} must be an RPC quantity",
    )
    try:
        parsed = int(value, 16)
    except ValueError as error:
        raise EvidenceError(f"{label} must be an RPC quantity") from error
    require(parsed >= 0, f"{label} must be nonnegative")
    return parsed


def normalized_hex32(value: Any, label: str) -> str:
    require(isinstance(value, str), f"{label} must be bytes32 hex")
    normalized = value.lower()
    require(HEX_32_RE.fullmatch(normalized) is not None, f"{label} must be bytes32 hex")
    return normalized


def normalized_address(value: Any, label: str) -> str:
    require(isinstance(value, str), f"{label} must be an EVM address")
    normalized = value.lower()
    require(ADDRESS_RE.fullmatch(normalized) is not None, f"{label} must be an EVM address")
    return normalized


def canonical_hash(value: Any) -> str:
    return metric.sha256_json(value)


def artifact_hash(document: dict[str, Any]) -> str:
    body = {key: value for key, value in document.items() if key != "artifact_hash"}
    return canonical_hash(body)


def registry_body(registry: dict[str, Any]) -> dict[str, Any]:
    return {
        key: deepcopy(value)
        for key, value in registry.items()
        if key not in {"status", "approval", "artifact_hash"}
    }


def registry_body_hash(registry: dict[str, Any]) -> str:
    return canonical_hash(registry_body(registry))


def _https_evidence_refs(value: Any, label: str) -> list[str]:
    require(isinstance(value, list) and value, f"{label} must be a non-empty array")
    refs: list[str] = []
    for raw in value:
        require(isinstance(raw, str) and raw, f"{label} contains an invalid reference")
        parsed = urlsplit(raw)
        require(
            parsed.scheme == "https"
            and parsed.hostname is not None
            and parsed.username is None
            and parsed.password is None,
            f"{label} must contain credential-free HTTPS references",
        )
        refs.append(raw)
    return refs


def validate_registry(registry: dict[str, Any]) -> dict[str, Any]:
    require(isinstance(registry, dict), "provider registry must be an object")
    require(registry.get("schema_version") == REGISTRY_SCHEMA, "provider registry schema mismatch")
    require(registry.get("network") == NETWORK, "provider registry network mismatch")
    require(registry.get("chain_id") == CHAIN_ID, "provider registry chain mismatch")
    require(
        isinstance(registry.get("registry_id"), str) and registry["registry_id"],
        "provider registry requires registry_id",
    )
    providers = registry.get("providers")
    require(isinstance(providers, list) and len(providers) >= 2, "at least two RPC providers are required")

    by_id: dict[str, dict[str, Any]] = {}
    authorities: set[str] = set()
    controllers: set[str] = set()
    for row in providers:
        require(isinstance(row, dict), "provider record must be an object")
        provider_id = row.get("provider_id")
        require(
            isinstance(provider_id, str) and PROVIDER_ID_RE.fullmatch(provider_id) is not None,
            "provider_id is invalid",
        )
        require(provider_id not in by_id, f"duplicate provider_id: {provider_id}")
        authority = row.get("endpoint_authority")
        require(isinstance(authority, str) and authority == authority.lower(), "endpoint_authority must be lowercase")
        parsed_authority = urlsplit(f"https://{authority}")
        require(
            parsed_authority.hostname == authority
            and parsed_authority.port is None
            and parsed_authority.path == "",
            f"invalid endpoint_authority for {provider_id}",
        )
        controller = row.get("control_principal_id")
        require(isinstance(controller, str) and controller, f"{provider_id} requires control_principal_id")
        require(authority not in authorities, "provider endpoint authorities must be distinct")
        require(controller not in controllers, "provider control principals must be distinct")
        require(row.get("archive_reads_supported") is True, f"{provider_id} must support archive reads")
        _https_evidence_refs(
            row.get("operator_evidence_refs"), f"{provider_id}.operator_evidence_refs"
        )
        authorities.add(authority)
        controllers.add(controller)
        by_id[provider_id] = row

    status = registry.get("status")
    require(status in {"candidate", "approved"}, "provider registry status is invalid")
    approval = registry.get("approval")
    require(isinstance(approval, dict), "provider registry requires approval object")
    approval_status = approval.get("status")
    require(approval_status in {"pending", "approved"}, "provider approval status is invalid")
    expected_body_hash = registry_body_hash(registry)
    declared_body_hash = approval.get("registry_body_hash")
    if declared_body_hash is not None:
        require(declared_body_hash == expected_body_hash, "provider approval body hash mismatch")

    approved = status == "approved" and approval_status == "approved"
    approvals = approval.get("approvals")
    require(isinstance(approvals, list), "provider approvals must be an array")
    if approved:
        require(declared_body_hash == expected_body_hash, "approved registry must bind its body hash")
        seen_roles: set[str] = set()
        reviewers: set[str] = set()
        for row in approvals:
            require(isinstance(row, dict), "provider approval must be an object")
            role = row.get("role")
            reviewer = row.get("reviewer_id")
            require(role in REQUIRED_APPROVAL_ROLES, "provider approval role is invalid")
            require(role not in seen_roles, f"duplicate provider approval role: {role}")
            require(isinstance(reviewer, str) and reviewer, "provider approval requires reviewer_id")
            require(reviewer not in reviewers, "provider approvals require distinct reviewers")
            require(row.get("decision") == "approved", "provider approval decision must be approved")
            parse_utc(row.get("approved_at"), "provider approval approved_at")
            _https_evidence_refs(row.get("evidence_refs"), "provider approval evidence_refs")
            seen_roles.add(role)
            reviewers.add(reviewer)
        require(seen_roles == REQUIRED_APPROVAL_ROLES, "provider registry lacks required role approvals")
    else:
        require(
            status == "candidate" and approval_status == "pending",
            "provider registry status and approval status disagree",
        )

    if registry.get("artifact_hash") is not None:
        require(registry["artifact_hash"] == artifact_hash(registry), "provider registry artifact hash mismatch")
    return {
        "registry_id": registry["registry_id"],
        "registry_body_hash": expected_body_hash,
        "approved": approved,
        "providers": by_id,
    }


def parse_rpc_assignments(values: list[str], registry_info: dict[str, Any]) -> dict[str, str]:
    assignments: dict[str, str] = {}
    for value in values:
        provider_id, separator, endpoint = value.partition("=")
        require(separator == "=" and endpoint, "--rpc must use provider_id=https://endpoint")
        require(provider_id in registry_info["providers"], f"unknown RPC provider: {provider_id}")
        require(provider_id not in assignments, f"duplicate RPC assignment: {provider_id}")
        parsed = urlsplit(endpoint)
        require(
            parsed.scheme == "https"
            and parsed.hostname is not None
            and parsed.username is None
            and parsed.password is None,
            f"RPC {provider_id} must use HTTPS without URL userinfo",
        )
        require(
            parsed.hostname.lower()
            == registry_info["providers"][provider_id]["endpoint_authority"],
            f"RPC {provider_id} authority does not match the registry",
        )
        assignments[provider_id] = endpoint
    require(
        set(assignments) == set(registry_info["providers"]),
        "an endpoint is required for every registered provider",
    )
    return assignments


def normalize_log(value: Any) -> dict[str, Any]:
    require(isinstance(value, dict), "RPC log must be an object")
    topics = value.get("topics")
    require(isinstance(topics, list), "RPC log topics must be an array")
    data = value.get("data")
    require(
        isinstance(data, str)
        and data.startswith("0x")
        and len(data) % 2 == 0
        and re.fullmatch(r"0x[0-9a-fA-F]*", data) is not None,
        "RPC log data must be even-length hex",
    )
    require(isinstance(value.get("removed"), bool), "RPC log removed must be boolean")
    return {
        "address": normalized_address(value.get("address"), "log.address"),
        "blockHash": normalized_hex32(value.get("blockHash"), "log.blockHash"),
        "blockNumber": hex(quantity(value.get("blockNumber"), "log.blockNumber")),
        "data": data.lower(),
        "logIndex": hex(quantity(value.get("logIndex"), "log.logIndex")),
        "removed": value["removed"],
        "topics": [normalized_hex32(item, "log.topic") for item in topics],
        "transactionHash": normalized_hex32(value.get("transactionHash"), "log.transactionHash"),
        "transactionIndex": hex(quantity(value.get("transactionIndex"), "log.transactionIndex")),
    }


def normalize_receipt(value: Any) -> dict[str, Any]:
    require(isinstance(value, dict), "transaction receipt must be an object")
    logs = value.get("logs")
    require(isinstance(logs, list), "transaction receipt logs must be an array")
    return {
        "transactionHash": normalized_hex32(value.get("transactionHash"), "receipt.transactionHash"),
        "blockHash": normalized_hex32(value.get("blockHash"), "receipt.blockHash"),
        "blockNumber": hex(quantity(value.get("blockNumber"), "receipt.blockNumber")),
        "status": hex(quantity(value.get("status"), "receipt.status")),
        "from": normalized_address(value.get("from"), "receipt.from"),
        "to": (
            normalized_address(value.get("to"), "receipt.to")
            if value.get("to") is not None
            else None
        ),
        "logs": [normalize_log(row) for row in logs],
    }


def normalize_block(value: Any) -> dict[str, Any]:
    require(isinstance(value, dict), "block result must be an object")
    return {
        "number": hex(quantity(value.get("number"), "block.number")),
        "hash": normalized_hex32(value.get("hash"), "block.hash"),
        "parentHash": normalized_hex32(value.get("parentHash"), "block.parentHash"),
        "timestamp": hex(quantity(value.get("timestamp"), "block.timestamp")),
        "stateRoot": normalized_hex32(value.get("stateRoot"), "block.stateRoot"),
        "transactionsRoot": normalized_hex32(
            value.get("transactionsRoot"), "block.transactionsRoot"
        ),
        "receiptsRoot": normalized_hex32(value.get("receiptsRoot"), "block.receiptsRoot"),
    }


def require_log_matches_receipt(
    log: dict[str, Any], receipt: dict[str, Any], label: str
) -> None:
    """Bind an embedded receipt log to the receipt's canonical identity."""

    require(log.get("removed") is False, f"{label} is marked removed")
    require(
        log.get("transactionHash") == receipt.get("transactionHash"),
        f"{label} transaction hash does not match its receipt",
    )
    require(
        log.get("blockNumber") == receipt.get("blockNumber"),
        f"{label} block number does not match its receipt",
    )
    require(
        log.get("blockHash") == receipt.get("blockHash"),
        f"{label} block hash does not match its receipt",
    )


def normalized_observation_result(method: str, result: Any) -> Any:
    if method == "eth_chainId":
        return hex(quantity(result, "chain id"))
    if method == "eth_getTransactionReceipt":
        return normalize_receipt(result)
    if method == "eth_getBlockByNumber":
        return normalize_block(result)
    raise EvidenceError(f"unsupported observation method: {method}")


def _word(data: str, index: int, label: str) -> int:
    require(isinstance(data, str) and data.startswith("0x"), f"{label} data is invalid")
    raw = data[2:]
    start = index * 64
    require(len(raw) >= start + 64, f"{label} data is truncated")
    try:
        return int(raw[start : start + 64], 16)
    except ValueError as error:
        raise EvidenceError(f"{label} data is invalid") from error


def _topic_address(value: str, label: str) -> str:
    topic = normalized_hex32(value, label)
    require(topic[2:26] == "0" * 24, f"{label} is not an encoded address")
    return "0x" + topic[-40:]


def decode_settlement(log: dict[str, Any], protocol: str) -> dict[str, Any]:
    topics = log["topics"]
    require(len(topics) == 4, "settlement log must have four topics")
    require(
        protocol in SETTLEMENT_TOPICS and topics[0] == SETTLEMENT_TOPICS[protocol],
        "settlement event signature mismatch",
    )
    solver = _topic_address(topics[3], "settlement solver topic")
    if protocol in {"autonomous", "open_competition_v1"}:
        solver_reward = _word(log["data"], 0, "BountySettled")
        returned_bond = _word(log["data"], 1, "BountySettled")
        completion_bonus = _word(log["data"], 2, "BountySettled")
        verifier_reward = _word(log["data"], 3, "BountySettled")
        solver_payout = solver_reward + completion_bonus
        settled_gmv = solver_payout + verifier_reward
    elif protocol == "open_competition_v2":
        solver_reward = _word(log["data"], 0, "CompetitionSettledV2")
        returned_bond = 0
        completion_bonus = 0
        verifier_reward = _word(log["data"], 2, "CompetitionSettledV2")
        solver_payout = solver_reward
        settled_gmv = solver_reward + verifier_reward
    else:
        raise EvidenceError(f"unsupported settlement protocol: {protocol}")
    return {
        "bounty_id": topics[1],
        "round_or_sequence": quantity(topics[2], "settlement round/sequence"),
        "solver_wallet": solver,
        "solver_reward_base_units": solver_reward,
        "completion_or_timeout_bonus_base_units": completion_bonus,
        "returned_bond_base_units": returned_bond,
        "verifier_or_keeper_reward_base_units": verifier_reward,
        "solver_payout_base_units": solver_payout,
        "settled_gmv_base_units": settled_gmv,
    }


def solver_transfer(logs: list[dict[str, Any]], bounty: str, solver: str) -> dict[str, Any]:
    matches = []
    for row in logs:
        topics = row.get("topics") or []
        if (
            row.get("address") == USDC
            and len(topics) == 3
            and topics[0] == TRANSFER_TOPIC
            and _topic_address(topics[1], "transfer from") == bounty
            and _topic_address(topics[2], "transfer to") == solver
        ):
            matches.append(row)
    require(len(matches) == 1, "settlement receipt requires exactly one bounty-to-solver USDC transfer")
    row = matches[0]
    amount = _word(row["data"], 0, "USDC Transfer")
    return {"log": row, "amount_base_units": amount}


def candidate_key(candidate: dict[str, Any]) -> str:
    value = metric._candidate_key(candidate)
    require(value is not None, "candidate identity is invalid")
    return value


def evidence_ref(payload_hash: str, observation_id: str, pointer: str = "") -> str:
    require(payload_hash.startswith("sha256:"), "evidence payload hash is invalid")
    suffix = f"#{pointer}" if pointer else ""
    return (
        f"qicswu-evidence://sha256/{payload_hash.removeprefix('sha256:')}/"
        f"{observation_id}{suffix}"
    )


def resolve_ref(bundle: dict[str, Any], reference: str) -> Any:
    match = EVIDENCE_REF_RE.fullmatch(reference)
    require(match is not None, f"evidence reference is not content-addressed: {reference}")
    digest, observation_id, fragment = match.groups()
    require(bundle.get("evidence_payload_hash") == f"sha256:{digest}", "evidence reference hash mismatch")
    observations = {
        row.get("observation_id"): row
        for row in bundle.get("observations", [])
        if isinstance(row, dict)
    }
    require(observation_id in observations, f"evidence observation is missing: {observation_id}")
    value: Any = observations[observation_id]
    pointer = (fragment or "").removeprefix("#")
    if not pointer:
        return value
    require(pointer.startswith("/"), "evidence fragment must be a JSON pointer")
    for raw in pointer[1:].split("/"):
        token = raw.replace("~1", "/").replace("~0", "~")
        if isinstance(value, list):
            require(token.isdigit(), "evidence array pointer must be numeric")
            index = int(token)
            require(0 <= index < len(value), "evidence array pointer is out of range")
            value = value[index]
        else:
            require(isinstance(value, dict) and token in value, "evidence object pointer is missing")
            value = value[token]
    return value


def _observation(
    observation_id: str,
    provider_id: str,
    method: str,
    params: list[Any],
    result: Any,
    normalized: Any,
) -> dict[str, Any]:
    require(OBSERVATION_ID_RE.fullmatch(observation_id) is not None, "observation_id is invalid")
    return {
        "observation_id": observation_id,
        "provider_id": provider_id,
        "method": method,
        "params": params,
        "result": result,
        "result_sha256": canonical_hash(result),
        "normalized_result_sha256": canonical_hash(normalized),
    }


def _call(
    rpc_call: Callable[..., Any], endpoint: str, method: str, params: list[Any], request_id: int
) -> Any:
    require(method in READ_ONLY_METHODS, f"refusing non-read-only RPC method: {method}")
    return rpc_call(endpoint, method, params, request_id, attempts=3, timeout=30)


def capture(
    input_document: dict[str, Any],
    registry: dict[str, Any],
    assignments: dict[str, str],
    *,
    captured_at: str | None = None,
    rpc_call: Callable[..., Any] = rpc,
) -> dict[str, Any]:
    registry_info = validate_registry(registry)
    require(set(assignments) == set(registry_info["providers"]), "RPC assignments are incomplete")
    require(input_document.get("schema_version") == metric.INPUT_SCHEMA, "evaluation input schema mismatch")
    require(input_document.get("network") == NETWORK, "evaluation input network mismatch")
    require(input_document.get("chain_id") == CHAIN_ID, "evaluation input chain mismatch")
    candidates = input_document.get("candidates")
    require(isinstance(candidates, list), "evaluation input candidates must be an array")
    captured_at = captured_at or utc_now()
    parse_utc(captured_at, "captured_at")

    observations: list[dict[str, Any]] = []
    safe_heads: dict[str, dict[str, Any]] = {}
    request_id = 1
    for provider_id in sorted(assignments):
        endpoint = assignments[provider_id]
        chain = _call(rpc_call, endpoint, "eth_chainId", [], request_id)
        request_id += 1
        require(quantity(chain, f"{provider_id} chain id") == CHAIN_ID, f"{provider_id} returned wrong chain")
        observations.append(
            _observation(f"provider.{provider_id}.chain", provider_id, "eth_chainId", [], chain, chain)
        )
        safe = _call(rpc_call, endpoint, "eth_getBlockByNumber", ["safe", False], request_id)
        request_id += 1
        normalized_safe = normalize_block(safe)
        safe_heads[provider_id] = normalized_safe
        observations.append(
            _observation(
                f"provider.{provider_id}.safe", provider_id, "eth_getBlockByNumber", ["safe", False], safe, normalized_safe
            )
        )

    pending_index: list[dict[str, Any]] = []
    for ordinal, candidate in enumerate(candidates):
        key = candidate_key(candidate)
        transaction_hash = normalized_hex32(candidate.get("transaction_hash"), "candidate transaction_hash")
        block_number = candidate.get("block_number")
        log_index = candidate.get("log_index")
        require(isinstance(block_number, int) and not isinstance(block_number, bool), "candidate block_number is invalid")
        require(isinstance(log_index, int) and not isinstance(log_index, bool), "candidate log_index is invalid")
        bounty = normalized_address(candidate.get("bounty_contract"), "candidate bounty_contract")
        solver = normalized_address(candidate.get("solver_wallet"), "candidate solver_wallet")
        occurred_at = parse_utc(candidate.get("occurred_at"), "candidate occurred_at")

        receipt_rows: list[dict[str, Any]] = []
        block_rows: list[dict[str, Any]] = []
        receipt_observation_ids: list[str] = []
        block_observation_ids: list[str] = []
        settlement_positions: list[int] = []
        transfer_positions: list[int] = []
        decoded_rows: list[dict[str, Any]] = []
        transfer_amounts: list[int] = []
        for provider_id in sorted(assignments):
            endpoint = assignments[provider_id]
            receipt = _call(
                rpc_call, endpoint, "eth_getTransactionReceipt", [transaction_hash], request_id
            )
            request_id += 1
            require(receipt is not None, f"{provider_id} returned no settlement receipt for candidate {ordinal}")
            normalized_receipt = normalize_receipt(receipt)
            block = _call(
                rpc_call, endpoint, "eth_getBlockByNumber", [hex(block_number), False], request_id
            )
            request_id += 1
            require(block is not None, f"{provider_id} returned no settlement block for candidate {ordinal}")
            normalized_block = normalize_block(block)
            require(quantity(normalized_receipt["status"], "receipt status") == 1, "settlement receipt failed")
            require(normalized_receipt["transactionHash"] == transaction_hash, "settlement transaction hash mismatch")
            require(quantity(normalized_receipt["blockNumber"], "receipt block") == block_number, "settlement receipt block mismatch")
            require(normalized_receipt["blockHash"] == normalized_block["hash"], "receipt/block hash mismatch")
            if candidate.get("block_hash") is not None:
                require(
                    normalized_hex32(candidate.get("block_hash"), "candidate block_hash")
                    == normalized_block["hash"],
                    "candidate block hash mismatch",
                )
            block_time = datetime.fromtimestamp(
                quantity(normalized_block["timestamp"], "block timestamp"),
                tz=timezone.utc,
            )
            require(block_time == occurred_at, "candidate occurrence time does not match its block")
            require(quantity(safe_heads[provider_id]["number"], "safe head") >= block_number, "settlement is not safe")

            matching_positions = [
                position
                for position, row in enumerate(normalized_receipt["logs"])
                if quantity(row["logIndex"], "settlement log index") == log_index
                and row["address"] == bounty
            ]
            require(len(matching_positions) == 1, "settlement log is absent or ambiguous")
            settlement_position = matching_positions[0]
            settlement = normalized_receipt["logs"][settlement_position]
            require_log_matches_receipt(settlement, normalized_receipt, "settlement log")
            decoded = decode_settlement(settlement, str(candidate.get("protocol")))
            require(decoded["bounty_id"] == normalized_hex32(candidate.get("bounty_id"), "candidate bounty_id"), "settlement bounty id mismatch")
            require(decoded["solver_wallet"] == solver, "settlement solver mismatch")
            transfer = solver_transfer(normalized_receipt["logs"], bounty, solver)
            require_log_matches_receipt(transfer["log"], normalized_receipt, "solver transfer log")
            expected_transfer = decoded["solver_payout_base_units"] + decoded["returned_bond_base_units"]
            require(transfer["amount_base_units"] == expected_transfer, "solver transfer does not reconcile to settlement")
            candidate_payout = candidate.get("solver_payout_base_units")
            if candidate_payout is not None:
                require(candidate_payout == decoded["solver_payout_base_units"], "candidate solver payout mismatch")
            candidate_principal = candidate.get("promised_reward_principal_base_units")
            if candidate_principal is not None:
                require(candidate_principal == decoded["solver_reward_base_units"], "candidate promised principal mismatch")
            require(
                candidate.get("settled_gmv_base_units") == decoded["settled_gmv_base_units"],
                "candidate settled GMV mismatch",
            )
            transfer_position = normalized_receipt["logs"].index(transfer["log"])

            receipt_id = f"candidate.{ordinal}.{provider_id}.receipt"
            block_id = f"candidate.{ordinal}.{provider_id}.block"
            observations.append(
                _observation(
                    receipt_id,
                    provider_id,
                    "eth_getTransactionReceipt",
                    [transaction_hash],
                    receipt,
                    normalized_receipt,
                )
            )
            observations.append(
                _observation(
                    block_id,
                    provider_id,
                    "eth_getBlockByNumber",
                    [hex(block_number), False],
                    block,
                    normalized_block,
                )
            )
            receipt_rows.append(normalized_receipt)
            block_rows.append(normalized_block)
            receipt_observation_ids.append(receipt_id)
            block_observation_ids.append(block_id)
            settlement_positions.append(settlement_position)
            transfer_positions.append(transfer_position)
            decoded_rows.append(decoded)
            transfer_amounts.append(transfer["amount_base_units"])

        require(len({canonical_hash(row) for row in receipt_rows}) == 1, "providers disagree on settlement receipt")
        require(len({canonical_hash(row) for row in block_rows}) == 1, "providers disagree on settlement block")
        require(len({canonical_hash(row) for row in decoded_rows}) == 1, "providers disagree on decoded settlement")
        require(len(set(transfer_amounts)) == 1, "providers disagree on solver transfer")
        pending_index.append(
            {
                "candidate_key": key,
                "candidate_ordinal": ordinal,
                "status": "canonical_settlement_verified",
                "provider_ids": sorted(assignments),
                "receipt_observation_ids": receipt_observation_ids,
                "block_observation_ids": block_observation_ids,
                "settlement_log_positions": settlement_positions,
                "solver_transfer_log_positions": transfer_positions,
                "decoded_settlement": decoded_rows[0],
            }
        )

    payload = {
        "schema_version": BUNDLE_SCHEMA,
        "bundle_id": f"qicswu-canonical-settlement-capture:{captured_at}",
        "network": NETWORK,
        "chain_id": CHAIN_ID,
        "captured_at": captured_at,
        "evaluation_input_hash": canonical_hash(input_document),
        "provider_registry_body_hash": registry_info["registry_body_hash"],
        "observations": observations,
    }
    payload_hash = canonical_hash(payload)
    index: list[dict[str, Any]] = []
    for row in pending_index:
        settlement_refs = []
        transfer_refs = []
        receipt_refs = []
        block_refs = []
        for provider_id, receipt_id, block_id, settlement_position, transfer_position in zip(
            row["provider_ids"],
            row["receipt_observation_ids"],
            row["block_observation_ids"],
            row["settlement_log_positions"],
            row["solver_transfer_log_positions"],
            strict=True,
        ):
            del provider_id
            receipt_refs.append(evidence_ref(payload_hash, receipt_id, "/result"))
            block_refs.append(evidence_ref(payload_hash, block_id, "/result"))
            settlement_refs.append(
                evidence_ref(payload_hash, receipt_id, f"/result/logs/{settlement_position}")
            )
            transfer_refs.append(
                evidence_ref(payload_hash, receipt_id, f"/result/logs/{transfer_position}")
            )
        index.append(
            {
                "candidate_key": row["candidate_key"],
                "candidate_ordinal": row["candidate_ordinal"],
                "status": row["status"],
                "provider_ids": row["provider_ids"],
                "settlement_receipt_refs": receipt_refs,
                "settlement_block_refs": block_refs,
                "settlement_log_refs": settlement_refs,
                "solver_transfer_refs": transfer_refs,
                "decoded_settlement": row["decoded_settlement"],
            }
        )
    document = {
        **payload,
        "evidence_payload_hash": payload_hash,
        "evidence_index": index,
        "coverage": {
            "candidate_count": len(candidates),
            "canonical_settlement_receipts": True,
            "canonical_settlement_logs": True,
            "canonical_settlement_blocks": True,
            "safe_finality": True,
            "solver_payment_transfers": True,
            "complete_dual_rpc_protocol_streams": False,
            "complete_participation_lifecycle": False,
            "complete_funding_lifecycle": False,
            "dual_rpc_payout_state_views": False,
            "raw_evidence_resolution_complete": False,
        },
        "boundary": (
            "This bundle proves the retained candidates' settlement receipts, logs, block identities, "
            "safe finality, and bounty-to-solver USDC transfers across the registered RPCs. It does not "
            "prove complete protocol-stream acquisition, participation/funding lifecycle, contract-state "
            "payout views, beneficial-principal independence, reimbursement, root lineage, or standalone value."
        ),
    }
    document["artifact_hash"] = artifact_hash(document)
    return document


def verify_bundle(
    bundle: dict[str, Any], registry: dict[str, Any], input_document: dict[str, Any]
) -> dict[str, Any]:
    registry_info = validate_registry(registry)
    require(bundle.get("schema_version") == BUNDLE_SCHEMA, "evidence bundle schema mismatch")
    require(bundle.get("network") == NETWORK and bundle.get("chain_id") == CHAIN_ID, "evidence bundle network mismatch")
    parse_utc(bundle.get("captured_at"), "bundle.captured_at")
    require(bundle.get("artifact_hash") == artifact_hash(bundle), "evidence bundle artifact hash mismatch")
    require(bundle.get("evaluation_input_hash") == canonical_hash(input_document), "evidence bundle input hash mismatch")
    require(
        bundle.get("provider_registry_body_hash") == registry_info["registry_body_hash"],
        "evidence bundle provider registry mismatch",
    )
    payload = {
        key: deepcopy(bundle[key])
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
    require(bundle.get("evidence_payload_hash") == canonical_hash(payload), "evidence payload hash mismatch")
    observations = bundle.get("observations")
    require(isinstance(observations, list), "evidence observations must be an array")
    ids: set[str] = set()
    for row in observations:
        require(isinstance(row, dict), "evidence observation must be an object")
        observation_id = row.get("observation_id")
        require(isinstance(observation_id, str) and OBSERVATION_ID_RE.fullmatch(observation_id) is not None, "invalid observation_id")
        require(observation_id not in ids, f"duplicate observation_id: {observation_id}")
        require(row.get("provider_id") in registry_info["providers"], "observation provider is not registered")
        require(row.get("method") in READ_ONLY_METHODS, "observation uses a non-read-only method")
        require(row.get("result_sha256") == canonical_hash(row.get("result")), "observation result hash mismatch")
        normalized = normalized_observation_result(str(row.get("method")), row.get("result"))
        require(
            row.get("normalized_result_sha256") == canonical_hash(normalized),
            "observation normalized result hash mismatch",
        )
        ids.add(observation_id)

    candidates = input_document.get("candidates")
    require(isinstance(candidates, list), "evaluation input candidates must be an array")
    index = bundle.get("evidence_index")
    require(isinstance(index, list) and len(index) == len(candidates), "evidence index coverage mismatch")
    index_by_key: dict[str, dict[str, Any]] = {}
    for row in index:
        require(isinstance(row, dict), "evidence index row must be an object")
        key = row.get("candidate_key")
        require(isinstance(key, str) and key not in index_by_key, "duplicate or invalid evidence candidate key")
        index_by_key[key] = row

    for candidate in candidates:
        key = candidate_key(candidate)
        require(key in index_by_key, f"candidate evidence missing: {key}")
        row = index_by_key[key]
        require(row.get("status") == "canonical_settlement_verified", "candidate evidence status is invalid")
        provider_ids = row.get("provider_ids")
        require(provider_ids == sorted(registry_info["providers"]), "candidate provider coverage mismatch")
        groups = [
            row.get("settlement_receipt_refs"),
            row.get("settlement_block_refs"),
            row.get("settlement_log_refs"),
            row.get("solver_transfer_refs"),
        ]
        for refs in groups:
            require(isinstance(refs, list) and len(refs) == len(provider_ids), "candidate evidence references are incomplete")
            for reference in refs:
                require(isinstance(reference, str), "candidate evidence reference is invalid")
                resolve_ref(bundle, reference)
        receipt_views = [normalize_receipt(resolve_ref(bundle, ref)) for ref in groups[0]]
        block_views = [normalize_block(resolve_ref(bundle, ref)) for ref in groups[1]]
        settlement_logs = [normalize_log(resolve_ref(bundle, ref)) for ref in groups[2]]
        transfer_logs = [normalize_log(resolve_ref(bundle, ref)) for ref in groups[3]]
        for provider_id, receipt_ref, block_ref, settlement_ref, transfer_ref in zip(
            provider_ids,
            groups[0],
            groups[1],
            groups[2],
            groups[3],
            strict=True,
        ):
            for reference in (receipt_ref, block_ref, settlement_ref, transfer_ref):
                observation_id = EVIDENCE_REF_RE.fullmatch(reference).group(2)
                observation = next(
                    item
                    for item in observations
                    if item.get("observation_id") == observation_id
                )
                require(
                    observation.get("provider_id") == provider_id,
                    "evidence reference provider binding mismatch",
                )
        for receipt, settlement_log, transfer_log in zip(
            receipt_views, settlement_logs, transfer_logs, strict=True
        ):
            require_log_matches_receipt(settlement_log, receipt, "stored settlement log")
            require_log_matches_receipt(transfer_log, receipt, "stored solver transfer log")
        require(len({canonical_hash(value) for value in receipt_views}) == 1, "stored providers disagree on receipt")
        require(len({canonical_hash(value) for value in block_views}) == 1, "stored providers disagree on block")
        require(len({canonical_hash(value) for value in settlement_logs}) == 1, "stored providers disagree on settlement log")
        require(len({canonical_hash(value) for value in transfer_logs}) == 1, "stored providers disagree on transfer log")
        decoded = decode_settlement(settlement_logs[0], str(candidate.get("protocol")))
        require(decoded == row.get("decoded_settlement"), "stored decoded settlement mismatch")
        transfer = solver_transfer(receipt_views[0]["logs"], normalized_address(candidate["bounty_contract"], "candidate bounty"), normalized_address(candidate["solver_wallet"], "candidate solver"))
        require(
            transfer["amount_base_units"]
            == decoded["solver_payout_base_units"] + decoded["returned_bond_base_units"],
            "stored solver transfer does not reconcile",
        )

    coverage = bundle.get("coverage")
    require(isinstance(coverage, dict), "evidence coverage is missing")
    proven_flags = (
        "canonical_settlement_receipts",
        "canonical_settlement_logs",
        "canonical_settlement_blocks",
        "safe_finality",
        "solver_payment_transfers",
    )
    require(all(coverage.get(flag) is True for flag in proven_flags), "settlement evidence coverage is incomplete")
    remaining = []
    if not registry_info["approved"]:
        remaining.append("rpc_provider_registry_approval_pending")
    missing_flags = {
        "complete_dual_rpc_protocol_streams": "complete_dual_rpc_protocol_streams_missing",
        "complete_participation_lifecycle": "complete_participation_lifecycle_missing",
        "complete_funding_lifecycle": "complete_funding_lifecycle_missing",
        "dual_rpc_payout_state_views": "dual_rpc_payout_state_views_missing",
        "raw_evidence_resolution_complete": "raw_evidence_resolution_incomplete",
    }
    remaining.extend(reason for flag, reason in missing_flags.items() if coverage.get(flag) is not True)
    report = {
        "schema_version": REPORT_SCHEMA,
        "bundle_id": bundle.get("bundle_id"),
        "bundle_artifact_hash": bundle.get("artifact_hash"),
        "evidence_payload_hash": bundle.get("evidence_payload_hash"),
        "provider_registry_body_hash": registry_info["registry_body_hash"],
        "provider_registry_approved": registry_info["approved"],
        "candidate_count": len(candidates),
        "canonical_settlements_verified": len(candidates),
        "status": "verified" if not remaining else "verified_partial_blocked",
        "publication_eligible": not remaining,
        "remaining_reason_codes": sorted(set(remaining)),
        "coverage": deepcopy(coverage),
        "boundary": bundle.get("boundary"),
    }
    report["artifact_hash"] = artifact_hash(report)
    return report


def write_immutable_json(path: Path, document: dict[str, Any]) -> None:
    encoded = json.dumps(document, indent=2, sort_keys=True) + "\n"
    if path.exists():
        require(path.read_text(encoding="utf-8") == encoded, f"refusing to overwrite differing artifact: {path}")
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(encoded, encoding="utf-8")


def load_json(path: Path) -> dict[str, Any]:
    value = metric.load_json(path)
    require(isinstance(value, dict), f"{path} must contain a JSON object")
    return value


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    registry_parser = subparsers.add_parser("check-registry")
    registry_parser.add_argument("--registry", type=Path, required=True)

    capture_parser = subparsers.add_parser("capture")
    capture_parser.add_argument("--input", type=Path, required=True)
    capture_parser.add_argument("--registry", type=Path, required=True)
    capture_parser.add_argument("--rpc", action="append", default=[], metavar="PROVIDER_ID=HTTPS_URL")
    capture_parser.add_argument("--captured-at")
    capture_parser.add_argument("--output", type=Path, required=True)

    verify_parser = subparsers.add_parser("verify")
    verify_parser.add_argument("--input", type=Path, required=True)
    verify_parser.add_argument("--registry", type=Path, required=True)
    verify_parser.add_argument("--bundle", type=Path, required=True)
    verify_parser.add_argument("--output", type=Path)

    args = parser.parse_args(argv)
    try:
        registry = load_json(args.registry)
        if args.command == "check-registry":
            result = validate_registry(registry)
            print(json.dumps({"valid": True, **{k: v for k, v in result.items() if k != "providers"}}, sort_keys=True))
            return 0 if result["approved"] else 2
        input_document = load_json(args.input)
        if args.command == "capture":
            registry_info = validate_registry(registry)
            assignments = parse_rpc_assignments(args.rpc, registry_info)
            bundle = capture(
                input_document,
                registry,
                assignments,
                captured_at=args.captured_at,
            )
            write_immutable_json(args.output, bundle)
            report = verify_bundle(bundle, registry, input_document)
            print(json.dumps({"output": str(args.output), **report}, sort_keys=True))
            return 0 if report["publication_eligible"] else 2
        bundle = load_json(args.bundle)
        report = verify_bundle(bundle, registry, input_document)
        if args.output:
            write_immutable_json(args.output, report)
        print(json.dumps(report, sort_keys=True))
        return 0 if report["publication_eligible"] else 2
    except (EvidenceError, OSError, json.JSONDecodeError) as error:
        sys.stderr.write(f"QICSWU evidence verification failed closed: {error}\n")
        return 3


if __name__ == "__main__":
    raise SystemExit(main())
