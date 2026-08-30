#!/usr/bin/env python3
"""Prove the immutable checker accepts safe behavior and rejects unsafe behavior."""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path


BENCHMARK = Path(__file__).resolve().parent
CHECKER = BENCHMARK / "check.py"

GOOD_PLANNER = r'''#!/usr/bin/env python3
import argparse
import hashlib
import json
from datetime import datetime, timezone


def parse_time(value):
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


parser = argparse.ArgumentParser()
sub = parser.add_subparsers(dest="command", required=True)
plan = sub.add_parser("plan")
plan.add_argument("--jobs", required=True)
plan.add_argument("--runs", required=True)
plan.add_argument("--policy", required=True)
plan.add_argument("--now", required=True)
args = parser.parse_args()
jobs_doc = json.load(open(args.jobs, encoding="utf-8"))
runs_doc = json.load(open(args.runs, encoding="utf-8"))
policy = json.load(open(args.policy, encoding="utf-8"))
now = parse_time(args.now)
records = []
for job in sorted(jobs_doc["jobs"], key=lambda item: (item["verification_expires_at"], item["job_id"])):
    job_runs = [item for item in runs_doc["runs"] if item["job_id"] == job["job_id"]]
    action = "dispatch_runner"
    owner = "regression-verifier-runner"
    automated = True
    provider = "runner"
    reason = "No candidate run exists for this live canonical job."
    expires = parse_time(job["verification_expires_at"])
    successful_signers = [
        item.get("signer")
        for item in job_runs
        if item["stage"].startswith("signer_") and item["conclusion"] == "success"
    ]
    stale = any(item["head_sha"] != policy["current_main_sha"] for item in job_runs)
    unknown_workflow = any(item["workflow"] not in policy["allowed_workflows"] for item in job_runs)
    canonical_drift = any(
        item.get("canonical_job_hash")
        and item["canonical_job_hash"] != job["canonical_job_hash"]
        for item in job_runs
    )
    nonretryable_failure = any(
        item["conclusion"] == "failure" and not item.get("retryable", False)
        for item in job_runs
    )
    exhausted = any(
        item["conclusion"] == "failure"
        and item["attempt"] >= policy["max_attempts_per_stage"]
        for item in job_runs
    )
    terminal = job.get("canonical_terminal_event") or job.get("status") in {"settled", "rejected", "cancelled"}
    if terminal:
        action, owner, automated, provider = "observe_terminal", "canonical-indexer", False, "canonical_chain"
        reason = "A canonical terminal event is authoritative."
    elif expires <= now:
        action, owner, automated, provider = "expire_submission", "permissionless-keeper", False, "canonical_chain"
        reason = "The immutable verification deadline has passed."
    elif job.get("input_readiness") != "ready":
        action, owner, automated, provider = "escalate_no_verdict", "maintainer-on-call", False, "none"
        reason = "The input is unavailable; infrastructure cannot produce a verdict."
    elif (expires - now).total_seconds() < policy["minimum_retry_budget_seconds"]:
        action, owner, automated, provider = "escalate_no_verdict", "maintainer-on-call", False, "none"
        reason = "Too little time remains for a safe bounded retry."
    elif (
        stale
        or unknown_workflow
        or canonical_drift
        or nonretryable_failure
        or exhausted
        or len(successful_signers) != len(set(successful_signers))
    ):
        action, owner, automated, provider = "escalate_no_verdict", "maintainer-on-call", False, "none"
        reason = "Stale, replay-like, or exhausted evidence blocks automation."
    else:
        latest = {stage: None for stage in ("runner", "signer_one", "signer_two", "relay")}
        for item in job_runs:
            latest[item["stage"]] = item
        runner = latest["runner"]
        one = latest["signer_one"]
        two = latest["signer_two"]
        relay = latest["relay"]
        if runner and runner["conclusion"] == "failure":
            action, owner, provider = "retry_runner", "regression-verifier-runner", "runner"
            reason = "The candidate runner failed retryably."
        elif runner and runner["conclusion"] == "success" and (not one or one["conclusion"] != "success"):
            action, owner, provider = "retry_signer_one", "regression-verifier-signer-one", "signer_one_secondary"
            reason = "Signer one is the first missing stage."
        elif one and one["conclusion"] == "success" and (not two or two["conclusion"] != "success"):
            action, owner, provider = "retry_signer_two", "regression-verifier-signer-two", "signer_two_secondary"
            reason = "Signer two is the only missing signer stage."
        elif two and two["conclusion"] == "success" and relay and relay["conclusion"] == "failure":
            action, owner, provider = "retry_relay", "regression-verifier-relay", "relay_secondary"
            reason = "Both signers succeeded and only the retryable relay failed."
        elif two and two["conclusion"] == "success":
            action, owner, automated, provider = "reconcile_canonical_state", "canonical-indexer", False, "canonical_chain"
            reason = "The quorum exists; canonical state must be reconciled."
    key_payload = json.dumps([job["job_id"], job["canonical_job_hash"], action, provider], separators=(",", ":"))
    records.append({
        "job_id": job["job_id"],
        "verification_expires_at": job["verification_expires_at"],
        "next_action": action,
        "next_owner": owner,
        "automation_allowed": automated,
        "provider_role": provider,
        "reason": reason,
        "recheck_at": args.now,
        "idempotency_key": "sha256:" + hashlib.sha256(key_payload.encode()).hexdigest(),
    })
print(json.dumps({
    "schema": "agent-bounties/regression-verifier-watchdog-plan-v1",
    "network": jobs_doc["network"],
    "generated_at": args.now,
    "fail_closed": True,
    "jobs": records,
}, sort_keys=True, separators=(",", ":")))
'''

