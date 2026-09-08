#!/usr/bin/env python3
"""Capture one complete UTC day for the fail-closed QICSWU production shadow."""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from copy import deepcopy
from datetime import date, datetime, time, timedelta, timezone
from pathlib import Path
from typing import Any, Callable
from urllib.request import Request, urlopen

try:
    from scripts import qicswu_metric as metric
except ImportError:  # Direct invocation adds scripts/ to sys.path.
    import qicswu_metric as metric


ROOT = Path(__file__).resolve().parents[1]
NETWORK = "base-mainnet"
CHAIN_ID = 8453
SOURCE_OBSERVATION_SCHEMA = "agent-bounties/qicswu-shadow-source-observations-v1"
RUN_SCHEMA = "agent-bounties/qicswu-production-shadow-run-v1"
PRIMARY_PATH = "open_competition"
PRIMARY_PROTOCOL = "open_competition_v2"

SOURCE_DEFINITIONS: dict[str, dict[str, Any]] = {
    "autonomous": {
        "url": "https://api.agentbounties.app/v1/base/autonomous-bounties/events?network=base-mainnet",
        "envelope_schema": None,
        "factory_contract": "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9",
        "protocol_version": "agent-bounties/autonomous-v1",
        "settlement_event_kind": "bounty_settled",
        "participation_event_kind": "bounty_claimed",
        "participation_path": "exclusive_claim",
        "protocol_role": "legacy",
        "verifier_reward_field": "verifier_reward",
    },
    "open_competition_v1": {
        "url": "https://api.agentbounties.app/v1/base/open-competition-v1/events?network=base-mainnet",
        "envelope_schema": "agent-bounties/open-competition-v1-events-v1",
        "factory_contract": "0x9e9382beb8b1a45b737d484b5eafa7b8779d4ca5",
        "protocol_version": "agent-bounties/open-competition-v1",
        "settlement_event_kind": "bounty_settled",
        "participation_event_kind": "solution_committed",
        "participation_path": PRIMARY_PATH,
        "protocol_role": "compatibility",
        "verifier_reward_field": "verifier_reward",
    },
    "open_competition_v2": {
        "url": "https://api.agentbounties.app/v1/base/open-competition-v2-beta3/events?network=base-mainnet",
        "envelope_schema": "agent-bounties/open-competition-v2-events-v1",
        "factory_contract": "0x29d0e39e0c03797c690633535722e6b34a69a78a",
        "protocol_version": "agent-bounties/open-competition-v2-beta3",
        "settlement_event_kind": "competition_settled",
        "participation_event_kind": "entry_qualified",
        "participation_path": PRIMARY_PATH,
        "protocol_role": "primary",
        "verifier_reward_field": "keeper_reward",
    },
}


