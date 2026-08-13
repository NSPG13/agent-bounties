#!/usr/bin/env python3
"""Exercise the built ChatGPT MCP server through its real JSON-RPC endpoint."""

from __future__ import annotations

import argparse
import json
import os
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from urllib.parse import urlparse


ROOT = Path(__file__).resolve().parents[1]
FULL_TOOLS = {
    "get_bounty_feed",
    "render_bounty_feed",
    "prepare_moonpay_onramp",
    "prepare_bounty_post",
    "prepare_bounty_action",
    "get_bounty_action_status",
    "compile_objective_with_cloud_agent",
    "list_autonomous_bounties",
    "list_bounty_comments",
    "add_bounty_comment",
    "create_share_bundle",
}
FEED_WIDGET_URI = "ui://agent-bounties/live-feed-v4.html"
MODERN_PROTOCOL_VERSION = "2026-07-28"
LEGACY_PROTOCOL_VERSION = "2025-06-18"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def available_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def request(
    endpoint: str,
    request_id: int,
    method: str,
    params: dict,
    modern: bool = False,
) -> dict:
    headers = {
        "Accept": "application/json, text/event-stream",
        "Content-Type": "application/json",
    }
    if modern:
        params = dict(params)
        params["_meta"] = {
            "io.modelcontextprotocol/protocolVersion": MODERN_PROTOCOL_VERSION,
            "io.modelcontextprotocol/clientInfo": {
                "name": "agent-bounties-release-harness",
                "version": "1",
            },
            "io.modelcontextprotocol/clientCapabilities": {},
        }
        headers["MCP-Protocol-Version"] = MODERN_PROTOCOL_VERSION
        headers["Mcp-Method"] = method
        if method == "resources/read":
            headers["Mcp-Name"] = params["uri"]
        elif method in ("tools/call", "prompts/get"):
            headers["Mcp-Name"] = params["name"]
    payload = {
        "jsonrpc": "2.0",
        "id": request_id,
        "method": method,
        "params": params,
    }
    http_request = urllib.request.Request(
        endpoint,
        data=json.dumps(payload).encode("utf-8"),
        headers=headers,
        method="POST",
    )
    with urllib.request.urlopen(http_request, timeout=5) as response:
        body = json.loads(response.read())
    require("error" not in body, f"{method} failed: {body.get('error')}")
    return body["result"]