BAD_PLANNER = r'''#!/usr/bin/env python3
import json
print(json.dumps({"schema":"agent-bounties/regression-verifier-watchdog-plan-v1","network":"base-mainnet","generated_at":"2026-09-01T12:00:00Z","fail_closed":False,"jobs":[]}))
'''

WATCHDOG_WORKFLOW = '''name: Regression Verifier Watchdog
on:
  schedule:
    - cron: "3,8,13,18,23,28,33,38,43,48,53,58 * * * *"
  workflow_dispatch:
permissions:
  contents: read
  actions: write
jobs:
  watchdog:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@pinned
      - run: python scripts/regression_verifier_watchdog.py execute --execute --allow-workflow regression-verifier-runner.yml --allow-workflow regression-verifier-signer.yml
        env:
          GITHUB_TOKEN: ${{ github.token }}
'''

SIGNER_WORKFLOW = '''name: Regression Verifier Signer
env:
  ONE: ${{ vars.REGRESSION_VERIFIER_ONE_RPC_URL }}
  TWO: ${{ vars.REGRESSION_VERIFIER_TWO_RPC_URL }}
  RELAY: ${{ vars.REGRESSION_VERIFIER_RELAY_RPC_URL }}
'''

DOC = '''# Verifier watchdog
The watchdog must fail closed. Idempotency bounds retries across each provider role.
It never creates a verdict, and only canonical BountySettled proves payment.
'''


def build(root: Path, planner: str) -> None:
    paths = {
        "scripts/regression_verifier_watchdog.py": planner,
        "scripts/test_regression_verifier_watchdog.py": "import unittest\nclass T(unittest.TestCase):\n    def test_ok(self): self.assertTrue(True)\nif __name__ == '__main__': unittest.main()\n",
        ".github/workflows/regression-verifier-watchdog.yml": WATCHDOG_WORKFLOW,
        ".github/workflows/regression-verifier-signer.yml": SIGNER_WORKFLOW,
        "docs/sandboxed-regression-verifier.md": DOC,
    }
    for relative, content in paths.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")


def check(root: Path) -> subprocess.CompletedProcess[str]:
    environment = os.environ.copy()
    environment["WORKSPACE_ROOT"] = str(root)
    return subprocess.run(
        [sys.executable, str(CHECKER)],
        cwd=BENCHMARK,
        env=environment,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=180,
        check=False,
    )


with tempfile.TemporaryDirectory(prefix="watchdog-known-good-") as temporary:
    good = Path(temporary)
    build(good, GOOD_PLANNER)
    result = check(good)
    if result.returncode != 0:
        raise SystemExit("known-good rehearsal failed:\n" + result.stdout[-5000:])

with tempfile.TemporaryDirectory(prefix="watchdog-known-bad-") as temporary:
    bad = Path(temporary)
    build(bad, BAD_PLANNER)
    result = check(bad)
    if result.returncode == 0:
        raise SystemExit("known-bad rehearsal was incorrectly accepted")
    if "fail closed" not in result.stdout.lower():
        raise SystemExit("known-bad rehearsal failed for the wrong reason:\n" + result.stdout[-5000:])

print("known-good accepted and known-bad rejected")
