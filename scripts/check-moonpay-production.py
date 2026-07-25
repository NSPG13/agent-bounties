#!/usr/bin/env python3
"""Verify the deployed MoonPay surface without making a purchase or funding a bounty."""

from __future__ import annotations

import argparse
import json
import ssl
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any

DEFAULT_SITE_BASE = "https://agentbounties.app"
DEFAULT_MCP_BASE = "https://mcp.agentbounties.app"
SCHEMA_VERSION = "agent-bounties/moonpay-production-smoke-v1"
CHECKOUT_SCHEMA = "agent-bounties/moonpay-onramp-checkout-v1"
CANARY_WALLET = "0x1111111111111111111111111111111111111111"
CANARY_BOUNTY = "0x2222222222222222222222222222222222222222"
CHECKOUT_HOSTS = {"buy.moonpay.com", "buy-sandbox.moonpay.com"}


class SmokeFailure(RuntimeError):
    """Raised when the deployed surface violates the expected contract."""


def fetch_text(url: str, *, timeout: float) -> tuple[int, str, dict[str, str]]:
    request = urllib.request.Request(
        url,
        headers={
            "Accept": "text/html,application/javascript,text/plain;q=0.9,*/*;q=0.8",
            "Cache-Control": "no-cache",
            "Pragma": "no-cache",
            "User-Agent": "agent-bounties-moonpay-production-smoke/1",
        },
        method="GET",
    )
    with urllib.request.urlopen(request, timeout=timeout, context=ssl.create_default_context()) as response:
        return response.status, response.read().decode("utf-8"), dict(response.headers.items())


