#!/usr/bin/env python3
"""Fail-closed V4 release and GitHub-environment evidence checks."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import urllib.error
import urllib.request
from typing import Any, Mapping


SCHEMA = "agent-bounties/standing-meta-v4-release-audit-v1"
ENVIRONMENT_SCHEMA = "agent-bounties/standing-meta-v4-environment-evidence-v1"
REPOSITORY_RE = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
ADDRESS_RE = re.compile(r"^0x[0-9a-fA-F]{40}$")
EXPECTED_REPOSITORY = "NSPG13/agent-bounties"
READY_STATUS = "ready_to_earn"
EXPECTED_LATENCY_POLICY_STATUS = "review_frozen"
EXPECTED_LATENCY_POLICY_DECISION = (
    "maximum_response_and_failure_bounds_with_immediate_success_paths_and_symmetric_human_appeals"
)
REQUIRED_ENVIRONMENTS = (
    "standing-meta-v4-sepolia",
    "standing-meta-v4-mainnet",
)
REQUIRED_R4_GATES = (
    "independent_review_complete",
    "base_sepolia_rehearsal_complete",
    "base_mainnet_fork_test_complete",
    "exact_bytecode_evidence_complete",
    "bounded_wallet_policy_review_complete",
    "repository_environment_protection_complete",
)
EXPECTED_CANONICAL_COMPONENTS = (
    "anonymous_protocol_controller",
    "anonymous_stake_pool",
    "verifier_sortition",
    "solver_sortition",
    "appealable_verifier",
    "standing_meta_child_factory",
    "standing_meta_parent_factory",
    "onchain_terms_registry",
    "canonical_independent_child_verifier",
    "standing_meta_v4_bundle",
)
LATENCY_POLICY: dict[str, Any] = {
    "minimum_request_confirmations": 3,
    "random_words": 1,
    "payment": "native",
    "fulfillment_deadline_seconds": 7_200,
    "solver_assignment_seconds": 120,
    "per_bounty_solver_enrollment_seconds": 0,
    "stake_activation_seconds": 604_800,
    "stake_unbonding_seconds": 604_800,
    "primary_response_seconds": 1_800,
    "primary_ranked_backups": 3,
    "appeal_filing_seconds": 14_400,
    "appeal_voting_seconds": 7_200,
    "bounty_verification_seconds": 86_400,
    "fast_path": "immediate_after_vrf_or_waiver_or_decisive_majority",
}


class AuditError(RuntimeError):
    pass


def read_object(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise AuditError(f"expected a JSON object in {path}")
    return value


def write_object(path: Path, value: Mapping[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def utc_now() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def api_get(repository: str, path: str, token: str) -> Any:
    request = urllib.request.Request(
        f"https://api.github.com/repos/{repository}/{path.lstrip('/')}",
        headers={
            "accept": "application/vnd.github+json",
            "authorization": f"Bearer {token}",
            "user-agent": "agent-bounties-v4-release-audit/1",
            "x-github-api-version": "2022-11-28",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", errors="replace")[:500]
        raise AuditError(f"GitHub API {error.code} for {path}: {detail}") from error
    except (urllib.error.URLError, TimeoutError) as error:
        raise AuditError(f"GitHub API unavailable for {path}: {error}") from error


def _reviewer_rows(rule: Mapping[str, Any]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for item in rule.get("reviewers", []):
        if not isinstance(item, dict):
            continue
        reviewer = item.get("reviewer")
        if not isinstance(reviewer, dict):
            continue
        rows.append(
            {
                "type": str(item.get("type", "")),
                "id": reviewer.get("id"),
                "login": reviewer.get("login") or reviewer.get("slug"),
            }
        )
    return rows


def evaluate_environment(
    name: str,
    environment: Mapping[str, Any],
    branch_policies: Mapping[str, Any],
    author_id: int,
) -> dict[str, Any]:
    rules = environment.get("protection_rules", [])
    reviewer_rule = next(
        (
            rule
            for rule in rules
            if isinstance(rule, dict) and rule.get("type") == "required_reviewers"
        ),
        {},
    )
    reviewers = _reviewer_rows(reviewer_rule)
    independent = [
        row
        for row in reviewers
        if row.get("type") == "User" and isinstance(row.get("id"), int) and row["id"] != author_id
    ]
    policies = branch_policies.get("branch_policies", [])
    policy_names = sorted(
        str(item.get("name"))
        for item in policies
        if isinstance(item, dict) and item.get("name")
    )
    branch_config = environment.get("deployment_branch_policy") or {}
    main_only = (
        branch_config.get("custom_branch_policies") is True
        and branch_config.get("protected_branches") is False
        and policy_names == ["main"]
    )
    prevent_self_review = reviewer_rule.get("prevent_self_review") is True
    admin_bypass_disabled = environment.get("can_admins_bypass") is False
    blockers: list[str] = []
    if not main_only:
        blockers.append("deployment branch policy is not exactly main")
    if not reviewers:
        blockers.append("required reviewers are absent")
    if not independent:
        blockers.append("no required user reviewer is independent of the maintainer author")
    if not prevent_self_review:
        blockers.append("self-review prevention is not active")
    if not admin_bypass_disabled:
        blockers.append("environment administrator bypass is enabled")
    return {
        "name": name,
        "main_only": main_only,
        "branch_policies": policy_names,
        "required_reviewers": reviewers,
        "independent_required_reviewers": independent,
        "prevent_self_review": prevent_self_review,
        "admin_bypass_disabled": admin_bypass_disabled,
        "complete": not blockers,
        "blockers": blockers,
    }


def collect_environment_evidence(repository: str, author: str, token: str) -> dict[str, Any]:
    if not REPOSITORY_RE.fullmatch(repository):
        raise AuditError("repository must be owner/name")
    author_payload = api_get(repository, f"collaborators/{author}/permission", token)
    user = author_payload.get("user") if isinstance(author_payload, dict) else None
    if not isinstance(user, dict) or not isinstance(user.get("id"), int):
        raise AuditError("maintainer author is not a direct repository collaborator")
    author_id = int(user["id"])
    environments: dict[str, Any] = {}
    for name in REQUIRED_ENVIRONMENTS:
        environment = api_get(repository, f"environments/{name}", token)
        branch_policies = api_get(repository, f"environments/{name}/deployment-branch-policies", token)
        if not isinstance(environment, dict) or not isinstance(branch_policies, dict):
            raise AuditError(f"malformed environment response for {name}")
        environments[name] = evaluate_environment(name, environment, branch_policies, author_id)
    complete = all(item["complete"] for item in environments.values())
    return {
        "schema": ENVIRONMENT_SCHEMA,
        "repository": repository,
        "maintainer_author": {"login": author, "id": author_id},
        "observed_at": utc_now(),
        "environments": environments,
        "complete": complete,
        "evidence_boundary": (
            "GitHub API read-back of environment policy. This is not independent contract review, "
            "deployment, funding, settlement, or payment evidence."
        ),
    }


def audit_manifest(manifest: Mapping[str, Any], environment_evidence: Mapping[str, Any] | None) -> dict[str, Any]:
    schema_valid = manifest.get("schema") == "agent-bounties/standing-meta-v4-deployment-readiness-v1"
    protocol_valid = manifest.get("protocol_version") == "standing-meta-v4"
    status_valid = manifest.get("status") == READY_STATUS
    latency_status_valid = manifest.get("latency_policy_status") == EXPECTED_LATENCY_POLICY_STATUS
    latency_decision_valid = manifest.get("latency_policy_decision") == EXPECTED_LATENCY_POLICY_DECISION
    configuration = manifest.get("configuration")
    if not isinstance(configuration, dict):
        raise AuditError("manifest configuration is missing")
    latency_mismatches = {
        name: {"expected": expected, "actual": configuration.get(name)}
        for name, expected in LATENCY_POLICY.items()
        if configuration.get(name) != expected
    }
    r4 = manifest.get("r4_evidence")
    if not isinstance(r4, dict):
        raise AuditError("manifest r4_evidence is missing")
    r4_gates = {name: r4.get(name) is True for name in REQUIRED_R4_GATES}
    networks = manifest.get("networks")
    if not isinstance(networks, dict):
        raise AuditError("manifest networks are missing")
    required_components = manifest.get("required_components")
    if (
        not isinstance(required_components, list)
        or len(required_components) != len(EXPECTED_CANONICAL_COMPONENTS)
        or set(required_components) != set(EXPECTED_CANONICAL_COMPONENTS)
    ):
        raise AuditError("manifest required_components does not match the immutable V4 component schema")
    required_component_names = set(required_components)
    network_checks: dict[str, Any] = {}
    for name in ("base-sepolia", "base-mainnet"):
        network = networks.get(name)
        if not isinstance(network, dict):
            raise AuditError(f"manifest network {name} is missing")
        components = network.get("components")
        component_names = set(components) if isinstance(components, dict) else set()
        component_values = components.values() if isinstance(components, dict) else ()
        checks = {
            "subscription_id": isinstance(network.get("subscription_id"), int)
            and int(network["subscription_id"]) > 0,
            "consumers_authorized": network.get("consumers_authorized") is True,
            "native_subscription_reserve_funded": network.get("native_subscription_reserve_funded") is True,
            "minimum_native_subscription_reserve_wei": isinstance(
                network.get("minimum_native_subscription_reserve_wei"), int
            )
            and int(network["minimum_native_subscription_reserve_wei"]) > 0,
            "gas_sponsorship_available": network.get("gas_sponsorship_available") is True,
            "minimum_gas_sponsorship_reserve_wei": isinstance(
                network.get("minimum_gas_sponsorship_reserve_wei"), int
            )
            and int(network["minimum_gas_sponsorship_reserve_wei"]) > 0,
            "exact_component_set": component_names == required_component_names,
            "all_component_addresses_valid": component_names == required_component_names
            and all(
                isinstance(address, str)
                and ADDRESS_RE.fullmatch(address)
                and int(address[2:], 16) != 0
                for address in component_values
            ),
        }
        network_checks[name] = {"checks": checks, "complete": all(checks.values())}
    environment_rows = (
        environment_evidence.get("environments") if isinstance(environment_evidence, Mapping) else None
    )
    environment_complete = (
        isinstance(environment_evidence, Mapping)
        and environment_evidence.get("schema") == ENVIRONMENT_SCHEMA
        and environment_evidence.get("repository") == EXPECTED_REPOSITORY
        and isinstance(environment_rows, Mapping)
        and set(environment_rows) == set(REQUIRED_ENVIRONMENTS)
        and all(
            isinstance(value, Mapping) and value.get("complete") is True
            for value in environment_rows.values()
        )
        and environment_evidence.get("complete") is True
    )
    blockers: list[str] = []
    if not schema_valid:
        blockers.append("manifest schema mismatch")
    if not protocol_valid:
        blockers.append("manifest protocol mismatch")
    if not status_valid:
        blockers.append(f"manifest status is not {READY_STATUS}")
    if not latency_status_valid:
        blockers.append("latency policy is not review-frozen")
    if not latency_decision_valid:
        blockers.append("latency policy decision drift")
    blockers.extend(f"latency policy mismatch: {name}" for name in latency_mismatches)
    blockers.extend(f"R4 gate incomplete: {name}" for name, passed in r4_gates.items() if not passed)
    if not environment_complete:
        blockers.append("live protected-environment evidence is missing or incomplete")
    for network, value in network_checks.items():
        blockers.extend(
            f"{network} incomplete: {name}" for name, passed in value["checks"].items() if not passed
        )
    ready_for_mainnet = (
        schema_valid
        and protocol_valid
        and status_valid
        and latency_status_valid
        and latency_decision_valid
        and not latency_mismatches
        and all(r4_gates.values())
        and environment_complete
        and network_checks["base-sepolia"]["complete"]
        and network_checks["base-mainnet"]["complete"]
    )
    return {
        "schema": SCHEMA,
        "observed_at": utc_now(),
        "protocol_version": manifest.get("protocol_version"),
        "manifest_status": manifest.get("status"),
        "manifest_schema_valid": schema_valid,
        "protocol_version_valid": protocol_valid,
        "manifest_status_valid": status_valid,
        "latency_policy_status": manifest.get("latency_policy_status"),
        "latency_policy_status_valid": latency_status_valid,
        "latency_policy_decision": manifest.get("latency_policy_decision"),
        "latency_policy_decision_valid": latency_decision_valid,
        "latency_policy": LATENCY_POLICY,
        "latency_policy_mismatches": latency_mismatches,
        "r4_gates": r4_gates,
        "environment_evidence_complete": environment_complete,
        "network_readiness": network_checks,
        "ready_for_mainnet": ready_for_mainnet,
        "blockers": blockers,
        "evidence_boundary": (
            "This audit validates declared and read-back release evidence. Only RPC-confirmed contract state and "
            "canonical lifecycle events prove deployment, funding, settlement, or payment."
        ),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    environments = subparsers.add_parser("github-environments")
    environments.add_argument("--repository", default="NSPG13/agent-bounties")
    environments.add_argument("--author", default="NSPG13")
    environments.add_argument("--token-env", default="GH_TOKEN")
    environments.add_argument("--output", type=Path, required=True)
    environments.add_argument("--require-complete", action="store_true")
    manifest = subparsers.add_parser("manifest")
    manifest.add_argument("--manifest", type=Path, default=Path("deployments/standing-meta-v4-config.json"))
    manifest.add_argument("--environment-evidence", type=Path)
    manifest.add_argument("--output", type=Path, required=True)
    manifest.add_argument("--require-mainnet-ready", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        if args.command == "github-environments":
            token = os.environ.get(args.token_env, "").strip()
            if not token:
                raise AuditError(f"{args.token_env} is required")
            report = collect_environment_evidence(args.repository, args.author, token)
            write_object(args.output, report)
            print(f"environment_evidence={args.output} complete={str(report['complete']).lower()}")
            return 2 if args.require_complete and not report["complete"] else 0
        environment_evidence = (
            read_object(args.environment_evidence) if args.environment_evidence is not None else None
        )
        report = audit_manifest(read_object(args.manifest), environment_evidence)
        write_object(args.output, report)
        print(f"release_audit={args.output} ready_for_mainnet={str(report['ready_for_mainnet']).lower()}")
        if report["latency_policy_mismatches"]:
            return 2
        return 2 if args.require_mainnet_ready and not report["ready_for_mainnet"] else 0
    except (AuditError, OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"standing-meta-v4 release audit failed: {error}") from error


if __name__ == "__main__":
    raise SystemExit(main())
