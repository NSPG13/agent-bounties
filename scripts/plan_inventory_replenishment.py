#!/usr/bin/env python3
"""Deterministic, read-only inventory replenishment planner (Issue #872).

Consumes a versioned inventory snapshot and a versioned candidate manifest to
produce a content-addressed, idempotent replenishment plan without signing,
broadcasting, mutating GitHub labels, or claiming payment.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Mapping, Sequence


INVENTORY_SCHEMA = "agent-bounties/inventory-snapshot-v1"
CANDIDATE_MANIFEST_SCHEMA = "agent-bounties/replenishment-candidates-v1"
PLAN_SCHEMA = "agent-bounties/inventory-replenishment-plan-v1"
DEFAULT_TARGET_READY_COUNT = 5
DEFAULT_MAX_SNAPSHOT_AGE_SECONDS = 900
BASE_CHAIN_ID = 8453
NATIVE_BASE_USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"

ADDRESS_RE = re.compile(r"^0x[0-9a-f]{40}$")
HEX32_RE = re.compile(r"^0x[0-9a-f]{64}$")
SHA256_DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
CANDIDATE_ID_RE = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")


class ReplenishmentPlanError(ValueError):
    """Raised when an input manifest is structurally malformed."""


def canonical_json(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def sha256_hex(value: Any) -> str:
    payload = value if isinstance(value, str) else canonical_json(value)
    return "0x" + hashlib.sha256(payload.encode("utf-8")).hexdigest()


def parse_utc_timestamp(raw: Any, field_name: str) -> datetime:
    if isinstance(raw, (int, float)) and not isinstance(raw, bool):
        return datetime.fromtimestamp(int(raw), tz=timezone.utc)
    if not isinstance(raw, str) or not raw.strip():
        raise ReplenishmentPlanError(f"{field_name} must be an ISO-8601 UTC string or unix seconds")
    text = raw.strip()
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    try:
        dt = datetime.fromisoformat(text)
    except ValueError as exc:
        raise ReplenishmentPlanError(f"{field_name} is not valid ISO-8601: {raw}") from exc
    if dt.tzinfo is None or dt.utcoffset() != timezone.utc.utcoffset(dt):
        raise ReplenishmentPlanError(f"{field_name} must use explicit UTC offset")
    return dt.astimezone(timezone.utc)


def _require_non_negative_int(value: Any, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ReplenishmentPlanError(f"{name} must be a non-negative integer")
    return value


def _normalize_address(value: Any, name: str) -> str:
    if not isinstance(value, str):
        raise ReplenishmentPlanError(f"{name} must be a 0x-prefixed 20-byte hex address")
    lowered = value.strip().lower()
    if not ADDRESS_RE.match(lowered):
        raise ReplenishmentPlanError(f"{name} is not a valid address: {value}")
    return lowered


def compute_idempotency_key(
    *,
    inventory_snapshot: Mapping[str, Any],
    candidate_manifest: Mapping[str, Any],
    target_ready_count: int,
    selected_candidates: Sequence[Mapping[str, Any]] | None = None,
    selected_candidate_ids: Sequence[str] | None = None,
    total_funding: int,
    blockers: Sequence[str],
) -> str:
    candidates_commitment: list[dict[str, Any]] = []
    if selected_candidates:
        for c in selected_candidates:
            candidates_commitment.append(
                {
                    "candidate_id": str(c.get("candidate_id") or c.get("slug") or ""),
                    "issue": c.get("issue"),
                    "benchmark_subdirectory": str(c.get("benchmark_subdirectory") or ""),
                    "benchmark_digest": str(c.get("benchmark_digest") or ""),
                    "threshold": c.get("threshold", 2),
                    "verifiers": sorted(str(v).lower() for v in c.get("verifiers", [])),
                    "solver_reward": c.get("solver_reward", 0),
                    "verifier_reward": c.get("verifier_reward", 0),
                    "initial_funding": c.get("initial_funding", 0),
                }
            )
    elif selected_candidate_ids:
        candidates_commitment = [{"candidate_id": cid} for cid in selected_candidate_ids]

    commitment = {
        "schema": PLAN_SCHEMA,
        "inventory_observed_at": str(inventory_snapshot.get("observed_at", "")),
        "inventory_safe_block": inventory_snapshot.get("safe_block"),
        "ready_count": inventory_snapshot.get("ready_count"),
        "target_ready_count": target_ready_count,
        "wallet": str(candidate_manifest.get("wallet", "")).lower(),
        "policy_version": candidate_manifest.get("policy_version"),
        "policy_hash": str(candidate_manifest.get("policy_hash", "")).lower(),
        "selected_candidates": candidates_commitment,
        "total_funding": total_funding,
        "blockers": list(blockers),
    }
    return sha256_hex(commitment)


def evaluate_candidate(candidate: Mapping[str, Any]) -> tuple[dict[str, Any], list[str]]:
    """Validate one candidate task and return normalized economics + candidate blockers."""
    blockers: list[str] = []
    cid = str(candidate.get("candidate_id") or candidate.get("slug") or "").strip()
    if not cid or not CANDIDATE_ID_RE.match(cid):
        blockers.append("invalid_candidate_id")

    issue = candidate.get("issue")
    if isinstance(issue, bool) or not isinstance(issue, int) or issue <= 0:
        blockers.append(f"invalid_issue:{cid or 'unknown'}")

    solver_reward = _require_non_negative_int(candidate.get("solver_reward", 0), "solver_reward")
    verifier_reward = _require_non_negative_int(candidate.get("verifier_reward", 0), "verifier_reward")
    claim_bond = _require_non_negative_int(
        candidate.get("claim_bond", verifier_reward), "claim_bond"
    )
    initial_funding = _require_non_negative_int(
        candidate.get("initial_funding", solver_reward + verifier_reward),
        "initial_funding",
    )
    estimated_solver_cost = _require_non_negative_int(
        candidate.get("estimated_solver_cost", 0), "estimated_solver_cost"
    )

    # Economic & margin checks
    solver_margin = solver_reward - claim_bond - estimated_solver_cost
    if solver_reward <= 0 or solver_margin <= 0:
        blockers.append(f"non_positive_solver_margin:{cid}")
    if verifier_reward <= 0 or claim_bond != verifier_reward:
        blockers.append(f"invalid_verifier_or_bond_economics:{cid}")
    if initial_funding != solver_reward + verifier_reward:
        blockers.append(f"funding_mismatch:{cid}")

    # Verification readiness checks
    benchmark_subdir = str(candidate.get("benchmark_subdirectory", "")).strip()
    benchmark_digest = str(candidate.get("benchmark_digest", "")).strip()
    if (
        not benchmark_subdir
        or not benchmark_subdir.startswith("benchmarks/")
        or ".." in benchmark_subdir
        or not SHA256_DIGEST_RE.match(benchmark_digest)
    ):
        blockers.append(f"unverifiable_benchmark:{cid}")

    threshold = candidate.get("threshold", 2)
    verifiers = candidate.get("verifiers", [])
    if (
        isinstance(threshold, bool)
        or not isinstance(threshold, int)
        or threshold < 1
        or not isinstance(verifiers, list)
        or len(verifiers) < threshold
    ):
        blockers.append(f"insufficient_verifiers:{cid}")
    else:
        normalized_verifiers = []
        for v in verifiers:
            try:
                normalized_verifiers.append(_normalize_address(v, f"verifier in {cid}"))
            except ReplenishmentPlanError:
                blockers.append(f"invalid_verifier_address:{cid}")
                break
        if len(set(normalized_verifiers)) != len(normalized_verifiers):
            blockers.append(f"duplicate_verifier_addresses:{cid}")

    normalized = {
        "candidate_id": cid,
        "issue": issue,
        "title": str(candidate.get("title", "")),
        "benchmark_subdirectory": benchmark_subdir,
        "benchmark_digest": benchmark_digest,
        "solver_reward": solver_reward,
        "verifier_reward": verifier_reward,
        "claim_bond": claim_bond,
        "initial_funding": initial_funding,
        "estimated_solver_cost": estimated_solver_cost,
        "solver_margin": solver_margin,
        "threshold": threshold,
        "verifiers": candidate.get("verifiers", []),
    }
    return normalized, blockers


def generate_replenishment_plan(
    inventory_snapshot: Mapping[str, Any],
    candidate_manifest: Mapping[str, Any],
    *,
    now: datetime | int | str | None = None,
    target_ready_count: int = DEFAULT_TARGET_READY_COUNT,
    max_snapshot_age_seconds: int = DEFAULT_MAX_SNAPSHOT_AGE_SECONDS,
) -> dict[str, Any]:
    """Generate a deterministic, non-financial inventory replenishment plan."""
    if not isinstance(inventory_snapshot, Mapping) or not isinstance(candidate_manifest, Mapping):
        raise ReplenishmentPlanError("inventory_snapshot and candidate_manifest must be JSON objects")

    blockers: list[str] = []

    if inventory_snapshot.get("schema") != INVENTORY_SCHEMA:
        blockers.append("invalid_inventory_schema")
    if candidate_manifest.get("schema") != CANDIDATE_MANIFEST_SCHEMA:
        blockers.append("invalid_candidate_manifest_schema")

    if isinstance(target_ready_count, bool) or not isinstance(target_ready_count, int) or target_ready_count <= 0:
        raise ReplenishmentPlanError("target_ready_count must be a positive integer")

    # 1. Freshness & source integrity check
    observed_at = parse_utc_timestamp(inventory_snapshot.get("observed_at"), "observed_at")
    eval_now = (
        parse_utc_timestamp(now, "now")
        if now is not None
        else datetime.now(timezone.utc)
    )
    snapshot_age_seconds = int((eval_now - observed_at).total_seconds())
    if snapshot_age_seconds < 0 or snapshot_age_seconds > max_snapshot_age_seconds:
        blockers.append("stale_inventory_snapshot")

    if bool(inventory_snapshot.get("source_degraded", False)):
        blockers.append("degraded_inventory_source")

    safe_block = _require_non_negative_int(inventory_snapshot.get("safe_block", 0), "safe_block")
    if safe_block <= 0:
        blockers.append("missing_safe_block")

    ready_count = _require_non_negative_int(inventory_snapshot.get("ready_count", 0), "ready_count")
    deficit = max(0, target_ready_count - ready_count)

    # 2. Wallet identity, policy authority and balance checks
    raw_wallet = candidate_manifest.get("wallet")
    if not raw_wallet or not isinstance(raw_wallet, str):
        blockers.append("missing_wallet_identity")
        wallet = "0x" + "00" * 20
    else:
        try:
            wallet = _normalize_address(raw_wallet, "wallet")
            if wallet == "0x" + "00" * 20:
                blockers.append("missing_wallet_identity")
        except ReplenishmentPlanError:
            blockers.append("missing_wallet_identity")
            wallet = "0x" + "00" * 20

    policy_version = candidate_manifest.get("policy_version")
    if (
        policy_version is None
        or isinstance(policy_version, bool)
        or not isinstance(policy_version, int)
        or policy_version <= 0
    ):
        blockers.append("missing_policy_version")

    policy_hash = candidate_manifest.get("policy_hash")
    if (
        not policy_hash
        or not isinstance(policy_hash, str)
        or not HEX32_RE.match(policy_hash.strip().lower())
    ):
        blockers.append("missing_policy_hash")

    settlement_token = _normalize_address(
        candidate_manifest.get("settlement_token", NATIVE_BASE_USDC),
        "settlement_token",
    )
    if settlement_token != NATIVE_BASE_USDC:
        blockers.append("wrong_settlement_token")
    if int(candidate_manifest.get("chain_id", BASE_CHAIN_ID)) != BASE_CHAIN_ID:
        blockers.append("wrong_chain_id")

    wallet_state = candidate_manifest.get("wallet_state")
    if not isinstance(wallet_state, Mapping):
        wallet_state = {}

    wallet_balance = _require_non_negative_int(
        wallet_state.get("wallet_balance", candidate_manifest.get("wallet_balance", 0)),
        "wallet_balance",
    )
    period_spent = _require_non_negative_int(
        wallet_state.get("period_spent", candidate_manifest.get("period_spent", 0)),
        "period_spent",
    )
    max_per_period = _require_non_negative_int(
        wallet_state.get("max_per_period", candidate_manifest.get("max_per_period", 0)),
        "max_per_period",
    )
    if max_per_period <= 0:
        blockers.append("zero_period_cap")

    lifetime_spent = _require_non_negative_int(
        wallet_state.get("lifetime_spent", candidate_manifest.get("lifetime_spent", 0)),
        "lifetime_spent",
    )
    max_lifetime_spend = _require_non_negative_int(
        wallet_state.get(
            "max_lifetime_spend",
            wallet_state.get("max_lifetime", candidate_manifest.get("max_lifetime_spend", 0)),
        ),
        "max_lifetime_spend",
    )
    if max_lifetime_spend <= 0:
        blockers.append("zero_lifetime_cap")

    max_per_action = _require_non_negative_int(
        wallet_state.get("max_per_action", candidate_manifest.get("max_per_action", max_per_period)),
        "max_per_action",
    )
    max_bounty_target = _require_non_negative_int(
        wallet_state.get("max_bounty_target", candidate_manifest.get("max_bounty_target", max_per_action)),
        "max_bounty_target",
    )
    revoked = bool(wallet_state.get("revoked", candidate_manifest.get("revoked", False)))
    if revoked:
        blockers.append("wallet_policy_revoked")

    period_remaining = max(0, max_per_period - period_spent)
    lifetime_remaining = max(0, max_lifetime_spend - lifetime_spent)

    # 3. Validate candidate pool and detect duplicates
    raw_candidates = candidate_manifest.get("candidates", [])
    if not isinstance(raw_candidates, list):
        raise ReplenishmentPlanError("candidates must be a list")

    seen_ids: set[str] = set()
    seen_issues: set[int] = set()
    seen_benchmarks: set[str] = set()
    valid_candidates: list[dict[str, Any]] = []

    for raw in raw_candidates:
        if not isinstance(raw, Mapping):
            blockers.append("malformed_candidate_entry")
            continue
        normalized, c_blockers = evaluate_candidate(raw)
        blockers.extend(c_blockers)

        cid = normalized["candidate_id"]
        issue = normalized["issue"]
        bench_key = (
            f"{normalized['benchmark_subdirectory']}@{normalized['benchmark_digest']}"
        )

        if cid in seen_ids:
            blockers.append(f"duplicate_candidate:{cid}")
        if issue in seen_issues:
            blockers.append(f"duplicate_issue:{issue}")
        if bench_key in seen_benchmarks:
            blockers.append(f"duplicate_benchmark:{bench_key}")

        seen_ids.add(cid)
        if isinstance(issue, int):
            seen_issues.add(issue)
        seen_benchmarks.add(bench_key)

        if not c_blockers:
            valid_candidates.append(normalized)

    # Sort deterministically by issue, then candidate_id
    valid_candidates.sort(key=lambda item: (int(item["issue"]), str(item["candidate_id"])))

    # 4. Select exact deficit tasks if no structural blockers
    selected_candidates: list[dict[str, Any]] = []
    if deficit > 0:
        if len(valid_candidates) < deficit:
            blockers.append(
                f"insufficient_candidates:needed_{deficit}_have_{len(valid_candidates)}"
            )
        selected_candidates = valid_candidates[:deficit]

    total_funding = sum(c["initial_funding"] for c in selected_candidates)
    per_task_funding = (
        selected_candidates[0]["initial_funding"] if selected_candidates else 0
    )

    # 5. Check wallet liquidity and policy caps against required total_funding
    wallet_sufficient = wallet_balance >= total_funding
    period_sufficient = period_remaining >= total_funding
    lifetime_sufficient = lifetime_remaining >= total_funding

    if deficit > 0 and total_funding > 0:
        if not wallet_sufficient:
            blockers.append("insufficient_wallet_balance")
        if not period_sufficient:
            blockers.append("period_cap_exceeded")
        if not lifetime_sufficient:
            blockers.append("lifetime_cap_exceeded")

    # Deduplicate blockers preserving order
    deduped_blockers = list(dict.fromkeys(blockers))

    if deduped_blockers:
        status = "blocked"
        plan_candidates: list[dict[str, Any]] = []
        effective_total_funding = total_funding
    elif deficit == 0:
        status = "no_replenishment_needed"
        plan_candidates = []
        effective_total_funding = 0
    else:
        status = "ready_to_plan"
        plan_candidates = selected_candidates
        effective_total_funding = total_funding

    idempotency_key = compute_idempotency_key(
        inventory_snapshot=inventory_snapshot,
        candidate_manifest=candidate_manifest,
        target_ready_count=target_ready_count,
        selected_candidates=plan_candidates,
        total_funding=effective_total_funding,
        blockers=deduped_blockers,
    )

    return {
        "schema": PLAN_SCHEMA,
        "status": status,
        "idempotency_key": idempotency_key,
        "financial_action_taken": False,
        "broadcasted": False,
        "labels_applied": [],
        "payment_claimed": False,
        "inventory": {
            "ready_count": ready_count,
            "target_ready_count": target_ready_count,
            "deficit": deficit,
            "observed_at": observed_at.isoformat().replace("+00:00", "Z"),
            "safe_block": safe_block,
            "snapshot_age_seconds": snapshot_age_seconds,
        },
        "economics": {
            "candidate_count": len(plan_candidates),
            "per_task_funding": per_task_funding if plan_candidates else 0,
            "total_funding": effective_total_funding,
            "min_solver_margin": min((c["solver_margin"] for c in plan_candidates), default=0),
        },
        "sufficiency": {
            "wallet": wallet,
            "settlement_token": settlement_token,
            "wallet_balance": wallet_balance,
            "period_spent": period_spent,
            "max_per_period": max_per_period,
            "period_remaining": period_remaining,
            "lifetime_spent": lifetime_spent,
            "max_lifetime_spend": max_lifetime_spend,
            "lifetime_remaining": lifetime_remaining,
            "wallet_sufficient": wallet_sufficient,
            "period_sufficient": period_sufficient,
            "lifetime_sufficient": lifetime_sufficient,
        },
        "candidates": plan_candidates,
        "blockers": deduped_blockers,
        "evidence_boundary": (
            "Read-only replenishment plan only. Does not sign, broadcast, fund, "
            "apply funded-live/claimable-live labels, or establish settlement."
        ),
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inventory", required=True, type=Path, help="Path to inventory snapshot JSON")
    parser.add_argument("--candidates", required=True, type=Path, help="Path to candidate manifest JSON")
    parser.add_argument("--target-ready", type=int, default=DEFAULT_TARGET_READY_COUNT)
    parser.add_argument("--max-age-seconds", type=int, default=DEFAULT_MAX_SNAPSHOT_AGE_SECONDS)
    parser.add_argument("--now", default=None, help="Optional evaluation timestamp (defaults to current clock)")
    args = parser.parse_args(argv)

    inventory = json.loads(args.inventory.read_text(encoding="utf-8"))
    candidates = json.loads(args.candidates.read_text(encoding="utf-8"))
    plan = generate_replenishment_plan(
        inventory,
        candidates,
        target_ready_count=args.target_ready,
        max_snapshot_age_seconds=args.max_age_seconds,
        now=args.now,
    )
    print(json.dumps(plan, indent=2, sort_keys=True))
    return 0 if plan["status"] in ("ready_to_plan", "no_replenishment_needed") else 1


if __name__ == "__main__":
    sys.exit(main())
