#!/usr/bin/env python3
"""Build a fail-closed, deterministic Open Competition V2 replenishment plan.

The planner never signs, broadcasts, funds, verifies, or settles. It consumes a
private inventory report, a reviewed candidate pool, and the signer's durable
execution ledger. A ready plan is only a request for the separately isolated
signer to revalidate against canonical state and its own caps.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from forward_canonical_gmv import verification_policy_hash as forward_policy_hash

CANDIDATE_SPECS_SCHEMA = "agent-bounties/open-competition-v2-forward-gmv-meta-candidate-specs-v2"
PRIVATE_RANKING_SCHEMA = "agent-bounties/open-competition-v2-private-ranking-v1"
LEDGER_SCHEMA = "agent-bounties/open-competition-v2-replenishment-ledger-v1"
PLAN_SCHEMA = "agent-bounties/open-competition-v2-replenishment-plan-v1"
PROTOCOL_VERSION = "agent-bounties/open-competition-v2-beta3"
PROFILE_ID = "forward-canonical-gmv-attribution-metric-v2"
GMV_METRIC_PROGRAM_HASH = "0xe1b52ffcfff0675b7dacea84dcabdf3fbcf1cde09b3d2fb55aa389acac5c2ff9"
GMV_JOURNAL_SCHEMA_HASH = "0x660ddc720ea9fc13e7bbdd88839a2ac7b19a124e5daf046518350fa6febe8a40"
GMV_EXECUTION_POLICY_HASH = "0x0f4a13e4bedc6c4e2445c75059153cca12ee4fade502850b661cc2d8a8b2f30a"
GMV_SETTLEMENT_POLICY_HASH = "0xa664183e3688ef42f3c48c0942e5dac1c4108a17b1556c20da4ad05d5e95e8ee"
SOLVER_REWARD_BASE_UNITS = 3_000_000
KEEPER_REWARD_BASE_UNITS = 40_000
TOTAL_PER_COMPETITION_BASE_UNITS = 3_040_000
INVENTORY_MAX_AGE_SECONDS = 900
ADDRESS = re.compile(r"^0x[0-9a-f]{40}$")
HASH = re.compile(r"^0x[0-9a-f]{64}$")
CANDIDATE_ID = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*-v[1-9][0-9]*$")
ALLOWED_LANES = {"external_supply", "acquisition", "retention", "feedback"}
ALLOWED_ROLES = {"initial", "standby"}
ALLOWED_FEEDBACK_KINDS = {
    "canonically_correlated_participant",
    "direct_public_comment",
    "documented_contributor_feedback",
    "observable_public_behavior",
}
ALLOWED_ANALYSIS_KINDS = {
    "canonical_platform_metric",
    "canonical_inventory",
    "first_party_analytics",
    "proof_attribution",
}
SPENDING_STATUSES = {"broadcast", "activated"}
RESERVED_STATUSES = {"planned", "broadcast", "activated"}
REQUIRED_EXCLUDED_WALLETS = [
    "0x1eaa1c68772cf76bc5f4e4174766076e33ace662",
    "0x6fe4d6da2a4371d82b4a7ff94810a94091fb4c35",
    "0x884834e884d6e93462655a2820140ad03e6747bc",
    "0xfb58949365e3a30fd62e86edb0daffccf4ef7477",
    "0xfd7be4c69541ab297aece2a674fc1418b898cc0a",
]
REQUIRED_EXCLUDED_BOUNTY_CONTRACTS = [
    "0x3e052b933628b960d61654a68fca23d869d8989f",
    "0x5f884d4a4cc2727ddbc22382efd776274bc3e7aa",
    "0xaa4a9300bb1c90f93b4048fd83298da6c6145734",
    "0xf8c8897e748e4057d52182c27beb4025f4d49d68",
]
WALLET_BOUNDARY = (
    "Known owner, reserve, delegate, deployer, and snapshot-attester wallets are ineligible. "
    "Wallets are not inferred to be unique people."
)
CONTRACT_BOUNDARY = (
    "Declared synthetic canaries and prior GMV reward contracts are ineligible "
    "and must be extended before each future snapshot review."
)
REQUIRED_LIVE_GMV_PROFILE = {
    "profile_id": PROFILE_ID,
    "classification": "reviewed",
    "program_vkey": "0x00ce47a82f13c6f575452683eb7f49c95eeb9891552b6137f924e9fd6a6d8178",
    "source_hash": "0xd180037adb4403f20fa6ca06a974fb67969e95e220ea5ba9093da1f1fef4876f",
    "elf_hash": "0x8351f4bb5a696090d13b1985e5549a749dfb4ca33a475a7d5749a735c377dae5",
    "journal_schema_hash": GMV_JOURNAL_SCHEMA_HASH,
    "metric_program_hash": GMV_METRIC_PROGRAM_HASH,
    "review_evidence_hash": "0x0d679fe190be3e2248953ee5d5f4cb5adfe3a64255404ac752f8caed9862bf86",
}


class PlanError(ValueError):
    pass


def canonical_json(value: object) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def digest(value: object) -> str:
    return "0x" + hashlib.sha256(canonical_json(value).encode("utf-8")).hexdigest()


def load_json(path: Path) -> object:
    return json.loads(path.read_text(encoding="utf-8-sig"))


def parse_timestamp(value: object, field: str) -> datetime:
    try:
        parsed = datetime.fromisoformat(str(value).replace("Z", "+00:00"))
    except ValueError as error:
        raise PlanError(f"{field} must be an ISO-8601 timestamp") from error
    if parsed.tzinfo is None:
        raise PlanError(f"{field} must include a timezone")
    return parsed.astimezone(timezone.utc)


def require_int(value: object, field: str, *, minimum: int = 0) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < minimum:
        raise PlanError(f"{field} must be an integer >= {minimum}")
    return value


def require_text(value: object, field: str, *, maximum: int) -> str:
    text = str(value or "").strip()
    if not text or len(text) > maximum:
        raise PlanError(f"{field} must contain 1..{maximum} characters")
    return text


def validate_source(source: object, field: str, allowed_kinds: set[str] | None = None) -> dict[str, str]:
    if not isinstance(source, dict):
        raise PlanError(f"{field} must be an object")
    url = require_text(source.get("url"), f"{field}.url", maximum=500)
    if not url.startswith("https://") or "@" in url.split("/", 3)[2]:
        raise PlanError(f"{field}.url must be credential-free HTTPS")
    kind = require_text(source.get("kind"), f"{field}.kind", maximum=80)
    if allowed_kinds is not None and kind not in allowed_kinds:
        raise PlanError(f"{field}.kind is not an accepted real-user evidence kind")
    return {"url": url, "kind": kind}


def validate_inventory(report: object, now: datetime, floor: int, target: int) -> dict[str, Any]:
    if not isinstance(report, dict):
        raise PlanError("inventory report must be an object")
    if report.get("inventory_evidence_valid") is not True:
        raise PlanError("inventory evidence is not valid")
    if (
        report.get("private_gmv_meta_floor") != floor
        or report.get("private_gmv_meta_target") != target
    ):
        raise PlanError("inventory floor/target does not match signer policy")
    active = require_int(
        report.get("verified_gmv_meta_competition_count"),
        "verified_gmv_meta_competition_count",
    )
    safe_block = require_int(
        report.get("private_v2_observed_safe_block"),
        "private_v2_observed_safe_block",
        minimum=1,
    )
    release_hash = str(report.get("private_v2_release_hash") or "").lower()
    factory = str(report.get("private_v2_factory_contract") or "").lower()
    live_profile = report.get("private_v2_gmv_profile")
    if not HASH.fullmatch(release_hash):
        raise PlanError("private_v2_release_hash is invalid")
    if not ADDRESS.fullmatch(factory):
        raise PlanError("private_v2_factory_contract is invalid")
    if live_profile != REQUIRED_LIVE_GMV_PROFILE:
        raise PlanError("the exact reviewed canonical GMV profile is not live")
    observed_at = parse_timestamp(
        report.get("private_inventory_observed_at"),
        "private_inventory_observed_at",
    )
    age = (now - observed_at).total_seconds()
    if age < -60 or age > INVENTORY_MAX_AGE_SECONDS:
        raise PlanError("inventory evidence is future-dated or stale")
    return {
        "active": active,
        "floor": floor,
        "target": target,
        "safe_block": safe_block,
        "release_hash": release_hash,
        "factory_contract": factory,
        "live_gmv_profile": dict(REQUIRED_LIVE_GMV_PROFILE),
        "observed_at": observed_at.isoformat().replace("+00:00", "Z"),
    }


def validate_candidate_specs(
    specs: object, now: datetime, inventory: dict[str, Any]
) -> list[dict[str, Any]]:
    if not isinstance(specs, dict) or specs.get("schema_version") != CANDIDATE_SPECS_SCHEMA:
        raise PlanError(f"candidate specs must use {CANDIDATE_SPECS_SCHEMA}")
    if specs.get("protocol_version") != PROTOCOL_VERSION:
        raise PlanError("candidate specs protocol version is invalid")
    profile = specs.get("profile_release")
    if not isinstance(profile, dict) or profile.get("profile_id") != PROFILE_ID:
        raise PlanError("candidate specs do not use the canonical GMV attribution profile")
    profile_status = profile.get("status")
    if profile_status not in {"awaiting_reproduction", "reviewed"}:
        raise PlanError("canonical GMV profile status is invalid")
    if str(profile.get("metric_program_hash") or "").lower() != GMV_METRIC_PROGRAM_HASH:
        raise PlanError("canonical GMV metric program hash is invalid")
    if str(profile.get("journal_schema_hash") or "").lower() != GMV_JOURNAL_SCHEMA_HASH:
        raise PlanError("canonical GMV journal schema hash is invalid")
    if str(profile.get("execution_policy_hash") or "").lower() != GMV_EXECUTION_POLICY_HASH:
        raise PlanError("canonical GMV execution policy hash is invalid")
    if str(profile.get("settlement_policy_hash") or "").lower() != GMV_SETTLEMENT_POLICY_HASH:
        raise PlanError("canonical GMV settlement policy hash is invalid")
    profile_hashes: dict[str, str | None] = {}
    for key in ("program_vkey", "source_hash", "elf_hash"):
        value = profile.get(key)
        if profile_status == "reviewed":
            normalized = str(value or "").lower()
            if not HASH.fullmatch(normalized):
                raise PlanError(f"reviewed canonical GMV profile {key} is invalid")
            profile_hashes[key] = normalized
        else:
            if value is not None:
                raise PlanError(f"unreproduced canonical GMV profile {key} must be null")
            profile_hashes[key] = None
    if profile_status == "reviewed" and any(
        profile_hashes[key] != inventory["live_gmv_profile"][key]
        for key in ("program_vkey", "source_hash", "elf_hash")
    ):
        raise PlanError("candidate specs canonical GMV profile differs from the live release")
    if specs.get("economics") != {
        "solver_reward_base_units": SOLVER_REWARD_BASE_UNITS,
        "keeper_reward_base_units": KEEPER_REWARD_BASE_UNITS,
        "total_per_competition_base_units": TOTAL_PER_COMPETITION_BASE_UNITS,
    }:
        raise PlanError("candidate specs economics do not match the bounded signer policy")
    eligibility = specs.get("eligibility_policy")
    if not isinstance(eligibility, dict):
        raise PlanError("candidate specs eligibility policy is required")
    excluded_wallets = [str(value).lower() for value in eligibility.get("excluded_wallets") or []]
    excluded_contracts = [
        str(value).lower() for value in eligibility.get("excluded_bounty_contracts") or []
    ]
    if excluded_wallets != REQUIRED_EXCLUDED_WALLETS:
        raise PlanError("candidate specs do not exclude every reviewed operator wallet")
    if excluded_contracts != REQUIRED_EXCLUDED_BOUNTY_CONTRACTS:
        raise PlanError("candidate specs do not exclude every reviewed reward contract")
    if eligibility.get("wallet_boundary") != WALLET_BOUNDARY:
        raise PlanError("candidate specs wallet boundary is invalid")
    if eligibility.get("contract_boundary") != CONTRACT_BOUNDARY:
        raise PlanError("candidate specs contract boundary is invalid")
    normalized_eligibility = {
        "excluded_wallets": excluded_wallets,
        "excluded_bounty_contracts": excluded_contracts,
        "wallet_boundary": WALLET_BOUNDARY,
        "contract_boundary": CONTRACT_BOUNDARY,
    }
    if str(specs.get("release_hash") or "").lower() != inventory["release_hash"]:
        raise PlanError("candidate specs release hash does not match canonical inventory")
    if str(specs.get("factory_contract") or "").lower() != inventory["factory_contract"]:
        raise PlanError("candidate specs factory does not match canonical inventory")
    approved_at = parse_timestamp(specs.get("approved_at"), "approved_at")
    expires_at = parse_timestamp(specs.get("expires_at"), "expires_at")
    if approved_at > now or expires_at <= now or approved_at >= expires_at:
        raise PlanError("candidate specs approval window is not current")
    scoring = specs.get("scoring_policy")
    if not isinstance(scoring, dict) or any(
        scoring.get(key) != expected
        for key, expected in {
            "score_unit": "usdc_base_units",
            "winner_mode": "best_score",
            "score_direction": "higher_is_better",
            "attribution": "settlement_gmv_times_entrant_funding_divided_by_total_funding",
            "tie_break": "earliest qualifying proof sequence",
        }.items()
    ):
        raise PlanError("candidate specs do not use the reviewed GMV scoring policy")
    raw = specs.get("candidates")
    if not isinstance(raw, list) or len(raw) != 20:
        raise PlanError("candidate specs must contain exactly twenty reviewed candidates")
    ids: set[str] = set()
    candidates: list[dict[str, Any]] = []
    for index, item in enumerate(raw):
        field = f"candidates[{index}]"
        if not isinstance(item, dict):
            raise PlanError(f"{field} must be an object")
        candidate_id = require_text(item.get("candidate_id"), f"{field}.candidate_id", maximum=100)
        if not CANDIDATE_ID.fullmatch(candidate_id) or candidate_id in ids:
            raise PlanError(f"{field}.candidate_id is invalid or duplicated")
        ids.add(candidate_id)
        lane = item.get("gmv_lane")
        if lane not in ALLOWED_LANES:
            raise PlanError(f"{field}.gmv_lane is invalid")
        epoch = item.get("epoch")
        if not isinstance(epoch, dict):
            raise PlanError(f"{field}.epoch must be an object")
        starts_at = parse_timestamp(epoch.get("starts_at"), f"{field}.epoch.starts_at")
        ends_at = parse_timestamp(epoch.get("ends_at"), f"{field}.epoch.ends_at")
        minimum_score = require_int(
            epoch.get("minimum_score_base_units"),
            f"{field}.epoch.minimum_score_base_units",
            minimum=1,
        )
        epoch_id = str(epoch.get("epoch_id") or "").lower()
        if not HASH.fullmatch(epoch_id):
            raise PlanError(f"{field}.epoch.epoch_id is invalid")
        if starts_at >= ends_at or ends_at <= now:
            raise PlanError(f"{field}.epoch must retain a future scoring interval")
        snapshot = item.get("snapshot")
        if not isinstance(snapshot, dict) or snapshot.get("status") != "scheduled":
            raise PlanError(f"{field}.snapshot must be a scheduled forward snapshot")
        verification_policy = str(snapshot.get("verification_policy_hash") or "").lower()
        attesters = [str(value).lower() for value in snapshot.get("snapshot_attesters") or []]
        threshold = require_int(
            snapshot.get("snapshot_attestation_threshold"),
            f"{field}.snapshot.snapshot_attestation_threshold",
            minimum=2,
        )
        due_after = parse_timestamp(
            snapshot.get("canonical_snapshot_due_after"),
            f"{field}.snapshot.canonical_snapshot_due_after",
        )
        if (
            not HASH.fullmatch(verification_policy)
            or attesters != [
                "0x6fe4d6da2a4371d82b4a7ff94810a94091fb4c35",
                "0xfd7be4c69541ab297aece2a674fc1418b898cc0a",
            ]
            or threshold != 2
            or due_after != ends_at
        ):
            raise PlanError(f"{field}.snapshot forward attestation policy is invalid")
        campaign_policy = {
            "epoch_id": epoch_id,
            "starts_at": int(starts_at.timestamp()),
            "ends_at": int(ends_at.timestamp()),
            "minimum_score_base_units": minimum_score,
            "excluded_wallets": excluded_wallets,
            "excluded_bounty_contracts": excluded_contracts,
            "snapshot_attesters": attesters,
            "snapshot_attestation_threshold": threshold,
        }
        if verification_policy != "0x" + forward_policy_hash(campaign_policy).hex():
            raise PlanError(f"{field}.snapshot verification policy hash is not reproducible")
        normalized_snapshot = {
            "status": "scheduled",
            "verification_policy_hash": verification_policy,
            "snapshot_attesters": attesters,
            "snapshot_attestation_threshold": threshold,
            "canonical_snapshot_due_after": due_after.isoformat().replace("+00:00", "Z"),
        }
        analysis_sources = item.get("analysis_sources")
        feedback_sources = item.get("feedback_sources")
        if not isinstance(analysis_sources, list) or not analysis_sources:
            raise PlanError(f"{field} requires quantitative analysis evidence")
        if not isinstance(feedback_sources, list) or not feedback_sources:
            raise PlanError(f"{field} requires real-user evidence")
        analysis = [
            validate_source(value, f"{field}.analysis_sources", ALLOWED_ANALYSIS_KINDS)
            for value in analysis_sources
        ]
        feedback = [
            validate_source(value, f"{field}.feedback_sources", ALLOWED_FEEDBACK_KINDS)
            for value in feedback_sources
        ]
        normalized = {
            "candidate_id": candidate_id,
            "title": require_text(item.get("title"), f"{field}.title", maximum=160),
            "summary": require_text(item.get("summary"), f"{field}.summary", maximum=500),
            "gmv_lane": lane,
            "epoch": {
                "starts_at": starts_at.isoformat().replace("+00:00", "Z"),
                "ends_at": ends_at.isoformat().replace("+00:00", "Z"),
                "minimum_score_base_units": minimum_score,
                "epoch_id": epoch_id,
            },
            "snapshot": normalized_snapshot,
            "profile_release": {
                "profile_id": PROFILE_ID,
                "status": profile_status,
                "metric_program_hash": GMV_METRIC_PROGRAM_HASH,
                "journal_schema_hash": GMV_JOURNAL_SCHEMA_HASH,
                "execution_policy_hash": GMV_EXECUTION_POLICY_HASH,
                "settlement_policy_hash": GMV_SETTLEMENT_POLICY_HASH,
                **profile_hashes,
            },
            "eligibility_policy": normalized_eligibility,
            "analysis_sources": analysis,
            "feedback_sources": feedback,
        }
        normalized["candidate_spec_hash"] = digest(normalized)
        candidates.append(normalized)
    return candidates


def validate_private_ranking(
    ranking: object, candidate_specs: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    if not isinstance(ranking, dict) or ranking.get("schema_version") != PRIVATE_RANKING_SCHEMA:
        raise PlanError(f"private ranking must use {PRIVATE_RANKING_SCHEMA}")
    if ranking.get("ranking_weights") != {
        "real_user_evidence": 50,
        "gmv_impact": 30,
        "evidence_quality": 20,
    }:
        raise PlanError("private ranking weights are not the reviewed 50/30/20 policy")
    raw = ranking.get("ranked_candidates")
    if not isinstance(raw, list) or len(raw) != 20:
        raise PlanError("private ranking must contain exactly twenty candidates")
    specs_by_id = {item["candidate_id"]: item for item in candidate_specs}
    ranked_ids: set[str] = set()
    candidates: list[dict[str, Any]] = []
    for index, item in enumerate(raw):
        field = f"ranked_candidates[{index}]"
        if not isinstance(item, dict):
            raise PlanError(f"{field} must be an object")
        candidate_id = require_text(item.get("candidate_id"), f"{field}.candidate_id", maximum=100)
        if candidate_id not in specs_by_id or candidate_id in ranked_ids:
            raise PlanError(f"{field}.candidate_id is missing from specs or duplicated")
        ranked_ids.add(candidate_id)
        role = item.get("launch_role")
        if role not in ALLOWED_ROLES:
            raise PlanError(f"{field}.launch_role is invalid")
        scores = item.get("scores")
        if not isinstance(scores, dict):
            raise PlanError(f"{field}.scores must be an object")
        feedback_score = require_int(
            scores.get("real_user_evidence"), f"{field}.scores.real_user_evidence"
        )
        gmv_score = require_int(scores.get("gmv_impact"), f"{field}.scores.gmv_impact")
        quality_score = require_int(
            scores.get("evidence_quality"), f"{field}.scores.evidence_quality"
        )
        if max(feedback_score, gmv_score, quality_score) > 100:
            raise PlanError(f"{field}.scores values must be <= 100")
        normalized = dict(specs_by_id[candidate_id])
        normalized.update(
            {
                "launch_role": role,
                "scores": {
                    "real_user_evidence": feedback_score,
                    "gmv_impact": gmv_score,
                    "evidence_quality": quality_score,
                },
                "weighted_score": feedback_score * 50 + gmv_score * 30 + quality_score * 20,
            }
        )
        normalized["candidate_hash"] = digest(normalized)
        candidates.append(normalized)
    if ranked_ids != set(specs_by_id):
        raise PlanError("private ranking candidate set does not exactly match reviewed specs")
    initial = sum(value["launch_role"] == "initial" for value in candidates)
    standby = sum(value["launch_role"] == "standby" for value in candidates)
    if initial != 10 or standby != 10:
        raise PlanError("private ranking must contain ten initial and ten standby candidates")
    return candidates


def validate_ledger(
    ledger: object, now: datetime, per_candidate_base_units: int
) -> tuple[list[dict[str, Any]], set[str], int, int]:
    if not isinstance(ledger, dict) or ledger.get("schema_version") != LEDGER_SCHEMA:
        raise PlanError(f"execution ledger must use {LEDGER_SCHEMA}")
    entries = ledger.get("executions")
    if not isinstance(entries, list):
        raise PlanError("execution ledger executions must be an array")
    seen_keys: set[str] = set()
    reserved: set[str] = set()
    daily_spent = 0
    lifetime_spent = 0
    normalized: list[dict[str, Any]] = []
    for index, item in enumerate(entries):
        field = f"executions[{index}]"
        if not isinstance(item, dict):
            raise PlanError(f"{field} must be an object")
        key = str(item.get("idempotency_key") or "").lower()
        candidate_id = str(item.get("candidate_id") or "")
        status = str(item.get("status") or "")
        if not HASH.fullmatch(key) or key in seen_keys:
            raise PlanError(f"{field}.idempotency_key is invalid or duplicated")
        if not CANDIDATE_ID.fullmatch(candidate_id):
            raise PlanError(f"{field}.candidate_id is invalid")
        if status not in RESERVED_STATUSES | {"rejected"}:
            raise PlanError(f"{field}.status is invalid")
        occurred_at = parse_timestamp(item.get("occurred_at"), f"{field}.occurred_at")
        if occurred_at > now:
            raise PlanError(f"{field}.occurred_at is future-dated")
        amount = require_int(item.get("amount_base_units"), f"{field}.amount_base_units")
        expected_amount = per_candidate_base_units if status in RESERVED_STATUSES else 0
        if amount != expected_amount:
            raise PlanError(f"{field}.amount_base_units does not match the exact policy amount")
        seen_keys.add(key)
        if status in RESERVED_STATUSES:
            if candidate_id in reserved:
                raise PlanError("a candidate is reserved more than once")
            reserved.add(candidate_id)
        if status in SPENDING_STATUSES:
            lifetime_spent += amount
            if occurred_at.date() == now.date():
                daily_spent += amount
        normalized.append(
            {
                "idempotency_key": key,
                "candidate_id": candidate_id,
                "status": status,
                "occurred_at": occurred_at.isoformat().replace("+00:00", "Z"),
                "amount_base_units": amount,
            }
        )
    return normalized, reserved, daily_spent, lifetime_spent


def blocked_plan(
    now: datetime | None,
    blockers: list[str],
    inventory: dict[str, Any] | None = None,
) -> dict[str, Any]:
    return {
        "schema_version": PLAN_SCHEMA,
        "generated_at": now.isoformat().replace("+00:00", "Z") if now else None,
        "status": "blocked",
        "severity": "critical" if inventory and inventory["active"] < inventory["floor"] else "warning",
        "inventory": inventory,
        "selected_candidates": [],
        "blockers": blockers,
        "evidence_boundary": "This plan cannot sign, approve, broadcast, fund, verify, settle, or prove payment.",
    }


def build_plan(
    inventory_report: object,
    candidate_specs: object,
    private_ranking: object,
    ledger: object,
    *,
    now: datetime,
    floor: int = 5,
    target: int = 10,
    per_candidate_base_units: int = TOTAL_PER_COMPETITION_BASE_UNITS,
    daily_cap_base_units: int = 30_400_000,
    lifetime_cap_base_units: int = 77_668_098,
) -> dict[str, Any]:
    require_int(floor, "floor", minimum=1)
    require_int(target, "target", minimum=floor)
    require_int(per_candidate_base_units, "per_candidate_base_units", minimum=1)
    require_int(daily_cap_base_units, "daily_cap_base_units", minimum=per_candidate_base_units)
    require_int(lifetime_cap_base_units, "lifetime_cap_base_units", minimum=daily_cap_base_units)
    inventory: dict[str, Any] | None = None
    try:
        inventory = validate_inventory(inventory_report, now, floor, target)
        specs = validate_candidate_specs(candidate_specs, now, inventory)
        candidates = validate_private_ranking(private_ranking, specs)
        executions, reserved, daily_spent, lifetime_spent = validate_ledger(
            ledger, now, per_candidate_base_units
        )
    except PlanError as error:
        return blocked_plan(now, [str(error)], inventory)

    pending = [entry for entry in executions if entry["status"] in {"planned", "broadcast"}]
    if pending:
        return blocked_plan(
            now,
            ["unreconciled signer executions must reach canonical activation or rejection before another plan"],
            inventory,
        )

    deficit = max(0, target - inventory["active"])
    if deficit == 0:
        return {
            "schema_version": PLAN_SCHEMA,
            "generated_at": now.isoformat().replace("+00:00", "Z"),
            "status": "noop",
            "severity": "none",
            "inventory": inventory,
            "selected_candidates": [],
            "blockers": [],
            "evidence_boundary": "Canonical inventory is already at or above the private target; no value-changing action was planned.",
        }

    unreserved = [candidate for candidate in candidates if candidate["candidate_id"] not in reserved]
    available = [
        candidate
        for candidate in unreserved
        if candidate["profile_release"]["status"] == "reviewed"
        and candidate["snapshot"]["status"] == "scheduled"
    ]
    available.sort(
        key=lambda candidate: (
            0 if candidate["launch_role"] == "initial" else 1,
            -candidate["weighted_score"],
            candidate["candidate_id"],
        )
    )
    required_spend = deficit * per_candidate_base_units
    blockers: list[str] = []
    if any(candidate["profile_release"]["status"] != "reviewed" for candidate in unreserved):
        blockers.append("canonical GMV metric profile has not completed independent reproduction and review")
    if len(available) < deficit:
        blockers.append(
            "reviewed pool has fewer unused, precommitted forward GMV campaigns than the full target deficit"
        )
    if daily_spent + required_spend > daily_cap_base_units:
        blockers.append("full target restoration would exceed the UTC-day spending cap")
    if lifetime_spent + required_spend > lifetime_cap_base_units:
        blockers.append("full target restoration would exceed the lifetime spending cap")
    if blockers:
        return blocked_plan(now, blockers, inventory)

    selected = available[:deficit]
    policy = {
        "solver_reward_base_units": SOLVER_REWARD_BASE_UNITS,
        "keeper_reward_base_units": KEEPER_REWARD_BASE_UNITS,
        "per_candidate_base_units": per_candidate_base_units,
        "daily_cap_base_units": daily_cap_base_units,
        "lifetime_cap_base_units": lifetime_cap_base_units,
        "daily_spent_before_plan": daily_spent,
        "lifetime_spent_before_plan": lifetime_spent,
        "required_spend_base_units": required_spend,
        "utc_day": now.date().isoformat(),
    }
    decision_inputs = {
        "inventory": inventory,
        "candidate_specs_hash": digest(candidate_specs),
        "private_ranking_hash": digest(private_ranking),
        "ledger_hash": digest(executions),
        "selected_candidate_hashes": [candidate["candidate_hash"] for candidate in selected],
        "policy": policy,
        "generated_at": now.isoformat().replace("+00:00", "Z"),
    }
    idempotency_key = digest(decision_inputs)
    return {
        "schema_version": PLAN_SCHEMA,
        "generated_at": decision_inputs["generated_at"],
        "status": "ready",
        "severity": "critical" if inventory["active"] < inventory["floor"] else "warning",
        "idempotency_key": idempotency_key,
        "inventory": inventory,
        "policy": policy,
        "candidate_specs_hash": decision_inputs["candidate_specs_hash"],
        "private_ranking_hash": decision_inputs["private_ranking_hash"],
        "ledger_hash": decision_inputs["ledger_hash"],
        "selected_candidates": selected,
        "blockers": [],
        "signer_requirements": {
            "revalidate_canonical_safe_block": True,
            "revalidate_release_and_factory": True,
            "revalidate_gmv_profile_and_snapshot_hashes": True,
            "revalidate_best_score_meta_terms": True,
            "revalidate_unused_candidate_ids": True,
            "revalidate_daily_and_lifetime_caps": True,
            "allowed_calls": ["usdc.approve_exact", "factory.create_competition_v2"],
        },
        "evidence_boundary": "This content-addressed plan is advisory until the isolated signer independently revalidates it. A transaction hash is not activation, funding, settlement, GMV, or payment evidence.",
    }


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inventory-private", type=Path, required=True)
    parser.add_argument("--candidate-specs", type=Path, required=True)
    parser.add_argument("--private-ranking", type=Path, required=True)
    parser.add_argument("--execution-ledger", type=Path, required=True)
    parser.add_argument("--now", required=True, help="Stable ISO-8601 decision cutoff")
    parser.add_argument("--json-out", type=Path)
    parser.add_argument("--fail-blocked", action="store_true")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        now = parse_timestamp(args.now, "now")
    except PlanError as error:
        plan = blocked_plan(None, [str(error)])
    else:
        try:
            plan = build_plan(
                load_json(args.inventory_private),
                load_json(args.candidate_specs),
                load_json(args.private_ranking),
                load_json(args.execution_ledger),
                now=now,
            )
        except (OSError, json.JSONDecodeError, PlanError) as error:
            plan = blocked_plan(now, [str(error)])
    output = json.dumps(plan, indent=2) + "\n"
    print(output, end="")
    if args.json_out:
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(output, encoding="utf-8")
    return 2 if args.fail_blocked and plan["status"] == "blocked" else 0


if __name__ == "__main__":
    sys.exit(main())
