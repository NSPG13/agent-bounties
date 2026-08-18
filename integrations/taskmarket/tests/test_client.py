"""Tests for the Taskmarket client wrapper and MCP tool dispatch."""
from __future__ import annotations

from unittest import mock

from taskmarket_client import TaskmarketClient, TaskmarketError


def _client(stdout: str = "", rc: int = 0, stderr: str = ""):
    c = TaskmarketClient()
    proc = mock.Mock(returncode=rc, stdout=stdout, stderr=stderr)
    c._run = mock.MagicMock(return_value=__import__("json").loads(stdout) if stdout.startswith("{") else {})
    return c


def test_stats_parses_json():
    c = _client('{"ok":true,"data":{"balanceUsdc":"0.009000","totalEarnings":"0"}}')
    stats = c.stats()
    assert stats["data"]["balanceUsdc"] == "0.009000"


def test_create_task_refuses_without_authorization():
    c = TaskmarketClient()
    c._run = mock.MagicMock()
    try:
        c.create_task(
            description="demo", reward=50000, deadline="2026-09-01",
            deliverables="x", network="base-mainnet", max_spend_usdc=100000,
            authorized=False,
        )
        assert False, "should have refused"
    except TaskmarketError as e:
        assert "authorization" in str(e).lower()


def test_create_task_refuses_over_spend():
    c = TaskmarketClient()
    c._run = mock.MagicMock()
    try:
        c.create_task(
            description="demo", reward=200000, deadline="2026-09-01",
            deliverables="x", network="base-mainnet", max_spend_usdc=100000,
            authorized=True,
        )
        assert False, "should have refused over spend"
    except TaskmarketError as e:
        assert "exceeds" in str(e)


def test_create_task_allows_when_authorized():
    c = TaskmarketClient()
    c._run = mock.MagicMock(return_value={"ok": True})
    out = c.create_task(
        description="demo", reward=50000, deadline="2026-09-01",
        deliverables="x", network="base-mainnet", max_spend_usdc=100000,
        authorized=True,
    )
    assert out == {"ok": True}
    c._run.assert_called_once()


def test_list_tasks_slim():
    payload = {"data": {"tasks": [
        {"id": "0xabc", "reward": "50000", "submissionCount": 4, "tags": ["arc"],
         "description": "Join my ARC branch"}
    ]}}
    c = _client()
    c._run = mock.MagicMock(return_value=payload)
    tasks = c.list_tasks()
    assert tasks[0]["id"] == "0xabc"
    assert tasks[0]["submissionCount"] == 4


def test_submit_calls_cli():
    c = TaskmarketClient()
    c._run = mock.MagicMock(return_value={"ok": True})
    c.submit("0xabc", "/tmp/deliverable.md")
    c._run.assert_called_once_with("task", "submit", "0xabc", "--file", "/tmp/deliverable.md")
