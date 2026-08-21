#!/usr/bin/env python3
"""Materialize a bounded signer request from a ready replenishment plan.

The output contains deterministic competition terms but no signature or private
ranking inputs. The isolated signer must still revalidate canonical state,
derive nonces and predicted addresses, enforce caps, consume exact allowances,
and persist its idempotency record before broadcasting.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any

PLAN_SCHEMA = "agent-bounties/open-competition-v2-replenishment-plan-v1"
REQUEST_SCHEMA = "agent-bounties/open-competition-v2-replenishment-request-v1"
ARTIFACT_SCHEMA = "agent-bounties/gmv-growth-artifact-v1"
HASH = re.compile(r"^0x[0-9a-f]{64}$")
CANDIDATE_ID = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*-v[1-9][0-9]*$")


class MaterializeError(ValueError):
    pass


def canonical_json(value: object) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def digest(value: object) -> str:
    return "0x" + hashlib.sha256(canonical_json(value).encode("utf-8")).hexdigest()


def require_int(value: object, field: str, minimum: int) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < minimum:
        raise MaterializeError(f"{field} must be an integer >= {minimum}")
    return value


def materialize(plan: object) -> dict[str, Any]:
    if not isinstance(plan, dict) or plan.get("schema_version") != PLAN_SCHEMA:
        raise MaterializeError(f"plan must use {PLAN_SCHEMA}")
    if plan.get("status") != "ready":
        raise MaterializeError("only a ready plan can be materialized")
    plan_key = str(plan.get("idempotency_key") or "").lower()
    if not HASH.fullmatch(plan_key):
        raise MaterializeError("plan idempotency key is invalid")
    inventory = plan.get("inventory")
    policy = plan.get("policy")
    selected = plan.get("selected_candidates")
    if not isinstance(inventory, dict) or not isinstance(policy, dict):
        raise MaterializeError("plan inventory and policy must be objects")
    if not isinstance(selected, list) or not 1 <= len(selected) <= 10:
        raise MaterializeError("ready plan must select 1..10 candidates")
    expected_policy = {
        "solver_reward_base_units": 3_000_000,
        "keeper_reward_base_units": 40_000,
        "per_candidate_base_units": 3_040_000,
        "daily_cap_base_units": 30_400_000,
        "lifetime_cap_base_units": 152_000_000,
    }
    for field, expected in expected_policy.items():
        if policy.get(field) != expected:
            raise MaterializeError(f"policy {field} does not match the bounded signer contract")
    required_spend = require_int(policy.get("required_spend_base_units"), "required_spend_base_units", 1)
    if required_spend != len(selected) * expected_policy["per_candidate_base_units"]:
        raise MaterializeError("required spend does not exactly match selected candidates")
    creations = []
    ids: set[str] = set()
    hashes: set[str] = set()
    for index, candidate in enumerate(selected):
        field = f"selected_candidates[{index}]"
        if not isinstance(candidate, dict):
            raise MaterializeError(f"{field} must be an object")
        candidate_id = str(candidate.get("candidate_id") or "")
        candidate_hash = str(candidate.get("candidate_hash") or "").lower()
        if not CANDIDATE_ID.fullmatch(candidate_id) or candidate_id in ids:
            raise MaterializeError(f"{field}.candidate_id is invalid or duplicated")
        if not HASH.fullmatch(candidate_hash) or candidate_hash in hashes:
            raise MaterializeError(f"{field}.candidate_hash is invalid or duplicated")
        ids.add(candidate_id)
        hashes.add(candidate_hash)
        lane = candidate.get("gmv_lane")
        if lane not in {"external_supply", "acquisition", "retention", "feedback"}:
            raise MaterializeError(f"{field}.gmv_lane is invalid")
        minimum_findings = require_int(candidate.get("minimum_findings"), f"{field}.minimum_findings", 3)
        minimum_recommendations = require_int(
            candidate.get("minimum_recommendations"),
            f"{field}.minimum_recommendations",
            1,
        )
        title = str(candidate.get("title") or "").strip()
        summary = str(candidate.get("summary") or "").strip()
        if not title or not summary:
            raise MaterializeError(f"{field} title and summary are required")
        artifact_template = {
            "schema_version": ARTIFACT_SCHEMA,
            "task_id": candidate_id,
            "gmv_lane": lane,
            "findings": [{"finding": "", "quantitative_source": "https://"}],
            "recommendations": [{"recommendation": "", "gmv_pathway": ""}],
            "user_evidence": [{"kind": "real_user_source", "reference": "https://"}],
        }
        creations.append(
            {
                "candidate_id": candidate_id,
                "candidate_hash": candidate_hash,
                "title": title,
                "summary": summary,
                "profile_id": "structured-artifact-metric-v1",
                "artifact_template": artifact_template,
                "requirements": [
                    {"kind": "json_valid", "weight": 1},
                    {"kind": "maximum_bytes", "maximum": 131_072, "weight": 1},
                    {"kind": "utf8_excludes", "needle": "localhost", "weight": 1},
                    {"kind": "utf8_excludes", "needle": "127.0.0.1", "weight": 1},
                    {
                        "kind": "json_pointer_string_equals",
                        "pointer": "/schema_version",
                        "expected": ARTIFACT_SCHEMA,
                        "weight": 1,
                    },
                    {
                        "kind": "json_pointer_string_equals",
                        "pointer": "/task_id",
                        "expected": candidate_id,
                        "weight": 1,
                    },
                    {
                        "kind": "json_pointer_string_equals",
                        "pointer": "/gmv_lane",
                        "expected": lane,
                        "weight": 1,
                    },
                    {
                        "kind": "json_array_minimum_length",
                        "pointer": "/findings",
                        "minimum": minimum_findings,
                        "weight": 1,
                    },
                    {
                        "kind": "json_array_minimum_length",
                        "pointer": "/recommendations",
                        "minimum": minimum_recommendations,
                        "weight": 1,
                    },
                    {
                        "kind": "json_array_minimum_length",
                        "pointer": "/user_evidence",
                        "minimum": 1,
                        "weight": 1,
                    },
                    {
                        "kind": "json_pointer_exists",
                        "pointer": "/findings/0/finding",
                        "weight": 1,
                    },
                    {
                        "kind": "json_pointer_exists",
                        "pointer": "/findings/0/quantitative_source",
                        "weight": 1,
                    },
                    {
                        "kind": "json_pointer_exists",
                        "pointer": "/recommendations/0/recommendation",
                        "weight": 1,
                    },
                    {
                        "kind": "json_pointer_exists",
                        "pointer": "/recommendations/0/gmv_pathway",
                        "weight": 1,
                    },
                    {
                        "kind": "json_pointer_string_equals",
                        "pointer": "/user_evidence/0/kind",
                        "expected": "real_user_source",
                        "weight": 1,
                    },
                    {
                        "kind": "json_pointer_exists",
                        "pointer": "/user_evidence/0/reference",
                        "weight": 1,
                    },
                    {
                        "kind": "utf8_contains",
                        "needle": "https://",
                        "minimum_occurrences": 2,
                        "weight": 1,
                    },
                ],
                "economics": {
                    "solver_reward_base_units": expected_policy["solver_reward_base_units"],
                    "keeper_reward_base_units": expected_policy["keeper_reward_base_units"],
                },
                "settlement": {
                    "winner_mode": "first_proven",
                    "proof_system": "groth16",
                    "score_direction": "higher_is_better",
                    "payment_evidence": "CompetitionSettledV2",
                },
            }
        )
    request_body = {
        "schema_version": REQUEST_SCHEMA,
        "plan_idempotency_key": plan_key,
        "generated_at": plan.get("generated_at"),
        "canonical_evidence": {
            "safe_block": inventory.get("safe_block"),
            "release_hash": inventory.get("release_hash"),
            "factory_contract": inventory.get("factory_contract"),
            "observed_at": inventory.get("observed_at"),
        },
        "candidate_specs_hash": plan.get("candidate_specs_hash"),
        "policy": policy,
        "creations": creations,
        "signer_revalidation_required": True,
        "authorization_boundary": "This unsigned request cannot authorize spending or prove activation, settlement, GMV, or payment.",
    }
    request_body["request_hash"] = digest(request_body)
    return request_body


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--json-out", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        plan = json.loads(args.plan.read_text(encoding="utf-8-sig"))
        request = materialize(plan)
    except (OSError, json.JSONDecodeError, MaterializeError) as error:
        print(f"materialization blocked: {error}", file=sys.stderr)
        return 2
    args.json_out.parent.mkdir(parents=True, exist_ok=True)
    args.json_out.write_text(json.dumps(request, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"status": "ready", "request_hash": request["request_hash"]}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
