"""Subprocess wrapper around the first-party `taskmarket` CLI.

argv is built exactly from the official Taskmarket command contract:

    taskmarket task create --description "..." --reward <usdc> --duration <hours>
    taskmarket task submit <taskId> --file <path>
    taskmarket task list --status open
    taskmarket task get <taskId>
    taskmarket inbox
    taskmarket stats

No other flags are sent: unsupported flags are how spend gets misstated. The
CLI takes human-readable USDC on `--reward`; conversion to base units happens
only for local cap checks, never in the argv.

Process output is sanitized before it reaches the MCP caller: CLI stderr is
logged to the host process (stderr of this server) and callers only see a
generic failure message.
"""
from __future__ import annotations

import json
import subprocess
import sys
from typing import Any, Callable, Optional, Sequence

from . import authorization as auth
from . import security
from .errors import TaskmarketError
from .units import parse_usdc, usdc_flag_value

Runner = Callable[..., Any]


def build_create_argv(description: str, reward_usdc_flag: str, duration_hours: int) -> list:
    """Exact argv (without the program name) for the official create contract."""
    return [
        "task", "create",
        "--description", description,
        "--reward", reward_usdc_flag,
        "--duration", str(duration_hours),
    ]


def build_submit_argv(task_id: str, file_path: str) -> list:
    """Exact argv (without the program name) for the official submit contract."""
    return ["task", "submit", task_id, "--file", file_path]


class TaskmarketClient:
    def __init__(
        self,
        cli: str = "taskmarket",
        timeout: int = 180,
        runner: Optional[Runner] = None,
    ) -> None:
        self.cli = cli
        self.timeout = timeout
        self._runner = runner or subprocess.run

    # ---------- low level ----------
    def _run(self, args: Sequence[str]) -> Any:
        try:
            proc = self._runner(
                [self.cli, *args],
                capture_output=True,
                text=True,
                timeout=self.timeout,
            )
        except FileNotFoundError:
            raise TaskmarketError("taskmarket CLI is not available on this host") from None
        except subprocess.TimeoutExpired:
            raise TaskmarketError("taskmarket CLI call timed out") from None
        if proc.returncode != 0:
            # Host-side log only; the caller-facing error stays sanitized.
            print(
                f"[taskmarket-adapter] cli failed (exit {proc.returncode}): "
                f"{(proc.stderr or '').strip()[:2000]}",
                file=sys.stderr,
                flush=True,
            )
            raise TaskmarketError(f"taskmarket CLI failed (exit {proc.returncode})")
        out = (proc.stdout or "").strip()
        if not out:
            raise TaskmarketError("taskmarket CLI returned an empty response")
        try:
            envelope = json.loads(out)
        except json.JSONDecodeError:
            raise TaskmarketError("taskmarket CLI returned an unreadable response") from None
        if isinstance(envelope, dict) and envelope.get("ok") is False:
            raise TaskmarketError("taskmarket CLI rejected the request")
        if isinstance(envelope, dict) and "data" in envelope:
            return envelope["data"]
        return envelope

    # ---------- read paths ----------
    def stats(self) -> Any:
        return self._run(["stats"])

    def list_tasks(self, status: str = "open") -> Any:
        return self._run(["task", "list", "--status", status])

    def get_task(self, task_id: str) -> Any:
        if not isinstance(task_id, str) or not task_id.strip():
            raise TaskmarketError("refused: task id required")
        return self._run(["task", "get", task_id])

    def inbox(self) -> Any:
        return self._run(["inbox"])

    # ---------- guarded write paths ----------
    def create_task(
        self,
        spec: auth.AuthorizationSpec,
        *,
        description: str,
        reward_usdc: str,
        duration_hours: int,
    ) -> Any:
        """Create a task; requires a verified operator artifact covering task_create."""
        if not isinstance(description, str) or not description.strip():
            raise TaskmarketError("refused: description required")
        if isinstance(duration_hours, bool) or not isinstance(duration_hours, int) or duration_hours <= 0:
            raise TaskmarketError("refused: duration must be a positive number of hours")

        reward = parse_usdc(reward_usdc)
        auth.enforce_spend(spec, reward, duration_hours)

        return self._run(build_create_argv(description, usdc_flag_value(reward), duration_hours))

    def submit(self, spec: auth.AuthorizationSpec, *, task_id: str, file_path: str) -> Any:
        """Submit an artifact; requires a verified operator artifact covering task_submit."""
        if not isinstance(task_id, str) or not task_id.strip():
            raise TaskmarketError("refused: task id required")
        auth.enforce_task_binding(spec, task_id)
        resolved = security.resolve_artifact(file_path)
        return self._run(build_submit_argv(task_id, str(resolved)))
