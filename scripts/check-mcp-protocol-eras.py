#!/usr/bin/env python3
"""Read-only probe for modern and legacy MCP on one Streamable HTTP endpoint."""

from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.request
from typing import Any


MODERN_VERSION = "2026-07-28"
LEGACY_VERSION = "2025-06-18"
WIDGET_URI = "ui://agent-bounties/live-feed-v4.html"


def post_json(
    endpoint: str, payload: dict[str, Any], headers: dict[str, str]
) -> tuple[int, dict[str, Any]]:
    request = urllib.request.Request(
        endpoint,
        data=json.dumps(payload, separators=(",", ":")).encode("utf-8"),
        headers={"Content-Type": "application/json", **headers},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            body = response.read().decode("utf-8", errors="replace")
            try:
                decoded = json.loads(body)
            except json.JSONDecodeError:
                decoded = {"invalid_json": body}
            return response.status, decoded
    except urllib.error.HTTPError as error:
        body = error.read().decode("utf-8", errors="replace")
        try:
            decoded = json.loads(body)
        except json.JSONDecodeError:
            decoded = {"http_error": body or str(error)}
        return error.code, decoded
    except (urllib.error.URLError, TimeoutError) as error:
        return 0, {"transport_error": str(error)}


def modern_request(
    endpoint: str,
    request_id: int,
    method: str,
    params: dict[str, Any] | None = None,
) -> tuple[int, dict[str, Any]]:
    params = dict(params or {})
    params["_meta"] = {
        "io.modelcontextprotocol/protocolVersion": MODERN_VERSION,
        "io.modelcontextprotocol/clientInfo": {
            "name": "agent-bounties-era-probe",
            "version": "1",
        },
        "io.modelcontextprotocol/clientCapabilities": {},
    }
    headers = {
        "Accept": "application/json, text/event-stream",
        "MCP-Protocol-Version": MODERN_VERSION,
        "Mcp-Method": method,
    }
    if method == "resources/read":
        headers["Mcp-Name"] = str(params["uri"])
    elif method in ("tools/call", "prompts/get"):
        headers["Mcp-Name"] = str(params["name"])
    return post_json(
        endpoint,
        {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params},
        headers,
    )


def legacy_request(
    endpoint: str,
    request_id: int,
    method: str,
    params: dict[str, Any] | None = None,
) -> tuple[int, dict[str, Any]]:
    return post_json(
        endpoint,
        {
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params or {},
        },
        {"Accept": "application/json, text/event-stream"},
    )


def modern_ok(response: dict[str, Any]) -> bool:
    result = response.get("result")
    return bool(
        isinstance(result, dict)
        and MODERN_VERSION in result.get("supportedVersions", [])
        and result.get("resultType") == "complete"
        and result.get("cacheScope") == "public"
        and type(result.get("ttlMs")) is int
        and result["ttlMs"] > 0
        and isinstance(
            result.get("_meta", {}).get("io.modelcontextprotocol/serverInfo"), dict
        )
    )


def legacy_ok(response: dict[str, Any]) -> bool:
    result = response.get("result")
    return bool(
        isinstance(result, dict)
        and result.get("protocolVersion") == LEGACY_VERSION
        and isinstance(result.get("serverInfo"), dict)
    )


def successful_result(status: int, response: dict[str, Any]) -> bool:
    return status == 200 and isinstance(response.get("result"), dict)


def successful_modern_result(
    status: int, response: dict[str, Any], *, cacheable: bool = False
) -> bool:
    result = response.get("result")
    if not (
        status == 200
        and isinstance(result, dict)
        and result.get("resultType") == "complete"
        and isinstance(
            result.get("_meta", {}).get("io.modelcontextprotocol/serverInfo"), dict
        )
    ):
        return False
    return not cacheable or bool(
        result.get("cacheScope") == "public"
        and type(result.get("ttlMs")) is int
        and result["ttlMs"] > 0
    )


def probe(endpoint: str) -> dict[str, Any]:
    modern_status, modern_discovery = modern_request(
        endpoint, 1, "server/discover"
    )
    modern_available = modern_status == 200 and modern_ok(modern_discovery)
    modern_checks: dict[str, bool] = {"server_discover": modern_available}
    if modern_available:
        status, response = modern_request(endpoint, 2, "tools/list")
        modern_checks["tools_list"] = successful_modern_result(
            status, response, cacheable=True
        ) and bool(response["result"].get("tools"))
        status, response = modern_request(endpoint, 3, "resources/read", {"uri": WIDGET_URI})
        modern_checks["resources_read"] = successful_modern_result(
            status, response, cacheable=True
        ) and bool(response["result"].get("contents"))
        status, response = modern_request(
            endpoint,
            4,
            "tools/call",
            {
                "name": "prepare_moonpay_onramp",
                "arguments": {
                    "bounty_contract": "0x1111111111111111111111111111111111111111",
                    "amount_base_units": 3_500_000,
                },
            },
        )
        modern_checks["tools_call"] = successful_modern_result(
            status, response
        ) and bool(response["result"].get("content"))
        modern_available = all(modern_checks.values())

    legacy_status, legacy_initialize = legacy_request(
        endpoint,
        11,
        "initialize",
        {
            "protocolVersion": LEGACY_VERSION,
            "capabilities": {},
            "clientInfo": {"name": "agent-bounties-era-probe", "version": "1"},
        },
    )
    legacy_available = legacy_status == 200 and legacy_ok(legacy_initialize)
    legacy_checks: dict[str, bool] = {"initialize": legacy_available}
    if legacy_available:
        status, response = legacy_request(endpoint, 12, "tools/list")
        legacy_checks["tools_list"] = successful_result(status, response) and bool(
            response["result"].get("tools")
        )
        status, response = legacy_request(
            endpoint, 13, "resources/read", {"uri": WIDGET_URI}
        )
        legacy_checks["resources_read"] = successful_result(status, response) and bool(
            response["result"].get("contents")
        )
        status, response = legacy_request(
            endpoint,
            14,
            "tools/call",
            {
                "name": "prepare_moonpay_onramp",
                "arguments": {
                    "bounty_contract": "0x1111111111111111111111111111111111111111",
                    "amount_base_units": 3_500_000,
                },
            },
        )
        legacy_checks["tools_call"] = successful_result(status, response) and bool(
            response["result"].get("content")
        )
        legacy_available = all(legacy_checks.values())

    modern_error = modern_discovery.get("error", {})
    return {
        "schema_version": "agent-bounties/mcp-era-probe-v1",
        "endpoint": endpoint,
        "modern": {
            "protocol_version": MODERN_VERSION,
            "available": modern_available,
            "checks": modern_checks,
            "discover_http_status": modern_status,
            "discover_error_code": modern_error.get("code"),
            "discover_error_message": modern_error.get("message"),
        },
        "legacy": {
            "protocol_version": LEGACY_VERSION,
            "available": legacy_available,
            "checks": legacy_checks,
            "initialize_http_status": legacy_status,
        },
    }


def expectation_met(report: dict[str, Any], expected: str) -> bool:
    modern = bool(report["modern"]["available"])
    legacy = bool(report["legacy"]["available"])
    return {
        "dual": modern and legacy,
        "modern": modern,
        "legacy": legacy,
        "either": modern or legacy,
    }[expected]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--endpoint",
        default="https://mcp.agentbounties.app/mcp",
        help="Full Streamable HTTP MCP endpoint.",
    )
    parser.add_argument(
        "--expect",
        choices=("dual", "modern", "legacy", "either"),
        default="dual",
        help="Required availability; dual is the release target.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    report = probe(args.endpoint)
    report["expected"] = args.expect
    report["passed"] = expectation_met(report, args.expect)
    print(json.dumps(report, indent=2))
    if report["passed"]:
        return 0
    print(
        f"mcp_era_probe=failed expected={args.expect} "
        f"modern={str(report['modern']['available']).lower()} "
        f"legacy={str(report['legacy']['available']).lower()}",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
