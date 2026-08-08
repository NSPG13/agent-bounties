#!/usr/bin/env python3
"""Select one canonical paid coding task and emit one exact next action."""

from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime, timezone
from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import Any

CONTAINER_KEYS = ("opportunities", "bounties", "items", "data")
TIME_KEYS = ("observed_at", "generated_at", "snapshot_at", "updated_at")
STALE_THRESHOLD_SECONDS = 86400  # 24 hours


def _records(payload: Any) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """Extract canonical records from a payload, handling nested containers."""
    if isinstance(payload, list):
        return {}, [item for item in payload if isinstance(item, dict)]
    if not isinstance(payload, dict):
        return {}, []
    for key in CONTAINER_KEYS:
        value = payload.get(key)
        if isinstance(value, list):
            return payload, [item for item in value if isinstance(item, dict)]
    if any(key in payload for key in ("status", "bounty_contract", "terms")):
        return {}, [payload]
    return payload, []


def _decimal(value: Any) -> Decimal:
    """Safely parse a decimal from any value, returning 0 on failure."""
    try:
        return Decimal(str(value if value is not None else "0"))
    except (InvalidOperation, ValueError):
        return Decimal(0)


def _parse_time(value: Any) -> datetime | None:
    """Parse an ISO-8601 timestamp, returning None on failure."""
    if not isinstance(value, str) or not value.strip():
        return None
    normalized = value.strip().replace("Z", "+00:00")
    try:
        parsed = datetime.fromisoformat(normalized)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def _is_stale(value: dict[str, Any], now: datetime, max_age_seconds: int) -> bool:
    """Check if a snapshot or record is stale based on explicit flag or timestamp."""
    explicit = str(value.get("snapshot_status", value.get("freshness", ""))).lower()
    if explicit in {"stale", "expired"} or value.get("stale") is True:
        return True
    for key in TIME_KEYS:
        observed = _parse_time(value.get(key))
        if observed is not None:
            return (now - observed).total_seconds() > max_age_seconds
    return False


def _document(record: dict[str, Any]) -> dict[str, Any]:
    """Extract the canonical terms document from a bounty record."""
    terms = record.get("terms")
    if not isinstance(terms, dict):
        return {}
    document = terms.get("document")
    return document if isinstance(document, dict) else terms


def _margin(record: dict[str, Any]) -> Decimal:
    """Calculate the net cash margin available to the solver."""
    if "gross_cash_margin" in record:
        return _decimal(record.get("gross_cash_margin"))
    reward = _decimal(record.get("solver_reward"))
    spend = _decimal(record.get("required_external_spend"))
    return reward - spend


def _is_coding(record: dict[str, Any]) -> bool:
    """Determine if a bounty record represents a paid coding task."""
    document = _document(record)
    benchmark = document.get("benchmark", {})
    if isinstance(benchmark, dict) and benchmark.get("engine") == "sandboxed_regression_v1":
        return True
    source = str(document.get("source_url", record.get("source_url", ""))).lower()
    labels = " ".join(str(value) for value in record.get("labels", []))
    kind = str(record.get("kind", record.get("template", "")))
    return "github.com/" in source and any(
        token in f"{labels} {kind}".lower()
        for token in ("code", "coding", "software", "small-code-change")
    )


def _claimant(record: dict[str, Any]) -> str:
    """Extract the exclusive claimant address if one exists."""
    for key in ("exclusive_claimant", "claimant", "solver"):
        value = record.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip().lower()
    return ""


def _contract(record: dict[str, Any]) -> str:
    """Get the canonical bounty contract identifier."""
    return str(record.get("bounty_contract", record.get("id", "unknown")))


def _is_claimable(record: dict[str, Any]) -> bool:
    """Check if a bounty is in a claimable state."""
    status = str(record.get("status", "")).lower()
    if status not in ("claimable", "open"):
        return False
    terms_valid = record.get("terms_valid")
    if terms_valid is False:
        return False
    verification_ready = record.get("verification_ready")
    if verification_ready is False:
        return False
    if _claimant(record):
        return False
    return True


def _next(action: str, reason: str, record: dict[str, Any] | None = None) -> dict[str, Any]:
    """Build a selector result with one exact next action."""
    contract = _contract(record) if record else None
    if action == "claim":
        exact = (
            f"Prepare the operator-authorized claim for canonical bounty {contract}. "
            "Include source_snapshot_digest, discovery_source, and evidence package. "
            "Never sign or broadcast a wallet action."
        )
    elif action == "refresh":
        exact = "Fetch a fresh canonical inventory snapshot, then rerun this selector."
    elif action == "wait":
        exact = "Wait for the next inventory update cycle before checking again."
    elif action == "skip":
        if record and _claimant(record):
            exact = f"Skipping {contract}: already claimed by another solver."
        else:
            exact = f"Skipping {contract}: no margin available for this task."
    else:
        exact = reason
    return {"action": action, "next_action": exact}


def select(inventory: dict[str, Any]) -> dict[str, Any]:
    """Core selection logic: one canonical coding task or a skip reason."""
    meta, records = _records(inventory)
    if not records:
        return _next("wait", "No bounty records found in inventory.")

    now = datetime.now(timezone.utc)

    # Check for stale snapshot
    if _is_stale(meta, now, STALE_THRESHOLD_SECONDS):
        return _next("refresh", "Inventory snapshot is stale (>24h old).")

    if _is_stale(meta, now, 3600) and any(
        _is_stale(r, now, 3600) for r in records
    ):
        return _next("refresh", "Inventory snapshot and records are stale.")

    # Filter to valid coding tasks with positive margin
    coding_tasks = []
    for record in records:
        if not _is_coding(record):
            continue

        # Check staleness
        if _is_stale(record, now, STALE_THRESHOLD_SECONDS):
            continue

        # Check exclusive claimant
        claimant = _claimant(record)
        if claimant:
            return _next("skip", f"Exclusive claimant {claimant} already assigned.", record)

        # Check claimability
        if not _is_claimable(record):
            continue

        # Check margin
        m = _margin(record)
        if m <= 0:
            return _next("skip", f"Non-positive margin ({m}) for {_contract(record)}.", record)

        coding_tasks.append((m, record))

    if not coding_tasks:
        return _next("wait", "No claimable coding tasks with positive margin found.")

    # Select the highest-margin task
    coding_tasks.sort(key=lambda x: x[0], reverse=True)
    best_margin, best_record = coding_tasks[0]

    return _next("claim", f"Selected {_contract(best_record)} with margin {best_margin}.", best_record)


def main() -> None:
    parser = argparse.ArgumentParser(description="Select one canonical paid coding task.")
    parser.add_argument("--input", required=True, help="Path to inventory JSON file.")
    args = parser.parse_args()

    input_path = Path(args.input)
    if not input_path.is_file():
        print(json.dumps({"action": "wait", "next_action": f"Input file not found: {args.input}"}))
        sys.exit(1)

    try:
        inventory = json.loads(input_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        print(json.dumps({"action": "wait", "next_action": f"Invalid JSON: {error}"}))
        sys.exit(1)

    result = select(inventory)
    print(json.dumps(result))


if __name__ == "__main__":
    main()
