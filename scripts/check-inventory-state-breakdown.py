#!/usr/bin/env python3
"""Build a truthful lifecycle breakdown from one canonical snapshot.

The script is intentionally offline and deterministic.  A production caller can
serialize the same snapshot shape after fetching the canonical projection, but
must never combine counts from different fetches or treat an unavailable source
as an empty inventory.
"""

from __future__ import annotations

import json
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


SCHEMA = "inventory-state-breakdown-v1"
STATES = ("ready_to_earn", "in_progress", "submitted", "paid")


def _parse_timestamp(value: Any) -> datetime:
    if not isinstance(value, str) or not value.strip():
        raise ValueError("generated_at must be a non-empty ISO timestamp")
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise ValueError("generated_at must include a timezone")
    return parsed.astimezone(timezone.utc)


def project(snapshot: dict[str, Any], *, now: datetime | None = None) -> dict[str, Any]:
    """Return all lifecycle counts derived from exactly one snapshot.

    Degraded or stale input is represented explicitly as
    ``verification_unavailable``.  It is never converted to zero, because zero
    would falsely imply that the source was checked and found empty.
    """

    if snapshot.get("schema_version") != SCHEMA:
        raise ValueError("unsupported snapshot schema")
    generated_at = _parse_timestamp(snapshot.get("generated_at"))
    source = snapshot.get("source")
    if not isinstance(source, dict) or not isinstance(source.get("name"), str):
        raise ValueError("snapshot source is incomplete")
    source_status = source.get("status")
    rows = snapshot.get("items")
    if not isinstance(rows, list):
        raise ValueError("snapshot items must be a list")

    reference = now or datetime.now(timezone.utc)
    age_seconds = max(0, int((reference - generated_at).total_seconds()))
    stale = age_seconds > int(snapshot.get("max_age_seconds", 900))
    degraded = source_status != "ready" or bool(snapshot.get("degraded"))
    counts = {state: 0 for state in STATES}
    counts["verification_unavailable"] = 0
    if degraded or stale:
        counts["verification_unavailable"] = len(rows)
    else:
        for row in rows:
            if not isinstance(row, dict) or row.get("state") not in STATES:
                raise ValueError("item has an unknown lifecycle state")
            counts[row["state"]] += 1

    return {
        "schema_version": SCHEMA,
        "generated_at": generated_at.isoformat().replace("+00:00", "Z"),
        "source": {"name": source["name"], "status": source_status},
        "stale": stale,
        "degraded": degraded,
        "counts": counts,
        "evidence_boundary": (
            "All counts come from this one canonical snapshot. "
            "verification_unavailable means the source was stale or degraded; "
            "only canonical settlement evidence proves payment."
        ),
    }


def _load(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as handle:
        value = json.load(handle)
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain an object")
    return value


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    fixture_dir = root / "scripts" / "fixtures" / "inventory-state-breakdown"
    expected = {
        "empty": {"ready_to_earn": 0, "in_progress": 0, "submitted": 0, "paid": 0, "verification_unavailable": 0},
        "mixed": {"ready_to_earn": 1, "in_progress": 1, "submitted": 1, "paid": 1, "verification_unavailable": 0},
        "degraded": {"ready_to_earn": 0, "in_progress": 0, "submitted": 0, "paid": 0, "verification_unavailable": 2},
        "stale": {"ready_to_earn": 0, "in_progress": 0, "submitted": 0, "paid": 0, "verification_unavailable": 1},
    }
    fixed_now = datetime(2026, 8, 27, 12, 0, tzinfo=timezone.utc)
    results = {}
    for name, counts in expected.items():
        result = project(_load(fixture_dir / f"{name}.json"), now=fixed_now)
        if result["counts"] != counts:
            raise SystemExit(f"{name}: expected {counts}, got {result['counts']}")
        results[name] = result
    json.dump({"schema_version": SCHEMA, "projections": results}, sys.stdout, indent=2)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
