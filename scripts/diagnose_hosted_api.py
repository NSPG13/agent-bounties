#!/usr/bin/env python3
"""Diagnose the documented hosted Agent Bounties API URL.

Classifies common failure modes (DNS, connection, HTTP status, wrong routes)
and prints actionable repair steps. Health success does **not** create funding,
credit balances, or authorize payout.
"""

from __future__ import annotations

import argparse
import json
import os
import socket
import sys
import urllib.error
import urllib.request
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

DEFAULT_BASE = os.environ.get(
    "PUBLIC_BASE_URL", "https://api.bountyboard.global"
).rstrip("/")

CHECK_PATHS = [
    "/health",
    "/v1/readiness/live-money",
    "/v1/bounties/funding-feed",
]
MAX_RESPONSE_BYTES = 4 * 1024 * 1024


@dataclass
class PathResult:
    path: str
    ok: bool
    status: int | None
    error: str | None
    body_preview: str | None = None


@dataclass
class Diagnosis:
    base_url: str
    hostname: str
    dns_ok: bool
    dns_error: str | None
    paths: list[PathResult] = field(default_factory=list)
    likely_causes: list[str] = field(default_factory=list)
    repair_steps: list[str] = field(default_factory=list)
    overall: str = "unknown"
    disclaimer: str = (
        "This diagnostic only checks HTTP reachability. Success does not create "
        "funding, credit balances, authorize payout, or mark any bounty payable."
    )


