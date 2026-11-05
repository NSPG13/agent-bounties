#!/usr/bin/env python3
"""Probe an isolated posting canary with chain actions disabled.

Requires explicit API/MCP origins and an exact expected revision. The only POSTs
are read-only MCP tools and an unauthenticated gas-budget rejection probe. No
credential, private draft content, or wallet signature is accepted by this CLI.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import re
import urllib.error
import urllib.request
from urllib.parse import urlsplit


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_args, **_kwargs):
        return None


def origin(value: str) -> str:
    parsed = urlsplit(value)
    if (parsed.scheme != "https" or not parsed.hostname or parsed.username
            or parsed.password or parsed.port or parsed.path not in ("", "/")
            or parsed.query or parsed.fragment):
        raise argparse.ArgumentTypeError("Use an explicit HTTPS canary origin")
    if parsed.hostname in {"api.agentizable.ai", "mcp.agentizable.ai", "agentizable.ai"}:
        raise argparse.ArgumentTypeError("This disabled-canary check is not a production smoke")
    return value.rstrip("/")


def exercise(api: str, mcp: str, revision: str, mcp_revision: str | None = None,
             github_slug: str | None = None, funded_sepolia_checkpoint: bool = False,
             expected_reserved_micro_usdc: int | None = None) -> dict:
    http = urllib.request.build_opener(NoRedirect())
    checks = []

    def request(base, path, body=None, headers=None):
        req = urllib.request.Request(base + path,
            data=None if body is None else json.dumps(body).encode(),
            headers={"Accept": "application/json", "Content-Type": "application/json",
                     "Cache-Control": "no-cache", **(headers or {})})
        try:
            response = http.open(req, timeout=45)
        except urllib.error.HTTPError as error:
            response = error
        raw = response.read(4_000_001)
        if len(raw) > 4_000_000:
            raise AssertionError("Oversized canary response")
        try:
            body = json.loads(raw)
        except ValueError:
            body = raw.decode(errors="replace")
        return response.status, body, response.headers

    def check(label, assertion):
        if not assertion:
            raise AssertionError(label)
        checks.append({"check": label, "passed": True})

    def get(path):
        status, value, headers = request(api, path)
        check(path + " HTTP 200", status == 200)
        check(path + " uncached", "no-store" in headers.get("Cache-Control", ""))
        return value

    for name, base in (("API", api), ("MCP", mcp)):
        status, body, headers = request(base, "/health")
        expected_revision = (mcp_revision or revision) if name == "MCP" else revision
        check(name + " exact revision and health", status == 200 and body == "ok"
              and headers.get("x-agent-bounties-revision") == expected_revision)
        if name == "API":
            check("Hosted x402 signer disabled", headers.get("x-agent-bounties-x402-relay") == "disabled")

    gas = get("/v1/base/gas-sponsorship")
    budget = gas["daily_budget"]
    expected_reserved = expected_reserved_micro_usdc
    if expected_reserved is None:
        expected_reserved = 1_000_000 if funded_sepolia_checkpoint else 0
    check("Explicit retained allowance is within the shared cap", 0 <= expected_reserved <= 1_000_000)
    reset = datetime.fromisoformat(budget["resets_at"].replace("Z", "+00:00"))
    check("Shared 1 USDC cap without rollover", budget["limit_micro_usdc"] == 1_000_000
          and budget["carries_forward"] is False
          and budget["reserved_micro_usdc"] == expected_reserved
          and budget["remaining_micro_usdc"] == 1_000_000 - expected_reserved)
    check("UTC midnight reset", reset.tzinfo is not None and reset.utcoffset().total_seconds() == 0
          and (reset.hour, reset.minute, reset.second) == (0, 0, 0)
          and 0 < (reset - datetime.now(timezone.utc)).total_seconds() <= 86400)
    check("No universal-coverage or paid-fallback claim", gas["universal_coverage"] is False
          and gas["silent_customer_paid_fallback"] is False)
    actions = ("creation", "creator_verdict", "submission", "contribution", "claim", "authorization_revocation")
    check("Every legacy sponsored action disabled", all(gas[a]["available"] is False
          and gas[a]["customer_gas_wei"] == "0" for a in actions))
    check("Bounded authorization and contract-wallet disclosure",
          gas["creation"]["authorization_lifetime_seconds"] == 7200
          and "ERC-1271" in gas["creation"]["wallet_support"])

    for path, expected in (
        ("/v1/base/creator-open/capabilities", {"create", "submit", "review", "increase", "cancel", "expire", "refund_bond", "refund_principal"}),
        ("/v1/base/sponsored-setup/capabilities", {"wallet_deploy", "participant_register"}),
    ):
        value = get(path)
        check(path + " gated release", len(value["networks"]) == 2
              and all(n["available"] is False for n in value["networks"]))
        expected_releases = {"base-mainnet": None, "base-sepolia": None}
        if funded_sepolia_checkpoint:
            filename = "posting-creator-open-sepolia-release-20260930.json" if "creator-open" in path else "posting-setup-sepolia-release-20260930.json"
            expected_releases["base-sepolia"] = json.loads((Path(__file__).resolve().parents[1] / "docs/evidence" / filename).read_text())
        check(path + " exact release manifests", {n["network"]: n["release"] for n in value["networks"]} == expected_releases)
        check(path + " exact coverage and cap", set(value["actions"]) == expected
              and value["customer_gas_wei"] == "0"
              and value["maximum_authorization_seconds"] == 7200
              and value["daily_budget"] == budget)

    github = get("/v1/github-app/capabilities")
    check("GitHub exact activation state", github["available"] is bool(github_slug)
          and github["installation_url"] == (
              f"https://github.com/apps/{github_slug}/installations/new" if github_slug else None))
    check("GitHub has no funding or provider-write authority", github["funding_authority"] is False
          and github["provider_writes"] is False and github["retention_days"] == 7)
    for path in ("/v1/site-auth/posting-drafts/00000000-0000-0000-0000-000000000001",
                 "/v1/site-auth/github-origin-drafts", "/v1/site-auth/push-notifications"):
        check(path + " requires private account", request(api, path)[0] == 401)
    check("Invalid wallet rejected", request(api, "/v1/base/posting-wallets/not-a-wallet")[0] == 400)
    status, _, _ = request(api, "/v1/operator/gas-sponsorship/reservations", {
        "network": "base-mainnet", "relayer_address": "0x" + "1" * 40,
        "transaction_digest": "0" * 64, "maximum_fee_wei": "1"})
    check("Anonymous gas spending denied", status == 401)

    for name, base in (("API", api), ("MCP", mcp)):
        status, discovery, _ = request(base, "/.well-known/agent-bounties.json")
        check(name + " discovery uses hosted API and MCP", status == 200
              and discovery["endpoints"]["api_base"] == api
              and discovery["endpoints"]["mcp_streamable_http"] == mcp + "/mcp"
              and discovery["endpoints"]["mcp_tools"] == mcp + "/tools")

    def tool(name, arguments):
        status, envelope, _ = request(mcp, "/tools/" + name, arguments)
        check(name + " HTTP 200", status == 200)
        check(name + " successful result", "error" not in envelope)
        value = envelope["content"][0]["json"]
        return value

    options = tool("get_posting_options", {})
    check("Remote setup and creator-open review handoffs",
          options["sponsored_setup"]["principal_moved"] == "0"
          and options["creator_open_candidate"]["protocol"] == "agent-bounties/creator-open-v1")
    result = tool("get_posting_status", {"operation_id": "00000000-0000-0000-0000-000000000001"})
    check("Remote status preserves private ownership and cap",
          result["private_draft"]["status"] == "authentication_required" and result["paid"] is False
          and result["gas_sponsorship"] == gas)
    result = tool("get_posting_status", {"protocol": "creator-open-v1"})
    check("Undeployed creator-open remains unavailable",
          result["protocol"] == "agent-bounties/creator-open-v1" and result["paid"] is False
          and result["canonical_status"] == "unavailable")
    def remote(method, params):
        params = {**params, "_meta": {
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientInfo": {"name": "posting-readonly-canary", "version": "1.0.0"},
            "io.modelcontextprotocol/clientCapabilities": {}}}
        headers = {"mcp-protocol-version": "2026-07-28", "mcp-method": method}
        if method == "tools/call":
            headers["mcp-name"] = params["name"]
        status, envelope, _ = request(mcp, "/mcp", {
            "jsonrpc": "2.0", "id": "canary-" + method, "method": method, "params": params}, headers)
        check("Remote MCP " + method + " succeeds", status == 200 and "error" not in envelope)
        return envelope["result"]

    discovery = remote("server/discover", {})
    check("Remote MCP protocol negotiation", discovery["supportedVersions"] == ["2026-07-28"])
    catalog = remote("tools/list", {})
    expected = json.loads((Path(__file__).resolve().parents[1]
        / "crates/mcp-server/fixtures/public-mcp-contract-v1.json").read_text())["catalogs"]["core"]["tools"]
    check("Remote MCP complete core catalog", sorted(t["name"] for t in catalog["tools"]) == expected)
    result = remote("tools/call", {"name": "get_posting_status", "arguments": {
        "operation_id": "00000000-0000-0000-0000-000000000001"}})
    check("Remote MCP status cannot approve a private draft", result.get("isError") is not True
          and result["structuredContent"]["private_draft"]["status"] == "authentication_required"
          and result["structuredContent"]["paid"] is False
          and result["structuredContent"]["gas_sponsorship"] == gas)
    check("Read-only probe spent no gas budget", get("/v1/base/gas-sponsorship")["daily_budget"] == budget)
    return {"revision": revision, "mcp_revision": mcp_revision or revision,
            "github_slug": github_slug, "api": api, "mcp": mcp,
            "funded_sepolia_checkpoint": funded_sepolia_checkpoint, "daily_budget": budget,
            "observed_at": datetime.now(timezone.utc).isoformat(),
            "chain_transactions_submitted": False, "checks": checks}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--api", type=origin, required=True)
    parser.add_argument("--mcp", type=origin, required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--mcp-revision", help="Exact MCP revision when it differs from the API")
    parser.add_argument("--github-slug", help="Exact staging App slug when intake is enabled")
    parser.add_argument("--funded-sepolia-checkpoint", action="store_true", help="Require exact Sepolia manifests; defaults to the historical exhausted allowance")
    parser.add_argument("--expected-reserved-micro-usdc", type=int, help="Exact retained reservation counter after a subsequent UTC day; never resets the ledger")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if any(not re.fullmatch(r"[0-9a-f]{40}", value) for value in (args.revision, args.mcp_revision or args.revision)):
        parser.error("Use an exact 40-character source revision")
    if args.github_slug and not re.fullmatch(r"[a-z0-9-]{1,100}", args.github_slug):
        parser.error("Use the exact staging App slug")
    if args.expected_reserved_micro_usdc is not None and not 0 <= args.expected_reserved_micro_usdc <= 1_000_000:
        parser.error("Expected reservations must be within the shared 1 USDC cap")
    result = exercise(args.api, args.mcp, args.revision, args.mcp_revision, args.github_slug,
                      args.funded_sepolia_checkpoint, args.expected_reserved_micro_usdc)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(f"PASS: {len(result['checks'])} read-only canary checks at {args.revision}")


if __name__ == "__main__":
    main()
