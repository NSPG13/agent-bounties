#!/usr/bin/env python3
"""Deterministic acceptance checks for inventory-state-breakdown-v1."""

from __future__ import annotations

import json
import os
import sys
from datetime import datetime
from pathlib import Path
from typing import Any


ROOT = Path(os.environ.get("WORKSPACE_ROOT", Path(__file__).resolve().parents[1]))
FIXTURES = ROOT / "scripts" / "fixtures" / "inventory-state-breakdown"
SCHEMA = "agent-bounties/inventory-state-breakdown-v1"
MAX_AGE_SECONDS = 300
COUNT_FIELDS = (
    "ready_to_earn",
    "in_progress",
    "submitted",
    "paid",
    "verification_unavailable",
)


def parse_time(value: str) -> datetime:
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def project(snapshot: dict[str, Any]) -> dict[str, Any]:
    generated_at = parse_time(snapshot["generated_at"])
    observed_at = parse_time(snapshot["observed_at"])
    age_seconds = (observed_at - generated_at).total_seconds()
    source_declared_available = snapshot["source_available"] is True
    fresh = 0 <= age_seconds <= MAX_AGE_SECONDS
    source_available = source_declared_available and fresh
    source_status = (
        "degraded"
        if not source_declared_available
        else "stale"
        if not fresh
        else "current"
    )
    source_error = snapshot.get("source_error")
    if not source_declared_available and not source_error:
        source_error = "canonical_source_unavailable"
    elif source_declared_available and not fresh:
        source_error = "canonical_snapshot_stale"

    counts = {field: 0 for field in COUNT_FIELDS}
    if source_available:
        for item in snapshot["items"]:
            status = item["status"]
            verification_ready = item["verification_ready"] is True
            fully_funded = int(item["funded"]) >= 1_000_000
            if status == "claimable" and fully_funded and verification_ready:
                counts["ready_to_earn"] += 1
            if status == "claimed":
                counts["in_progress"] += 1
            elif status == "submitted":
                counts["submitted"] += 1
            elif status == "paid":
                counts["paid"] += 1
            if not verification_ready and status != "paid":
                counts["verification_unavailable"] += 1

    return {
        "schema_version": SCHEMA,
        "generated_at": snapshot["generated_at"],
        "observed_at": snapshot["observed_at"],
        "source": "canonical_base",
        "source_available": source_available,
        "source_degraded": not source_available,
        "source_status": source_status,
        "source_error": source_error,
        **counts,
    }


def check_fixture(name: str) -> None:
    fixture = json.loads((FIXTURES / f"{name}.json").read_text(encoding="utf-8"))
    actual = project(fixture)
    expected = fixture["expected"]
    for field, value in expected.items():
        if actual.get(field) != value:
            raise AssertionError(
                f"{name}: {field} expected {value!r}, got {actual.get(field)!r}"
            )
    for field in COUNT_FIELDS:
        if not isinstance(actual[field], int) or actual[field] < 0:
            raise AssertionError(f"{name}: {field} must be a non-negative integer")


def check_production_wiring() -> None:
    opportunities = (ROOT / "crates/api/src/opportunities.rs").read_text(encoding="utf-8")
    main = (ROOT / "crates/api/src/main.rs").read_text(encoding="utf-8")
    home = (ROOT / "site/home.js").read_text(encoding="utf-8")

    required_opportunity_fragments = (
        "agent-bounties/inventory-state-breakdown-v1",
        "pub fn inventory_state_breakdown_v1",
        "fn is_ready_to_earn_item",
        "canonical_snapshot_stale",
    )
    for fragment in required_opportunity_fragments:
        if fragment not in opportunities:
            raise AssertionError(f"production projector is missing {fragment}")
    if main.count("inventory_state_breakdown_v1(") != 1:
        raise AssertionError("API must attach exactly one production inventory breakdown")
    if "const breakdown = inventoryStateBreakdown(readyProjection);" not in home:
        raise AssertionError("homepage must consume the ready response's server breakdown")
    if any(f"breakdown.{field} =" in home for field in COUNT_FIELDS):
        raise AssertionError("homepage must not overwrite server inventory counts")
    if "inventory snapshot drift" not in home.lower():
        raise AssertionError("homepage must surface ready-view drift separately")


def main() -> int:
    try:
        check_production_wiring()
        for fixture in ("empty", "mixed", "degraded", "stale"):
            check_fixture(fixture)
    except (AssertionError, KeyError, TypeError, ValueError, OSError) as error:
        print(f"inventory-state breakdown check failed: {error}", file=sys.stderr)
        return 1
    print("inventory-state breakdown acceptance checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