def fetch(url: str, timeout: float = 20.0) -> PathResult:
    path = urlparse(url).path or "/"
    req = urllib.request.Request(
        url,
        headers={"User-Agent": "agent-bounties-hosted-api-diagnose", "Accept": "*/*"},
        method="GET",
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read(MAX_RESPONSE_BYTES + 1)
            body = raw.decode("utf-8", errors="replace")
            preview = body[:200]
            status_ok = 200 <= resp.status < 300
            body_ok = len(raw) <= MAX_RESPONSE_BYTES
            body_error = None if body_ok else "response exceeds 4 MiB diagnostic limit"
            if body_ok and path == "/health":
                body_ok = body.strip() == "ok"
                if not body_ok:
                    body_error = "expected body 'ok'"
            elif body_ok and path in {
                "/v1/readiness/live-money",
                "/v1/bounties/funding-feed",
            }:
                try:
                    json.loads(body)
                except json.JSONDecodeError:
                    body_ok = False
                    body_error = "expected JSON body"
            return PathResult(
                path=path,
                ok=status_ok and body_ok,
                status=resp.status,
                error=body_error,
                body_preview=preview,
            )
    except urllib.error.HTTPError as exc:
        try:
            body = exc.read(200).decode("utf-8", errors="replace")
        except Exception:
            body = None
        return PathResult(
            path=path,
            ok=False,
            status=exc.code,
            error=f"HTTPError {exc.code}",
            body_preview=body,
        )
    except urllib.error.URLError as exc:
        return PathResult(
            path=path, ok=False, status=None, error=f"URLError: {exc.reason}"
        )
    except TimeoutError:
        return PathResult(path=path, ok=False, status=None, error="timeout")
    except Exception as exc:  # noqa: BLE001
        return PathResult(path=path, ok=False, status=None, error=str(exc))


def normalize_base_url(base_url: str) -> str:
    """Ensure base URL has an https scheme so DNS + fetch use the same host."""
    raw = (base_url or "").strip().rstrip("/")
    if not raw:
        raise ValueError("base URL must not be empty")
    if "://" not in raw:
        raw = f"https://{raw}"
    parsed = urlparse(raw)
    if not parsed.hostname:
        raise ValueError(f"invalid base URL (no hostname): {base_url!r}")
    netloc = parsed.netloc or parsed.hostname
    return f"{parsed.scheme}://{netloc}".rstrip("/")


def diagnose(base_url: str) -> Diagnosis:
    try:
        base = normalize_base_url(base_url)
    except ValueError as exc:
        return Diagnosis(
            base_url=base_url,
            hostname="",
            dns_ok=False,
            dns_error=str(exc),
            overall="invalid_url",
            likely_causes=[str(exc)],
            repair_steps=[
                "Pass a full URL, e.g. https://api.bountyboard.global",
                "Or a hostname only (https will be assumed), e.g. api.bountyboard.global",
            ],
        )

    parsed = urlparse(base)
    host = parsed.hostname or ""
    dns_ok = False
    dns_error = None
    try:
        socket.getaddrinfo(host, 443)
        dns_ok = True
    except socket.gaierror as exc:
        dns_error = str(exc)

    d = Diagnosis(
        base_url=base,
        hostname=host,
        dns_ok=dns_ok,
        dns_error=dns_error,
    )

    if not dns_ok:
        d.overall = "dns_failure"
        d.likely_causes = [
            "Hostname does not resolve — service never created, renamed, or DNS not published.",
            "Stale PUBLIC_BASE_URL in docs or env pointing at a non-existent Render hostname.",
        ]
        d.repair_steps = [
            "Open https://dashboard.render.com and confirm service agent-bounties-api exists.",
            "Apply Blueprint: https://dashboard.render.com/blueprint/new?repo=https://github.com/NSPG13/agent-bounties",
            "Copy the real service URL from Render and set PUBLIC_BASE_URL to match.",
            "Re-run: python scripts/diagnose_hosted_api.py --base-url <new-url>",
        ]
        return d

    for path in CHECK_PATHS:
        d.paths.append(fetch(f"{base}{path}"))

    statuses = [p.status for p in d.paths]
    non_null_statuses = [s for s in statuses if s is not None]
    errors = [p.error or "" for p in d.paths]

    if all(p.ok for p in d.paths):
        d.overall = "healthy"
        d.likely_causes = ["Hosted API responds on documented health/readiness paths."]
        d.repair_steps = [
            "No reachability repair needed.",
            "Before public Checkout: configure Stripe webhooks and set ENABLE_STRIPE_PUBLIC_CHECKOUT only after readiness gates pass (see docs/live-money-activation.md).",
        ]
        return d

    if any("timeout" in e.lower() or "timed out" in e.lower() for e in errors):
        d.overall = "timeout"
        d.likely_causes.append("Service spinning up (Render free/suspended) or overloaded.")
        d.repair_steps.append("Open Render dashboard → service → Logs; wait for deploy ready; retry.")

    if any(s in {502, 503, 504} for s in non_null_statuses):
        d.overall = "upstream_unavailable"
        d.likely_causes.extend(
            [
                "Container crash loop or failed health checks on Render.",
                "Wrong Docker start command / APP_BINARY not listening on $PORT.",
            ]
        )
        d.repair_steps.extend(
            [
                "Confirm Dockerfile / render.yaml: APP_PACKAGE=api, APP_BINARY=api, healthCheckPath=/health.",
                "Confirm process binds 0.0.0.0:$PORT (API falls back to Render PORT).",
                "Inspect deploy logs for panic/migration failures.",
            ]
        )

    # not_found only when we observed ≥1 real HTTP 404 and every non-null status is 404.
    # all([]) is True in Python — do not classify all-None connection errors as 404.
    if non_null_statuses and all(s == 404 for s in non_null_statuses):
        d.overall = "not_found"
        d.likely_causes.extend(
            [
                "Missing Render deployment for agent-bounties-api (Blueprint never applied).",
                "Wrong service URL (hostname exists as placeholder/parking but no app routes).",
                "Service deployed but routes not exposed (wrong binary, e.g. worker instead of api).",
                "Stale docs advertising agent-bounties-api.onrender.com while real host differs.",
            ]
        )
        d.repair_steps.extend(
            [
                "Apply Blueprint from repo root render.yaml (Dashboard → New Blueprint → this repo main).",
                "Confirm web service name agent-bounties-api, runtime docker, healthCheckPath=/health.",
                "Confirm env APP_PACKAGE=api and APP_BINARY=api (not worker).",
                "After deploy succeeds, open Render URL + /health and expect 200 with ok body.",
                "If Render assigned a different hostname, update PUBLIC_BASE_URL / MCP_BASE_URL and docs.",
                "Set repo vars PRODUCTION_API_BASE_URL only after production smoke passes.",
                "Run: python scripts/check-render-blueprint.py && python scripts/diagnose_hosted_api.py",
            ]
        )
    elif not non_null_statuses and d.paths and d.overall not in {"timeout"}:
        # Every path returned status=None (connection refused / reset / TLS, etc.)
        d.overall = "connection_failure"
        d.likely_causes.extend(
            [
                "DNS resolves but TCP/TLS connection fails (service suspended, not listening, or firewall).",
                "Render free tier spun down and edge returns connection errors instead of HTTP.",
            ]
        )
        d.repair_steps.extend(
            [
                "Open Render dashboard → service → confirm Live (not Suspended).",
                "Check service logs for crash loop / failed start.",
                "Confirm APP_BINARY=api binds 0.0.0.0:$PORT.",
                "Re-run: python scripts/diagnose_hosted_api.py --base-url <url>",
            ]
        )

    if not d.likely_causes:
        if non_null_statuses and all(200 <= s < 300 for s in non_null_statuses):
            d.overall = "route_mismatch"
            d.likely_causes.append(
                "Routes return 2xx but not the API contract (health must be 'ok'; readiness and funding feed must be JSON)."
            )
            d.repair_steps.extend(
                [
                    "Confirm the Render service runs APP_PACKAGE=api and APP_BINARY=api.",
                    "Check whether a proxy, static site, or stale service is bound to this hostname.",
                    "Confirm the deployed revision matches the expected main commit, then redeploy the API service.",
                ]
            )
        else:
            d.overall = "degraded"
            d.likely_causes.append("Mixed or unexpected HTTP errors; inspect path results.")
            d.repair_steps.append("Compare path statuses below with API route table in crates/api.")

    # Deduplicate while preserving order
    def uniq(items: list[str]) -> list[str]:
        seen: set[str] = set()
        out: list[str] = []
        for i in items:
            if i not in seen:
                seen.add(i)
                out.append(i)
        return out

    d.likely_causes = uniq(d.likely_causes)
    d.repair_steps = uniq(d.repair_steps)
    return d


def to_markdown(d: Diagnosis) -> str:
    lines = [
        f"# Hosted API diagnosis — `{d.overall}`",
        "",
        f"- Base URL: `{d.base_url}`",
        f"- Hostname: `{d.hostname}`",
        f"- DNS: {'ok' if d.dns_ok else f'FAIL ({d.dns_error})'}",
        "",
        "## Path checks",
        "",
    ]
    for p in d.paths:
        st = p.status if p.status is not None else "-"
        lines.append(f"- `{p.path}` → status={st} ok={p.ok} error={p.error}")
        if p.body_preview:
            lines.append(f"  - body: `{p.body_preview[:80].replace(chr(10), ' ')}`")
    lines.extend(["", "## Likely causes", ""])
    lines.extend(f"- {c}" for c in d.likely_causes)
    lines.extend(["", "## Repair steps", ""])
    for i, step in enumerate(d.repair_steps, 1):
        lines.append(f"{i}. {step}")
    lines.extend(["", "## Disclaimer", "", d.disclaimer, ""])
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--base-url", default=DEFAULT_BASE)
    p.add_argument("--json-out", type=Path, default=None)
    p.add_argument("--md-out", type=Path, default=None)
    p.add_argument(
        "--fixture",
        type=Path,
        default=None,
        help="JSON Diagnosis override for offline tests",
    )
    args = p.parse_args(argv)

    if args.fixture:
        data = json.loads(args.fixture.read_text(encoding="utf-8"))
        d = Diagnosis(**data)
        # rebuild path objects
        d.paths = [PathResult(**x) if isinstance(x, dict) else x for x in d.paths]
    else:
        d = diagnose(args.base_url)

    md = to_markdown(d)
    print(md)
    print("--- JSON ---")
    print(json.dumps(asdict(d), indent=2))

    if args.json_out:
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(json.dumps(asdict(d), indent=2) + "\n", encoding="utf-8")
    if args.md_out:
        args.md_out.parent.mkdir(parents=True, exist_ok=True)
        args.md_out.write_text(md, encoding="utf-8")

    return 0 if d.overall == "healthy" else 1


if __name__ == "__main__":
    sys.exit(main())
