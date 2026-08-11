#!/usr/bin/env python3
"""Canonical inventory-state-breakdown-v1 checker and projector.

Derives ready_to_earn / in_progress / submitted / paid /
verification_unavailable counts from one accepted inventory snapshot.
Fixtures cover empty, mixed, degraded, and stale projections.
"""

from __future__ import annotations

import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Mapping

SCHEMA = "inventory-state-breakdown-v1"
COUNT_KEYS = (
    "ready_to_earn",
    "in_progress",
    "submitted",
    "paid",
    "verification_unavailable",
)

ROOT = Path(os.environ.get("WORKSPACE_ROOT", Path(__file__).resolve().parents[1]))
FIXTURE_DIR = ROOT / "scripts" / "fixtures" / "inventory-state-breakdown"


def load_snapshot(path: Path) -> Mapping[str, Any]:
    if not path.is_file():
        raise FileNotFoundError(f"missing inventory snapshot: {path}")
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise ValueError("inventory snapshot must be a JSON object")
    return data


def compute_breakdown(snapshot: Mapping[str, Any]) -> dict[str, Any]:
    """Project one canonical snapshot into inventory-state-breakdown-v1."""
    items = snapshot.get("items") or []
    if not isinstance(items, list):
        raise ValueError("snapshot.items must be a list")

    counts = {key: 0 for key in COUNT_KEYS}
    for item in items:
        if not isinstance(item, dict):
            continue
        status = item.get("status")
        if status in counts:
            counts[status] += 1

    generated_at = snapshot.get("generated_at") or datetime.now(timezone.utc).strftime(
        "%Y-%m-%dT%H:%M:%SZ"
    )
    source = snapshot.get("source") or "canonical"
    safe_block = snapshot.get("safe_block")
    source_degraded = bool(snapshot.get("source_degraded", False))
    if source in {"degraded-on-chain-feed", "stale-cache"}:
        source_degraded = True

    body: dict[str, Any] = {
        "ready_to_earn": counts["ready_to_earn"],
        "in_progress": counts["in_progress"],
        "submitted": counts["submitted"],
        "paid": counts["paid"],
        "verification_unavailable": counts["verification_unavailable"],
        "total": sum(counts.values()),
        "generated_at": generated_at,
        "source": source,
        "source_degraded": source_degraded,
    }
    if safe_block is not None:
        body["safe_block"] = safe_block
    return {SCHEMA: body}


def check_fixture(name: str) -> None:
    snapshot = load_snapshot(FIXTURE_DIR / f"{name}.json")
    result = compute_breakdown(snapshot)
    body = result[SCHEMA]
    for key in (*COUNT_KEYS, "generated_at", "source", "total"):
        if key not in body:
            raise AssertionError(f"{name}: missing {key}")
    for key in COUNT_KEYS:
        if not isinstance(body[key], int) or body[key] < 0:
            raise AssertionError(f"{name}: {key} must be non-negative int")
    if sum(body[k] for k in COUNT_KEYS) != body["total"]:
        raise AssertionError(f"{name}: counts do not sum to total")
    print(f"  {name}: OK counts={ {k: body[k] for k in COUNT_KEYS} }")


def main() -> int:
    print(f"{SCHEMA} checker")
    errors = 0
    for name in ("empty", "mixed", "degraded", "stale"):
        try:
            check_fixture(name)
        except Exception as exc:  # noqa: BLE001 - surface fixture failures
            print(f"  {name}: FAIL - {exc}", file=sys.stderr)
            errors += 1
    if errors:
        print(f"{errors} fixture checks failed", file=sys.stderr)
        return 1
    print("inventory-state breakdown acceptance checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
