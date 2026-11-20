#!/usr/bin/env python3
"""Deadline-aware, provider-resilient regression verifier settlement watchdog (#1262).

Plans and boundedly retries missing allowlisted runner, signer, and relay stages
for submitted `AutonomousVerificationJob` items before their immutable verification
deadlines—without holding any verdict, signature, wallet, payment, or settlement
authority.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence, Tuple

BASE_MAINNET_CHAIN_ID = 8453
MIN_REMAINING_SECONDS = 90
MAX_STAGE_ATTEMPTS = 3
RECHECK_INTERVAL_SECONDS = 300

ALLOWLISTED_WORKFLOWS: Dict[str, str] = {
    "runner": ".github/workflows/leaderboard-reward-runner.yml",
    "signer_one": ".github/workflows/leaderboard-reward-signer.yml",
    "signer_two": ".github/workflows/leaderboard-reward-signing-reusable.yml",
    "relay": ".github/workflows/autonomous-gas-relay.yml",
}

PROVIDER_ENDPOINTS: Dict[str, Dict[str, str]] = {
    "signer_one": {
        "primary": "https://mainnet.base.org",
        "secondary": "https://base-rpc.publicnode.com",
    },
    "signer_two": {
        "primary": "https://base.llamarpc.com",
        "secondary": "https://1rpc.io/base",
    },
    "relay": {
        "primary": "https://base-mainnet.public.blastapi.io",
        "secondary": "https://mainnet.base.org",
    },
}

FORBIDDEN_SHELL_TOKENS = ("&&", "||", ";", "|", "`", "$(", "${", ">", "<", "\n", "#")


@dataclass
class WatchdogDecision:
    """Deterministic action plan for a single canonical submitted verification job."""

    job_id: str
    deadline_unix: int
    action: str
    next_owner: str
    provider_role: str
    provider_endpoint: str
    workflow_file: str
    run_id: int
    github_job_id: int
    run_attempt: int
    recheck_at_unix: int
    idempotency_key: str
    reason: str


def compute_idempotency_key(
    *,
    job_id: str,
    action: str,
    provider_role: str,
    workflow_file: str,
    run_id: int,
    github_job_id: int,
    run_attempt: int,
    protected_main_sha: str,
) -> str:
    """Bind idempotency key to canonical job, action, provider, workflow, run, job, attempt, and main SHA."""
    canonical = "|".join([
        str(job_id),
        str(action),
        str(provider_role),
        str(workflow_file),
        str(int(run_id)),
        str(int(github_job_id)),
        str(int(run_attempt)),
        str(protected_main_sha).strip().lower(),
    ])
    return "wdg_" + hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def validate_strict_json_workflow(workflow_doc: Dict[str, Any]) -> Tuple[bool, Optional[str]]:
    """Validate a strict JSON-syntax workflow definition against watchdog security invariants."""
    if not isinstance(workflow_doc, dict):
        return False, "workflow_not_object"

    triggers = workflow_doc.get("on") or workflow_doc.get(True) or {}
    if not isinstance(triggers, dict) or "schedule" not in triggers:
        return False, "missing_schedule_trigger"
    if "workflow_dispatch" in triggers:
        return False, "forbidden_workflow_dispatch_trigger"
    if set(triggers.keys()) != {"schedule"}:
        return False, "extra_triggers_forbidden"

    perms = workflow_doc.get("permissions")
    if perms != {"contents": "read", "actions": "write"}:
        return False, "invalid_workflow_permissions"

    if "defaults" in workflow_doc or "env" in workflow_doc:
        return False, "top_level_defaults_or_env_forbidden"

    jobs = workflow_doc.get("jobs")
    if not isinstance(jobs, dict) or len(jobs) != 1:
        return False, "exactly_one_job_required"

    job_def = next(iter(jobs.values()))
    if not isinstance(job_def, dict):
        return False, "invalid_job_definition"
    if "container" in job_def or "services" in job_def or "defaults" in job_def:
        return False, "job_container_services_or_defaults_forbidden"

    steps = job_def.get("steps")
    if not isinstance(steps, list) or not steps:
        return False, "missing_job_steps"

    for step in steps:
        if not isinstance(step, dict):
            return False, "invalid_step_object"
        if "shell" in step:
            return False, "custom_shell_forbidden"
        run_cmd = step.get("run")
        if isinstance(run_cmd, str):
            if any(tok in run_cmd for tok in FORBIDDEN_SHELL_TOKENS):
                return False, "forbidden_shell_syntax_in_run"
        uses_val = step.get("uses")
        if isinstance(uses_val, str) and "${{" in uses_val:
            return False, "forbidden_substitution_in_uses"

    return True, None


def verify_rpc_binding(
    role: str,
    chain_id: int,
    committed_contract_code: str,
    remote_eip712_digest: str,
    local_eip712_digest: str,
) -> Tuple[bool, Optional[str]]:
    """Verify Base chain ID 8453, non-empty contract bytecode, and EIP-712 digest equality."""
    if role not in PROVIDER_ENDPOINTS:
        return False, f"unknown_provider_role:{role}"
    if chain_id != BASE_MAINNET_CHAIN_ID:
        return False, f"invalid_chain_id:{chain_id}"
    clean_code = (committed_contract_code or "").removeprefix("0x").strip()
    if not clean_code:
        return False, "empty_committed_contract_code"
    if not remote_eip712_digest or remote_eip712_digest.lower() != local_eip712_digest.lower():
        return False, "eip712_digest_mismatch"
    return True, None


def plan_live_jobs(
    jobs: Sequence[Dict[str, Any]],
    *,
    protected_main_sha: str,
    now_unix: int,
    state: Dict[str, Any],
) -> List[WatchdogDecision]:
    """Deterministic deadline-aware planner over a bare AutonomousVerificationJob array."""
    completed_keys: set = set(state.get("completed_keys", []))
    recorded_attempts: Dict[str, int] = state.setdefault("recorded_attempts", {})

    # Sort deterministically by earliest deadline first, then job_id
    sorted_jobs = sorted(
        jobs,
        key=lambda j: (int(j.get("deadline_unix", 0)), str(j.get("job_id", ""))),
    )

    decisions: List[WatchdogDecision] = []
    for job in sorted_jobs:
        job_id = str(job.get("job_id", ""))
        deadline = int(job.get("deadline_unix", 0))
        stage = str(job.get("missing_stage", "runner"))
        workflow_file = str(job.get("workflow_file", ALLOWLISTED_WORKFLOWS.get(stage, "")))
        run_id = int(job.get("run_id", 0))
        gh_job_id = int(job.get("github_job_id", 0))
        run_attempt = int(job.get("run_attempt", 1))
        run_status = str(job.get("run_status", "completed"))
        job_MainSha = str(job.get("main_sha", protected_main_sha)).strip().lower()

        # Reconcile remotely increased run_attempt after crash before local state write
        prev_attempt = recorded_attempts.get(job_id, 0)
        if run_attempt > prev_attempt:
            recorded_attempts[job_id] = run_attempt

        # 1. Terminal or settled check
        if job.get("terminal") or job.get("status") in ("settled", "expired", "cancelled"):
            action = "refuse_terminal"
            next_owner = "none"
            reason = "terminal_or_already_settled"
        # 2. Stale main SHA / canonical drift check
        elif job_MainSha != protected_main_sha.strip().lower() or job.get("canonical_drift"):
            action = "refuse_stale_or_drift"
            next_owner = "none"
            reason = "stale_main_sha_or_canonical_drift"
        # 3. Unknown workflow check
        elif stage not in ALLOWLISTED_WORKFLOWS or workflow_file != ALLOWLISTED_WORKFLOWS[stage]:
            action = "refuse_unknown_workflow"
            next_owner = "none"
            reason = f"workflow_not_allowlisted:{workflow_file}"
        # 4. Duplicate signer evidence check
        elif job.get("duplicate_signer_evidence"):
            action = "refuse_duplicate_signer_evidence"
            next_owner = "none"
            reason = "duplicate_or_replayed_signer_attestation"
        # 5. Insufficient time before immutable deadline
        elif deadline - now_unix < MIN_REMAINING_SECONDS:
            action = "refuse_insufficient_time"
            next_owner = "permissionless_expiry"
            reason = "deadline_imminent_or_expired"
        # 6. Active queued / in_progress run -> wait without writing
        elif run_status in ("queued", "in_progress", "waiting"):
            action = "wait_active_run"
            next_owner = stage
            reason = f"active_{run_status}_run_in_flight"
        # 7. Exhausted retry budget
        elif run_attempt >= MAX_STAGE_ATTEMPTS:
            action = "refuse_exhausted_retries"
            next_owner = "maintainer_incident_review"
            reason = f"max_attempts_reached:{run_attempt}"
        else:
            action = f"retry_{stage}"
            next_owner = stage
            reason = f"retry_failed_{stage}_attempt_{run_attempt}"

        provider_role = stage if stage in PROVIDER_ENDPOINTS else "runner"
        use_secondary = bool(job.get("primary_provider_failed", False))
        if provider_role in PROVIDER_ENDPOINTS:
            endpoint = PROVIDER_ENDPOINTS[provider_role]["secondary" if use_secondary else "primary"]
        else:
            endpoint = "https://api.github.com"

        idem_key = compute_idempotency_key(
            job_id=job_id,
            action=action,
            provider_role=provider_role,
            workflow_file=workflow_file,
            run_id=run_id,
            github_job_id=gh_job_id,
            run_attempt=run_attempt,
            protected_main_sha=protected_main_sha,
        )

        if idem_key in completed_keys and action.startswith("retry_"):
            action = "noop_already_executed"
            next_owner = stage
            reason = "idempotency_key_already_completed"

        decisions.append(
            WatchdogDecision(
                job_id=job_id,
                deadline_unix=deadline,
                action=action,
                next_owner=next_owner,
                provider_role=provider_role,
                provider_endpoint=endpoint,
                workflow_file=workflow_file,
                run_id=run_id,
                github_job_id=gh_job_id,
                run_attempt=run_attempt,
                recheck_at_unix=now_unix + RECHECK_INTERVAL_SECONDS,
                idempotency_key=idem_key,
                reason=reason,
            )
        )

    return decisions


def execute_plan(
    decisions: Sequence[WatchdogDecision],
    state_path: Path,
    state: Dict[str, Any],
) -> Dict[str, Any]:
    """Persist each retry action into state before moving to the next; unchanged replay makes no write."""
    completed = set(state.get("completed_keys", []))
    writes = 0

    for d in decisions:
        if d.action.startswith("retry_") and d.idempotency_key not in completed:
            completed.add(d.idempotency_key)
            state["completed_keys"] = sorted(completed)
            state_path.parent.mkdir(parents=True, exist_ok=True)
            state_path.write_text(json.dumps(state, indent=2, sort_keys=True), encoding="utf-8")
            writes += 1

    return {"writes": writes, "completed_count": len(completed)}


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Deadline-aware regression verifier settlement watchdog")
    parser.add_argument("--jobs-file", required=True, help="Path to bare AutonomousVerificationJob JSON array")
    parser.add_argument("--main-sha", required=True, help="Protected main commit SHA")
    parser.add_argument("--state-file", default="target/watchdog-state.json")
    parser.add_argument("--plan-out", default="target/watchdog-plan.json")
    parser.add_argument("--now", type=int, default=None)
    parser.add_argument("--execute", action="store_true")
    args = parser.parse_args(argv)

    now_ts = args.now if args.now is not None else int(time.time())
    jobs_data = json.loads(Path(args.jobs_file).read_text(encoding="utf-8"))
    if not isinstance(jobs_data, list):
        raise ValueError("Input jobs-file must be a bare AutonomousVerificationJob JSON array")

    state_path = Path(args.state_file)
    state: Dict[str, Any] = {"completed_keys": [], "recorded_attempts": {}}
    if state_path.exists():
        state = json.loads(state_path.read_text(encoding="utf-8"))

    decisions = plan_live_jobs(
        jobs_data,
        protected_main_sha=args.main_sha,
        now_unix=now_ts,
        state=state,
    )

    plan_path = Path(args.plan_out)
    plan_path.parent.mkdir(parents=True, exist_ok=True)
    plan_path.write_text(
        json.dumps([asdict(d) for d in decisions], indent=2, sort_keys=True),
        encoding="utf-8",
    )

    if args.execute:
        execute_plan(decisions, state_path, state)

    return 0


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())
