"""Shared helpers: a recording fake `taskmarket` CLI and env builders.

The fake CLI is a real executable process. It appends each invocation's argv
to FAKE_CLI_LOG so tests can prove exactly which commands ran - and, for
refusal tests, that NO process was launched at all.
"""
from __future__ import annotations

import json
import os
import pathlib
import sys
from datetime import datetime, timedelta, timezone

from taskmarket_adapter import authorization as auth

FAKE_CLI = r'''#!/usr/bin/env python3
import json, os, sys

log_path = os.environ["FAKE_CLI_LOG"]
with open(log_path, "a", encoding="utf-8") as fh:
    fh.write(json.dumps(sys.argv[1:]) + "\n")

if os.environ.get("FAKE_CLI_MODE") == "fail":
    sys.stderr.write("SECRET-HOST-DETAIL leaked-token\n")
    sys.exit(1)

sys.stdout.write(json.dumps({"ok": True, "data": {"argv": sys.argv[1:]}}))
'''

ENV_KEYS = (
    "TASKMARKET_AUTHORIZATION_FILE",
    "TASKMARKET_OPERATOR_SECRET",
    "TASKMARKET_ARTIFACT_ROOTS",
    "TASKMARKET_MAX_ARTIFACT_BYTES",
    "FAKE_CLI_LOG",
    "FAKE_CLI_MODE",
)


def make_fake_cli(bin_dir: pathlib.Path) -> pathlib.Path:
    bin_dir.mkdir(parents=True, exist_ok=True)
    cli = bin_dir / "taskmarket"
    cli.write_text(FAKE_CLI)
    cli.chmod(0o755)
    return cli


def clean_env(base: dict) -> dict:
    return {k: v for k, v in base.items() if k not in ENV_KEYS}


def write_create_artifact(
    path: pathlib.Path,
    secret: str,
    *,
    action: str = auth.ACTION_TASK_CREATE,
    expires_in: timedelta = timedelta(hours=1),
    expires_at: datetime | None = None,
    max_reward_usdc: str | None = None,
    max_duration_hours: int | None = None,
    task_id: str | None = None,
) -> pathlib.Path:
    expiry = expires_at if expires_at is not None else datetime.now(timezone.utc) + expires_in
    expiry_text = expiry if isinstance(expiry, str) else expiry.isoformat()
    spec: dict = {
        "version": 1,
        "action": action,
        "expires_at": expiry_text,
    }
    if max_reward_usdc is not None:
        spec["max_reward_usdc"] = max_reward_usdc
    if max_duration_hours is not None:
        spec["max_duration_hours"] = max_duration_hours
    if task_id is not None:
        spec["task_id"] = task_id
    path.parent.mkdir(parents=True, exist_ok=True)
    auth.write_artifact(path, spec, secret)
    return path


def read_invocations(log_path: pathlib.Path) -> list:
    if not log_path.exists():
        return []
    return [json.loads(line) for line in log_path.read_text().splitlines() if line.strip()]
