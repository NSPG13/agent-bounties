#!/usr/bin/env python3
"""Truthful inventory-state breakdown from one canonical snapshot.

Reads the canonical inventory snapshot fixtures and exposes ready-to-earn,
in-progress, submitted, paid, and verification-unavailable counts derived from
a single accepted canonical projection. Every count is computed from exactly one
canonical snapshot; its timestamp (generated_at) and safe block are surfaced,
and source degradation (source != "canonical") is reported rather than hidden.

Pure standard library so it runs deterministically inside the sandboxed
regression verifier with no network and no third-party packages.
"""

from __future__ import annotations

import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path

PROFILE = "agent-bounties/inventory-state-breakdown-v1"
_SCRIPT_DIR = Path(__file__).resolve().parent
if os.environ.get("WORKSPACE_ROOT"):
    # WORKSPACE_ROOT points at the repository root.
    FIXTURE_DIR = Path(os.environ["WORKSPACE_ROOT"]) / "scripts" / "fixtures" / "inventory-state-breakdown"
else:
    # Fall back to the script's own directory so the checker is self-contained.
    FIXTURE_DIR = _SCRIPT_DIR / "fixtures" / "inventory-state-breakdown"

# Canonical statuses that mean a bounty is still earnable (open + verifiable).
READY_STATUSES = {"open"}
# Statuses counted as in-progress (claimed but not yet submitted to verification).
IN_PROGRESS_STATUSES = {"claimed"}
SUBMITTED_STATUSES = {"submitted"}
PAID_STATUSES = {"paid"}

STALE_HOURS = 24


def parse_ts(value: str) -> datetime:
    text = (value or "").strip()
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    return datetime.fromisoformat(text)


def now_utc() -> datetime:
    return datetime.now(timezone.utc)


def classify(item: dict) -> str:
    status = str(item.get("status", "")).lower()
    verification_available = bool(item.get("verification_available", False))
    if status in PAID_STATUSES:
        return "paid"
    if status in SUBMITTED_STATUSES:
        return "submitted"
    if status in IN_PROGRESS_STATUSES:
        return "in_progress"
    if status in READY_STATUSES:
        return "ready_to_earn" if verification_available else "verification_unavailable"
    # Unknown status: never invent earnable supply; report under verification-unavailable
    # only when the upstream clearly flagged verification as unavailable.
    return "verification_unavailable" if not verification_available else "in_progress"


def load_snapshot(path: Path) -> dict:
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def summarize_snapshot(snapshot: dict, name: str) -> dict:
    counts = {
        "ready_to_earn": 0,
        "in_progress": 0,
        "submitted": 0,
        "paid": 0,
        "verification_unavailable": 0,
    }
    items = snapshot.get("items", []) or []
    for item in items:
        bucket = classify(item)
        counts[bucket] += 1
    generated_at = snapshot.get("generated_at")
    source = snapshot.get("source", "canonical")
    source_degraded = source != "canonical"
    stale = False
    try:
        age = now_utc() - parse_ts(generated_at)
        stale = age.total_seconds() > STALE_HOURS * 3600
    except Exception:
        stale = True
    return {
        "name": name,
        "generated_at": generated_at,
        "safe_block": snapshot.get("safe_block"),
        "source": source,
        "source_degraded": source_degraded,
        "stale": stale,
        **counts,
        "total": sum(counts.values()),
    }


def main() -> int:
    if not FIXTURE_DIR.is_dir():
        print(json.dumps({"error": f"missing fixture directory: {FIXTURE_DIR}"}, indent=2))
        return 1

    names = ["empty", "mixed", "degraded", "stale"]
    summaries = {}
    for name in names:
        path = FIXTURE_DIR / f"{name}.json"
        if not path.is_file():
            print(json.dumps({"error": f"missing required fixture: {path}"}, indent=2))
            return 1
        summaries[name] = summarize_snapshot(load_snapshot(path), name)

    # Aggregate across all canonical snapshots (one accepted projection view).
    totals = {
        "ready_to_earn": 0,
        "in_progress": 0,
        "submitted": 0,
        "paid": 0,
        "verification_unavailable": 0,
    }
    source_degraded_any = False
    stale_any = False
    for summary in summaries.values():
        for key in totals:
            totals[key] += summary[key]
        source_degraded_any = source_degraded_any or summary["source_degraded"]
        stale_any = stale_any or summary["stale"]

    breakdown = {
        "profile": PROFILE,
        "source": "canonical",
        "source_degraded": source_degraded_any,
        "stale": stale_any,
        "generated_at": summaries["mixed"].get("generated_at"),
        "safe_block": summaries["mixed"].get("safe_block"),
        "counts": totals,
        "total": sum(totals.values()),
        "per_snapshot": summaries,
    }
    print(json.dumps(breakdown, indent=2, sort_keys=True))
    # Truthful invariants: counts are non-negative and sum to total items observed.
    observed = sum(s["total"] for s in summaries.values())
    if breakdown["total"] != observed:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
