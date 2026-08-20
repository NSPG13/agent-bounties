#!/usr/bin/env python3
"""Collect redacted recent runtime diagnostics for Beta3 Render workers."""

from __future__ import annotations

import argparse
from datetime import datetime, timedelta, timezone
import json
import os
from pathlib import Path
import urllib.parse
from typing import Any

import render_deploy_recovery as render


WORKERS = (
    render.ServiceSpec(
        "agent-bounties-open-competition-v2-beta3-indexer",
        "background_worker",
        None,
    ),
    render.ServiceSpec(
        "agent-bounties-open-competition-v2-beta3-shadow",
        "background_worker",
        None,
    ),
    render.ServiceSpec(
        "agent-bounties-open-competition-v2-beta3-keeper",
        "background_worker",
        None,
    ),
    render.ServiceSpec(
        "agent-bounties-open-competition-v2-beta3-broker",
        "background_worker",
        None,
    ),
)


def collect(
    client: render.RenderClient,
    *,
    now: datetime,
) -> dict[str, Any]:
    end = now.astimezone(timezone.utc)
    start = end - timedelta(hours=1)
    workers = []
    for spec in WORKERS:
        service = client.resolve_service(spec)
        service_id = service.get("id")
        owner_id = service.get("ownerId")
        if not isinstance(service_id, str) or not service_id.startswith("srv-"):
            raise render.RecoveryError(f"{spec.name} is missing its Render service id")
        if not isinstance(owner_id, str) or not owner_id:
            raise render.RecoveryError(f"{spec.name} is missing its Render workspace id")
        parameters: dict[str, object] = {
            "ownerId": owner_id,
            "resource": [service_id],
            "type": ["app"],
            "direction": "backward",
            "startTime": start.isoformat().replace("+00:00", "Z"),
            "endTime": end.isoformat().replace("+00:00", "Z"),
            "limit": "100",
        }
        query = urllib.parse.urlencode(parameters, doseq=True)
        summary = render.summarize_build_logs(client._read_with_retry(f"/logs?{query}"))
        workers.append(
            {
                "name": spec.name,
                "service_id": service_id,
                "runtime_logs": summary,
            }
        )
    return {
        "schema_version": "agent-bounties/open-competition-v2-beta3-render-diagnostic-v1",
        "observed_at": end.isoformat().replace("+00:00", "Z"),
        "workers": workers,
        "read_only": True,
        "secrets_redacted": True,
        "evidence_boundary": (
            "Runtime diagnostics can explain readiness failures. They cannot prove "
            "funding, qualification, settlement, payout, or refund."
        ),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    client = render.RenderClient(os.environ.get("RENDER_API_KEY", ""))
    result = collect(client, now=datetime.now(timezone.utc))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