def parse_args() -> argparse.Namespace:
    executable = "mcp-server.exe" if os.name == "nt" else "mcp-server"
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--binary",
        type=Path,
        default=ROOT / "target" / "debug" / executable,
        help="Path to the already-built mcp-server binary.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    binary = args.binary.resolve()
    if not binary.is_file():
        raise SystemExit(
            f"chatgpt_runtime_check=failed reason=missing binary {binary}; "
            "run cargo build -p mcp-server"
        )

    port = available_port()
    endpoint = f"http://127.0.0.1:{port}/mcp"
    environment = os.environ.copy()
    environment.update(
        {
            "MCP_BIND_ADDR": f"127.0.0.1:{port}",
            "MCP_BASE_URL": "https://mcp.agentbounties.app",
            # Keep every live hosted API request local and guaranteed to fail.
            "PUBLIC_BASE_URL": "http://127.0.0.1:9",
            "CHATGPT_APP_SANDBOX_MODE": "false",
            # Prove this retired flag cannot reduce the production product.
            "CHATGPT_APP_PUBLIC_REVIEW_MODE": "true",
        }
    )
    process = subprocess.Popen(
        [str(binary)],
        cwd=ROOT,
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )

    request_id = 0

    def rpc(method: str, params: dict | None = None, modern: bool = True) -> dict:
        nonlocal request_id
        request_id += 1
        return request(endpoint, request_id, method, params or {}, modern=modern)

    try:
        last_error: Exception | None = None
        for _ in range(100):
            if process.poll() is not None:
                stdout, stderr = process.communicate()
                raise AssertionError(
                    f"server exited {process.returncode}: {stdout}\n{stderr}"
                )
            try:
                discovered = rpc("server/discover")
                break
            except (OSError, urllib.error.URLError) as error:
                last_error = error
                time.sleep(0.1)
        else:
            raise AssertionError(f"server did not become ready: {last_error}")

        require(
            discovered["supportedVersions"] == [MODERN_PROTOCOL_VERSION],
            "modern MCP version discovery drifted",
        )
        require(
            discovered["resultType"] == "complete"
            and discovered["cacheScope"] == "public"
            and discovered["ttlMs"] > 0,
            "modern discovery lost typed cache metadata",
        )
        server_info = discovered["_meta"]["io.modelcontextprotocol/serverInfo"]
        require(server_info["name"] == "agent-bounties", "server name drifted")
        require(server_info["title"] == "Agent Bounties", "server title drifted")
        require(
            "Public review mode" not in discovered["instructions"],
            "retired public-review profile is still reachable",
        )

        initialized = rpc(
            "initialize",
            {
                "protocolVersion": LEGACY_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "agent-bounties-release-harness",
                    "version": "1",
                },
            },
            modern=False,
        )
        require(
            initialized["protocolVersion"] == LEGACY_PROTOCOL_VERSION,
            "legacy MCP compatibility drifted",
        )
        require(
            "resultType" not in initialized,
            "legacy response shape unexpectedly became modern",
        )

        tools = rpc("tools/list")["tools"]
        tool_names = {tool["name"] for tool in tools}
        require(
            len(tools) == len(FULL_TOOLS) and tool_names == FULL_TOOLS,
            f"runtime tool set drifted: {sorted(tool_names)}",
        )
        legacy_tools = rpc("tools/list", modern=False)["tools"]
        require(
            {tool["name"] for tool in legacy_tools} == FULL_TOOLS,
            "legacy tool catalog drifted",
        )
        moonpay_descriptor = next(
            tool for tool in tools if tool["name"] == "prepare_moonpay_onramp"
        )
        require(
            moonpay_descriptor["annotations"]
            == {
                "readOnlyHint": True,
                "destructiveHint": False,
                "openWorldHint": False,
                "idempotentHint": True,
            },
            "MoonPay handoff annotations drifted",
        )
        post_descriptor = next(
            tool for tool in tools if tool["name"] == "prepare_bounty_post"
        )
        require(
            post_descriptor["annotations"]
            == {
                "readOnlyHint": False,
                "destructiveHint": True,
                "openWorldHint": True,
                "idempotentHint": True,
            },
            "ChatGPT image handoff annotations drifted",
        )
        require(
            post_descriptor["_meta"]["openai/fileParams"] == ["bounty_image"]
            and "bounty_image"
            in post_descriptor["inputSchema"]["required"],
            "ChatGPT file handoff metadata drifted",
        )

        resources = rpc("resources/list")["resources"]
        require(
            len(resources) == 1 and resources[0]["uri"] == FEED_WIDGET_URI,
            "feed widget resource descriptor drifted",
        )
        resource = rpc("resources/read", {"uri": FEED_WIDGET_URI})["contents"][0]
        require(
            resource["mimeType"] == "text/html;profile=mcp-app",
            "feed widget MIME type drifted",
        )
        require(
            'bridgeNotify("ui/message", message)' in resource["text"]
            and "sendFollowUpMessage" in resource["text"],
            "mounted widget lost its conversation handoff",
        )
        require(
            all(
                element not in resource["text"].lower()
                for element in ("<input", "<textarea", "<select", "<form")
            ),
            "mounted widget unexpectedly exposes a form",
        )
        require(
            all(
                label in resource["text"]
                for label in (
                    ">Post bounty<",
                    ">Comment<",
                    ">Share<",
                    ">Solve<",
                )
            ),
            "mounted widget lost one of its approved actions",
        )
        require(
            "APP_PUBLIC_REVIEW = false" in resource["text"],
            "mounted widget does not use the full production profile",
        )
        require(
            "https://agentbounties.app"
            in resource["_meta"]["ui"]["csp"]["redirectDomains"],
            "first-party hosted actions are absent from widget CSP",
        )

        called = rpc(
            "tools/call",
            {
                "name": "prepare_moonpay_onramp",
                "arguments": {
                    "bounty_contract": (
                        "0x1111111111111111111111111111111111111111"
                    ),
                    "amount_base_units": 3_500_000,
                    "intent_id": "00000000-0000-4000-8000-000000000002",
                },
            },
        )
        handoff = called["structuredContent"]
        onramp = urlparse(handoff["onramp_url"])
        require(
            onramp.scheme == "https"
            and onramp.netloc == "agentbounties.app"
            and onramp.path == "/onramp.html",
            "MoonPay handoff escaped the first-party origin",
        )
        require(
            "buy.moonpay.com" not in json.dumps(called),
            "provider checkout URL leaked through the ChatGPT tool",
        )
        require(handoff["provider"] == "moonpay", "provider drifted")
        require(handoff["checkout_created"] is False, "checkout was overclaimed")
        require(handoff["purchase_completed"] is False, "purchase was overclaimed")
        require(handoff["bounty_funded"] is False, "funding was overclaimed")
        require(
            handoff["canonical_funding_event"] is None,
            "canonical funding was overclaimed",
        )
    except AssertionError as error:
        print(
            f"chatgpt_runtime_check=failed reason={error}",
            file=sys.stderr,
        )
        return 1
    finally:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)

    print(
        "chatgpt_runtime_check=ok "
        f"mcp_modern={MODERN_PROTOCOL_VERSION} "
        f"mcp_legacy={LEGACY_PROTOCOL_VERSION} "
        f"tools={len(FULL_TOOLS)} "
        "profile=full_hosted_execution "
        "chatgpt_image_handoff=file_param "
        "moonpay_handoff=first_party "
        "legacy_public_review_flag=ignored"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
