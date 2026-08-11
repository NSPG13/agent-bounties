#!/usr/bin/env python3
"""Verify the deployed MoonPay surface without making a purchase or funding a bounty."""

from __future__ import annotations

import argparse
import json
import ssl
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any

DEFAULT_SITE_BASE = "https://agentbounties.app"
DEFAULT_MCP_BASE = "https://mcp.agentbounties.app"
SCHEMA_VERSION = "agent-bounties/moonpay-production-smoke-v2"
CHECKOUT_SCHEMA = "agent-bounties/moonpay-onramp-checkout-v1"
CANARY_WALLET = "0x1111111111111111111111111111111111111111"
CANARY_BOUNTY = "0x2222222222222222222222222222222222222222"
CHECKOUT_HOSTS = {"buy.moonpay.com", "buy-sandbox.moonpay.com"}
DIRECT_CHECKOUT_URLS = {
    "usdc": "https://www.moonpay.com/buy/usdc",
    "eth": "https://www.moonpay.com/buy/eth",
}
RETRYABLE_ENDPOINT_STATUSES = {404, 502, 504}


class SmokeFailure(RuntimeError):
    """Raised when the deployed surface violates the expected contract."""


def fetch_text(url: str, *, timeout: float) -> tuple[int, str, dict[str, str]]:
    request = urllib.request.Request(
        url,
        headers={
            "Accept": "text/html,application/javascript,text/plain;q=0.9,*/*;q=0.8",
            "Cache-Control": "no-cache",
            "Pragma": "no-cache",
            "User-Agent": "agent-bounties-moonpay-production-smoke/2",
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
            "User-Agent": "agent-bounties-moonpay-production-smoke/2",
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


def verify_static(site_base: str, timeout: float) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    targets = [
        (
            "onramp.html",
            f"{site_base}/onramp.html",
            (
                "Step 1 of 2",
                "Buying crypto does not fund the bounty",
                "Direct MoonPay fallback",
                "cannot prefill or cryptographically bind your wallet",
                "moonpay-onramp.js",
                "moonpay-direct-fallback.js",
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
        (
            "moonpay-direct-fallback.js",
            f"{site_base}/moonpay-direct-fallback.js",
            (
                DIRECT_CHECKOUT_URLS["usdc"],
                DIRECT_CHECKOUT_URLS["eth"],
                "USDC on Base (USDC_BASE)",
                "ETH on Base (ETH_BASE)",
                "navigator.clipboard.writeText",
                'setAttribute("aria-disabled"',
                "stop if the final screen shows another network or address",
            ),
        ),
    ]
    results: list[dict[str, Any]] = []
    direct_body = ""
    for name, url, tokens in targets:
        status, body, headers = fetch_text(url, timeout=timeout)
        if status != 200:
            raise SmokeFailure(f"{name} returned HTTP {status}")
        require_tokens(name, body, tokens)
        if name == "moonpay-direct-fallback.js":
            direct_body = body
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

    forbidden = [term for term in ("apiKey=", "walletAddress=", "signature=") if term in direct_body]
    if forbidden:
        raise SmokeFailure(f"direct MoonPay fallback imitates a signed or wallet-prefilled URL: {forbidden}")
    direct = {
        "active": True,
        "provider": "moonpay",
        "mode": "direct_consumer_checkout",
        "checkout_urls": DIRECT_CHECKOUT_URLS,
        "wallet_prefilled": False,
        "context_cryptographically_bound": False,
        "explicit_wallet_copy_required": True,
        "base_asset_review_required": True,
        "bounty_funded": False,
        "canonical_funding_event": None,
    }
    return results, direct


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
        raise SmokeFailure("configured checkout response did not identify provider=moonpay")
    if str(body.get("destination_wallet", "")).lower() != CANARY_WALLET:
        raise SmokeFailure("configured checkout response changed the reviewed destination wallet")
    if str(body.get("bounty_contract", "")).lower() != CANARY_BOUNTY:
        raise SmokeFailure("configured checkout response changed the reviewed bounty contract")
    checkout_url = str(body.get("checkout_url") or "")
    parsed = urllib.parse.urlparse(checkout_url)
    if parsed.scheme != "https" or parsed.hostname not in CHECKOUT_HOSTS:
        raise SmokeFailure(f"configured checkout returned an unapproved host: {parsed.hostname!r}")
    pairs = urllib.parse.parse_qsl(parsed.query, keep_blank_values=True)
    if not pairs or pairs[-1][0] != "signature" or not pairs[-1][1]:
        raise SmokeFailure("configured checkout URL does not append a non-empty signature last")
    query = dict(pairs)
    if query.get("walletAddress", "").lower() != CANARY_WALLET:
        raise SmokeFailure("signed checkout URL changed the destination wallet")
    if not query.get("apiKey") or not query.get("currencyCode"):
        raise SmokeFailure("signed checkout URL is missing its publishable key or currency code")
    if "secret" in checkout_url.lower() or "sk_live_" in checkout_url or "sk_test_" in checkout_url:
        raise SmokeFailure("signed checkout URL appears to expose a secret key")
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

    status = 0
    body: dict[str, Any] = {}
    headers: dict[str, str] = {}
    attempts = 0
    for attempts in range(1, 5):
        status, body, headers = post_json(endpoint, payload, origin=site_base, timeout=timeout)
        if status not in RETRYABLE_ENDPOINT_STATUSES or attempts == 4:
            break
        time.sleep(float(attempts))

    verify_common_response(status, body)
    result: dict[str, Any] = {
        "url": endpoint,
        "status": status,
        "attempts": attempts,
        "content_type": headers.get("Content-Type") or headers.get("content-type"),
        "code": body.get("code"),
        "bounty_funded": body.get("bounty_funded"),
        "canonical_funding_event": body.get("canonical_funding_event"),
    }
    if status == 200:
        result["activation_state"] = "configured"
        result["route_healthy"] = True
        result["checkout"] = verify_checkout_plan(body)
        return result
    if status == 503 and str(body.get("code") or "").startswith("moonpay_"):
        result["activation_state"] = "not_configured"
        result["route_healthy"] = True
        result["error"] = body.get("error")
        result["next_action"] = body.get("next_action")
        return result
    if status == 429 and body.get("code") == "moonpay_checkout_rate_limited":
        result["activation_state"] = "rate_limited"
        result["route_healthy"] = True
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
        static, direct = verify_static(site_base, args.timeout)
        endpoint = verify_endpoint(mcp_base, site_base, args.timeout)
        active_paths = {
            "direct_consumer": direct["active"],
            "signed_partner": endpoint["activation_state"] == "configured",
        }
        success = any(active_paths.values()) and endpoint["route_healthy"]
        if args.require_checkout:
            success = success and active_paths["signed_partner"]
        report = {
            "schema_version": SCHEMA_VERSION,
            "success": success,
            "checkout_required": args.require_checkout,
            "site_base": site_base,
            "mcp_base": mcp_base,
            "static": static,
            "direct_fallback": direct,
            "endpoint": endpoint,
            "active_paths": active_paths,
        }
        if args.require_checkout and not active_paths["signed_partner"]:
            report["failure"] = (
                "The MoonPay on-ramp is available through the direct consumer fallback, "
                "but the server-signed partner checkout is not active."
            )
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
