#!/usr/bin/env python3
"""Inject and verify public wallet-provider deployment configuration."""

from __future__ import annotations

import argparse
import json
import os
import re
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Callable

PLACEHOLDER = "__COINBASE_CDP_PROJECT_ID__"
CANONICAL_REPOSITORY = "NSPG13/agent-bounties"
CANONICAL_PROJECT_ID = "9dfed88a-0b37-47e8-b867-96f1dfd0d4ee"
CANONICAL_ORIGIN = "https://agentbounties.app"
COINBASE_API_BASE = "https://api.cdp.coinbase.com/platform"
PROJECT_ID = re.compile(r"^[A-Za-z0-9_-]{8,128}$")
OpenUrl = Callable[..., object]


def configured_default() -> str:
    supplied = os.getenv("COINBASE_CDP_PROJECT_ID", "").strip()
    if supplied:
        return supplied
    if (
        os.getenv("GITHUB_ACTIONS", "").lower() == "true"
        and os.getenv("GITHUB_REPOSITORY") == CANONICAL_REPOSITORY
    ):
        return CANONICAL_PROJECT_ID
    return ""


def canonical_production_build() -> bool:
    return (
        os.getenv("GITHUB_ACTIONS", "").lower() == "true"
        and os.getenv("GITHUB_REPOSITORY") == CANONICAL_REPOSITORY
        and os.getenv("GITHUB_REF_NAME") == "main"
    )


def normalized_headers(headers: object) -> dict[str, str]:
    items = headers.items() if hasattr(headers, "items") else []
    return {str(key).lower(): str(value).strip() for key, value in items}


def comma_header_values(value: str | None, *, uppercase: bool) -> set[str]:
    values = {item.strip() for item in (value or "").split(",") if item.strip()}
    return {item.upper() if uppercase else item.lower() for item in values}


def request_evidence(
    request: urllib.request.Request,
    *,
    timeout_seconds: float,
    opener: OpenUrl,
) -> tuple[int, dict[str, str], bytes]:
    try:
        with opener(request, timeout=timeout_seconds) as response:
            return (
                int(response.status),
                normalized_headers(response.headers),
                response.read(),
            )
    except urllib.error.HTTPError as error:
        return int(error.code), normalized_headers(error.headers), error.read()
    except urllib.error.URLError as error:
        raise SystemExit(
            "Coinbase production-origin verification could not reach the Embedded Wallet API; "
            "deployment stopped before publishing the wallet integration."
        ) from error


def require_credentialed_origin(
    *,
    label: str,
    status: int,
    headers: dict[str, str],
    origin: str,
) -> None:
    observed_origin = headers.get("access-control-allow-origin")
    credentials = headers.get("access-control-allow-credentials", "").lower()
    if not 200 <= status < 300 or observed_origin != origin or credentials != "true":
        raise SystemExit(
            f"Coinbase rejected the {label} browser-origin check: HTTP {status}, "
            f"origin {observed_origin or 'missing'!r}, credentials {credentials or 'missing'!r}. "
            "Open the CDP Portal Embedded Wallets domain allowlist, add the exact HTTPS origin, "
            "save it, and rerun deployment."
        )


def require_preflight(
    *,
    label: str,
    status: int,
    headers: dict[str, str],
    origin: str,
    required_headers: set[str],
) -> None:
    require_credentialed_origin(
        label=label,
        status=status,
        headers=headers,
        origin=origin,
    )
    methods = comma_header_values(headers.get("access-control-allow-methods"), uppercase=True)
    allowed_headers = comma_header_values(
        headers.get("access-control-allow-headers"),
        uppercase=False,
    )
    missing = sorted(required_headers - allowed_headers)
    if "POST" not in methods or missing:
        raise SystemExit(
            f"Coinbase rejected the {label} preflight: methods {sorted(methods) or ['missing']}, "
            f"allowed headers {sorted(allowed_headers) or ['missing']}, missing {missing}. "
            "Do not publish the wallet integration until the exact SDK request is accepted."
        )


