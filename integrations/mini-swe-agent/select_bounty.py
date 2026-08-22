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


def _records(payload: Any) -> tuple[dict[str, Any], list[dict[str, Any]]]:
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
    try:
        return Decimal(str(value if value is not None else "0"))
    except (InvalidOperation, ValueError):
        return Decimal(0)


def _parse_time(value: Any) -> datetime | None:
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
    explicit = str(value.get("snapshot_status", value.get("freshness", ""))).lower()
    if explicit in {"stale", "expired"} or value.get("stale") is True:
        return True
    for key in TIME_KEYS:
        observed = _parse_time(value.get(key))
        if observed is not None:
            return (now - observed).total_seconds() > max_age_seconds
    return False


def _document(record: dict[str, Any]) -> dict[str, Any]:
    terms = record.get("terms")
    if not isinstance(terms, dict):
        return {}
    document = terms.get("document")
    return document if isinstance(document, dict) else terms


def _margin(record: dict[str, Any]) -> Decimal:
    if "gross_cash_margin" in record:
        return _decimal(record.get("gross_cash_margin"))
    reward = _decimal(record.get("solver_reward"))
    spend = _decimal(record.get("required_external_spend"))
    return reward - spend


def _is_coding(record: dict[str, Any]) -> bool:
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
    for key in ("exclusive_claimant", "claimant", "solver"):
        value = record.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip().lower()
    return ""


def _contract(record: dict[str, Any]) -> str:
    return str(record.get("bounty_contract", record.get("id", "unknown")))


def _next(action: str, reason: str, record: dict[str, Any] | None = None) -> dict[str, Any]:
    contract = _contract(record) if record else None
    if action == "claim":
        exact = f"Prepare the operator-authorized claim for canonical bounty {contract}."
    elif action == "refresh":
        exact = "Fetch a fresh canonical inventory snapshot, then rerun this selector."
    elif action == "skip":
        exact = "Do not claim from this snapshot; inspect the next fresh canonical opportunity."
    else:
        exact = "Wait for a fresh canonical claimable coding opportunity, then rerun this selector."
    result: dict[str, Any] = {"action": action, "next_action": exact, "reason": reason}
    if record is not None:
        result["selected"] = {
            "bounty_contract": contract,
            "gross_cash_margin": str(_margin(record)),
            "source_url": _document(record).get("source_url", record.get("source_url")),
        }
    return result


def select(
    payload: Any,
    *,
    solver_wallet: str = "",
    max_age_seconds: int = 300,
    now: datetime | None = None,
) -> dict[str, Any]:
    current_time = now or datetime.now(timezone.utc)
    envelope, records = _records(payload)
    if _is_stale(envelope, current_time, max_age_seconds):
        return _next("refresh", "inventory_snapshot_stale")
    if not records:
        return _next("wait", "inventory_empty")

    fresh = [item for item in records if not _is_stale(item, current_time, max_age_seconds)]
    if not fresh:
        return _next("refresh", "all_inventory_records_stale")

    canonical = [
        item
        for item in fresh
        if str(item.get("status", "")).lower() == "claimable"
        and item.get("terms_valid", True) is not False
        and item.get("verification_ready", True) is not False
        and item.get("recovery_reserved", False) is not True
        and _is_coding(item)
    ]
    if not canonical:
        return _next("wait", "no_canonical_claimable_coding_work")

    profitable = [item for item in canonical if _margin(item) > 0]
    if not profitable:
        return _next("skip", "no_positive_margin_work", canonical[0])

    wallet = solver_wallet.strip().lower()
    available = [
        item
        for item in profitable
        if not _claimant(item) or (wallet and _claimant(item) == wallet)
    ]
    if not available:
        return _next("skip", "exclusive_claimant_mismatch", profitable[0])

    def rank(item: dict[str, Any]) -> tuple[Decimal, str]:
        return (-_margin(item), _contract(item).lower())

    selected = sorted(available, key=rank)[0]
    return _next("claim", "highest_positive_margin_canonical_coding_work", selected)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--solver-wallet", default="")
    parser.add_argument("--max-age-seconds", type=int, default=300)
    args = parser.parse_args(argv)
    if args.max_age_seconds < 1:
        parser.error("--max-age-seconds must be positive")
    try:
        payload = json.loads(args.input.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        print(json.dumps({"action": "refresh", "next_action": "Fetch a valid canonical inventory snapshot, then rerun this selector.", "reason": f"invalid_input:{error.__class__.__name__}"}))
        return 0
    print(
        json.dumps(
            select(
                payload,
                solver_wallet=args.solver_wallet,
                max_age_seconds=args.max_age_seconds,
            ),
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