class ShadowCaptureError(ValueError):
    """A production-shadow source or invariant failed closed."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ShadowCaptureError(message)


def canonical_hash(value: Any) -> str:
    return metric.sha256_json(value)


def artifact_hash(value: dict[str, Any]) -> str:
    return canonical_hash({key: row for key, row in value.items() if key != "artifact_hash"})


def parse_utc(value: Any, label: str) -> datetime:
    require(isinstance(value, str), f"{label} must be a UTC timestamp")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise ShadowCaptureError(f"{label} is not a valid timestamp") from error
    require(
        parsed.tzinfo is not None and parsed.utcoffset() == timedelta(0),
        f"{label} must use UTC",
    )
    return parsed.astimezone(timezone.utc)


def utc_text(value: datetime) -> str:
    return value.astimezone(timezone.utc).isoformat().replace("+00:00", "Z")


def positive_integer(value: Any, label: str) -> int:
    require(
        isinstance(value, int) and not isinstance(value, bool) and value > 0,
        f"{label} must be a positive integer",
    )
    return value


def nonnegative_integer(value: Any, label: str) -> int:
    require(
        isinstance(value, int) and not isinstance(value, bool) and value >= 0,
        f"{label} must be a nonnegative integer",
    )
    return value


def fetch_json(url: str, timeout: int = 30) -> tuple[Any, bytes]:
    request = Request(url, headers={"User-Agent": "agent-bounties-qicswu-shadow/1"})
    with urlopen(request, timeout=timeout) as response:  # noqa: S310 - URLs are fixed above.
        raw = response.read()
    try:
        return json.loads(raw), raw
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ShadowCaptureError(f"source returned invalid JSON: {url}") from error


def validate_policy(policy: dict[str, Any]) -> dict[str, dict[str, Any]]:
    require(policy.get("schema_version") == metric.POLICY_SCHEMA, "policy schema mismatch")
    require(policy.get("network") == NETWORK and policy.get("chain_id") == CHAIN_ID, "policy network mismatch")
    priority = policy.get("product_path_priority")
    require(isinstance(priority, dict), "policy product path priority is missing")
    require(
        priority.get("primary_participation_path") == PRIMARY_PATH,
        "Open Competition must remain the primary participation path",
    )
    require(
        priority.get("primary_protocol") == PRIMARY_PROTOCOL,
        "Open Competition V2 must remain the primary protocol",
    )
    expected_open = {"open_competition_v1", "open_competition_v2"}
    require(
        set(priority.get("open_competition_protocols", [])) == expected_open,
        "policy must declare both Open Competition protocols",
    )
    require(
        priority.get("compatibility_protocols") == ["open_competition_v1"],
        "Open Competition V1 must remain the compatibility protocol",
    )

    rows = policy.get("canonical_settlement_protocols")
    require(isinstance(rows, list), "policy protocols are missing")
    by_protocol = {
        row.get("protocol"): row for row in rows if isinstance(row, dict)
    }
    require(
        set(SOURCE_DEFINITIONS) == set(by_protocol),
        "every policy protocol must have an exact production-shadow source",
    )
    for protocol, source in SOURCE_DEFINITIONS.items():
        row = by_protocol[protocol]
        require(
            source["protocol_version"] in row.get("accepted_protocol_versions", []),
            f"{protocol} version is not accepted",
        )
        require(
            source["factory_contract"] in row.get("accepted_factory_contracts", []),
            f"{protocol} current factory is not accepted",
        )
        require(
            source["settlement_event_kind"] in row.get("canonical_settlement_event_kinds", []),
            f"{protocol} settlement event is not accepted",
        )
        require(
            row.get("participation_event_kind") == source["participation_event_kind"],
            f"{protocol} participation event does not match its canonical protocol",
        )
        if source["participation_path"] == PRIMARY_PATH:
            require(
                row.get("participation_event_kind") != "bounty_claimed",
                f"{protocol} must not use the exclusive-claim event",
            )
    require(
        SOURCE_DEFINITIONS[PRIMARY_PROTOCOL]["protocol_role"] == "primary",
        "the primary protocol source must retain its primary role",
    )
    return by_protocol


def extract_events(protocol: str, payload: Any) -> list[dict[str, Any]]:
    expected_schema = SOURCE_DEFINITIONS[protocol]["envelope_schema"]
    if expected_schema is None:
        require(isinstance(payload, list), f"{protocol} source must be an event array")
        events = payload
    else:
        require(isinstance(payload, dict), f"{protocol} source must be an object")
        require(payload.get("schema_version") == expected_schema, f"{protocol} source schema drifted")
        require(payload.get("network") == NETWORK, f"{protocol} source network drifted")
        events = payload.get("events")
        require(isinstance(events, list), f"{protocol} source events are missing")
    require(all(isinstance(row, dict) for row in events), f"{protocol} source contains a malformed event")
    return events


def normalize_candidate(protocol: str, event: dict[str, Any]) -> dict[str, Any]:
    source = SOURCE_DEFINITIONS[protocol]
    require(event.get("kind") == source["settlement_event_kind"], "event is not a settlement")
    if event.get("protocol_version") is not None:
        require(event.get("protocol_version") == source["protocol_version"], f"{protocol} event version drifted")
    data = event.get("data")
    require(isinstance(data, dict), f"{protocol} settlement data is missing")
    reward = positive_integer(data.get("solver_reward"), f"{protocol} solver_reward")
    bonus = nonnegative_integer(data.get("timeout_bond_bonus", 0), f"{protocol} timeout_bond_bonus")
    verifier_reward = positive_integer(
        data.get(source["verifier_reward_field"]),
        f"{protocol} {source['verifier_reward_field']}",
    )
    block_number = positive_integer(event.get("block_number"), f"{protocol} block_number")
    log_index = nonnegative_integer(event.get("log_index"), f"{protocol} log_index")
    round_value = data.get("round") if protocol == "autonomous" else None
    if protocol == "autonomous":
        positive_integer(round_value, "autonomous round")
    return {
        "network": NETWORK,
        "chain_id": CHAIN_ID,
        "protocol": protocol,
        "protocol_version": source["protocol_version"],
        "factory_contract": source["factory_contract"],
        "event_kind": source["settlement_event_kind"],
        "bounty_contract": event.get("contract_address"),
        "bounty_id": event.get("bounty_id"),
        "round": round_value,
        "transaction_hash": event.get("tx_hash"),
        "log_index": log_index,
        "block_number": block_number,
        "block_hash": None,
        "occurred_at": utc_text(parse_utc(event.get("occurred_at"), f"{protocol} occurred_at")),
        "solver_wallet": data.get("solver"),
        "solver_payout_base_units": reward + bonus,
        "promised_reward_principal_base_units": reward,
        "settled_gmv_base_units": reward + bonus + verifier_reward,
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
            "raw_evidence_refs": [f"qicswu-shadow-source:{protocol}:{event.get('tx_hash')}:{log_index}"],
        },
    }


def write_immutable_json(path: Path, value: Any) -> None:
    encoded = json.dumps(value, indent=2, sort_keys=True) + "\n"
    if path.exists():
        require(path.read_text(encoding="utf-8") == encoded, f"refusing to overwrite differing artifact: {path}")
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(encoded, encoding="utf-8")


def capture_day(
    policy: dict[str, Any],
    principals: dict[str, Any],
    root_map: dict[str, Any],
    *,
    day: date,
    captured_at: str,
    output_dir: Path,
    fetcher: Callable[[str], tuple[Any, bytes]] = fetch_json,
) -> dict[str, Any]:
    protocol_policy = validate_policy(policy)
    captured = parse_utc(captured_at, "captured_at")
    started = datetime.combine(day, time.min, tzinfo=timezone.utc)
    ended = started + timedelta(days=1)
    require(captured >= ended, "the shadow window must be a complete UTC day")

    candidates: list[dict[str, Any]] = []
    source_rows: list[dict[str, Any]] = []
    source_coverage: list[dict[str, Any]] = []
    seen_settlements: set[tuple[str, int]] = set()
    for protocol, source in SOURCE_DEFINITIONS.items():
        payload, _raw = fetcher(source["url"])
        events = extract_events(protocol, payload)
        raw_path = output_dir / "sources" / f"{protocol}.json"
        write_immutable_json(raw_path, payload)
        event_kind_counts = Counter(str(row.get("kind")) for row in events)
        selected: list[dict[str, Any]] = []
        for event in events:
            occurred = parse_utc(event.get("occurred_at"), f"{protocol} occurred_at")
            if event.get("kind") != source["settlement_event_kind"] or not (started <= occurred < ended):
                continue
            candidate = normalize_candidate(protocol, event)
            identity = (str(candidate["transaction_hash"]).lower(), candidate["log_index"])
            require(identity not in seen_settlements, "a settlement log was reused across protocol streams")
            seen_settlements.add(identity)
            selected.append(candidate)
        candidates.extend(selected)
        source_rows.append(
            {
                "protocol": protocol,
                "participation_path": source["participation_path"],
                "protocol_role": source["protocol_role"],
                "participation_event_kind": source["participation_event_kind"],
                "endpoint": source["url"],
                "raw_response_path": str(raw_path.relative_to(output_dir)),
                "response_canonical_json_sha256": canonical_hash(payload),
                "response_event_count": len(events),
                "event_kind_counts": dict(sorted(event_kind_counts.items())),
                "window_settlement_candidate_count": len(selected),
            }
        )
        source_coverage.append(
            {
                "protocol": protocol,
                "protocol_role": source["protocol_role"],
                "factory_contracts": [source["factory_contract"]],
                "coverage_started_at": utc_text(started),
                "coverage_ended_at": utc_text(ended),
                "raw_response_retained": True,
                "independent_rpc_observation_count": 1,
                "safe_block_hash": None,
            }
        )

    candidates.sort(
        key=lambda row: (
            row["occurred_at"], row["protocol"], row["transaction_hash"], row["log_index"]
        )
    )
    source_observations = {
        "schema_version": SOURCE_OBSERVATION_SCHEMA,
        "captured_at": captured_at,
        "window": {
            "started_at": utc_text(started),
            "ended_at": utc_text(ended),
            "boundary": "[started_at,ended_at)",
            "complete_utc_days": 1,
        },
        "primary_participation_path": PRIMARY_PATH,
        "primary_protocol": PRIMARY_PROTOCOL,
        "sources": source_rows,
        "boundary": "These are retained production API event-stream observations. They are discovery evidence, not independent RPC, identity, funding, reimbursement, root-work, or settlement qualification evidence.",
    }
    source_observations["artifact_hash"] = artifact_hash(source_observations)
    source_path = output_dir / "source-observations.json"
    write_immutable_json(source_path, source_observations)

    input_document = {
        "schema_version": metric.INPUT_SCHEMA,
        "input_id": f"qicswu-production-shadow-{day.isoformat()}",
        "policy_id": policy["policy_id"],
        "network": NETWORK,
        "chain_id": CHAIN_ID,
        "frozen_at": captured_at,
        "window": deepcopy(source_observations["window"]),
        "source_reconciliation": {
            "status": "production_shadow_qualification_inputs_incomplete",
            "raw_evidence_retained": True,
            "raw_evidence_retention_scope": "All three current production event responses are retained. Historical factory, lifecycle, typed payout, and identity evidence remain separately gated.",
            "reason_codes": [
                "beneficial_principal_evidence_incomplete",
                "candidate_lifecycle_evidence_incomplete",
                "dual_rpc_finality_evidence_incomplete",
                "historical_factory_stream_incomplete",
                "reimbursement_review_incomplete",
                "root_work_review_incomplete",
            ],
            "source_observations_path": "source-observations.json",
            "source_observations_file_sha256": canonical_hash(source_observations),
            "protocols": source_coverage,
        },
        "candidate_set_boundary": {
            "raw_canonical_settlement_rows": len(candidates),
            "public_current_scope_rows": len(candidates),
            "historical_factory_rows": 0,
            "note": "Current production streams are captured without asserting beneficial independence or standalone root-work value.",
        },
        "candidates": candidates,
    }
    input_path = output_dir / "qicswu-input.json"
    write_immutable_json(input_path, input_document)

    result = metric.evaluate(input_document, policy, principals, root_map)
    require(result.get("status") == "unavailable", "shadow metric must remain unavailable while evidence gates are blocked")
    require(result.get("north_star", {}).get("per_day") is None, "shadow metric leaked a numeric north star")
    require(result.get("ledger") is None, "shadow metric leaked its unpublished qualification ledger")
    result_path = output_dir / "qicswu-result.json"
    write_immutable_json(result_path, result)

    path_counts = Counter(source["participation_path"] for source in source_rows)
    role_counts = Counter(source["protocol_role"] for source in source_rows)
    candidate_counts = Counter(row["protocol"] for row in candidates)
    run = {
        "schema_version": RUN_SCHEMA,
        "captured_at": captured_at,
        "window": deepcopy(source_observations["window"]),
        "policy_id": policy["policy_id"],
        "policy_hash": canonical_hash(policy),
        "primary_participation_path": PRIMARY_PATH,
        "primary_protocol": PRIMARY_PROTOCOL,
        "measured_protocols": sorted(protocol_policy),
        "measured_protocol_roles": dict(sorted(role_counts.items())),
        "participation_path_source_counts": dict(sorted(path_counts.items())),
        "observed_settlement_candidates_by_protocol": {
            protocol: candidate_counts.get(protocol, 0) for protocol in sorted(SOURCE_DEFINITIONS)
        },
        "metric": {
            "status": result["status"],
            "north_star_per_day": result["north_star"]["per_day"],
            "result_hash": result["result_hash"],
            "unavailable_reason_codes": result["unavailable_reason_codes"],
        },
        "known_evidence_blockers": [
            "approved_independent_rpc_provider_registry_missing",
            "beneficial_principal_evidence_incomplete",
            "complete_dual_rpc_protocol_streams_missing",
            "complete_funding_lifecycle_missing",
            "complete_participation_lifecycle_missing",
            "dual_rpc_payout_state_views_missing",
            "historical_open_competition_v2_factory_stream_missing",
            "reimbursement_review_incomplete",
            "root_work_and_standalone_value_review_incomplete",
        ],
        "artifacts": {
            "source_observations": {
                "path": "source-observations.json",
                "sha256": canonical_hash(source_observations),
            },
            "evaluation_input": {"path": "qicswu-input.json", "sha256": canonical_hash(input_document)},
            "metric_result": {"path": "qicswu-result.json", "sha256": canonical_hash(result)},
        },
        "publication_eligible": False,
        "boundary": "This internal read-only production shadow identifies Open Competition V2 as the primary new-work protocol, retains V1 for compatibility and autonomous claims as legacy, and measures every supported canonical settlement protocol at equal unit weight. It cannot publish a number, move funds, or authorize a later rollout slice.",
    }
    run["artifact_hash"] = artifact_hash(run)
    write_immutable_json(output_dir / "run-manifest.json", run)
    return run


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--day", help="complete UTC day (defaults to yesterday)")
    parser.add_argument("--captured-at", help="capture time in UTC")
    parser.add_argument("--policy", type=Path, default=ROOT / "ops/metrics/qicswu-policy-v1.json")
    parser.add_argument("--principals", type=Path, default=ROOT / "ops/metrics/qicswu-principal-registry-v1.json")
    parser.add_argument("--root-map", type=Path, default=ROOT / "ops/metrics/qicswu-root-work-map-v1.json")
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args(argv)
    now = datetime.now(timezone.utc)
    try:
        selected_day = date.fromisoformat(args.day) if args.day else now.date() - timedelta(days=1)
        captured_at = args.captured_at or utc_text(now)
        run = capture_day(
            metric.load_json(args.policy),
            metric.load_json(args.principals),
            metric.load_json(args.root_map),
            day=selected_day,
            captured_at=captured_at,
            output_dir=args.output_dir,
        )
        print(json.dumps(run, sort_keys=True))
        return 0
    except (ShadowCaptureError, OSError, json.JSONDecodeError, metric.MetricContractError) as error:
        sys.stderr.write(f"QICSWU production shadow failed closed: {error}\n")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
