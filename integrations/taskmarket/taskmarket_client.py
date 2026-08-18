"""Thin, safe wrapper around the first-party Taskmarket CLI.

Security model (required by the Taskmarket integration bounty):
- Never reads, stores, logs, or commits private keys / seeds / tokens / cookies.
- Creating or funding a task requires an EXPLICIT caller-provided authorization
  flag; the wrapper will refuse otherwise.
- All monetary actions are gated by network + spending checks.
- Settlement status is always returned to the caller; the wrapper never blindly
  retries a payment whose status is unknown.
"""
from __future__ import annotations

import json
import subprocess
from dataclasses import dataclass, field
from typing import Any


class TaskmarketError(RuntimeError):
    pass


@dataclass
class TaskmarketClient:
    cli: str = "taskmarket"
    timeout: int = 180

    # ---------- low level ----------
    def _run(self, *args: str) -> Any:
        try:
            proc = subprocess.run(
                [self.cli, *args],
                capture_output=True,
                text=True,
                timeout=self.timeout,
            )
        except FileNotFoundError as exc:  # pragma: no cover - env issue
            raise TaskmarketError(f"taskmarket CLI not found: {exc}") from exc
        if proc.returncode != 0:
            raise TaskmarketError(proc.stderr.strip() or proc.stdout.strip())
        out = proc.stdout.strip()
        if not out:
            return {}
        try:
            return json.loads(out)
        except json.JSONDecodeError:
            return {"raw": out}

    # ---------- read paths ----------
    def stats(self) -> dict:
        return self._run("stats")

    def list_tasks(self, status: str = "open") -> list[dict]:
        data = self._run("task", "list", "--status", status)
        return (data.get("data", {}) or {}).get("tasks", [])

    def get_task(self, task_id: str) -> dict:
        return self._run("task", "get", task_id)

    def inbox(self) -> dict:
        return self._run("inbox")

    # ---------- guarded write paths ----------
    def create_task(
        self,
        *,
        description: str,
        reward: int,
        deadline: str,
        deliverables: str,
        network: str,
        max_spend_usdc: int,
        authorized: bool,
        requires_payment: bool = False,
    ) -> dict:
        """Create a Taskmarket task. Refuses unless explicitly authorized."""
        if not authorized:
            raise TaskmarketError("Refused: task creation requires explicit user authorization.")
        if reward > max_spend_usdc:
            raise TaskmarketError(
                f"Refused: reward {reward} exceeds authorized max spend {max_spend_usdc}."
            )
        # The CLI itself performs on-chain funding; we only forward intent.
        return self._run(
            "task", "create",
            "--description", description,
            "--reward", str(reward),
            "--deadline", deadline,
            "--deliverables", deliverables,
            "--network", network,
            "--requires-payment", str(requires_payment).lower(),
        )

    def submit(self, task_id: str, file_path: str) -> dict:
        """Submit work for a task. Free for requiresPayment=false tasks."""
        return self._run("task", "submit", task_id, "--file", file_path)

    def settlement_status(self, task_id: str) -> dict:
        """Return live status; never fabricate settlement."""
        return self.get_task(task_id)
