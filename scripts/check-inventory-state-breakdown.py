#!/usr/bin/env python3
"""Deterministic checker for inventory-state-breakdown-v1.

Validates that the breakdown response schema covers the five canonical
lifecycle buckets and that fixture projections conform to the contract.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

REQUIRED_KEYS = (
    "ready_to_earn",
    "in_progress",
    "submitted",
    "paid",
    "verification_unavailable",
)
SCHEMA_VERSION = "inventory-state-breakdown-v1"
FIXTURE_DIR = Path(__file__).resolve().parent.parent / "scripts" / "fixtures" / "inventory-state-breakdown"


def validate_projection(projection: dict, label: str) -> None:
    for key in ("schema_version", "generated_at", "source"):
        if key not in projection:
            raise SystemExit(f"{label}: missing top-level key '{key}'")
    if projection["schema_version"] != SCHEMA_VERSION:
        raise SystemExit(
            f"{label}: schema_version mismatch: expected {SCHEMA_VERSION!r}, "
            f"got {projection['schema_version']!r}"
        )
    counts = projection.get("counts")
    if not isinstance(counts, dict):
        raise SystemExit(f"{label}: 'counts' must be a dict")
    for key in REQUIRED_KEYS:
        if key not in counts:
            raise SystemExit(f"{label}: counts missing '{key}'")
        if not isinstance(counts[key], int) or counts[key] < 0:
            raise SystemExit(f"{label}: counts['{key}'] must be a non-negative integer")
    total = sum(counts[k] for k in REQUIRED_KEYS)
    if "total" in counts and counts["total"] != total:
        raise SystemExit(f"{label}: total mismatch: {counts['total']} != {total}")


def main() -> None:
    if not FIXTURE_DIR.is_dir():
        raise SystemExit(f"fixture directory missing: {FIXTURE_DIR}")
    for name in ("empty", "mixed", "degraded", "stale"):
        path = FIXTURE_DIR / f"{name}.json"
        if not path.is_file():
            raise SystemExit(f"missing fixture: {path}")
        try:
            projection = json.loads(path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as exc:
            raise SystemExit(f"{name}.json: invalid JSON: {exc}")
        validate_projection(projection, name)
        if name == "empty":
            total = sum(projection["counts"][k] for k in REQUIRED_KEYS)
            if total != 0:
                raise SystemExit(f"empty fixture must have zero totals, got {total}")
        if name == "degraded":
            if projection["counts"].get("verification_unavailable", 0) == 0:
                raise SystemExit("degraded fixture must have verification_unavailable > 0")
        if name == "stale":
            if "stale_since" not in projection:
                raise SystemExit("stale fixture must include 'stale_since' timestamp")
    print("inventory-state-breakdown-v1 checks passed")


if __name__ == "__main__":
    main()
