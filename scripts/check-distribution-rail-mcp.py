#!/usr/bin/env python3
"""Side-effect-free MCP protocol canary for every attributed distribution rail.

The probe calls only `initialize` and `tools/list`. It creates an explicitly marked,
measurement-excluded analytics acquisition but never prepares a draft, opens
wallet review, signs, funds, or invokes any write tool.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import urllib.error
import urllib.request


RAILS = (
    "bankr",
    "openclaw",
    "vscode",
    "cursor",
    "cline",
    "github",
    "linear",
    "claude-custom",
    "chatgpt-dev",
    "glama",
    "mcp-so",
    "mcpservers",
)
ACQUISITION_HEADER = "x-agent-bounties-acquisition-id"
RAIL_HEADER = "x-agent-bounties-attribution-rail"
FIRST_TOUCH_HEADER = "x-agent-bounties-first-touch-rail"
CANARY_HEADER = "x-agent-bounties-canary"
MEASUREMENT_ELIGIBLE_HEADER = "x-agent-bounties-measurement-eligible"
TOKEN = re.compile(r"^aba1_[0-9a-f]{64}\.[0-9a-f]{64}$")


def post_json(url: str, payload: dict[str, object], headers: dict[str, str] | None = None):
    request_headers = {
        "accept": "application/json",
        "content-type": "application/json",
        "user-agent": "agent-bounties-distribution-canary/1",
    }
    request_headers.update(headers or {})
    request = urllib.request.Request(
        url,
        data=json.dumps(payload, separators=(",", ":")).encode("utf-8"),
        headers=request_headers,
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            return response.status, dict(response.headers.items()), json.load(response)
    except urllib.error.HTTPError as error:
        body = error.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"{url} returned HTTP {error.code}: {body[:500]}") from error


def header(headers: dict[str, str], name: str) -> str | None:
    lowered = name.lower()
    return next((value for key, value in headers.items() if key.lower() == lowered), None)


def check_rail(endpoint: str, rail: str, canary_kind: str) -> None:
    url = f"{endpoint.rstrip('/')}/r/{rail}/mcp"
    initialize = {
        "jsonrpc": "2.0",
        "id": f"canary-{rail}-initialize",
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "distribution-canary", "version": "1"},
        },
    }
    status, headers, body = post_json(url, initialize, {CANARY_HEADER: canary_kind})
    if status != 200 or body.get("error") or not body.get("result", {}).get("serverInfo"):
        raise RuntimeError(f"{rail}: initialize did not return canonical serverInfo")
    acquisition = header(headers, ACQUISITION_HEADER)
    if not acquisition or not TOKEN.fullmatch(acquisition):
        raise RuntimeError(f"{rail}: missing or malformed signed acquisition header")
    if header(headers, RAIL_HEADER) != rail or header(headers, FIRST_TOUCH_HEADER) != rail:
        raise RuntimeError(f"{rail}: initialize attribution headers do not match route")
    if header(headers, CANARY_HEADER) != canary_kind:
        raise RuntimeError(f"{rail}: initialize did not attest the bounded canary kind")
    if header(headers, MEASUREMENT_ELIGIBLE_HEADER) != "false":
        raise RuntimeError(f"{rail}: initialize canary was not measurement-excluded")

    tools_list = {
        "jsonrpc": "2.0",
        "id": f"canary-{rail}-tools-list",
        "method": "tools/list",
        "params": {},
    }
    status, headers, body = post_json(
        url,
        tools_list,
        {ACQUISITION_HEADER: acquisition, CANARY_HEADER: canary_kind},
    )
    if status != 200 or body.get("error"):
        raise RuntimeError(f"{rail}: tools/list failed")
    tools = body.get("result", {}).get("tools", [])
    if "prepare_bounty_post" not in {
        tool.get("name") for tool in tools if isinstance(tool, dict)
    }:
        raise RuntimeError(f"{rail}: canonical prepare_bounty_post tool is absent")
    if header(headers, ACQUISITION_HEADER) != acquisition:
        raise RuntimeError(f"{rail}: acquisition was not retry-stable")
    if header(headers, RAIL_HEADER) != rail or header(headers, FIRST_TOUCH_HEADER) != rail:
        raise RuntimeError(f"{rail}: tools/list attribution headers do not match route")
    if header(headers, CANARY_HEADER) != canary_kind:
        raise RuntimeError(f"{rail}: tools/list did not retain the bounded canary kind")
    if header(headers, MEASUREMENT_ELIGIBLE_HEADER) != "false":
        raise RuntimeError(f"{rail}: tools/list canary was not measurement-excluded")
    attribution = (
        body.get("result", {})
        .get("_meta", {})
        .get("agentbounties.app/acquisition", {})
    )
    if (
        attribution.get("acquisition_id") != acquisition
        or attribution.get("first_touch_rail") != rail
        or attribution.get("measurement_eligible") is not False
        or attribution.get("authority") != "analytics_only"
    ):
        raise RuntimeError(f"{rail}: MCP result attribution evidence is incomplete")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--endpoint",
        default="http://127.0.0.1:8090",
        help="MCP service origin (default: local port 8090)",
    )
    parser.add_argument(
        "--repetitions",
        type=int,
        default=3,
        help="Complete matrix repetitions; must be at least 3 (default: 3)",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Emit one machine-readable JSON result instead of progress lines",
    )
    parser.add_argument(
        "--canary-kind",
        choices=("dry-run-v1", "mainnet-v1"),
        default="dry-run-v1",
        help="Bounded measurement-excluded canary classification (default: dry-run-v1)",
    )
    args = parser.parse_args()
    if args.repetitions < 3:
        parser.error("--repetitions must be at least 3")
    failures: list[str] = []
    successes: list[dict[str, object]] = []
    for repetition in range(1, args.repetitions + 1):
        for rail in RAILS:
            try:
                check_rail(args.endpoint, rail, args.canary_kind)
                successes.append(
                    {
                        "repetition": repetition,
                        "rail": rail,
                        "initialize": "ok",
                        "tools_list": "ok",
                        "attribution": "ok",
                        "side_effects": "analytics_only",
                        "canary_kind": args.canary_kind,
                    }
                )
                if not args.json:
                    print(
                        f"repetition={repetition} rail={rail} initialize=ok "
                        "tools_list=ok attribution=ok"
                    )
            except Exception as error:  # The matrix should report every failing rail.
                failures.append(f"repetition={repetition} rail={rail}: {error}")
    evidence = {
        "schema_version": "agent-bounties/distribution-rail-canary-v1",
        "endpoint": args.endpoint.rstrip("/"),
        "repetitions": args.repetitions,
        "rails": list(RAILS),
        "checks": successes,
        "failures": failures,
        "ok": not failures,
        "side_effects": "analytics_only",
        "canary_kind": args.canary_kind,
    }
    if args.json:
        print(json.dumps(evidence, sort_keys=True, separators=(",", ":")))
    if failures:
        if not args.json:
            for failure in failures:
                print(f"ERROR {failure}", file=sys.stderr)
        return 1
    if not args.json:
        print(
            f"distribution_rail_mcp_canary=ok rails={len(RAILS)} "
            f"repetitions={args.repetitions} side_effects=analytics_only"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
