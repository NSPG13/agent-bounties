#!/usr/bin/env python3
"""Deterministic contract checks for inventory-state-breakdown-v1."""
from __future__ import annotations
import json
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "scripts" / "fixtures" / "inventory-state-breakdown"

def derive(snapshot):
    source = next((s for s in snapshot.get("source_statuses", []) if s.get("source_type") == "canonical_base"), None)
    items = [i for i in snapshot.get("items", []) if i.get("source_type") == "canonical_base"]
    def ready(i):
        return i.get("work_state") == "claimable" and i.get("payment_state") == "escrowed" and i.get("payment_committed") is True and i.get("verification_ready") is True and i.get("gross_cash_margin_positive") is True
    return {
        "schema_version": "inventory-state-breakdown-v1",
        "generated_at": snapshot["generated_at"],
        "source": {"source_type": "canonical_base", "available": bool(source and source.get("available") is True)},
        "ready_to_earn": sum(ready(i) for i in items),
        "in_progress": sum(i.get("work_state") == "in_progress" for i in items),
        "submitted": sum(i.get("work_state") == "submitted" for i in items),
        "paid": sum(i.get("work_state") == "completed" and i.get("payment_state") == "paid" for i in items),
        "verification_unavailable": sum(i.get("payment_committed") is True and i.get("work_state") in {"claimable", "in_progress", "submitted"} and i.get("verification_ready") is False for i in items),
    }

def main():
    rust = (ROOT / "crates/api/src/opportunities.rs").read_text()
    main_rs = (ROOT / "crates/api/src/main.rs").read_text()
    home = (ROOT / "site/home.js").read_text()
    for token in ("inventory-state-breakdown-v1", "ready_to_earn", "in_progress", "submitted", "paid", "verification_unavailable", "generated_at", "source", "fn is_ready_to_earn", "pub fn inventory_state_breakdown"):
        if token not in rust:
            raise SystemExit(f"production projector missing {token}")
    if "inventory_state_breakdown(&items, &source_statuses, &generated_at)" not in main_rs:
        raise SystemExit("API response is not derived from one accepted projection snapshot")
    if 'breakdown.schema_version !== "inventory-state-breakdown-v1"' not in home:
        raise SystemExit("homepage does not consume inventory-state-breakdown-v1")
    if 'Number(breakdown.ready_to_earn) !== readyItems.length' not in home:
        raise SystemExit("homepage does not preserve strict ready-to-earn filtering")
    fields=("ready_to_earn","in_progress","submitted","paid","verification_unavailable")
    for name in ("empty","mixed","degraded","stale"):
        snapshot=json.loads((FIXTURES/f"{name}.json").read_text())
        actual=derive(snapshot); expected=snapshot["expected"]
        if actual["generated_at"] != snapshot["generated_at"]:
            raise SystemExit(f"{name}: generated_at changed")
        if actual["source"]["available"] != expected["source_available"]:
            raise SystemExit(f"{name}: source availability mismatch")
        for field in fields:
            if actual[field] != expected[field]:
                raise SystemExit(f"{name}: {field} expected {expected[field]} got {actual[field]}")
    print("inventory-state-breakdown-v1 deterministic fixtures passed")
if __name__ == "__main__": main()