def post_json(
    url: str,
    payload: dict[str, Any],
    *,
    origin: str,
    timeout: float,
) -> tuple[int, dict[str, Any], dict[str, str]]:
    request = urllib.request.Request(
        url,
        data=json.dumps(payload, separators=(",", ":")).encode("utf-8"),
        headers={
            "Accept": "application/json",
            "Cache-Control": "no-cache",
            "Content-Type": "application/json",
            "Origin": origin,
            "Pragma": "no-cache",
            "User-Agent": "agent-bounties-moonpay-production-smoke/1",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout, context=ssl.create_default_context()) as response:
            raw = response.read().decode("utf-8")
            status = response.status
            headers = dict(response.headers.items())
    except urllib.error.HTTPError as error:
        raw = error.read().decode("utf-8")
        status = error.code
        headers = dict(error.headers.items())
    try:
        body = json.loads(raw)
    except json.JSONDecodeError as error:
        raise SmokeFailure(f"MoonPay endpoint returned non-JSON HTTP {status}: {raw[:200]!r}") from error
    if not isinstance(body, dict):
        raise SmokeFailure(f"MoonPay endpoint returned a non-object JSON response: {type(body).__name__}")
    return status, body, headers


def require_tokens(name: str, body: str, required: tuple[str, ...]) -> None:
    missing = [token for token in required if token not in body]
    if missing:
        raise SmokeFailure(f"{name} is missing deployed MoonPay markers: {missing}")


def verify_static(site_base: str, timeout: float) -> list[dict[str, Any]]:
    targets = [
        (
            "onramp.html",
            f"{site_base}/onramp.html",
            (
                "MoonPay wallet top-up",
                "Step 1 of 2",
                "Buying crypto does not fund the bounty",
                "moonpay-onramp.js",
            ),
        ),
        (
            "moonpay-onramp.js",
            f"{site_base}/moonpay-onramp.js",
            (
                "/v1/onramps/moonpay/checkout",
                "agent-bounties/moonpay-onramp-checkout-v1",
                "bounty_funded !== false",
                "canonical_funding_event !== null",
            ),
        ),
        (
            "moonpay-link.js",
            f"{site_base}/moonpay-link.js",
            (
                "onramp.html",
                "MoonPay",
                "bountyContract",
                "return",
            ),
        ),
    ]
    results: list[dict[str, Any]] = []
    for name, url, tokens in targets:
        status, body, headers = fetch_text(url, timeout=timeout)
        if status != 200:
            raise SmokeFailure(f"{name} returned HTTP {status}")
        require_tokens(name, body, tokens)
        results.append(
            {
                "name": name,
                "url": url,
                "status": status,
                "bytes": len(body.encode("utf-8")),
                "content_type": headers.get("Content-Type") or headers.get("content-type"),
                "verified_markers": list(tokens),
            }
        )
    return results


def verify_common_response(status: int, body: dict[str, Any]) -> None:
    if body.get("schema_version") != CHECKOUT_SCHEMA:
        raise SmokeFailure(
            f"MoonPay endpoint schema mismatch on HTTP {status}: {body.get('schema_version')!r}"
        )
    if body.get("bounty_funded") is not False:
        raise SmokeFailure("MoonPay endpoint did not preserve bounty_funded=false")
    if body.get("canonical_funding_event") is not None:
        raise SmokeFailure("MoonPay endpoint claimed a canonical funding event")


def verify_checkout_plan(body: dict[str, Any]) -> dict[str, Any]:
    if body.get("provider") != "moonpay":
        raise SmokeFailure("Configured checkout response did not identify provider=moonpay")
    if str(body.get("destination_wallet", "")).lower() != CANARY_WALLET:
        raise SmokeFailure("Configured checkout response changed the reviewed destination wallet")
    if str(body.get("bounty_contract", "")).lower() != CANARY_BOUNTY:
        raise SmokeFailure("Configured checkout response changed the reviewed bounty contract")
    checkout_url = str(body.get("checkout_url") or "")
    parsed = urllib.parse.urlparse(checkout_url)
    if parsed.scheme != "https" or parsed.hostname not in CHECKOUT_HOSTS:
        raise SmokeFailure(f"Configured checkout returned an unapproved host: {parsed.hostname!r}")
    pairs = urllib.parse.parse_qsl(parsed.query, keep_blank_values=True)
    if not pairs or pairs[-1][0] != "signature" or not pairs[-1][1]:
        raise SmokeFailure("Configured checkout URL does not append a non-empty signature last")
    query = dict(pairs)
    if query.get("walletAddress", "").lower() != CANARY_WALLET:
        raise SmokeFailure("Signed checkout URL changed the destination wallet")
    if not query.get("apiKey") or not query.get("currencyCode"):
        raise SmokeFailure("Signed checkout URL is missing its publishable key or currency code")
    if "secret" in checkout_url.lower() or "sk_live_" in checkout_url or "sk_test_" in checkout_url:
        raise SmokeFailure("Signed checkout URL appears to expose a secret key")
    return {
        "environment": body.get("environment"),
        "checkout_host": parsed.hostname,
        "currency_code": query.get("currencyCode"),
        "signature_last": True,
        "wallet_preserved": True,
        "bounty_boundary_preserved": True,
    }


def verify_endpoint(mcp_base: str, site_base: str, timeout: float) -> dict[str, Any]:
    endpoint = f"{mcp_base}/v1/onramps/moonpay/checkout"
    payload = {
        "wallet_address": CANARY_WALLET,
        "base_currency_amount": "20.00",
        "base_currency_code": "usd",
        "asset": "usdc",
        "return_url": f"{site_base}/onramp.html?moonpay_canary=1",
        "intent_id": None,
        "bounty_contract": CANARY_BOUNTY,
    }
    status, body, headers = post_json(endpoint, payload, origin=site_base, timeout=timeout)
    verify_common_response(status, body)
    result: dict[str, Any] = {
        "url": endpoint,
        "status": status,
        "content_type": headers.get("Content-Type") or headers.get("content-type"),
        "code": body.get("code"),
        "bounty_funded": body.get("bounty_funded"),
        "canonical_funding_event": body.get("canonical_funding_event"),
    }
    if status == 200:
        result["activation_state"] = "configured"
        result["checkout"] = verify_checkout_plan(body)
        return result
    if status == 503 and str(body.get("code") or "").startswith("moonpay_"):
        result["activation_state"] = "not_configured"
        result["error"] = body.get("error")
        result["next_action"] = body.get("next_action")
        return result
    raise SmokeFailure(
        f"MoonPay endpoint returned unexpected HTTP {status}: {json.dumps(body, sort_keys=True)[:500]}"
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--site-base", default=DEFAULT_SITE_BASE)
    parser.add_argument("--mcp-base", default=DEFAULT_MCP_BASE)
    parser.add_argument("--timeout", type=float, default=25.0)
    parser.add_argument("--require-checkout", action="store_true")
    parser.add_argument("--output")
    return parser.parse_args()


def normalized_base(value: str, name: str) -> str:
    parsed = urllib.parse.urlparse(value.rstrip("/"))
    if parsed.scheme != "https" or not parsed.hostname or parsed.path not in ("", "/"):
        raise SmokeFailure(f"{name} must be an HTTPS origin without a path")
    return f"https://{parsed.netloc}"


def main() -> int:
    args = parse_args()
    try:
        site_base = normalized_base(args.site_base, "--site-base")
        mcp_base = normalized_base(args.mcp_base, "--mcp-base")
        static = verify_static(site_base, args.timeout)
        endpoint = verify_endpoint(mcp_base, site_base, args.timeout)
        report = {
            "schema_version": SCHEMA_VERSION,
            "success": endpoint["activation_state"] == "configured" or not args.require_checkout,
            "checkout_required": args.require_checkout,
            "site_base": site_base,
            "mcp_base": mcp_base,
            "static": static,
            "endpoint": endpoint,
        }
        if args.require_checkout and endpoint["activation_state"] != "configured":
            report["failure"] = "MoonPay endpoint is deployed but partner credentials are not active."
        rendered = json.dumps(report, indent=2, sort_keys=True)
        print(rendered)
        if args.output:
            output = Path(args.output)
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(rendered + "\n", encoding="utf-8")
        return 0 if report["success"] else 1
    except (SmokeFailure, urllib.error.URLError, TimeoutError) as error:
        report = {
            "schema_version": SCHEMA_VERSION,
            "success": False,
            "checkout_required": args.require_checkout,
            "error": str(error),
        }
        rendered = json.dumps(report, indent=2, sort_keys=True)
        print(rendered, file=sys.stderr)
        if args.output:
            output = Path(args.output)
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(rendered + "\n", encoding="utf-8")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
