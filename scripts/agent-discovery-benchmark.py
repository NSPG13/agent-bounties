#!/usr/bin/env python3
"""Capture bounded, read-only Agentizable discovery journeys and their evidence.

No wallet, bounty preparation, publication, or A2A task-creation request is made.
An empty inventory is a valid observation, not a claim of available paid work.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
from typing import Any
import urllib.error
import urllib.parse
import urllib.request


MODERN_VERSION = "2026-07-28"
LEGACY_VERSION = "2025-06-18"
MAX_BYTES = 4 * 1024 * 1024
USER_AGENT = "AgentizableDiscoveryBenchmark/1.0 (synthetic read-only audit)"
READ_ONLY_TOOLS = {"get_bounty_feed", "get_posting_options"}


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def public_base(value: str) -> str:
    parsed = urllib.parse.urlsplit(value)
    local = parsed.hostname in {"localhost", "127.0.0.1", "::1"}
    if (
        not parsed.hostname
        or parsed.username
        or parsed.password
        or parsed.query
        or parsed.fragment
        or parsed.path not in {"", "/"}
        or (parsed.scheme != "https" and not (local and parsed.scheme == "http"))
    ):
        raise argparse.ArgumentTypeError("use an HTTPS origin (or local HTTP origin), without credentials")
    return value.rstrip("/")


class NoRedirect(urllib.request.HTTPRedirectHandler):
    # Keep credentials on the explicitly selected host; a redirect is evidence.
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class Probe:
    def __init__(self, api: str, mcp: str, token: str | None = None):
        self.credential_origins = {api, mcp}
        self.token = token
        self.records: list[dict[str, Any]] = []
        self.opener = urllib.request.build_opener(NoRedirect())

    def request(self, label: str, url: str, payload: dict | None = None,
                headers: dict[str, str] | None = None) -> dict[str, Any]:
        record: dict[str, Any] = {
            "label": label, "url": url, "method": "POST" if payload else "GET",
            "request": payload, "observed_at": now(), "synthetic": True,
        }
        request_headers = {"User-Agent": USER_AGENT, "Accept": "application/json, text/event-stream"}
        request_headers.update(headers or {})
        origin = urllib.parse.urlsplit(url)
        origin_text = f"{origin.scheme}://{origin.netloc}"
        credential_sent = bool(self.token and origin_text in self.credential_origins)
        if credential_sent:
            request_headers["X-Agent-Bounties-Analytics-Exclusion"] = self.token
        if payload is not None:
            request_headers["Content-Type"] = "application/json"
        record["analytics_exclusion_requested"] = credential_sent
        record["analytics_exclusion_confirmed"] = False
        started = time.monotonic()
        try:
            request = urllib.request.Request(
                url, data=json.dumps(payload).encode() if payload is not None else None,
                headers=request_headers, method=record["method"],
            )
            try:
                response = self.opener.open(request, timeout=30)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                record["status"] = response.status
                record["response_headers"] = {
                    key.lower(): value.replace(self.token, "[redacted]") if self.token else value
                    for key, value in response.headers.items()
                    if key.lower() in {
                        "content-type", "date", "location", "etag", "last-modified",
                        "x-agent-bounties-revision", "x-agent-bounties-protocol",
                        "x-agent-bounties-analytics-excluded",
                    }
                }
                record["analytics_exclusion_confirmed"] = (
                    response.headers.get("X-Agent-Bounties-Analytics-Excluded", "").lower() == "true"
                )
                raw = response.read(MAX_BYTES + 1)
            if len(raw) > MAX_BYTES:
                record["error"] = "response_exceeds_4_mib"
            else:
                record["body_sha256"] = hashlib.sha256(raw).hexdigest()
                text = raw.decode("utf-8", errors="replace")
                if self.token:
                    text = text.replace(self.token, "[redacted]")
                try:
                    record["body"] = json.loads(text)
                except json.JSONDecodeError:
                    record["body"] = text
        except (urllib.error.URLError, TimeoutError, OSError) as error:
            # Do not log credentials, arbitrary URLs from errors, or request headers.
            record["error"] = type(error).__name__
        record["duration_seconds"] = round(time.monotonic() - started, 3)
        self.records.append(record)
        return record

    def modern(self, label: str, method: str, params: dict | None = None,
               client_name: str = "agentizable-discovery-benchmark") -> dict:
        parameters = dict(params or {})
        parameters["_meta"] = {
            "io.modelcontextprotocol/protocolVersion": MODERN_VERSION,
            "io.modelcontextprotocol/clientInfo": {"name": client_name, "version": "1"},
            "io.modelcontextprotocol/clientCapabilities": {},
        }
        headers = {"MCP-Protocol-Version": MODERN_VERSION, "Mcp-Method": method}
        if method == "tools/call":
            if parameters.get("name") not in READ_ONLY_TOOLS:
                raise ValueError("benchmark permits only its two read-only tool names")
            headers["Mcp-Name"] = parameters["name"]
        return self.request(label, self.mcp_endpoint, {
            "jsonrpc": "2.0", "id": len(self.records) + 1,
            "method": method, "params": parameters,
        }, headers)


def http_ok(record: dict) -> bool:
    return record.get("status") == 200 and not record.get("error")


def result(record: dict) -> dict:
    body = record.get("body")
    candidate = body.get("result") if isinstance(body, dict) else None
    return candidate if http_ok(record) and isinstance(candidate, dict) else {}


def catalog(record: dict) -> dict[str, dict]:
    tools = result(record).get("tools", [])
    if not isinstance(tools, list):
        return {}
    return {tool["name"]: tool for tool in tools
            if isinstance(tool, dict) and isinstance(tool.get("name"), str)}


def inventory_observation(record: dict) -> dict[str, Any]:
    answer = result(record)
    if not answer or answer.get("isError"):
        return {"state": "unavailable", "matching_items": None, "reason": "request_or_tool_error"}
    content = answer.get("structuredContent")
    if not isinstance(content, dict) or not isinstance(content.get("items"), list):
        return {"state": "unavailable", "matching_items": None, "reason": "invalid_inventory_shape"}
    coverage = content.get("coverage")
    count = len(content["items"])
    if coverage != "complete" or content.get("degraded") is not False:
        return {"state": "partial" if coverage == "partial" else "unavailable",
                "matching_items": count, "coverage": coverage,
                "reason": "coverage_does_not_establish_complete_inventory"}
    return {
        "state": "observed_empty" if count == 0 else "observed_items",
        "matching_items": count, "coverage": coverage,
        "coverage_scope": content.get("coverage_scope"),
        "has_more": content.get("has_more"), "generated_at": content.get("generated_at"),
        "evidence_boundary": content.get("evidence_boundary"),
    }


def git_revision() -> str | None:
    try:
        return subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=Path(__file__).resolve().parents[1],
            stderr=subprocess.DEVNULL, text=True,
        ).strip()
    except (OSError, subprocess.CalledProcessError):
        return None


def run(website: str, api: str, mcp: str, token: str | None = None) -> dict:
    probe = Probe(api, mcp, token)
    probe.mcp_endpoint = mcp + "/mcp"
    orientation = [
        probe.request("agent_entry", website + "/agent/index.md"),
        probe.request("llms", website + "/llms.txt"),
        probe.request("website_manifest", website + "/.well-known/agent-bounties.json"),
        probe.request("api_manifest", api + "/.well-known/agent-bounties.json"),
        probe.request("a2a_card", api + "/.well-known/agent-card.json"),
        probe.request("openapi", api + "/api-docs/openapi.json"),
    ]
    health = [probe.request("api_health", api + "/health"),
              probe.request("mcp_health", mcp + "/health")]
    rest = probe.request("rest_claimable_feed", api + "/v1/base/autonomous-bounties/feed?network=base-mainnet&claimable_only=true")
    discovery = probe.modern("modern_discover", "server/discover")
    tool_list = probe.modern("modern_core_catalog", "tools/list")
    app_list = probe.modern("modern_app_catalog", "tools/list", client_name="openai-mcp")
    legacy = probe.request("legacy_initialize", mcp + "/mcp", {
        "jsonrpc": "2.0", "id": len(probe.records) + 1, "method": "initialize",
        "params": {"protocolVersion": LEGACY_VERSION, "capabilities": {},
                   "clientInfo": {"name": "agentizable-discovery-benchmark", "version": "1"}},
    })
    tools = catalog(tool_list)
    feed = {}
    options = {}
    for name, arguments in [
        ("get_bounty_feed", {"network": "base-mainnet", "view": "ready_to_earn",
                            "source_type": "canonical_base", "work_state": "claimable",
                            "payment_state": "escrowed", "limit": 3}),
        ("get_posting_options", {}),
    ]:
        if tools.get(name, {}).get("annotations", {}).get("readOnlyHint") is True:
            record = probe.modern(name, "tools/call", {"name": name, "arguments": arguments})
            if name == "get_bounty_feed":
                feed = record
            else:
                options = record
    inventory = inventory_observation(feed)
    orientation_checks = {entry["label"]: http_ok(entry) for entry in orientation}
    for entry in orientation[2:]:
        orientation_checks[entry["label"] + "_object"] = isinstance(entry.get("body"), dict)
    orientation_checks["agent_entry_points_to_mcp"] = mcp + "/mcp" in str(orientation[0].get("body", ""))
    orientation_checks["manifest_canonical_website"] = (
        isinstance(orientation[2].get("body"), dict)
        and orientation[2]["body"].get("website") == website + "/"
    )
    mcp_checks = {
        "modern_version": MODERN_VERSION in result(discovery).get("supportedVersions", []),
        "modern_complete": result(discovery).get("resultType") == "complete",
        "core_catalog": bool(tools), "app_catalog": bool(catalog(app_list)),
        "legacy_version": result(legacy).get("protocolVersion") == LEGACY_VERSION,
        "feed_read_only": tools.get("get_bounty_feed", {}).get("annotations", {}).get("readOnlyHint") is True,
        "feed_complete": inventory["state"] in {"observed_empty", "observed_items"},
    }
    rest_ok = http_ok(rest) and isinstance(rest.get("body"), list)
    journeys = {
        "orientation": {"passed": all(orientation_checks.values()), "checks": orientation_checks},
        "rest_inventory": {"passed": rest_ok, "matching_items": len(rest["body"]) if rest_ok else None,
                           "scope": "claimable autonomous-v1 REST feed; no independent chain recheck"},
        "mcp_discovery_and_inventory": {"passed": all(mcp_checks.values()), "checks": mcp_checks,
                                        "inventory": inventory},
    }
    openapi_body = orientation[-1].get("body")
    retired = None
    if isinstance(openapi_body, dict):
        retired = "410" in openapi_body.get("paths", {}).get(
            "/v1/cloud-agent/objective-plans", {}).get("post", {}).get("responses", {})
    return {
        "schema": "agentizable/discovery-benchmark-v1", "observed_at": now(),
        "source_commit": git_revision(), "targets": {"website": website, "api": api, "mcp": mcp},
        "benchmark_script_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "synthetic": True, "passed": all(journey["passed"] for journey in journeys.values()),
        "health_revisions": {entry["label"]: entry.get("response_headers", {}).get("x-agent-bounties-revision") for entry in health},
        "catalog_counts": {"core": len(tools), "app": len(catalog(app_list))},
        "catalog_tool_names": {"core": sorted(tools), "app": sorted(catalog(app_list))},
        "journeys": journeys,
        "posting_options": {"advertised_read_only": tools.get("get_posting_options", {}).get("annotations", {}).get("readOnlyHint") is True,
                            "read_success": bool(result(options)) and result(options).get("isError") is not True},
        "hosted_objective_compiler_410_in_openapi": retired,
        "measurement_limits": {
            "independent_adoption": False, "a2a_task_execution": "not attempted; card inspection only",
            "funding_claim_submission_or_payment": "not attempted",
            "analytics_exclusion_credential_available": bool(token),
            "request_count": len(probe.records),
            "exclusion_confirmed_request_count": sum(record["analytics_exclusion_confirmed"] for record in probe.records),
            "unconfirmed_request_count": sum(not record["analytics_exclusion_confirmed"] for record in probe.records),
            "caveat": "Unconfirmed synthetic requests may contribute to service aggregates; the user-agent label does not exclude them.",
        },
        "requests": probe.records,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--website", type=public_base, default="https://agentizable.ai")
    parser.add_argument("--api", type=public_base, default="https://api.agentizable.ai")
    parser.add_argument("--mcp", type=public_base, default="https://mcp.agentizable.ai")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = run(args.website, args.api, args.mcp, os.environ.get("AGENT_BOUNTIES_ANALYTICS_EXCLUSION_TOKEN"))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"passed": report["passed"], "output": str(args.output),
                      "journeys": report["journeys"], "measurement_limits": report["measurement_limits"]}, indent=2))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
