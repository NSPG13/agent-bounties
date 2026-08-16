#!/usr/bin/env python3
"""Verify production API, MCP and discovery surfaces expose one Beta3 release."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from urllib.request import Request, urlopen
from typing import Any

import wait_open_competition_v2_beta3_runtime as runtime_wait


class InterfaceError(RuntimeError):
    pass


def request_json(method: str, url: str, payload: dict[str, Any] | None = None) -> dict[str, Any]:
    body = None if payload is None else json.dumps(payload, separators=(",", ":")).encode()
    request = Request(
        url,
        method=method,
        data=body,
        headers={"accept": "application/json", "content-type": "application/json", "cache-control": "no-store"},
    )
    with urlopen(request, timeout=30) as response:
        return json.loads(response.read())


def contains_beta3(value: Any) -> bool:
    if isinstance(value, str):
        return "open-competition-v2-beta3" in value or "open_competition_v2" in value
    if isinstance(value, dict):
        return any(contains_beta3(key) or contains_beta3(item) for key, item in value.items())
    if isinstance(value, list):
        return any(contains_beta3(item) for item in value)
    return False


def verify(api: str, mcp: str, expected: dict[str, Any]) -> dict[str, Any]:
    api = api.rstrip("/")
    mcp = mcp.rstrip("/")
    api_release = request_json("GET", f"{api}/v1/base/open-competition-v2-beta3/release?network=base-mainnet")
    mcp_release = request_json(
        "POST",
        f"{mcp}/tools/inspect_open_competition_v2",
        {"operation": "release", "network": "base-mainnet"},
    )
    if not runtime_wait.exact_runtime(api_release, expected, True, False):
        raise InterfaceError("API release or indexer agreement differs from expected Beta3 runtime")
    if not runtime_wait.exact_runtime(mcp_release, expected, True, False):
        raise InterfaceError("MCP release proxy differs from expected Beta3 runtime")
    api_discovery = request_json("GET", f"{api}/.well-known/agent-bounties.json")
    mcp_discovery = request_json("GET", f"{mcp}/.well-known/agent-bounties.json")
    if not contains_beta3(api_discovery) or not contains_beta3(mcp_discovery):
        raise InterfaceError("API or MCP discovery manifest omits Beta3")
    canonical = json.dumps(expected, sort_keys=True, separators=(",", ":")).encode()
    return {
        "schema_version": "agent-bounties/open-competition-v2-beta3-interface-agreement-v1",
        "passed": True,
        "release_sha256": hashlib.sha256(canonical).hexdigest(),
        "factory_contract": expected["factory_contract"],
        "api_indexer_agreement": api_release["indexer_agreement"],
        "mcp_indexer_agreement": mcp_release["indexer_agreement"],
        "api_discovery_beta3": True,
        "mcp_discovery_beta3": True,
        "evidence_boundary": "This proves production interface identity agreement, not funding or settlement.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--api", default="https://api.agentbounties.app")
    parser.add_argument("--mcp", default="https://mcp.agentbounties.app")
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    expected = json.loads(args.runtime.read_text(encoding="utf-8"))
    evidence = verify(args.api, args.mcp, expected)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"passed": True, "output": str(args.output)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
