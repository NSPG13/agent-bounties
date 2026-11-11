# SPDX-License-Identifier: MIT
"""Deterministic unit tests for scripts/regression_verifier_watchdog.py (#1262)."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from scripts.regression_verifier_watchdog import (
    ALLOWLISTED_WORKFLOWS,
    execute_plan,
    plan_live_jobs,
    validate_strict_json_workflow,
    verify_rpc_binding,
)


def test_earliest_deadline_ordering_and_active_run_waiting(tmp_path: Path) -> None:
    main_sha = "a" * 40
    now = 1_700_000_000
    jobs = [
        {
            "job_id": "job_later",
            "deadline_unix": now + 3600,
            "missing_stage": "signer_one",
            "workflow_file": ALLOWLISTED_WORKFLOWS["signer_one"],
            "run_id": 101,
            "github_job_id": 201,
            "run_attempt": 1,
            "run_status": "completed",
            "main_sha": main_sha,
            "primary_provider_failed": True,
        },
        {
            "job_id": "job_earlier",
            "deadline_unix": now + 600,
            "missing_stage": "runner",
            "workflow_file": ALLOWLISTED_WORKFLOWS["runner"],
            "run_id": 100,
            "github_job_id": 200,
            "run_attempt": 1,
            "run_status": "completed",
            "main_sha": main_sha,
        },
        {
            "job_id": "job_in_progress",
            "deadline_unix": now + 900,
            "missing_stage": "relay",
            "workflow_file": ALLOWLISTED_WORKFLOWS["relay"],
            "run_id": 102,
            "github_job_id": 202,
            "run_attempt": 1,
            "run_status": "in_progress",
            "main_sha": main_sha,
        },
    ]

    state: dict = {"completed_keys": [], "recorded_attempts": {}}
    decisions = plan_live_jobs(jobs, protected_main_sha=main_sha, now_unix=now, state=state)

    assert [d.job_id for d in decisions] == ["job_earlier", "job_in_progress", "job_later"]
    assert decisions[0].action == "retry_runner"
    assert decisions[1].action == "wait_active_run"
    assert decisions[2].action == "retry_signer_one"
    # Secondary provider failover selected when primary_provider_failed is True
    assert decisions[2].provider_endpoint == "https://base-rpc.publicnode.com"

    # Execute and verify idempotent replay makes zero writes
    state_file = tmp_path / "state.json"
    exec1 = execute_plan(decisions, state_file, state)
    assert exec1["writes"] == 2

    decisions_replay = plan_live_jobs(jobs, protected_main_sha=main_sha, now_unix=now, state=state)
    assert decisions_replay[0].action == "noop_already_executed"
    exec2 = execute_plan(decisions_replay, state_file, state)
    assert exec2["writes"] == 0


def test_strict_json_workflow_and_rpc_binding_validation() -> None:
    wf_path = ROOT / ".github" / "workflows" / "regression-verifier-watchdog.yml"
    wf_doc = json.loads(wf_path.read_text(encoding="utf-8"))
    ok, err = validate_strict_json_workflow(wf_doc)
    assert ok is True and err is None

    # Reject workflow_dispatch trigger
    bad_wf = json.loads(wf_path.read_text(encoding="utf-8"))
    bad_wf["on"]["workflow_dispatch"] = {}
    ok2, err2 = validate_strict_json_workflow(bad_wf)
    assert ok2 is False and err2 == "forbidden_workflow_dispatch_trigger"

    # Verify Base 8453 RPC binding and EIP-712 digest check
    assert verify_rpc_binding("signer_one", 8453, "0x60806040", "0xabc", "0xABC") == (True, None)
    assert verify_rpc_binding("signer_one", 1, "0x6080", "0xabc", "0xabc")[0] is False
    assert verify_rpc_binding("signer_one", 8453, "0x", "0xabc", "0xabc")[0] is False
