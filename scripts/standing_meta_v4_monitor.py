#!/usr/bin/env python3
"""Build a fail-closed, dual-RPC Standing Meta V4 operational snapshot."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import time
from typing import Any, Mapping


DEPLOY_SCRIPT = Path(__file__).with_name("standing_meta_v4_deploy.py")
DEPLOY_SPEC = importlib.util.spec_from_file_location("standing_meta_v4_deploy", DEPLOY_SCRIPT)
assert DEPLOY_SPEC and DEPLOY_SPEC.loader
DEPLOY = importlib.util.module_from_spec(DEPLOY_SPEC)
DEPLOY_SPEC.loader.exec_module(DEPLOY)

ACTIVITY_SCHEMA = "agent-bounties/standing-meta-v4-monitor-activity-v1"
SNAPSHOT_SCHEMA = "agent-bounties/standing-meta-v4-monitor-snapshot-v1"
ADDRESS_RE = re.compile(r"^0x[0-9a-fA-F]{40}$")

RANDOMNESS_REQUESTED = "RandomnessRequested(bytes32,uint256,uint64)"
VERIFICATION_CASE_OPENED = "VerificationCaseOpened(bytes32,address,uint256)"
PRIMARY_ASSIGNED = "PrimaryAssigned(bytes32,address,uint64,uint8)"
PRIMARY_VERDICT = "PrimaryVerdictSubmitted(bytes32,address,bool,bytes32,uint64)"
APPEAL_OPENED = "AppealOpened(bytes32,address,uint256,uint256)"
VERIFICATION_FINALIZED = "VerificationFinalized(bytes32,bool,bool,bool)"
VERIFICATION_TIMED_OUT = "VerificationTimedOut(bytes32,bytes32)"
STAKE_SLASHED = "StakeSlashed(bytes32,address,uint8,uint256,address)"
BOUNTY_SETTLED = (
    "BountySettled(bytes32,uint64,address,uint256,uint256,uint256,uint256,bytes32,bytes32,bytes32,bytes32)"
)
SOLUTION_COMMITTED = "SolutionCommitted(bytes32,address,uint8,bytes32,uint64,uint64,uint256)"
SOLUTION_REVEALED = "SolutionRevealed(bytes32,uint64,address,bytes32,bytes32,bool,bytes32)"
COMPETITION_REJECTED = "CompetitionSubmissionRejected(bytes32,uint64,address,uint256,bytes32)"
COMMITMENT_EXPIRED = "CommitmentExpired(bytes32,address,uint256,uint256)"

REQUEST_STATUS_SIGNATURE = (
    "requestStatus(uint256)(bytes32,bytes32,uint64,uint64,uint8,uint8,bool,bool,bool,uint256)"
)
CASE_STATE_SIGNATURE = "caseState(bytes32)(uint8)"
CASE_TIMING_SIGNATURE = "caseTiming(bytes32)(uint64,uint64,uint64,uint64)"


class MonitorError(RuntimeError):
    pass


def read_object(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise MonitorError(f"expected a JSON object in {path}")
    return value


def write_object(path: Path, value: Mapping[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def utc_now() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def snapshot_sha256(value: Mapping[str, Any]) -> str:
    canonical = dict(value)
    canonical.pop("content_sha256", None)
    payload = json.dumps(canonical, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return "sha256:" + hashlib.sha256(payload).hexdigest()


def _address(value: object, label: str) -> str:
    text = str(value).strip().lower()
    if not ADDRESS_RE.fullmatch(text) or int(text[2:], 16) == 0:
        raise MonitorError(f"{label} is not a nonzero EVM address")
    return text


def _uint(value: object, label: str) -> int:
    try:
        parsed = DEPLOY.parse_uint(value, label)
    except DEPLOY.DeploymentError as error:
        raise MonitorError(str(error)) from error
    if parsed < 0:
        raise MonitorError(f"{label} is negative")
    return parsed


def _bool(value: object, label: str) -> bool:
    try:
        return DEPLOY.parse_bool(value, label)
    except DEPLOY.DeploymentError as error:
        raise MonitorError(str(error)) from error


def _lines(value: str, expected: int, label: str) -> list[str]:
    lines = [line.strip().strip("(),") for line in value.splitlines() if line.strip()]
    if len(lines) != expected:
        raise MonitorError(f"{label} returned {len(lines)} fields; expected {expected}")
    return lines


def _address_array(value: str, label: str) -> list[str]:
    text = value.strip()
    if not (text.startswith("[") and text.endswith("]")):
        raise MonitorError(f"{label} is not an address array")
    body = text[1:-1].strip()
    addresses = [] if not body else [_address(item.strip(), label) for item in body.split(",")]
    if len(addresses) != len(set(addresses)):
        raise MonitorError(f"{label} contains duplicate wallets")
    return addresses


def validate_activity(activity: Mapping[str, Any], network: str) -> dict[str, Any]:
    if activity.get("schema") != ACTIVITY_SCHEMA:
        raise MonitorError("monitor activity schema mismatch")
    if activity.get("network") != network:
        raise MonitorError("monitor activity network mismatch")
    from_block = activity.get("from_block")
    if not isinstance(from_block, int) or isinstance(from_block, bool) or from_block < 0:
        raise MonitorError("monitor activity from_block must be a nonnegative integer")
    normalized: dict[str, Any] = {"from_block": from_block}
    for field in ("standing_meta_parent_canaries", "open_competition_canaries"):
        values = activity.get(field)
        if not isinstance(values, list) or not values:
            raise MonitorError(f"monitor activity {field} must contain at least one address")
        addresses = [_address(value, field) for value in values]
        if len(addresses) != len(set(addresses)):
            raise MonitorError(f"monitor activity {field} contains duplicates")
        normalized[field] = addresses
    normalized["open_competition_factory"] = _address(
        activity.get("open_competition_factory"), "open competition factory"
    )
    return normalized


def _topics(log: Mapping[str, Any], label: str) -> list[str]:
    values = log.get("topics")
    if not isinstance(values, list) or not all(isinstance(item, str) for item in values):
        raise MonitorError(f"{label} topics malformed")
    return [item.lower() for item in values]


def _indexed_uint(log: Mapping[str, Any], index: int, label: str) -> int:
    topics = _topics(log, label)
    if len(topics) <= index:
        raise MonitorError(f"{label} missing indexed field {index}")
    return _uint(topics[index], label)


def _indexed_bytes32(log: Mapping[str, Any], index: int, label: str) -> str:
    topics = _topics(log, label)
    if len(topics) <= index:
        raise MonitorError(f"{label} missing indexed field {index}")
    return DEPLOY.require_bytes32(topics[index], label)


def _data_words(log: Mapping[str, Any], label: str) -> list[int]:
    raw = str(log.get("data", "")).removeprefix("0x")
    if len(raw) % 64:
        raise MonitorError(f"{label} data is not ABI words")
    return [int(raw[index : index + 64], 16) for index in range(0, len(raw), 64)]


def _logs(foundry: Any, address: str, signature: str, from_block: int, to_block: int) -> list[dict[str, Any]]:
    values = foundry.logs(address, signature, from_block, to_block)
    for item in values:
        if _address(item.get("address"), "event address") != address:
            raise MonitorError(f"{signature} log came from a noncanonical address")
    return values


def _subscription_at(
    foundry: Any, coordinator: str, subscription_id: int, block_number: int
) -> dict[str, Any]:
    fields = _lines(
        foundry.call_at(
            coordinator,
            "getSubscription(uint256)(uint96,uint96,uint64,address,address[])",
            block_number,
            str(subscription_id),
        ),
        5,
        "VRF subscription",
    )
    return {
        "link_balance": _uint(fields[0], "subscription LINK balance"),
        "native_balance": _uint(fields[1], "subscription native balance"),
        "request_count": _uint(fields[2], "subscription request count"),
        "owner": _address(fields[3], "subscription owner"),
        "consumers": _address_array(fields[4], "subscription consumers"),
    }


def _request_status(
    foundry: Any, coordinator: str, request_id: int, now: int, block_number: int
) -> dict[str, Any]:
    fields = _lines(
        foundry.call_at(coordinator, REQUEST_STATUS_SIGNATURE, block_number, str(request_id)),
        10,
        "VRF request status",
    )
    requested_at = _uint(fields[2], "VRF requestedAt")
    fulfilled_at = _uint(fields[3], "VRF fulfilledAt")
    fulfilled = _bool(fields[6], "VRF fulfilled")
    late = _bool(fields[7], "VRF late")
    ranking_derived = _bool(fields[8], "VRF rankingDerived")
    if requested_at == 0 or requested_at > now:
        raise MonitorError("tracked VRF request does not exist at the observation block")
    latency = fulfilled_at - requested_at if fulfilled and fulfilled_at >= requested_at else now - requested_at
    ranking_delay = now - fulfilled_at if fulfilled and not ranking_derived and fulfilled_at <= now else 0
    return {
        "request_id": request_id,
        "requested_at": requested_at,
        "fulfilled_at": fulfilled_at,
        "candidate_count": _uint(fields[4], "VRF candidate count"),
        "selection_count": _uint(fields[5], "VRF selection count"),
        "fulfilled": fulfilled,
        "late": late,
        "ranking_derived": ranking_derived,
        "latency_or_pending_age_seconds": latency,
        "ranking_derivation_delay_seconds": ranking_delay,
    }


def _verification_case(
    foundry: Any, verifier: str, case_id: str, now: int, block_number: int
) -> dict[str, Any]:
    state = _uint(
        foundry.call_at(verifier, CASE_STATE_SIGNATURE, block_number, case_id),
        "verification case state",
    )
    timing = _lines(
        foundry.call_at(verifier, CASE_TIMING_SIGNATURE, block_number, case_id),
        4,
        "verification case timing",
    )
    opened_at, primary_deadline, appeal_deadline, vote_deadline = (
        _uint(timing[0], "case openedAt"),
        _uint(timing[1], "case primaryDeadline"),
        _uint(timing[2], "case appealDeadline"),
        _uint(timing[3], "case voteDeadline"),
    )
    relevant_deadline = {2: primary_deadline, 3: appeal_deadline, 5: vote_deadline}.get(state, 0)
    overdue = relevant_deadline > 0 and now > relevant_deadline
    if state == 0 or opened_at == 0:
        raise MonitorError("tracked verification case does not exist")
    return {
        "case_id": case_id,
        "state": state,
        "opened_at": opened_at,
        "primary_deadline": primary_deadline,
        "appeal_deadline": appeal_deadline,
        "vote_deadline": vote_deadline,
        "overdue_action_available": overdue,
    }


def _canary_state(
    foundry: Any,
    canonical: Mapping[str, str],
    activity: Mapping[str, Any],
    from_block: int,
    to_block: int,
) -> dict[str, Any]:
    standing: list[dict[str, Any]] = []
    standing_protocol = foundry.keccak_text("agent-bounties/standing-meta-v4")
    for parent in activity["standing_meta_parent_canaries"]:
        if foundry.code(parent) in {"0x", "0x0"}:
            raise MonitorError(f"standing-meta canary has no code: {parent}")
        factory = _address(
            foundry.call_at(parent, "factory()(address)", to_block),
            "standing-meta parent factory",
        )
        protocol = DEPLOY.require_bytes32(
            foundry.call_at(parent, "protocolVersion()(bytes32)", to_block),
            "standing-meta protocol",
        )
        child = _address(
            foundry.call_at(parent, "preparedChild()(address)", to_block), "prepared child"
        )
        status = _uint(
            foundry.call_at(parent, "status()(uint8)", to_block),
            "standing-meta parent status",
        )
        parent_reward = _uint(
            foundry.call_at(parent, "solverReward()(uint256)", to_block), "parent reward"
        )
        child_target = _uint(
            foundry.call_at(child, "targetAmount()(uint256)", to_block), "child target"
        )
        settlements = _logs(foundry, parent, BOUNTY_SETTLED, from_block, to_block)
        standing.append(
            {
                "parent": parent,
                "child": child,
                "canonical_factory": factory == canonical["standing_meta_parent_factory"],
                "protocol_version_valid": protocol == standing_protocol,
                "status": status,
                "settlement_events": len(settlements),
                "successful_settlement_margin_base_units": parent_reward - child_target
                if parent_reward >= child_target
                else None,
            }
        )

    competitions: list[dict[str, Any]] = []
    competition_protocol = foundry.keccak_text("agent-bounties/open-competition-v1")
    expected_factory = activity["open_competition_factory"]
    if foundry.code(expected_factory) in {"0x", "0x0"}:
        raise MonitorError("open-competition factory has no runtime code")
    for bounty in activity["open_competition_canaries"]:
        if foundry.code(bounty) in {"0x", "0x0"}:
            raise MonitorError(f"open-competition canary has no code: {bounty}")
        factory = _address(
            foundry.call_at(bounty, "factory()(address)", to_block),
            "open-competition factory",
        )
        protocol = DEPLOY.require_bytes32(
            foundry.call_at(bounty, "protocolVersion()(bytes32)", to_block),
            "open-competition protocol",
        )
        status = _uint(
            foundry.call_at(bounty, "status()(uint8)", to_block), "open-competition status"
        )
        commits = _logs(foundry, bounty, SOLUTION_COMMITTED, from_block, to_block)
        reveals = _logs(foundry, bounty, SOLUTION_REVEALED, from_block, to_block)
        rejections = _logs(foundry, bounty, COMPETITION_REJECTED, from_block, to_block)
        expirations = _logs(foundry, bounty, COMMITMENT_EXPIRED, from_block, to_block)
        settlements = _logs(foundry, bounty, BOUNTY_SETTLED, from_block, to_block)
        competitions.append(
            {
                "bounty": bounty,
                "canonical_factory": factory == expected_factory,
                "protocol_version_valid": protocol == competition_protocol,
                "status": status,
                "commit_count": len(commits),
                "reveal_count": len(reveals),
                "invalid_or_expired_count": len(rejections) + len(expirations),
                "settlement_events": len(settlements),
            }
        )
    return {"standing_meta": standing, "open_competition": competitions}


def audit_live_pass(
    foundry: Any,
    deployment: Mapping[str, Any],
    manifest: Mapping[str, Any],
    activity: Mapping[str, Any],
    endpoint_label: str,
    to_block: int,
) -> dict[str, Any]:
    chain_id = foundry.chain_id()
    if chain_id != deployment.get("chain_id"):
        raise MonitorError(f"{endpoint_label} chain does not match deployment evidence")
    network = str(deployment.get("network"))
    policy = manifest.get("monitoring_policy")
    if policy != DEPLOY.EXPECTED_MONITORING_POLICY:
        raise MonitorError("monitoring policy drift")
    network_manifest = manifest.get("networks", {}).get(network)
    if not isinstance(network_manifest, Mapping):
        raise MonitorError(f"manifest is missing {network}")
    minimum_subscription = network_manifest.get("minimum_native_subscription_reserve_wei")
    minimum_gas = network_manifest.get("minimum_gas_sponsorship_reserve_wei")
    if not isinstance(minimum_subscription, int) or minimum_subscription <= 0:
        raise MonitorError("positive minimum native subscription reserve is not configured")
    if not isinstance(minimum_gas, int) or minimum_gas <= 0:
        raise MonitorError("positive minimum gas sponsorship reserve is not configured")
    if activity["from_block"] > to_block:
        raise MonitorError("monitoring from_block is ahead of the common RPC block")

    try:
        deployment_verification = DEPLOY.verify_deployment(foundry, deployment)
    except DEPLOY.DeploymentError as error:
        raise MonitorError(str(error)) from error
    canonical = deployment_verification["canonical_component_addresses"]
    subscription = _subscription_at(
        foundry,
        _address(deployment["vrf_coordinator"], "VRF coordinator"),
        _uint(deployment["subscription_id"], "subscription id"),
        to_block,
    )
    expected_consumers = {
        canonical["verifier_sortition"],
        canonical["solver_sortition"],
    }
    if subscription["owner"] != _address(deployment["deployer"], "deployment keeper"):
        raise MonitorError("subscription owner drift at the common observation block")
    if set(subscription["consumers"]) != expected_consumers or len(subscription["consumers"]) != 2:
        raise MonitorError("subscription consumers drift at the common observation block")
    pool = canonical["anonymous_stake_pool"]
    verifier_wallets = _address_array(
        foundry.call_at(
            pool, "eligibleWallets(uint8,address[])(address[])", to_block, "1", "[]"
        ),
        "eligible verifier wallets",
    )
    solver_wallets = _address_array(
        foundry.call_at(
            pool, "eligibleWallets(uint8,address[])(address[])", to_block, "0", "[]"
        ),
        "eligible solver wallets",
    )
    keeper_balance = foundry.balance_at(
        _address(deployment["deployer"], "deployment keeper"), to_block
    )
    observed_timestamp = foundry.block_timestamp(to_block)

    request_rows: list[dict[str, Any]] = []
    request_ids: set[tuple[str, int]] = set()
    for name in ("verifier_sortition", "solver_sortition"):
        coordinator = canonical[name]
        for log in _logs(foundry, coordinator, RANDOMNESS_REQUESTED, activity["from_block"], to_block):
            request_id = _indexed_uint(log, 2, "RandomnessRequested")
            key = (name, request_id)
            if key in request_ids:
                raise MonitorError("duplicate VRF request event")
            request_ids.add(key)
            row = _request_status(
                foundry, coordinator, request_id, observed_timestamp, to_block
            )
            row["sortition"] = name
            request_rows.append(row)

    verifier = canonical["appealable_verifier"]
    case_rows: list[dict[str, Any]] = []
    case_ids: set[str] = set()
    for log in _logs(foundry, verifier, VERIFICATION_CASE_OPENED, activity["from_block"], to_block):
        case_id = _indexed_bytes32(log, 1, "VerificationCaseOpened")
        if case_id in case_ids:
            raise MonitorError("duplicate verification case event")
        case_ids.add(case_id)
        case_rows.append(
            _verification_case(foundry, verifier, case_id, observed_timestamp, to_block)
        )

    event_counts = {
        "primary_assignments": len(_logs(foundry, verifier, PRIMARY_ASSIGNED, activity["from_block"], to_block)),
        "primary_verdicts": len(_logs(foundry, verifier, PRIMARY_VERDICT, activity["from_block"], to_block)),
        "appeals": len(_logs(foundry, verifier, APPEAL_OPENED, activity["from_block"], to_block)),
        "verification_timeouts": len(
            _logs(foundry, verifier, VERIFICATION_TIMED_OUT, activity["from_block"], to_block)
        ),
        "stake_slashes": len(
            _logs(foundry, pool, STAKE_SLASHED, activity["from_block"], to_block)
        ),
    }
    finalized_logs = _logs(
        foundry, verifier, VERIFICATION_FINALIZED, activity["from_block"], to_block
    )
    overturns = 0
    for log in finalized_logs:
        words = _data_words(log, "VerificationFinalized")
        if len(words) != 3:
            raise MonitorError("VerificationFinalized event has an unexpected shape")
        overturns += int(bool(words[2]))
    event_counts["verification_finalizations"] = len(finalized_logs)
    event_counts["overturns"] = overturns
    assignments = event_counts["primary_assignments"]
    event_counts["assignment_response_rate_ppm"] = (
        min(event_counts["primary_verdicts"], assignments) * 1_000_000 // assignments
        if assignments
        else 0
    )

    canaries = _canary_state(
        foundry,
        canonical,
        activity,
        activity["from_block"],
        to_block,
    )
    settled_standing = [
        item
        for item in canaries["standing_meta"]
        if item["status"] == 4 and item["settlement_events"] == 1
    ]
    settled_competitions = [
        item
        for item in canaries["open_competition"]
        if item["status"] == 2 and item["settlement_events"] == 1
    ]
    max_vrf = policy["maximum_vrf_fulfillment_latency_seconds"]
    max_ranking_delay = manifest["configuration"]["solver_assignment_seconds"]
    checks = {
        "exact_deployment_and_wiring": deployment_verification.get("rpc_confirmed") is True,
        "subscription_reserve": subscription["native_balance"] >= minimum_subscription,
        "keeper_gas_reserve": keeper_balance >= minimum_gas,
        "eligible_verifier_pool": len(verifier_wallets) >= policy["minimum_eligible_verifier_wallets"],
        "eligible_solver_pool": len(solver_wallets) >= policy["minimum_eligible_solver_wallets"],
        "vrf_activity_observed": bool(request_rows),
        "vrf_latency_and_no_late_fulfillment": bool(request_rows)
        and all(
            not item["late"]
            and item["latency_or_pending_age_seconds"] <= max_vrf
            and item["ranking_derivation_delay_seconds"] <= max_ranking_delay
            for item in request_rows
        ),
        "verification_cases_not_overdue": bool(case_rows)
        and all(not item["overdue_action_available"] for item in case_rows),
        "standing_meta_canary_settled": len(settled_standing)
        >= policy["required_standing_meta_canary_settlements"],
        "successful_settlement_margin": bool(settled_standing)
        and all(
            item["canonical_factory"]
            and item["protocol_version_valid"]
            and item["successful_settlement_margin_base_units"]
            >= policy["minimum_successful_settlement_margin_base_units"]
            for item in settled_standing
        ),
        "open_competition_canary_settled": len(settled_competitions)
        >= policy["required_open_competition_canary_settlements"]
        and all(item["canonical_factory"] and item["protocol_version_valid"] for item in settled_competitions),
    }
    blockers = [f"{endpoint_label}: {name}" for name, passed in checks.items() if not passed]
    return {
        "endpoint_label": endpoint_label,
        "observation_block": to_block,
        "observation_block_timestamp": observed_timestamp,
        "checks": checks,
        "healthy": not blockers,
        "blockers": blockers,
        "subscription": subscription,
        "keeper_native_balance_wei": keeper_balance,
        "eligible_verifier_wallet_count": len(verifier_wallets),
        "eligible_solver_wallet_count": len(solver_wallets),
        "vrf_requests": request_rows,
        "verification_cases": case_rows,
        "event_metrics": event_counts,
        "canaries": canaries,
        "deployment_verification": deployment_verification,
    }


def audit_pair(
    primary: Any,
    secondary: Any,
    deployment: Mapping[str, Any],
    manifest: Mapping[str, Any],
    activity: Mapping[str, Any],
) -> dict[str, Any]:
    primary_head = primary.block_number()
    secondary_head = secondary.block_number()
    policy = manifest.get("monitoring_policy")
    if policy != DEPLOY.EXPECTED_MONITORING_POLICY:
        raise MonitorError("monitoring policy drift")
    common_block = min(primary_head, secondary_head)
    passes = [
        audit_live_pass(primary, deployment, manifest, activity, "primary", common_block),
        audit_live_pass(secondary, deployment, manifest, activity, "secondary", common_block),
    ]
    head_difference = abs(primary_head - secondary_head)
    agreement = (
        passes[0]["deployment_verification"]["canonical_component_addresses"]
        == passes[1]["deployment_verification"]["canonical_component_addresses"]
        and passes[0]["subscription"]["owner"] == passes[1]["subscription"]["owner"]
        and set(passes[0]["subscription"]["consumers"])
        == set(passes[1]["subscription"]["consumers"])
        and {
            (item["sortition"], item["request_id"]) for item in passes[0]["vrf_requests"]
        }
        == {
            (item["sortition"], item["request_id"]) for item in passes[1]["vrf_requests"]
        }
        and {item["case_id"] for item in passes[0]["verification_cases"]}
        == {item["case_id"] for item in passes[1]["verification_cases"]}
    )
    blockers = [blocker for item in passes for blocker in item["blockers"]]
    if head_difference > policy["maximum_rpc_head_difference_blocks"]:
        blockers.append("independent RPC head difference exceeds policy")
    if not agreement:
        blockers.append("independent RPC observations disagree")
    healthy = all(item["healthy"] for item in passes) and not blockers
    snapshot: dict[str, Any] = {
        "schema": SNAPSHOT_SCHEMA,
        "network": deployment.get("network"),
        "chain_id": deployment.get("chain_id"),
        "observed_at": utc_now(),
        "observed_at_unix": int(time.time()),
        "common_observation_block": common_block,
        "rpc_head_difference_blocks": head_difference,
        "independent_rpc_agreement": agreement,
        "monitoring_policy": policy,
        "monitoring_active": healthy,
        "ready_to_earn_dependencies_healthy": healthy,
        "earning_suppressed": not healthy,
        "rpc_passes": passes,
        "blockers": blockers,
        "next_action": (
            "Keep monitoring active; publish only the content hash and refresh within five minutes."
            if healthy
            else "Suppress new earning. Investigate the blockers; do not auto-top-up, reroll, deploy, judge, settle, cancel, refund, swap, or withdraw."
        ),
        "evidence_boundary": (
            "This content-addressed dual-RPC snapshot is point-in-time operational evidence. "
            "Only a confirmed canonical BountySettled event proves payment."
        ),
    }
    snapshot["content_sha256"] = snapshot_sha256(snapshot)
    return snapshot


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--network", choices=("base-sepolia", "base-mainnet"), required=True)
    parser.add_argument("--deployment", type=Path, required=True)
    parser.add_argument("--activity", type=Path, required=True)
    parser.add_argument(
        "--manifest", type=Path, default=Path("deployments/standing-meta-v4-config.json")
    )
    parser.add_argument("--rpc-url")
    parser.add_argument("--secondary-rpc-url", required=True)
    parser.add_argument("--forge", default=os.environ.get("FORGE_BIN", "forge"))
    parser.add_argument("--cast", default=os.environ.get("CAST_BIN", "cast"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--require-healthy", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    repo = Path(__file__).resolve().parents[1]
    try:
        deployment = read_object(args.deployment)
        manifest = read_object(args.manifest)
        if deployment.get("network") != args.network:
            raise MonitorError("deployment network does not match --network")
        activity = validate_activity(read_object(args.activity), args.network)
        network = DEPLOY.network_config(int(deployment.get("chain_id", 0)))
        primary_url = args.rpc_url or os.environ.get(
            "BASE_MAINNET_RPC_URL" if args.network == "base-mainnet" else "BASE_SEPOLIA_RPC_URL",
            network["rpc_default"],
        )
        if primary_url.strip() == args.secondary_rpc_url.strip():
            raise MonitorError("primary and secondary RPC endpoints must be distinct")
        primary = DEPLOY.Foundry(repo, primary_url, args.forge, args.cast)
        secondary = DEPLOY.Foundry(repo, args.secondary_rpc_url, args.forge, args.cast)
        snapshot = audit_pair(primary, secondary, deployment, manifest, activity)
        write_object(args.output, snapshot)
        print(
            f"monitor_snapshot={args.output} content_sha256={snapshot['content_sha256']} "
            f"healthy={str(snapshot['monitoring_active']).lower()}"
        )
        return 2 if args.require_healthy and not snapshot["monitoring_active"] else 0
    except (MonitorError, DEPLOY.DeploymentError, OSError, ValueError, json.JSONDecodeError) as error:
        print(f"error: {error}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
