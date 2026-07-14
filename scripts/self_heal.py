#!/usr/bin/env python3
"""Build deterministic, bounded recovery plans for Agent Bounties.

The controller can restore availability and rebuild read models from canonical
evidence. It never signs, funds, verifies, settles, rewrites financial state,
or treats a transaction hash or hosted row as payment evidence.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
import urllib.error
import urllib.request
from copy import deepcopy
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

POLICY_SCHEMA = "agent-bounties/self-healing-policy-v1"
SNAPSHOT_SCHEMA = "agent-bounties/operations-snapshot-v1"
PLAN_SCHEMA = "agent-bounties/recovery-plan-v1"
BENCH_SCHEMA = "agent-bounties/recovery-bench-v1"
ALLOWED_STATES = {"healthy", "degraded", "unavailable", "stale", "unknown"}
CRITICAL_INVARIANTS = {
    "canonical_event_integrity",
    "contract_code_hashes_match",
    "cursor_not_regressed",
    "ledger_conserved",
    "payment_evidence_consistent",
}
CONTAINMENT_ACTIONS = {
    "freeze_value_movement",
    "block_rollout",
    "restore_database",
    "reconcile_from_canonical_chain",
    "investigate_webhook_integrity",
}


class ContractError(ValueError):
    pass


@dataclass(frozen=True)
class RecoveryAction:
    action_id: str
    component: str
    action: str
    risk_class: str
    automatic: bool
    reason: str
    prerequisites: list[str]
    verify: list[str]


@dataclass(frozen=True)
class HttpProbe:
    ok: bool
    status: int | None
    body: str | None
    revision: str | None
    protocol: str | None
    error: str | None


def load_json(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise ContractError(f"{path} must contain a JSON object")
    return data


def require_positive_int(container: dict[str, Any], key: str) -> int:
    value = container.get(key)
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise ContractError(f"{key} must be a positive integer")
    return value


def validate_policy(policy: dict[str, Any]) -> None:
    if policy.get("schema") != POLICY_SCHEMA:
        raise ContractError(f"policy schema must be {POLICY_SCHEMA}")
    require_positive_int(policy, "policy_version")
    probe = policy.get("probe")
    thresholds = policy.get("thresholds")
    budgets = policy.get("automatic_action_budget")
    automatic_actions = policy.get("automatic_actions")
    prohibited = policy.get("prohibited_automatic_actions")
    if not isinstance(probe, dict):
        raise ContractError("policy probe must be an object")
    if not isinstance(thresholds, dict):
        raise ContractError("policy thresholds must be an object")
    if not isinstance(budgets, dict):
        raise ContractError("policy automatic_action_budget must be an object")
    if not isinstance(automatic_actions, dict) or not automatic_actions:
        raise ContractError("policy automatic_actions must be a non-empty object")
    if not isinstance(prohibited, list) or not all(
        isinstance(item, str) and item for item in prohibited
    ):
        raise ContractError("policy prohibited_automatic_actions must be strings")
    if len(prohibited) != len(set(prohibited)):
        raise ContractError("policy prohibited_automatic_actions contains duplicates")
    for key in ("attempts", "interval_seconds", "timeout_seconds"):
        require_positive_int(probe, key)
    for key in (
        "restart_after_consecutive_failures",
        "indexer_stale_after_seconds",
        "indexer_lag_blocks",
        "webhook_backlog_age_seconds",
        "minimum_claimable_bounties",
    ):
        require_positive_int(thresholds, key)
    for name, definition in automatic_actions.items():
        if not isinstance(name, str) or not name:
            raise ContractError("automatic action names must be non-empty strings")
        if not isinstance(definition, dict):
            raise ContractError(f"automatic action {name} must be an object")
        if definition.get("risk_class") not in {"R0", "R1", "R2"}:
            raise ContractError(f"automatic action {name} must be R0, R1, or R2")
        if not isinstance(definition.get("executor"), str):
            raise ContractError(f"automatic action {name} requires an executor")
        requires = definition.get("requires")
        if not isinstance(requires, list) or not all(
            isinstance(item, str) and item for item in requires
        ):
            raise ContractError(f"automatic action {name} requires string prerequisites")
    overlap = set(automatic_actions) & set(prohibited)
    if overlap:
        raise ContractError(
            f"actions cannot be both automatic and prohibited: {sorted(overlap)}"
        )
    if not isinstance(policy.get("evidence_boundary"), str):
        raise ContractError("policy evidence_boundary is required")


def validate_snapshot(snapshot: dict[str, Any]) -> None:
    if snapshot.get("schema") != SNAPSHOT_SCHEMA:
        raise ContractError(f"snapshot schema must be {SNAPSHOT_SCHEMA}")
    if not isinstance(snapshot.get("observed_at"), str) or not snapshot["observed_at"]:
        raise ContractError("snapshot observed_at is required")
    expected_revision = snapshot.get("expected_revision")
    if expected_revision is not None and not isinstance(expected_revision, str):
        raise ContractError("snapshot expected_revision must be a string or null")
    components = snapshot.get("components")
    invariants = snapshot.get("invariants")
    if not isinstance(components, dict):
        raise ContractError("snapshot components must be an object")
    if not isinstance(invariants, dict):
        raise ContractError("snapshot invariants must be an object")
    for name, component in components.items():
        if not isinstance(name, str) or not isinstance(component, dict):
            raise ContractError("snapshot components must map names to objects")
        if component.get("state") not in ALLOWED_STATES:
            raise ContractError(
                f"component {name} state must be one of {sorted(ALLOWED_STATES)}"
            )
        failures = component.get("consecutive_failures", 0)
        if not isinstance(failures, int) or isinstance(failures, bool) or failures < 0:
            raise ContractError(
                f"component {name} consecutive_failures must be a non-negative integer"
            )
    for name, value in invariants.items():
        if value is not None and not isinstance(value, bool):
            raise ContractError(f"invariant {name} must be boolean or null")


def automatic_action(
    policy: dict[str, Any],
    component: str,
    action: str,
    reason: str,
    verify: list[str],
) -> RecoveryAction:
    definition = policy["automatic_actions"].get(action)
    if not isinstance(definition, dict):
        raise ContractError(f"planner requested non-allowlisted automatic action {action}")
    return RecoveryAction(
        action_id=f"{component}:{action}",
        component=component,
        action=action,
        risk_class=definition["risk_class"],
        automatic=True,
        reason=reason,
        prerequisites=list(definition["requires"]),
        verify=verify,
    )


def manual_action(
    component: str,
    action: str,
    reason: str,
    verify: list[str],
    risk_class: str = "R3",
) -> RecoveryAction:
    return RecoveryAction(
        action_id=f"{component}:{action}",
        component=component,
        action=action,
        risk_class=risk_class,
        automatic=False,
        reason=reason,
        prerequisites=["operator_or_precommitted_protocol_authority"],
        verify=verify,
    )


def _add_action(target: list[RecoveryAction], action: RecoveryAction) -> None:
    if action.action_id not in {existing.action_id for existing in target}:
        target.append(action)


def _component(snapshot: dict[str, Any], name: str) -> dict[str, Any] | None:
    component = snapshot["components"].get(name)
    return component if isinstance(component, dict) else None


def evaluate(policy: dict[str, Any], snapshot: dict[str, Any]) -> dict[str, Any]:
    validate_policy(policy)
    validate_snapshot(snapshot)
    automatic: list[RecoveryAction] = []
    manual: list[RecoveryAction] = []
    reasons: list[str] = []
    thresholds = policy["thresholds"]
    invariants = snapshot["invariants"]

    failed_invariants = sorted(
        name
        for name in CRITICAL_INVARIANTS
        if invariants.get(name) is False
    )
    if failed_invariants:
        reasons.append(f"critical invariants failed: {', '.join(failed_invariants)}")
        _add_action(
            manual,
            manual_action(
                "platform",
                "freeze_value_movement",
                "Integrity or accounting evidence is inconsistent; value-changing hosted actions must fail closed.",
                [
                    "canonical chain state is independently reconciled",
                    "ledger conservation and contract hashes pass",
                    "an incident owner records the recovery decision",
                ],
            ),
        )
        _add_action(
            manual,
            manual_action(
                "platform",
                "reconcile_from_canonical_chain",
                "Rebuild observations from one exact safe block without rewriting canonical events.",
                [
                    "factory and implementation runtime hashes match",
                    "event ordering and cursor monotonicity pass",
                    "BountySettled remains the only solver payment evidence",
                ],
            ),
        )

    expected_revision = (snapshot.get("expected_revision") or "").strip()
    observed_revisions: dict[str, str] = {}
    for service_name in ("api", "mcp"):
        component = _component(snapshot, service_name)
        if component and isinstance(component.get("revision"), str):
            observed_revisions[service_name] = component["revision"]
    revision_values = {value for value in observed_revisions.values() if value}
    revision_mismatch = (
        bool(expected_revision)
        and any(value != expected_revision for value in revision_values)
    ) or len(revision_values) > 1
    if revision_mismatch:
        reasons.append("API/MCP deployed revision skew")
        _add_action(
            manual,
            manual_action(
                "release",
                "block_rollout",
                "API and MCP do not agree with the expected immutable release revision.",
                [
                    "API and MCP health headers report the same reviewed commit",
                    "production smoke passes against that exact revision",
                ],
                risk_class="R2",
            ),
        )

    for service_name in ("api", "mcp"):
        component = _component(snapshot, service_name)
        if not component or component["state"] == "healthy":
            continue
        failures = int(component.get("consecutive_failures", 0))
        reasons.append(f"{service_name} is {component['state']}")
        if failed_invariants or revision_mismatch:
            continue
        if failures >= thresholds["restart_after_consecutive_failures"]:
            _add_action(
                automatic,
                automatic_action(
                    policy,
                    service_name,
                    "restart_service",
                    f"{service_name} failed {failures} consecutive probes.",
                    [
                        f"{service_name} /health returns 200 with body ok",
                        "deployed revision is unchanged and matches its peer",
                        "read-only production smoke passes",
                    ],
                ),
            )
        else:
            _add_action(
                automatic,
                automatic_action(
                    policy,
                    service_name,
                    "retry_probe",
                    f"{service_name} has not exhausted the bounded read-only probe budget.",
                    [f"{service_name} /health returns 200 with body ok"],
                ),
            )

    database = _component(snapshot, "database")
    if database and database["state"] in {"degraded", "unavailable", "stale"}:
        reasons.append(f"database is {database['state']}")
        _add_action(
            manual,
            manual_action(
                "database",
                "restore_database",
                "Durable state is unavailable or stale; restarting clients cannot prove data safety.",
                [
                    "database connectivity and migration history pass",
                    "durable lifecycle watermark converges across processes",
                    "backup and point-in-time recovery status are known",
                ],
            ),
        )

    indexer = _component(snapshot, "indexer")
    if indexer:
        heartbeat_age = indexer.get("heartbeat_age_seconds")
        lag_blocks = indexer.get("lag_blocks")
        stale = indexer["state"] in {"degraded", "unavailable", "stale"}
        stale = stale or (
            isinstance(heartbeat_age, int)
            and heartbeat_age > thresholds["indexer_stale_after_seconds"]
        )
        stale = stale or (
            isinstance(lag_blocks, int)
            and lag_blocks > thresholds["indexer_lag_blocks"]
        )
        if stale:
            reasons.append("indexer heartbeat or confirmed cursor is stale")
            cursor_safe = (
                indexer.get("cursor_monotonic") is True
                and invariants.get("canonical_event_integrity") is True
                and invariants.get("cursor_not_regressed") is True
            )
            if cursor_safe and not failed_invariants:
                _add_action(
                    automatic,
                    automatic_action(
                        policy,
                        "indexer",
                        "resume_indexer_from_persisted_cursor",
                        "Indexer liveness failed but its durable cursor and event graph remain monotonic.",
                        [
                            "heartbeat returns to success or skipped",
                            "cursor advances without moving backward",
                            "replayed events remain idempotent",
                        ],
                    ),
                )
            else:
                _add_action(
                    manual,
                    manual_action(
                        "indexer",
                        "investigate_indexer_integrity",
                        "Indexer recovery prerequisites are not proven; replay must not guess a cursor.",
                        [
                            "compare durable cursor to exact safe-block logs",
                            "prove event identity and ordering before resuming",
                        ],
                    ),
                )

    rpc = _component(snapshot, "base_rpc")
    if rpc and rpc["state"] in {"degraded", "unavailable", "stale"}:
        reasons.append(f"Base RPC is {rpc['state']}")
        if rpc.get("attested_failover_available") is True and not failed_invariants:
            _add_action(
                automatic,
                automatic_action(
                    policy,
                    "base_rpc",
                    "switch_to_attested_rpc",
                    "The primary RPC failed and a preconfigured endpoint passed exact-chain attestation.",
                    [
                        "safe block advances",
                        "factory and implementation hashes still match",
                        "indexer cursor resumes monotonically",
                    ],
                ),
            )
        else:
            _add_action(
                manual,
                manual_action(
                    "base_rpc",
                    "attest_rpc_failover",
                    "No failover endpoint has proven chain identity and canonical runtime hashes.",
                    [
                        "chain id and safe block are available",
                        "factory and implementation runtime hashes match",
                    ],
                    risk_class="R2",
                ),
            )

    verifiers = _component(snapshot, "verifier_fleet")
    if verifiers:
        unready = verifiers.get("unready_claimable_count", 0)
        if verifiers["state"] != "healthy" or (isinstance(unready, int) and unready > 0):
            reasons.append("one or more advertised bounties lack verifier readiness")
            _add_action(
                automatic,
                automatic_action(
                    policy,
                    "verifier_fleet",
                    "suppress_unready_inventory",
                    "Unexecutable verification must not be advertised as claimable work.",
                    [
                        "every advertised bounty reports verification_ready=true",
                        "canonical contract state remains unchanged",
                    ],
                ),
            )

    webhooks = _component(snapshot, "stripe_webhooks")
    if webhooks:
        backlog_age = webhooks.get("oldest_pending_age_seconds", 0)
        stale_backlog = (
            webhooks["state"] in {"degraded", "stale"}
            or isinstance(backlog_age, int)
            and backlog_age > thresholds["webhook_backlog_age_seconds"]
        )
        if stale_backlog:
            reasons.append("Stripe webhook reconciliation backlog exceeded its threshold")
            replay_safe = all(
                webhooks.get(key) is True
                for key in (
                    "signature_valid",
                    "event_idempotency_key_present",
                    "amount_binding_valid",
                    "destination_binding_valid",
                )
            )
            if replay_safe and not failed_invariants:
                _add_action(
                    automatic,
                    automatic_action(
                        policy,
                        "stripe_webhooks",
                        "replay_verified_webhook",
                        "The signed event is fully bound and its reconciliation key is idempotent.",
                        [
                            "one ledger credit exists for the Stripe event id",
                            "amount and bounty destination remain unchanged",
                        ],
                    ),
                )
            else:
                _add_action(
                    manual,
                    manual_action(
                        "stripe_webhooks",
                        "investigate_webhook_integrity",
                        "A stale event is missing signature, idempotency, amount, or destination evidence.",
                        [
                            "verified Stripe signature is present",
                            "event id is unique",
                            "amount and destination bindings match",
                        ],
                    ),
                )

    inventory = _component(snapshot, "inventory")
    if inventory:
        claimable_count = inventory.get("claimable_count")
        if isinstance(claimable_count, int) and claimable_count < thresholds[
            "minimum_claimable_bounties"
        ]:
            reasons.append("claimable bounty inventory is below policy minimum")
            _add_action(
                manual,
                manual_action(
                    "inventory",
                    "publish_inventory_alert",
                    "Low inventory is a distribution condition, not authority to spend a wallet.",
                    [
                        "new bounty terms are explicit",
                        "funding is signed by an authorized wallet",
                        "canonical FundingAdded and BountyBecameClaimable events reconcile",
                    ],
                    risk_class="R2",
                ),
            )

    automatic.sort(key=lambda action: action.action_id)
    manual.sort(key=lambda action: action.action_id)
    prohibited = set(policy["prohibited_automatic_actions"])
    bad = sorted(action.action for action in automatic if action.action in prohibited)
    if bad:
        raise ContractError(f"planner emitted prohibited automatic actions: {bad}")

    if any(action.action in CONTAINMENT_ACTIONS for action in manual):
        decision = "contained"
        severity = "critical"
    elif manual:
        decision = "escalation_required"
        severity = "high"
    elif automatic:
        decision = "recovering"
        severity = "medium"
    else:
        decision = "healthy"
        severity = "none"

    return {
        "schema": PLAN_SCHEMA,
        "policy_version": policy["policy_version"],
        "snapshot_observed_at": snapshot["observed_at"],
        "decision": decision,
        "severity": severity,
        "reasons": reasons,
        "automatic_actions": [asdict(action) for action in automatic],
        "manual_actions": [asdict(action) for action in manual],
        "recovery_verification": [
            "repeat the same probes after each action",
            "require API and MCP revision agreement",
            "require monotonic confirmed indexer progress",
            "rerun production smoke",
            "convert every incident into a deterministic recovery fixture",
        ],
        "evidence_boundary": policy["evidence_boundary"],
    }


def normalize_base_url(value: str) -> str:
    raw = value.strip().rstrip("/")
    if not raw:
        raise ContractError("base URL must not be empty")
    if "://" not in raw:
        raw = f"https://{raw}"
    parsed = urlparse(raw)
    if parsed.scheme not in {"http", "https"} or not parsed.hostname:
        raise ContractError(f"invalid HTTP base URL: {value!r}")
    if parsed.scheme == "http" and parsed.hostname not in {"127.0.0.1", "localhost", "::1"}:
        raise ContractError("remote self-healing probes require HTTPS")
    return raw


def fetch_health(url: str, timeout_seconds: int) -> HttpProbe:
    request = urllib.request.Request(
        url,
        method="GET",
        headers={
            "Accept": "text/plain",
            "User-Agent": "agent-bounties-self-heal/1",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            body = response.read(1024).decode("utf-8", errors="replace")
            return HttpProbe(
                ok=response.status == 200 and body.strip() == "ok",
                status=response.status,
                body=body[:200],
                revision=response.headers.get("x-agent-bounties-revision"),
                protocol=response.headers.get("x-agent-bounties-protocol"),
                error=None,
            )
    except urllib.error.HTTPError as error:
        body = error.read(200).decode("utf-8", errors="replace")
        return HttpProbe(False, error.code, body, None, None, f"HTTP {error.code}")
    except (urllib.error.URLError, TimeoutError, OSError) as error:
        return HttpProbe(False, None, None, None, None, str(error))


def observe_health(
    base_url: str, attempts: int, interval_seconds: int, timeout_seconds: int
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    observations: list[HttpProbe] = []
    for attempt in range(attempts):
        result = fetch_health(f"{base_url}/health", timeout_seconds)
        observations.append(result)
        if result.ok:
            break
        if attempt + 1 < attempts:
            time.sleep(interval_seconds)
    last = observations[-1]
    failures = 0 if last.ok else len(observations)
    component = {
        "state": "healthy" if last.ok else "unavailable",
        "consecutive_failures": failures,
        "revision": last.revision,
        "protocol": last.protocol,
        "restartable": True,
    }
    return component, [asdict(observation) for observation in observations]


def build_public_snapshot(
    policy: dict[str, Any], api_url: str, mcp_url: str, expected_revision: str | None
) -> dict[str, Any]:
    probe = policy["probe"]
    api_base = normalize_base_url(api_url)
    mcp_base = normalize_base_url(mcp_url)
    api, api_attempts = observe_health(
        api_base,
        probe["attempts"],
        probe["interval_seconds"],
        probe["timeout_seconds"],
    )
    mcp, mcp_attempts = observe_health(
        mcp_base,
        probe["attempts"],
        probe["interval_seconds"],
        probe["timeout_seconds"],
    )
    return {
        "schema": SNAPSHOT_SCHEMA,
        "observed_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "expected_revision": expected_revision,
        "components": {"api": api, "mcp": mcp},
        "invariants": {name: None for name in sorted(CRITICAL_INVARIANTS)},
        "probe_evidence": {
            "api_url": api_base,
            "mcp_url": mcp_base,
            "api_attempts": api_attempts,
            "mcp_attempts": mcp_attempts,
        },
        "coverage_note": "Public probes cover availability and revision agreement. Durable DB, indexer, chain, verifier, webhook, and accounting signals require trusted runtime observations.",
    }


def write_json(path: Path | None, value: dict[str, Any]) -> None:
    rendered = json.dumps(value, indent=2, sort_keys=True) + "\n"
    if path:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(rendered, encoding="utf-8")
    else:
        print(rendered, end="")


def deep_merge(base: dict[str, Any], patch: dict[str, Any]) -> dict[str, Any]:
    merged = deepcopy(base)
    for key, value in patch.items():
        if isinstance(value, dict) and isinstance(merged.get(key), dict):
            merged[key] = deep_merge(merged[key], value)
        else:
            merged[key] = deepcopy(value)
    return merged


def run_bench(policy: dict[str, Any], fixture_path: Path) -> dict[str, Any]:
    fixture = load_json(fixture_path)
    cases = fixture.get("cases")
    base_snapshot = fixture.get("base_snapshot")
    if not isinstance(cases, list) or not cases:
        raise ContractError("recovery fixture must contain a non-empty cases array")
    if not isinstance(base_snapshot, dict):
        raise ContractError("recovery fixture must contain a base_snapshot object")
    failures: list[dict[str, Any]] = []
    for case in cases:
        if not isinstance(case, dict) or not isinstance(case.get("name"), str):
            raise ContractError("every recovery case requires a name")
        patch = case.get("patch", {})
        if not isinstance(patch, dict):
            raise ContractError(f"case {case['name']} patch must be an object")
        plan = evaluate(policy, deep_merge(base_snapshot, patch))
        expected = case.get("expected")
        if not isinstance(expected, dict):
            raise ContractError(f"case {case['name']} requires expected output")
        observed = {
            "decision": plan["decision"],
            "automatic_action_ids": sorted(
                action["action_id"] for action in plan["automatic_actions"]
            ),
            "manual_action_ids": sorted(
                action["action_id"] for action in plan["manual_actions"]
            ),
        }
        normalized_expected = {
            "decision": expected.get("decision"),
            "automatic_action_ids": sorted(expected.get("automatic_action_ids", [])),
            "manual_action_ids": sorted(expected.get("manual_action_ids", [])),
        }
        forbidden = set(expected.get("forbidden_action_ids", []))
        emitted = set(observed["automatic_action_ids"] + observed["manual_action_ids"])
        if observed != normalized_expected or emitted & forbidden:
            failures.append(
                {
                    "case": case["name"],
                    "expected": normalized_expected,
                    "observed": observed,
                    "forbidden_emitted": sorted(emitted & forbidden),
                }
            )
    passed = len(cases) - len(failures)
    return {
        "schema": BENCH_SCHEMA,
        "policy_version": policy["policy_version"],
        "cases": len(cases),
        "passed": passed,
        "failed": len(failures),
        "score": passed / len(cases),
        "gate": len(failures) == 0,
        "failures": failures,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    validate_parser = subparsers.add_parser("validate-policy")
    validate_parser.add_argument("--policy", type=Path, required=True)

    evaluate_parser = subparsers.add_parser("evaluate")
    evaluate_parser.add_argument("--policy", type=Path, required=True)
    evaluate_parser.add_argument("--snapshot", type=Path, required=True)
    evaluate_parser.add_argument("--output", type=Path)

    observe_parser = subparsers.add_parser("observe")
    observe_parser.add_argument("--policy", type=Path, required=True)
    observe_parser.add_argument("--api-url", required=True)
    observe_parser.add_argument("--mcp-url", required=True)
    observe_parser.add_argument("--expected-revision")
    observe_parser.add_argument("--snapshot-out", type=Path, required=True)
    observe_parser.add_argument("--plan-out", type=Path, required=True)

    bench_parser = subparsers.add_parser("bench")
    bench_parser.add_argument("--policy", type=Path, required=True)
    bench_parser.add_argument("--fixtures", type=Path, required=True)
    bench_parser.add_argument("--output", type=Path)

    args = parser.parse_args(argv)
    try:
        policy = load_json(args.policy)
        validate_policy(policy)
        if args.command == "validate-policy":
            print(f"self-healing policy v{policy['policy_version']} is valid")
            return 0
        if args.command == "evaluate":
            plan = evaluate(policy, load_json(args.snapshot))
            write_json(args.output, plan)
            return 0 if plan["decision"] in {"healthy", "recovering"} else 2
        if args.command == "observe":
            snapshot = build_public_snapshot(
                policy, args.api_url, args.mcp_url, args.expected_revision
            )
            plan = evaluate(policy, snapshot)
            write_json(args.snapshot_out, snapshot)
            write_json(args.plan_out, plan)
            print(json.dumps(plan, indent=2, sort_keys=True))
            return 0 if plan["decision"] in {"healthy", "recovering"} else 2
        if args.command == "bench":
            report = run_bench(policy, args.fixtures)
            write_json(args.output, report)
            return 0 if report["gate"] else 1
    except (ContractError, json.JSONDecodeError, OSError) as error:
        print(f"self-healing contract failed: {error}", file=sys.stderr)
        return 1
    raise AssertionError("unreachable")


if __name__ == "__main__":
    sys.exit(main())
