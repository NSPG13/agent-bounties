"""Taskmarket MCP server (stdio JSON-RPC 2.0, no third-party deps).

Exposes Taskmarket work discovery, creation, submission, wallet status, and
inbox retrieval as MCP tools so an agent or product can delegate real work to
Taskmarket workers. All monetary actions require an explicit `authorized`
argument and are gated by spending checks; settlement status is always returned
to the caller and never fabricated or blindly retried.
"""
from __future__ import annotations

import asyncio
import json
import sys
from typing import Any

from taskmarket_client import TaskmarketClient, TaskmarketError

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
        "description": "Get full details (description, reward, deadline, deliverables, network, max spend) for one task.",
        "inputSchema": {
            "type": "object",
            "properties": {"task_id": {"type": "string"}},
            "required": ["task_id"],
        },
    },
    {
        "name": "taskmarket_wallet_stats",
        "description": "Return the agent wallet stats (balance, total earnings, completed tasks). Read-only.",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "taskmarket_inbox",
        "description": "Show tasks you created and tasks you are working on. Read-only.",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "taskmarket_submit",
        "description": "Submit completed work (a local file) for a task. Free when the task requiresPayment=false.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "task_id": {"type": "string"},
                "file_path": {"type": "string", "description": "Absolute path to the deliverable file."},
            },
            "required": ["task_id", "file_path"],
        },
    },
    {
        "name": "taskmarket_create_task",
        "description": "Create and fund a Taskmarket task. REQUIRES explicit authorization and respects max spend.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "description": {"type": "string"},
                "reward": {"type": "integer", "description": "Reward in USDC base units (1 USDC = 1_000_000)."},
                "deadline": {"type": "string", "description": "ISO deadline."},
                "deliverables": {"type": "string"},
                "network": {"type": "string", "default": "base-mainnet"},
                "max_spend_usdc": {"type": "integer", "description": "Hard cap the caller authorizes."},
                "authorized": {"type": "boolean", "description": "MUST be true; otherwise the call is refused."},
            },
            "required": ["description", "reward", "deadline", "deliverables", "max_spend_usdc", "authorized"],
        },
    },
]


def _tool(name: str, args: dict) -> dict:
    client = TaskmarketClient()
    if name == "taskmarket_list_tasks":
        tasks = client.list_tasks(args.get("status", "open"))
        slim = [
            {
                "id": t["id"],
                "reward": t.get("reward"),
                "submissions": t.get("submissionCount"),
                "tags": t.get("tags"),
                "title": (t.get("description") or "")[:60],
            }
            for t in tasks
        ]
        return {"tasks": slim, "count": len(slim)}
    if name == "taskmarket_get_task":
        return client.get_task(args["task_id"])
    if name == "taskmarket_wallet_stats":
        return client.stats()
    if name == "taskmarket_inbox":
        return client.inbox()
    if name == "taskmarket_submit":
        return client.submit(args["task_id"], args["file_path"])
    if name == "taskmarket_create_task":
        return client.create_task(
            description=args["description"],
            reward=int(args["reward"]),
            deadline=args["deadline"],
            deliverables=args["deliverables"],
            network=args.get("network", "base-mainnet"),
            max_spend_usdc=int(args["max_spend_usdc"]),
            authorized=bool(args.get("authorized")),
        )
    raise TaskmarketError(f"unknown tool: {name}")


async def _handle(req: dict) -> dict:
    method = req.get("method")
    rid = req.get("id")
    if method == "initialize":
        return {
            "jsonrpc": "2.0", "id": rid,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "taskmarket", "version": "0.1.0"},
            },
        }
    if method == "notifications/initialized":
        return {}
    if method == "tools/list":
        return {"jsonrpc": "2.0", "id": rid, "result": {"tools": TOOLS}}
    if method == "tools/call":
        name = req["params"]["name"]
        args = req["params"].get("arguments", {})
        try:
            result = _tool(name, args)
            return {
                "jsonrpc": "2.0", "id": rid,
                "result": {"content": [{"type": "text", "text": json.dumps(result, indent=2)}]},
            }
        except TaskmarketError as exc:
            return {
                "jsonrpc": "2.0", "id": rid,
                "result": {"content": [{"type": "text", "text": f"ERROR: {exc}"}], "isError": True},
            }
    return {"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "method not found"}}


async def main() -> None:
    stdin, stdout = sys.stdin, sys.stdout
    loop = asyncio.get_event_loop()
    while True:
        line = await loop.run_in_executor(None, stdin.readline)
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
            stdout.write(json.dumps(resp) + "\n")
            stdout.flush()


if __name__ == "__main__":
    asyncio.run(main())
