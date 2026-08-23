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
REQUEST_SCHEMA = "agent-bounties/open-competition-v2-forward-gmv-meta-replenishment-request-v2"
PROFILE_ID = "forward-canonical-gmv-attribution-metric-v2"
GMV_METRIC_PROGRAM_HASH = "0xe1b52ffcfff0675b7dacea84dcabdf3fbcf1cde09b3d2fb55aa389acac5c2ff9"
GMV_JOURNAL_SCHEMA_HASH = "0x660ddc720ea9fc13e7bbdd88839a2ac7b19a124e5daf046518350fa6febe8a40"
GMV_EXECUTION_POLICY_HASH = "0x0f4a13e4bedc6c4e2445c75059153cca12ee4fade502850b661cc2d8a8b2f30a"
GMV_SETTLEMENT_POLICY_HASH = "0xa664183e3688ef42f3c48c0942e5dac1c4108a17b1556c20da4ad05d5e95e8ee"
HASH = re.compile(r"^0x[0-9a-f]{64}$")
ADDRESS = re.compile(r"^0x[0-9a-f]{40}$")
CANDIDATE_ID = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*-v[1-9][0-9]*$")
BASE_REQUIRED_EXCLUDED_WALLETS = [
    "0x1eaa1c68772cf76bc5f4e4174766076e33ace662",
    "0x6fe4d6da2a4371d82b4a7ff94810a94091fb4c35",
    "0x884834e884d6e93462655a2820140ad03e6747bc",
    "0xb358898d34c5e907877a1cd7540b234f6851f61b",
    "0xfb58949365e3a30fd62e86edb0daffccf4ef7477",
    "0xfd7be4c69541ab297aece2a674fc1418b898cc0a",
]
REQUIRED_EXCLUDED_BOUNTY_CONTRACTS = [
    "0x3e052b933628b960d61654a68fca23d869d8989f",
    "0x5f884d4a4cc2727ddbc22382efd776274bc3e7aa",
    "0xaa4a9300bb1c90f93b4048fd83298da6c6145734",
    "0xf8c8897e748e4057d52182c27beb4025f4d49d68",
]


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
        "lifetime_cap_base_units": 77_668_098,
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
        if candidate.get("gmv_lane") != "external_supply":
            raise MaterializeError(f"{field}.gmv_lane must reward external demand")
        title = str(candidate.get("title") or "").strip()
        summary = str(candidate.get("summary") or "").strip()
        if not title or not summary:
            raise MaterializeError(f"{field} title and summary are required")
        epoch = candidate.get("epoch")
        snapshot = candidate.get("snapshot")
        profile = candidate.get("profile_release")
        eligibility = candidate.get("eligibility_policy")
        if (
            not isinstance(epoch, dict)
            or not isinstance(snapshot, dict)
            or not isinstance(profile, dict)
            or not isinstance(eligibility, dict)
        ):
            raise MaterializeError(
                f"{field} epoch, snapshot, profile, and eligibility policy are required"
            )
        reserve_wallet = str(candidate.get("reserve_wallet") or "").lower()
        if (
            not ADDRESS.fullmatch(reserve_wallet)
            or reserve_wallet in BASE_REQUIRED_EXCLUDED_WALLETS
        ):
            raise MaterializeError(f"{field} reserve wallet is invalid")
        required_excluded_wallets = sorted(
            [*BASE_REQUIRED_EXCLUDED_WALLETS, reserve_wallet]
        )
        if eligibility.get("excluded_wallets") != required_excluded_wallets:
            raise MaterializeError(f"{field} operator-wallet exclusions are invalid")
        if (
            eligibility.get("excluded_bounty_contracts")
            != REQUIRED_EXCLUDED_BOUNTY_CONTRACTS
        ):
            raise MaterializeError(f"{field} reward-contract exclusions are invalid")
        minimum_score = require_int(
            epoch.get("minimum_score_base_units"),
            f"{field}.epoch.minimum_score_base_units",
            1,
        )
        if snapshot.get("status") != "scheduled":
            raise MaterializeError(f"{field} forward GMV campaign is not scheduled")
        if profile.get("profile_id") != PROFILE_ID or profile.get("status") != "reviewed":
            raise MaterializeError(f"{field} canonical GMV metric profile is not reviewed")
        expected_profile = {
            "metric_program_hash": GMV_METRIC_PROGRAM_HASH,
            "journal_schema_hash": GMV_JOURNAL_SCHEMA_HASH,
            "execution_policy_hash": GMV_EXECUTION_POLICY_HASH,
            "settlement_policy_hash": GMV_SETTLEMENT_POLICY_HASH,
        }
        for key, expected in expected_profile.items():
            if str(profile.get(key) or "").lower() != expected:
                raise MaterializeError(f"{field}.profile_release.{key} is invalid")
        for key in ("program_vkey", "source_hash", "elf_hash"):
            if not HASH.fullmatch(str(profile.get(key) or "").lower()):
                raise MaterializeError(f"{field}.profile_release.{key} is invalid")
        if not HASH.fullmatch(str(snapshot.get("verification_policy_hash") or "").lower()):
            raise MaterializeError(f"{field}.snapshot.verification_policy_hash is invalid")
        if snapshot.get("snapshot_attesters") != [
            "0x6fe4d6da2a4371d82b4a7ff94810a94091fb4c35",
            "0xfd7be4c69541ab297aece2a674fc1418b898cc0a",
        ] or snapshot.get("snapshot_attestation_threshold") != 2:
            raise MaterializeError(f"{field}.snapshot attester quorum is invalid")
        creations.append(
            {
                "candidate_id": candidate_id,
                "candidate_hash": candidate_hash,
                "title": title,
                "summary": summary,
                "profile_id": PROFILE_ID,
                "profile_release": profile,
                "meta_bounty": {
                    "objective": "highest_external_canonical_gmv",
                    "reserve_wallet": reserve_wallet,
                    "epoch": epoch,
                    "snapshot": snapshot,
                    "score_unit": "usdc_base_units",
                    "attribution": "settlement_gmv_times_entrant_funding_divided_by_total_funding",
                    "excluded_wallets": required_excluded_wallets,
                    "excluded_bounty_contracts": REQUIRED_EXCLUDED_BOUNTY_CONTRACTS,
                    "exclusions": [
                        "operator_or_reserve_wallet funding",
                        "operator_or_reserve_wallet created settlements",
                        "excluded reward contracts",
                        "creator-equals-solver settlements",
                        "entrant-equals-solver settlements",
                        "noncanonical or unconfirmed activity",
                    ],
                },
                "economics": {
                    "solver_reward_base_units": expected_policy["solver_reward_base_units"],
                    "keeper_reward_base_units": expected_policy["keeper_reward_base_units"],
                },
                "settlement": {
                    "winner_mode": "best_score",
                    "proof_system": "plonk",
                    "score_direction": "higher_is_better",
                    "score_threshold_base_units": minimum_score,
                    "tie_break": "earliest qualifying proof sequence",
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
