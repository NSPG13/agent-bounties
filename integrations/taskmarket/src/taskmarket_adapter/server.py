"""Taskmarket MCP server (stdio JSON-RPC 2.0, standard library only).

Read tools are safe. Write tools (`taskmarket_create_task`, `taskmarket_submit`)
take NO authority argument from the caller: they require an operator-issued,
HMAC-signed authorization artifact configured out of band via
`TASKMARKET_AUTHORIZATION_FILE` + `TASKMARKET_OPERATOR_SECRET`. Without a valid
artifact the CLI process is never launched.

Run with `taskmarket-mcp` (installed console script) or
`python -m taskmarket_adapter`.
"""
from __future__ import annotations

import asyncio
import json
import sys
from typing import Any

from . import authorization as auth
from . import security
from .client import TaskmarketClient
from .errors import TaskmarketError

PROTOCOL_VERSION = "2024-11-05"
SERVER_NAME = "taskmarket-adapter"
SERVER_VERSION = "0.2.0"

TOOLS = [
    {
        "name": "taskmarket_list_tasks",
        "description": "List open Taskmarket tasks with reward, submission count, and tags. Safe read.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "status": {"type": "string", "default": "open",
                           "description": "Task status filter (open|active|resolved)."}
            },
        },
    },
    {
        "name": "taskmarket_get_task",
        "description": "Get full details for one task, including pendingActions. Safe read.",
        "inputSchema": {
            "type": "object",
            "properties": {"task_id": {"type": "string"}},
            "required": ["task_id"],
        },
    },
    {
        "name": "taskmarket_wallet_stats",
        "description": "Return wallet stats (balance, earnings, completed tasks). Read-only.",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "taskmarket_inbox",
        "description": "Show tasks you created and tasks you are working on. Read-only.",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "taskmarket_submit",
        "description": (
            "Submit a deliverable file for a task. Requires an operator-issued "
            f"'{auth.ACTION_TASK_SUBMIT}' authorization artifact; the file must be a regular "
            "file inside an operator-configured artifact root (TASKMARKET_ARTIFACT_ROOTS)."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "task_id": {"type": "string"},
                "file_path": {"type": "string",
                              "description": "Absolute path inside an allowlisted artifact root."},
            },
            "required": ["task_id", "file_path"],
        },
    },
    {
        "name": "taskmarket_create_task",
        "description": (
            "Create and fund a Taskmarket task via `task create --description --reward <usdc> "
            "--duration <h>`. Requires an operator-issued "
            f"'{auth.ACTION_TASK_CREATE}' authorization artifact; reward is human-readable USDC "
            "(e.g. \"5\" means 5 USDC) capped by the artifact."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "description": {"type": "string"},
                "reward": {"type": "string",
                           "description": "Human-readable USDC amount, at most six decimals."},
                "duration_hours": {"type": "integer", "minimum": 1},
                "network": {"type": "string",
                            "enum": sorted(security.ALLOWED_NETWORKS),
                            "description": "Declared network; validated against the Base allowlist."},
            },
            "required": ["description", "reward", "duration_hours"],
        },
    },
]


def _tool(name: str, args: dict) -> dict:
    client = TaskmarketClient()
    if name == "taskmarket_list_tasks":
        tasks = client.list_tasks(args.get("status", "open"))
        entries = tasks if isinstance(tasks, list) else []
        slim = [
            {
                "id": t["id"] if isinstance(t, dict) else None,
                "reward": t.get("reward") if isinstance(t, dict) else None,
                "submissions": t.get("submissionCount") if isinstance(t, dict) else None,
                "tags": t.get("tags") if isinstance(t, dict) else None,
                "title": ((t.get("description") or "")[:60] if isinstance(t, dict) else ""),
            }
            for t in entries
        ]
        return {"tasks": slim, "count": len(slim)}
    if name == "taskmarket_get_task":
        return client.get_task(args.get("task_id", ""))
    if name == "taskmarket_wallet_stats":
        return client.stats()
    if name == "taskmarket_inbox":
        return client.inbox()
    if name == "taskmarket_submit":
        spec = auth.load_authorization(auth.ACTION_TASK_SUBMIT)
        return client.submit(spec, task_id=args.get("task_id", ""), file_path=args.get("file_path", ""))
    if name == "taskmarket_create_task":
        security.validate_network(args.get("network"))
        spec = auth.load_authorization(auth.ACTION_TASK_CREATE)
        duration = args.get("duration_hours")
        return client.create_task(
            spec,
            description=args.get("description", ""),
            reward_usdc=str(args.get("reward", "")),
            duration_hours=duration if isinstance(duration, int) and not isinstance(duration, bool) else -1,
        )
    raise TaskmarketError(f"unknown tool: {name}")


def _error_result(rid: Any, message: str) -> dict:
    return {
        "jsonrpc": "2.0", "id": rid,
        "result": {"content": [{"type": "text", "text": f"ERROR: {message}"}], "isError": True},
    }


async def _handle(req: dict) -> dict:
    method = req.get("method")
    rid = req.get("id")
    if method == "initialize":
        return {
            "jsonrpc": "2.0", "id": rid,
            "result": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION},
            },
        }
    if method == "notifications/initialized":
        return {}
    if method == "tools/list":
        return {"jsonrpc": "2.0", "id": rid, "result": {"tools": TOOLS}}
    if method == "tools/call":
        params = req.get("params") or {}
        name = params.get("name")
        args = params.get("arguments") or {}
        try:
            result = _tool(name, args)
            return {
                "jsonrpc": "2.0", "id": rid,
                "result": {"content": [{"type": "text", "text": json.dumps(result, indent=2)}]},
            }
        except TaskmarketError as exc:
            # Policy refusals are safe to show; CLI failures were already sanitized.
            return _error_result(rid, str(exc))
        except Exception:
            print(f"[taskmarket-adapter] unexpected error in tool '{name}'", file=sys.stderr, flush=True)
            return _error_result(rid, "internal error")
    return {"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "method not found"}}


async def _serve() -> None:
    loop = asyncio.get_event_loop()
    while True:
        line = await loop.run_in_executor(None, sys.stdin.readline)
        if not line:
            break
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError:
            continue
        resp = await _handle(req)
        if resp:
            sys.stdout.write(json.dumps(resp) + "\n")
            sys.stdout.flush()


def main() -> None:
    """Synchronous entrypoint (console script target)."""
    asyncio.run(_serve())


if __name__ == "__main__":
    main()
