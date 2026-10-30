#!/usr/bin/env python3
"""Deterministic inventory replenishment planning (#872).

Turns an inventory-guard deficit into an idempotent, non-financial plan.

Hard rule: this module never signs, never broadcasts, never labels an issue
funded, and never calls a transaction paid. `financial_action_taken` is always
False and exists purely so callers and tests can assert that no financial
action was taken.
"""
from __future__ import annotations

import hashlib
import json

SCHEMA_VERSION = "inventory-replenishment-plan/v1"

# Constant, always False. Present so every consumer can assert it.
financial_action_taken = False


def _canonical(payload) -> str:
    return json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def idempotency_key(guard_report: dict, candidates: list) -> str:
    """Stable key: identical inputs always produce the same plan and key."""
    payload = {"schema": SCHEMA_VERSION,
               "deficit": guard_report.get("deficit"),
               "tasks": sorted(str(c.get("task_id")) for c in candidates)}
    return hashlib.sha256(_canonical(payload).encode("utf-8")).hexdigest()


def _reject(reason: str) -> dict:
    return {"ok": False, "blocker": reason, "selected": [], "idempotency_key": None,
            "financial_action_taken": financial_action_taken}


def plan_replenishment(guard_report: dict, candidates: list) -> dict:
    """Return a deterministic replenishment plan, or a blocked result."""
    deficit = guard_report.get("deficit")
    if not isinstance(deficit, (int, float)) or deficit <= 0:
        return _reject("non_positive_deficit")

    wallet_balance = guard_report.get("wallet_balance")
    policy = guard_report.get("policy") or {}
    max_lifetime = policy.get("max_lifetime")
    max_per_period = policy.get("max_per_period")
    lifetime_spent = policy.get("lifetime_spent", 0)
    period_spent = policy.get("period_spent", 0)
    now = guard_report.get("observed_at", 0)

    seen, selected, blockers = set(), [], []
    for cand in candidates:
        task_id = cand.get("task_id")
        if not task_id:
            blockers.append("candidate_without_task_id")
            continue
        if task_id in seen:
            blockers.append(f"duplicate_task:{task_id}")
            continue
        seen.add(task_id)
        if not cand.get("verifier_ready", False):
            blockers.append(f"unverifiable_task:{task_id}")
            continue
        solver_margin = cand.get("solver_margin")
        if not isinstance(solver_margin, (int, float)) or solver_margin <= 0:
            blockers.append(f"non_positive_solver_margin:{task_id}")
            continue
        evidenced_at = cand.get("evidenced_at")
        max_age = guard_report.get("max_evidence_age", 0)
        if evidenced_at is not None and max_age and (now - evidenced_at) > max_age:
            blockers.append(f"stale_evidence:{task_id}")
            continue
        selected.append(cand)
        if len(selected) >= 5:
            break

    if not selected:
        blocked = _reject(blockers[0] if blockers else "no_eligible_candidates")
        blocked["blockers"] = blockers
        return blocked

    total_funding = sum(c.get("funding_usdc", 0) for c in selected)
    remaining_lifetime = (max_lifetime - lifetime_spent) if max_lifetime is not None else None
    remaining_period = (max_per_period - period_spent) if max_per_period is not None else None

    if wallet_balance is not None and total_funding > wallet_balance:
        blockers.append("insufficient_wallet_balance")
    if remaining_lifetime is not None and total_funding > remaining_lifetime:
        blockers.append("exceeds_lifetime_cap")
    if remaining_period is not None and total_funding > remaining_period:
        blockers.append("exceeds_period_cap")

    per_task = [{"task_id": c.get("task_id"),
                 "funding_usdc": c.get("funding_usdc"),
                 "solver_margin": c.get("solver_margin"),
                 "verifier_ready": c.get("verifier_ready", False)} for c in selected]

    return {"ok": not blockers,
            "schema": SCHEMA_VERSION,
            "deficit": deficit,
            "idempotency_key": idempotency_key(guard_report, candidates),
            "idempotency_keys": {t["task_id"]: f"{idempotency_key(guard_report, candidates)}:{t['task_id']}"
                                 for t in per_task},
            "selected": per_task,
            "total_funding_usdc": total_funding,
            "wallet_balance": wallet_balance,
            "wallet_sufficient": wallet_balance is None or total_funding <= wallet_balance,
            "lifetime_spent": lifetime_spent, "max_lifetime": max_lifetime,
            "period_spent": period_spent, "max_per_period": max_per_period,
            "policy_sufficient": all(not b.startswith("exceeds_") for b in blockers),
            "blockers": blockers,
            "blocker": blockers[0] if blockers else None,
            "financial_action_taken": financial_action_taken}
