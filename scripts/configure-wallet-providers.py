#!/usr/bin/env python3
"""Inject and verify public wallet-provider deployment configuration."""

from __future__ import annotations

import argparse
import os
import re
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

PLACEHOLDER = "__COINBASE_CDP_PROJECT_ID__"
CANONICAL_REPOSITORY = "NSPG13/agent-bounties"
CANONICAL_PROJECT_ID = "9dfed88a-0b37-47e8-b867-96f1dfd0d4ee"
CANONICAL_ORIGIN = "https://agentbounties.app"
COINBASE_EMBEDDED_WALLET_API = "https://api.cdp.coinbase.com/embedded-wallet-api/projects"
PROJECT_ID = re.compile(r"^[A-Za-z0-9_-]{8,128}$")


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


def comma_header_values(value: str | None, *, uppercase: bool) -> set[str]:
    values = {
        item.strip()
        for item in (value or "").split(",")
        if item.strip()
    }
    return {item.upper() if uppercase else item.lower() for item in values}


def verify_production_origin(project_id: str, origin: str, timeout_seconds: float = 20.0) -> None:
    endpoint = f"{COINBASE_EMBEDDED_WALLET_API}/{urllib.parse.quote(project_id, safe='')}"
    request = urllib.request.Request(
        endpoint,
        method="OPTIONS",
        headers={
            "Origin": origin,
            "Access-Control-Request-Method": "POST",
            "Access-Control-Request-Headers": "content-type",
            "User-Agent": "agent-bounties-coinbase-origin-check/1",
        },
    )
    status = 0
    headers = None
    try:
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            status = response.status
            headers = response.headers
    except urllib.error.HTTPError as error:
        status = error.code
        headers = error.headers
    except urllib.error.URLError as error:
        raise SystemExit(
            "Coinbase production-origin verification could not reach the Embedded Wallet API; "
            "deployment stopped before publishing the wallet integration."
        ) from error

    allowed_origin = headers.get("Access-Control-Allow-Origin") if headers is not None else None
    allowed_methods = comma_header_values(
        headers.get("Access-Control-Allow-Methods") if headers is not None else None,
        uppercase=True,
    )
    allowed_headers = comma_header_values(
        headers.get("Access-Control-Allow-Headers") if headers is not None else None,
        uppercase=False,
    )
    if (
        not 200 <= status < 300
        or allowed_origin != origin
        or "POST" not in allowed_methods
        or "content-type" not in allowed_headers
    ):
        observed_origin = allowed_origin or "missing"
        observed_methods = ",".join(sorted(allowed_methods)) or "missing"
        observed_headers = ",".join(sorted(allowed_headers)) or "missing"
        raise SystemExit(
            "Coinbase has not authorized the exact production browser request. "
            f"Expected origin {origin!r}, method POST, and header content-type; observed HTTP {status}, "
            f"origin {observed_origin!r}, methods {observed_methods!r}, and headers {observed_headers!r}. "
            "In CDP Portal, open the project Security/Domains configuration, add the exact origin, "
            "save it, and rerun deployment."
        )
    print(f"Verified Coinbase Embedded Wallet browser authorization for {origin}")


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
        raise SystemExit("--verify-origin must be an HTTPS origin without credentials, path, query, or fragment")
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
        help="Require Coinbase CORS authorization for this exact HTTPS origin.",
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
