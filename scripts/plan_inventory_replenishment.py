#!/usr/bin/env python3
"""Deterministic, non-financial inventory replenishment planning (#872).

Converts an inventory-guard deficit into an idempotent replenishment plan
that names exact candidate tasks, per-task funding, total funding, wallet
sufficiency, policy sufficiency, and blockers.

Hard constraints:

- The plan is a *plan only*: it never signs, never broadcasts, never labels
  an issue funded, and never calls a transaction paid
  (``financial_action_taken`` is always false).
- Identical inputs produce the identical plan and the identical
  ``idempotency_key`` (canonical SHA-256 over the normalized inputs).
- Duplicate tasks, unverifiable tasks, non-positive solver margin, stale
  evidence, and totals above wallet balance or policy limits are rejected
  and surfaced as blockers, never silently dropped.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

PLAN_VERSION = "inventory-replenishment-plan-v1"


@dataclass(frozen=True)
class CandidateTask:
    """One candidate task that could absorb replenishment funding."""

    task_id: str
    title: str
    funding_usdc: float
    solver_margin_usdc: float
    verifier_ready: bool
    evidence_updated_at: str
    evidence_ref: str


@dataclass(frozen=True)
class WalletState:
    """Bounded-wallet state used for sufficiency checks."""

    wallet_balance: float
    period_spent: float
    max_per_period: float
    lifetime_spent: float
    max_lifetime: float


def _canonical_json(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), default=str)


def _idempotency_key(guard_report: dict, tasks: list[dict], wallet: dict) -> str:
    canonical = _canonical_json(
        {
            "guard_report": guard_report,
            "candidate_tasks": tasks,
            "wallet": wallet,
            "plan_version": PLAN_VERSION,
        }
    )
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def _parse_ts(value: str) -> datetime:
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def compute_deficit(guard_report: dict[str, Any]) -> int:
    """Number of missing meta-bounties versus the replenishment target."""
    if "meta_replenishment_count" in guard_report:
        return max(0, int(guard_report["meta_replenishment_count"]))
    target = int(guard_report.get("meta_replenishment_target", 2))
    held = len(guard_report.get("meta_claimable_bounty_ids", []))
    return max(0, target - held)


def plan_replenishment(
    guard_report: dict[str, Any],
    candidate_tasks: list[dict[str, Any]],
    wallet: dict[str, Any],
    *,
    max_evidence_age_days: int = 7,
    now: str | None = None,
) -> dict[str, Any]:
    """Build the deterministic replenishment plan for one deficit window.

    ``now`` is injectable so callers and tests get fully deterministic
    output; it defaults to the current UTC time.
    """

    now_dt = _parse_ts(now) if now else datetime.now(timezone.utc)
    wallet_state = WalletState(
        wallet_balance=float(wallet["wallet_balance"]),
        period_spent=float(wallet.get("period_spent", 0.0)),
        max_per_period=float(wallet["max_per_period"]),
        lifetime_spent=float(wallet.get("lifetime_spent", 0.0)),
        max_lifetime=float(wallet["max_lifetime"]),
    )

    deficit = compute_deficit(guard_report)
    blockers: list[str] = []

    # --- Validation pass -------------------------------------------------
    seen_ids: set[str] = set()
    valid: list[CandidateTask] = []
    for raw in candidate_tasks:
        task = CandidateTask(
            task_id=str(raw["task_id"]),
            title=str(raw["title"]),
            funding_usdc=float(raw["funding_usdc"]),
            solver_margin_usdc=float(raw["solver_margin_usdc"]),
            verifier_ready=bool(raw.get("verifier_ready", False)),
            evidence_updated_at=str(raw["evidence_updated_at"]),
            evidence_ref=str(raw.get("evidence_ref", "")),
        )
        if task.task_id in seen_ids:
            blockers.append(f"duplicate task: {task.task_id}")
            continue
        seen_ids.add(task.task_id)
        if not task.verifier_ready:
            blockers.append(f"unverifiable task: {task.task_id} (verifier not ready)")
            continue
        if task.solver_margin_usdc <= 0:
            blockers.append(
                f"non-positive solver margin: {task.task_id} "
                f"({task.solver_margin_usdc} USDC)"
            )
            continue
        evidence_age = now_dt - _parse_ts(task.evidence_updated_at)
        if evidence_age.days > max_evidence_age_days:
            blockers.append(f"stale evidence: {task.task_id} ({evidence_age.days}d old)")
            continue
        valid.append(task)

    # Deterministic selection order: task_id ascending.
    valid.sort(key=lambda t: t.task_id)
    selected = valid[:deficit]
    per_task_funding = {t.task_id: t.funding_usdc for t in selected}
    total_funding = sum(t.funding_usdc for t in selected)

    # --- Sufficiency checks ----------------------------------------------
    wallet_sufficient = wallet_state.wallet_balance >= total_funding
    if not wallet_sufficient:
        blockers.append(
            f"insufficient balance: need {total_funding} USDC, "
            f"wallet_balance is {wallet_state.wallet_balance} USDC"
        )
    period_ok = (
        wallet_state.period_spent + total_funding <= wallet_state.max_per_period
    )
    lifetime_ok = (
        wallet_state.lifetime_spent + total_funding <= wallet_state.max_lifetime
    )
    policy_sufficient = period_ok and lifetime_ok
    if not period_ok:
        blockers.append(
            "period cap exceeded: "
            f"{wallet_state.period_spent + total_funding} > {wallet_state.max_per_period}"
        )
    if not lifetime_ok:
        blockers.append(
            "lifetime cap exceeded: "
            f"{wallet_state.lifetime_spent + total_funding} > {wallet_state.max_lifetime}"
        )

    return {
        "plan_version": PLAN_VERSION,
        "generated_at": now_dt.isoformat(),
        "deficit": deficit,
        "selected_candidates": [t.task_id for t in selected],
        "per_task_funding": per_task_funding,
        "total_funding": total_funding,
        "wallet": {
            "wallet_balance": wallet_state.wallet_balance,
            "wallet_sufficient": wallet_sufficient,
        },
        "policy": {
            "period_spent": wallet_state.period_spent,
            "max_per_period": wallet_state.max_per_period,
            "lifetime_spent": wallet_state.lifetime_spent,
            "max_lifetime": wallet_state.max_lifetime,
            "policy_sufficient": policy_sufficient,
        },
        "blockers": blockers,
        "financial_action_taken": False,
        "idempotency_key": _idempotency_key(
            guard_report,
            sorted(candidate_tasks, key=lambda t: str(t["task_id"])),
            wallet,
        ),
        "disclaimer": (
            "plan only: no signing, no broadcast, no funding label, and no "
            "payment claim is implied by this document"
        ),
    }


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--guard-report", required=True, help="inventory-guard report JSON")
    parser.add_argument("--tasks", required=True, help="candidate task manifest JSON")
    parser.add_argument("--wallet", required=True, help="bounded-wallet state JSON")
    parser.add_argument("--max-evidence-age-days", type=int, default=7)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    guard = json.loads(Path(args.guard_report).read_text(encoding="utf-8"))
    tasks = json.loads(Path(args.tasks).read_text(encoding="utf-8"))
    wallet = json.loads(Path(args.wallet).read_text(encoding="utf-8"))
    plan = plan_replenishment(
        guard, tasks, wallet, max_evidence_age_days=args.max_evidence_age_days
    )
    print(json.dumps(plan, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