def verify_production_origin(
    project_id: str,
    origin: str,
    timeout_seconds: float = 20.0,
    *,
    opener: OpenUrl = urllib.request.urlopen,
) -> dict[str, object]:
    quoted_project = urllib.parse.quote(project_id, safe="")
    project_root = f"{COINBASE_API_BASE}/v2/embedded-wallet-api/projects/{quoted_project}"

    config_request = urllib.request.Request(
        f"{project_root}/config",
        method="GET",
        headers={
            "Origin": origin,
            "Accept": "application/json",
            "User-Agent": "agent-bounties-coinbase-origin-check/2",
        },
    )
    config_status, config_headers, config_body = request_evidence(
        config_request,
        timeout_seconds=timeout_seconds,
        opener=opener,
    )
    require_credentialed_origin(
        label="project configuration",
        status=config_status,
        headers=config_headers,
        origin=origin,
    )
    try:
        config_json = json.loads(config_body)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise SystemExit("Coinbase project configuration did not return valid JSON.") from error
    if not isinstance(config_json, dict):
        raise SystemExit("Coinbase project configuration must be a JSON object.")

    preflights = (
        (
            "authentication initialization",
            {"content-type", "x-idempotency-key"},
        ),
        (
            "authenticated method linking",
            {"content-type", "x-wallet-auth"},
        ),
    )
    preflight_results: list[dict[str, object]] = []
    for label, requested_headers in preflights:
        request = urllib.request.Request(
            f"{project_root}/auth/init",
            method="OPTIONS",
            headers={
                "Origin": origin,
                "Access-Control-Request-Method": "POST",
                "Access-Control-Request-Headers": ",".join(sorted(requested_headers)),
                "User-Agent": "agent-bounties-coinbase-origin-check/2",
            },
        )
        status, headers, _ = request_evidence(
            request,
            timeout_seconds=timeout_seconds,
            opener=opener,
        )
        require_preflight(
            label=label,
            status=status,
            headers=headers,
            origin=origin,
            required_headers=requested_headers,
        )
        preflight_results.append(
            {
                "label": label,
                "status": status,
                "requested_headers": sorted(requested_headers),
            }
        )

    print(f"Verified Coinbase Embedded Wallet browser authorization for {origin}")
    return {
        "project_config_status": config_status,
        "project_config_json": True,
        "preflights": preflight_results,
    }


def normalized_https_origin(value: str) -> str:
    parsed = urllib.parse.urlparse(value)
    if (
        parsed.scheme != "https"
        or not parsed.hostname
        or parsed.username is not None
        or parsed.password is not None
        or parsed.path not in ("", "/")
        or parsed.params
        or parsed.query
        or parsed.fragment
    ):
        raise SystemExit(
            "--verify-origin must be an HTTPS origin without credentials, path, query, or fragment"
        )
    try:
        _ = parsed.port
    except ValueError as error:
        raise SystemExit("--verify-origin contains an invalid port") from error
    return f"https://{parsed.netloc}"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--site-root", default="site")
    parser.add_argument("--coinbase-project-id", default=configured_default())
    parser.add_argument("--allow-disabled", action="store_true")
    parser.add_argument(
        "--verify-origin",
        help="Require Coinbase browser authorization for this exact HTTPS origin.",
    )
    parser.add_argument(
        "--skip-canonical-origin-check",
        action="store_true",
        help="Skip the automatic production check; intended only for deterministic offline tests.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    site_root = Path(args.site_root).resolve()
    config_path = site_root / "wallet-config.js"
    if not config_path.exists():
        raise SystemExit(f"wallet provider config is missing: {config_path}")

    project_id = args.coinbase_project_id.strip()
    if not project_id:
        if args.allow_disabled:
            print("Coinbase embedded wallet remains disabled: no COINBASE_CDP_PROJECT_ID was supplied")
            return 0
        raise SystemExit("COINBASE_CDP_PROJECT_ID is required for a production wallet build")
    if not PROJECT_ID.fullmatch(project_id):
        raise SystemExit("COINBASE_CDP_PROJECT_ID contains unsupported characters or length")

    origin = args.verify_origin
    if canonical_production_build() and not args.skip_canonical_origin_check:
        origin = origin or CANONICAL_ORIGIN
    if origin:
        verify_production_origin(project_id, normalized_https_origin(origin))

    source = config_path.read_text(encoding="utf-8")
    occurrences = source.count(PLACEHOLDER)
    if occurrences != 1:
        raise SystemExit(f"expected exactly one Coinbase project-id placeholder, found {occurrences}")
    configured = source.replace(PLACEHOLDER, project_id)
    forbidden = ("CDP_API_KEY_SECRET", "CDP_WALLET_SECRET", "privateKey", "seedPhrase")
    if any(term in configured for term in forbidden):
        raise SystemExit("server-side wallet secret material must never enter the static config")
    config_path.write_text(configured, encoding="utf-8")
    print(f"Configured Coinbase embedded wallets for project {project_id[:4]}…{project_id[-4:]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
