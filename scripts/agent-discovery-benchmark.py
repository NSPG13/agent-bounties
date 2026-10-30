#!/usr/bin/env python3
"""Capture bounded, read-only Agentizable discovery journeys and their evidence.

No wallet, bounty preparation, publication, or A2A task-creation request is made.
An empty inventory is a valid observation, not a claim of available paid work.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import importlib.util
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
SOURCE_TYPES = {"canonical_base", "unfunded_offchain", "legacy_bounty"}
PROTOCOLS = {"autonomous-v1", "autonomous-v2", "open-competition-v1", "open-competition-v2-beta3"}
COVERAGE_VALUES = {"complete", "partial", "unavailable"}
SOURCE_ERRORS = {
    "canonical_read_model_unavailable", "open_competition_read_model_unavailable",
    "open_competition_v2_read_model_unavailable", "unfunded_bounty_store_unavailable",
    "unfunded_solution_store_unavailable", "durable_store_not_configured",
}
MAX_DIAGNOSTIC_ROWS = 8

# Reuse the established validator, without its independent HTTP observer or any
# release/recovery activation. File loading also supports offline import callers.
READINESS_SPEC = importlib.util.spec_from_file_location(
    "discovery_marketplace_readiness", Path(__file__).with_name("marketplace_readiness.py")
)
assert READINESS_SPEC and READINESS_SPEC.loader
readiness = importlib.util.module_from_spec(READINESS_SPEC)
READINESS_SPEC.loader.exec_module(readiness)


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


def diagnostic_code(value: Any, allowed: set[str], unknown: str = "unknown") -> str:
    return value if isinstance(value, str) and value in allowed else unknown


def source_error(value: Any) -> str | None:
    if value is None:
        return None
    # Aggregate errors may join a bounded set of protocol-specific codes.
    if isinstance(value, str) and len(value) <= 256:
        parts = value.split("+")
        if len(parts) <= MAX_DIAGNOSTIC_ROWS and all(part in SOURCE_ERRORS for part in parts):
            return value
    return "unrecognized_error"


def source_observations(value: Any) -> list[dict[str, Any]]:
    """Compact known public fields only; never echo arbitrary provider errors."""
    if not isinstance(value, list):
        return []
    sources = []
    for source in value[:MAX_DIAGNOSTIC_ROWS]:
        if not isinstance(source, dict):
            continue
        count = source.get("item_count")
        protocols = source.get("protocol_statuses")
        compact = {
            "source_type": diagnostic_code(source.get("source_type"), SOURCE_TYPES),
            "available": source.get("available") if type(source.get("available")) is bool else None,
            "coverage": diagnostic_code(source.get("coverage"), COVERAGE_VALUES),
            "item_count": count if readiness.bounded_integer(count) else None,
            "error": source_error(source.get("error")), "protocol_statuses": [],
        }
        for protocol in (protocols if isinstance(protocols, list) else [])[:MAX_DIAGNOSTIC_ROWS]:
            if not isinstance(protocol, dict):
                continue
            count = protocol.get("item_count")
            compact["protocol_statuses"].append({
                "protocol": diagnostic_code(protocol.get("protocol"), PROTOCOLS),
                "coverage": diagnostic_code(protocol.get("coverage"), COVERAGE_VALUES),
                "item_count": count if readiness.bounded_integer(count) else None,
                "error": source_error(protocol.get("error")),
            })
        sources.append(compact)
    return sources


def inventory_observation(record: dict) -> dict[str, Any]:
    answer = result(record)
    if not answer or answer.get("isError"):
        return {"state": "unavailable", "matching_items": None, "reason": "request_or_tool_error"}
    content = answer.get("structuredContent")
    if not isinstance(content, dict) or not isinstance(content.get("items"), list):
        return {"state": "unavailable", "matching_items": None, "reason": "invalid_inventory_shape"}
    coverage = content.get("coverage")
    count = len(content["items"])
    sources = source_observations(content.get("source_statuses"))
    if coverage != "complete" or content.get("degraded") is not False:
        return {"state": "partial" if coverage == "partial" else "unavailable",
                "matching_items": count, "coverage": diagnostic_code(coverage, COVERAGE_VALUES),
                "source_statuses": sources,
                "reason": "coverage_does_not_establish_complete_inventory"}
    return {
        "state": "observed_empty" if count == 0 else "observed_items",
        "matching_items": count, "coverage": coverage,
        "coverage_scope": content.get("coverage_scope"),
        "has_more": content.get("has_more"), "generated_at": content.get("generated_at"),
        "evidence_boundary": content.get("evidence_boundary"),
        "source_statuses": sources,
    }


def capture_readiness(probe: Probe, api: str) -> dict[str, Any]:
    """One diagnostic GET; never retries inventory or changes its acceptance."""
    record = probe.request("marketplace_readiness_diagnostic", api + "/readiness/marketplace")
    if record.get("error") == "response_exceeds_4_mib":
        observation = readiness.unavailable("marketplace_readiness_invalid", "response_too_large")
    elif record.get("error"):
        observation = readiness.unavailable("marketplace_probe_unavailable", "transport_unavailable")
    elif record.get("status") not in (200, 503):
        observation = readiness.unavailable("marketplace_probe_unavailable", "http_status_unavailable")
    else:
        observation = readiness.inspect(record.get("body"))
        if record["status"] != 200 and observation["state"] == "healthy":
            observation = readiness.unavailable(
                "marketplace_readiness_http_inconsistent", "http_status_inconsistent")
    # Keep the bounded raw response hash, but persist only the validator's known
    # fields. Unknown bodies, provider URLs and exception text are not receipts.
    record["body"] = observation
    record["body_scope"] = "validated_readiness_summary"
    # Even a rejected redirect can put credentials in Location or opaque metadata.
    record["response_headers"] = {
        key: value.lower() for key, value in record.get("response_headers", {}).items()
        if key == "x-agent-bounties-analytics-excluded" and value.lower() in {"true", "false"}
    }
    return observation


def git_revision() -> str | None:
    try:
        return subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=Path(__file__).resolve().parents[1],
            stderr=subprocess.DEVNULL, text=True,
        ).strip()
    except (OSError, subprocess.CalledProcessError):
        return None


def run(website: str, api: str, mcp: str, token: str | None = None, *,
        capture_readiness_on_partial: bool = False) -> dict:
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
    diagnostic = {
        "enabled": capture_readiness_on_partial, "attempted": False,
        "evidence_boundary": "A later reader diagnostic cannot change the original inventory result, establish its cause, or authorize recovery/payment.",
    }
    if capture_readiness_on_partial and inventory["state"] == "partial":
        diagnostic.update(attempted=True, request_label="marketplace_readiness_diagnostic",
                          observation=capture_readiness(probe, api))
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
        "readiness_validator_sha256": hashlib.sha256(
            Path(__file__).with_name("marketplace_readiness.py").read_bytes()).hexdigest(),
        "synthetic": True, "passed": all(journey["passed"] for journey in journeys.values()),
        "health_revisions": {entry["label"]: entry.get("response_headers", {}).get("x-agent-bounties-revision") for entry in health},
        "catalog_counts": {"core": len(tools), "app": len(catalog(app_list))},
        "catalog_tool_names": {"core": sorted(tools), "app": sorted(catalog(app_list))},
        "journeys": journeys,
        "readiness_diagnostic": diagnostic,
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
    parser.add_argument("--capture-readiness-on-partial", action="store_true",
                        help="Capture one read-only API readiness diagnostic after partial MCP inventory; never retries or overrides the failed journey.")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = run(args.website, args.api, args.mcp, os.environ.get("AGENT_BOUNTIES_ANALYTICS_EXCLUSION_TOKEN"),
                 capture_readiness_on_partial=args.capture_readiness_on_partial)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"passed": report["passed"], "output": str(args.output),
                      "journeys": report["journeys"], "readiness_diagnostic": report["readiness_diagnostic"],
                      "measurement_limits": report["measurement_limits"]}, indent=2))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
