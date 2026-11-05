#!/usr/bin/env python3
"""Replay safe HTTP/dependency diagnostics against disposable local API/MCP processes."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid


ROOT = Path(__file__).resolve().parents[1]
REVISION = "0123456789abcdef0123456789abcdef01234567"


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_args, **_kwargs):
        return None


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    suffix = ".exe" if os.name == "nt" else ""
    parser.add_argument("--api-binary", type=Path, default=ROOT / "target/debug" / ("api" + suffix))
    parser.add_argument("--mcp-binary", type=Path, default=ROOT / "target/debug" / ("mcp-server" + suffix))
    args = parser.parse_args()
    binaries = {"api": args.api_binary.resolve(), "mcp": args.mcp_binary.resolve()}
    if not all(path.is_file() for path in binaries.values()):
        parser.error("build local services first: cargo build -p api -p mcp-server")
    marker = "private-fixture-" + uuid.uuid4().hex
    resource = uuid.uuid4().hex * 2
    ports = [socket.socket(), socket.socket()]
    try:
        for sock in ports:
            sock.bind(("127.0.0.1", 0))
        api, mcp = [f"http://127.0.0.1:{sock.getsockname()[1]}" for sock in ports]
    finally:
        for sock in ports:
            sock.close()
    http = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    private_headers = {"x-operator-token": marker, "x-request-id": marker,
                       "authorization": "Bearer " + marker, "cookie": "private=" + marker}

    def request(base, path, body=None, extra=None):
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(base + path, data=data, headers={
            "content-type": "application/json", **private_headers, **(extra or {})})
        try:
            response = http.open(req, timeout=10)
        except urllib.error.HTTPError as error:
            response = error
        return response.status, response.read()

    with tempfile.TemporaryDirectory(prefix="agent-bounties-http-errors-") as directory:
        processes, logs = [], []
        try:
            for service, base in (("api", api), ("mcp", mcp)):
                env = {name: os.environ[name] for name in ("PATH", "SystemRoot") if name in os.environ}
                env.update({"PUBLIC_BASE_URL": api, "API_UPSTREAM_BASE_URL": api,
                            "MCP_BASE_URL": mcp, "API_BIND_ADDR": api.removeprefix("http://"),
                            "MCP_BIND_ADDR": mcp.removeprefix("http://"), "OPERATOR_API_TOKEN": marker,
                            "RENDER_GIT_COMMIT": REVISION, "ENABLE_BASE_TX_BROADCAST": "false",
                            "ENABLE_STRIPE_LIVE_EXECUTION": "false", "ENABLE_X402_HOSTED_RELAY": "false",
                            "BASE_MAINNET_RPC_URL": "http://127.0.0.1:9", "BASE_SEPOLIA_RPC_URL": "http://127.0.0.1:9"})
                log = (Path(directory) / f"{service}.log").open("w")
                logs.append(log)
                process = subprocess.Popen([str(binaries[service])], cwd=ROOT, env=env,
                                           stdin=subprocess.DEVNULL, stdout=log, stderr=log)
                processes.append(process)
                for _ in range(120):
                    if process.poll() is not None:
                        raise AssertionError(f"{service} exited during fixture startup")
                    try:
                        if request(base, "/health")[0] == 200:
                            break
                    except (OSError, urllib.error.URLError):
                        pass
                    time.sleep(0.25)
                else:
                    raise AssertionError(f"{service} startup timed out")
            # A failed scoped read is a degraded 200, not an HTTP server error.
            # Its failure must remain visible to a JSON-only log pipeline.
            scoped = "canonical:base-mainnet:0x" + "1" * 40

            def assert_degraded(value):
                assert value["degraded"] is True and value["coverage"] == "unavailable"
                assert value["coverage_scope"] == "canonical_identity" and value["items"] == []
                protocol = value["source_statuses"][0]["protocol_statuses"][0]
                assert protocol["coverage"] == "unavailable" and protocol["item_count"] is None
                assert protocol["error"] == "canonical_identity_read_unavailable"

            status, body = request(api, "/v1/opportunities?opportunity_id=" + scoped + "&private=" + marker)
            assert status == 200
            assert_degraded(json.loads(body))
            status, body = request(mcp, "/tools/list_opportunities?private=" + marker,
                                   {"opportunity_id": scoped})
            assert status == 200
            assert_degraded(json.loads(body)["content"][0]["json"])
            # Missing storage produces a real server error before any mutation.
            registration = {"handle": marker, "payout_wallet": None}
            assert request(api, "/v1/agents?private=" + marker, registration)[0] == 503
            status, body = request(mcp, "/tools/register_agent?private=" + marker, registration)
            value = json.loads(body)
            assert status == 200 and "error" in value
            assert value["error_code"] == "upstream_unavailable"
            assert request(mcp, "/public/bounty-images/" + resource + "?private=" + marker)[0] == 503
            for modern in (False, True):
                params = {"name": "list_bounty_comments", "arguments": {"bounty_id": resource}}
                headers = {}
                if modern:
                    params["_meta"] = {"io.modelcontextprotocol/protocolVersion": "2026-07-28",
                                       "io.modelcontextprotocol/clientCapabilities": {}}
                    headers = {"mcp-protocol-version": "2026-07-28", "mcp-method": "tools/call",
                               "mcp-name": "list_bounty_comments"}
                payload = {"jsonrpc": "2.0", "id": marker, "method": "tools/call", "params": params}
                status, body = request(mcp, "/mcp?private=" + marker, payload, headers)
                value = json.loads(body)
                # Upstream failure is a server error, not invalid tool arguments.
                # Legacy keeps its 200 envelope; modern MCP reports HTTP 503.
                assert status == (503 if modern else 200) and value["error"]["code"] == -32603
                assert value["error"]["data"] == {"error_code": "upstream_unavailable"}
                assert marker not in str(value["error"]) and resource not in str(value["error"])
                invalid_payload = {**payload, "params": {**params, "arguments": {}}}
                status, body = request(mcp, "/mcp?private=" + marker, invalid_payload, headers)
                invalid = json.loads(body)
                assert status == (400 if modern else 200) and invalid["error"]["code"] == -32602
                assert "data" not in invalid["error"]
            assert request(api, "/unknown/" + resource + "?private=" + marker)[0] == 404
        finally:
            for process in processes:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)
            for log in logs:
                log.close()
        events, stages = [], []
        for path in Path(directory).glob("*.log"):
            text = path.read_text()
            assert marker not in text and resource not in text, "private fixture appeared in service logs"
            for line in text.splitlines():
                if line.startswith("{"):
                    event = json.loads(line)
                    if event.get("event") == "http_server_error":
                        events.append(event)
                    if event.get("event") == "service_failure_stage":
                        stages.append(event)
        fields = {"event", "level", "service", "route", "method", "status", "revision", "code"}
        assert all(set(event) == fields and event["level"] == "error" and event["revision"] == REVISION for event in events)
        expected = {("api", "/v1/agents", "POST"): 2,
                    ("api", "/v1/opportunities/:opportunity_id/comments", "GET"): 2,
                    ("mcp", "/public/bounty-images/:sha256", "GET"): 1,
                    ("mcp", "/mcp", "POST"): 1}
        actual = {}
        for event in events:
            assert event["status"] == 503 and event["code"] == "service_unavailable"
            key = (event["service"], event["route"], event["method"])
            actual[key] = actual.get(key, 0) + 1
        assert actual == expected, f"unexpected operational error counts: {actual}"
        stage_fields = {"schema", "event", "level", "component", "revision", "action",
                        "attempt", "stage", "evidence_boundary"}
        assert all(set(event) == stage_fields and event["schema"] == "agent-bounties/service-failure-stage-v1"
                   and event["level"] == "error" and event["revision"] == REVISION
                   and event["attempt"] == 1 for event in stages)
        scoped_stages = [event for event in stages if event["stage"] == "scoped_autonomous_read"]
        assert len(scoped_stages) == 2, f"degraded 200 diagnostics missing from JSON pipeline: {scoped_stages}"
        assert all(event["component"] == "api" and event["action"] == "opportunity_projection"
                   for event in scoped_stages)
        assert any(event["component"] == "mcp" and event["action"] == "hosted_api_request"
                   for event in stages), "MCP upstream failure is missing structured diagnostics"
        print("HTTP error observability passed: degraded 200 API/direct MCP, both MCP error eras; six bounded HTTP errors; structured failure stages; no private fixture data")


if __name__ == "__main__":
    main()
