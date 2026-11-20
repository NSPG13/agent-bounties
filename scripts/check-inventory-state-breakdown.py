"""Checker for inventory-state breakdown fixtures and response."""

from __future__ import annotations

import json
import sys
from pathlib import Path

def main() -> None:
    fixtures_dir = Path(__file__).resolve().parent / "fixtures" / "inventory-state-breakdown"
    required_keys = {
        "schema_version",
        "ready_to_earn",
        "in_progress",
        "submitted",
        "paid",
        "verification_unavailable",
        "generated_at",
        "source",
    }
    for name in ("empty", "mixed", "degraded", "stale"):
        p = fixtures_dir / f"{name}.json"
        if not p.is_file():
            print(f"Missing fixture: {p}", file=sys.stderr)
            sys.exit(1)
        data = json.loads(p.read_text(encoding="utf-8"))
        missing = required_keys - set(data.keys())
        if missing:
            print(f"Fixture {name}.json missing keys: {missing}", file=sys.stderr)
            sys.exit(1)
        if data["schema_version"] != "inventory-state-breakdown-v1":
            print(f"Fixture {name}.json invalid schema_version", file=sys.stderr)
            sys.exit(1)
    print("All inventory-state breakdown fixtures passed validation.")

if __name__ == "__main__":
    main()
