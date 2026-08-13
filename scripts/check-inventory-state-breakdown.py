#!/usr/bin/env python3
"""Canonical inventory-state-breakdown-v1 checker and projector.

Derives ready_to_earn / in_progress / submitted / paid /
verification_unavailable counts from one accepted inventory snapshot.
Fixtures cover empty, mixed, degraded, and stale projections.

Also gates the production API projector (opportunities.rs / main.rs) and
homepage consumer so fixture-only drift cannot pass review.
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
    if name == "stale":
        if not body.get("source_degraded"):
            raise AssertionError("stale: source_degraded must be true")
        if body.get("source") not in {"stale-cache", "degraded-on-chain-feed"}:
            raise AssertionError("stale: source must signal degraded/stale feed")
    if name == "degraded" and not body.get("source_degraded"):
        raise AssertionError("degraded: source_degraded must be true")
    print(f"  {name}: OK counts={ {k: body[k] for k in COUNT_KEYS} }")


def assert_production_projector() -> None:
    """Review gate: versioned breakdown lives at the API projection boundary."""
    opportunities = (ROOT / "crates/api/src/opportunities.rs").read_text(encoding="utf-8")
    main_rs = (ROOT / "crates/api/src/main.rs").read_text(encoding="utf-8")
    home = (ROOT / "site/home.js").read_text(encoding="utf-8")
    opp_l = opportunities.lower()
    main_l = main_rs.lower()
    if "inventory-state-breakdown-v1" not in opp_l and "inventory_state_breakdown" not in opp_l:
        raise SystemExit("opportunities.rs lacks inventory-state-breakdown-v1")
    if "fn inventory_state_breakdown_v1" not in opp_l and "pub fn inventory_state_breakdown_v1" not in opp_l:
        raise SystemExit("opportunities.rs missing inventory_state_breakdown_v1 projector fn")
    if "ready_to_earn" not in opp_l or "source_degraded" not in opp_l:
        raise SystemExit("opportunities.rs missing ready_to_earn/source_degraded fields")
    if "inventory_state_breakdown" not in main_l:
        raise SystemExit("main.rs must attach inventory-state-breakdown-v1 at projection boundary")
    if "inventory_state_breakdown_v1" not in main_l:
        raise SystemExit("main.rs must call inventory_state_breakdown_v1")
    if "inventory-state-breakdown-v1" not in home:
        raise SystemExit("site/home.js must consume inventory-state-breakdown-v1")
    if '["inventory-state-breakdown-v1"]' not in home and "['inventory-state-breakdown-v1']" not in home:
        raise SystemExit("site/home.js must read projection['inventory-state-breakdown-v1'] from API")
    print("  production projector boundary: OK")


def main() -> int:
    print(f"{SCHEMA} checker")
    try:
        assert_production_projector()
    except SystemExit as exc:
        print(f"  production: FAIL - {exc}", file=sys.stderr)
        return 1
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
