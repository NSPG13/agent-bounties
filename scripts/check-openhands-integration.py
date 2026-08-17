#!/usr/bin/env python3
"""Smoke test for the OpenHands Agent Bounties earning integration.

Validates the pieces an agent depends on and, critically, exercises the evidence
guard against every fixture so a regression in its decision logic fails here rather
than silently letting a session end with an unsubmitted claim.

Run: python3 scripts/check-openhands-integration.py
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SKILL = ROOT / ".agents/skills/agent-bounties/SKILL.md"
HOOKS = ROOT / ".openhands/hooks.json"
GUARD = ROOT / ".openhands/hooks/agent-bounties-evidence.py"
FIXTURES = ROOT / "integrations/openhands/fixtures"

failures: list[str] = []


def check(label: str, ok: bool, detail: str = "") -> None:
    print(f"  {'PASS' if ok else 'FAIL'}  {label}")
    if not ok:
        if detail:
            print(f"        {detail}")
        failures.append(label)


def run_guard(state: dict) -> dict:
    """Invoke the stop hook with a state document and parse its decision."""
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
        json.dump(state, handle)
        path = handle.name
    completed = subprocess.run(
        [sys.executable, str(GUARD)],
        cwd=ROOT,
        env={"AGENT_BOUNTIES_STATE": path, "PATH": "/usr/bin:/bin:/usr/local/bin"},
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=30,
        check=False,
    )
    if completed.returncode != 0:
        return {"decision": "error", "raw": completed.stdout[-500:]}
    try:
        return json.loads(completed.stdout)
    except json.JSONDecodeError:
        return {"decision": "unparseable", "raw": completed.stdout[-500:]}


print("=== files present ===")
for path in (SKILL, HOOKS, GUARD):
    check(str(path.relative_to(ROOT)), path.is_file())

print("\n=== skill content ===")
if SKILL.is_file():
    text = SKILL.read_text(encoding="utf-8").lower()
    for phrase in ("canonical", "claimable", "bountysettled", "one exact next action",
                   "post your own bounty"):
        check(f"skill mentions '{phrase}'", phrase in text)

print("\n=== stop hook wiring ===")
if HOOKS.is_file():
    hooks = json.loads(HOOKS.read_text(encoding="utf-8"))
    stop = hooks.get("stop")
    check("stop is a non-empty list", isinstance(stop, list) and bool(stop))
    check("stop hook invokes the evidence guard",
          ".openhands/hooks/agent-bounties-evidence" in json.dumps(stop, sort_keys=True))

print("\n=== fixtures present ===")
for name in ("claimable", "unfunded", "verifier-unready", "submitted-not-paid"):
    check(f"fixtures/{name}.json", (FIXTURES / f"{name}.json").is_file())

print("\n=== evidence guard decisions ===")
# No claim -> allow (must not block unrelated sessions).
check("no active claim -> allow",
      run_guard({"claim": {"active": False}}).get("decision") == "allow")

# Claimed but untested -> deny.
res = run_guard({"claim": {"active": True, "bounty_contract": "0xabc"}})
check("claimed, no test -> deny", res.get("decision") == "deny", res.get("reason", ""))

# Tested but failing -> deny.
res = run_guard({"claim": {"active": True}, "test": {"command": "pytest", "passed": False}})
check("test failed -> deny", res.get("decision") == "deny")

# Passing but evidence incomplete -> deny.
res = run_guard({
    "claim": {"active": True},
    "test": {"command": "python3 check.py", "passed": True},
    "evidence": {"repository": "https://example.com/repo", "commit": "abc123"},
})
check("incomplete evidence -> deny", res.get("decision") == "deny")
check("deny names the missing fields", "test_command" in res.get("reason", ""))

# Complete evidence but not submitted -> deny.
full_evidence = {
    "repository": "https://example.com/repo",
    "commit": "abc123",
    "test_command": "python3 check.py",
    "source_snapshot_digest": "deadbeef",
    "discovery_source": "canonical opportunities feed",
    "participation_reason": "needed this tool",
    "improvement_feedback": "surface claim expiry inline",
}
res = run_guard({
    "claim": {"active": True, "bounty_contract": "0xabc"},
    "test": {"command": "python3 check.py", "passed": True},
    "evidence": full_evidence,
})
check("not submitted on-chain -> deny", res.get("decision") == "deny")
check("deny names submit(bytes32,bytes32)", "submit(bytes32,bytes32)" in res.get("reason", ""))

# Submitted but unsettled -> allow, and must NOT imply payment.
res = run_guard({
    "claim": {"active": True},
    "test": {"command": "python3 check.py", "passed": True},
    "evidence": full_evidence,
    "submission": {"submitted_onchain": True, "bounty_settled": False},
})
check("submitted, unsettled -> allow", res.get("decision") == "allow")
check("unsettled reason says payment NOT confirmed",
      "not confirmed" in res.get("reason", "").lower())
check("unsettled reason requires reporting $0.00",
      "$0.00" in res.get("reason", ""))

# Settled -> allow.
res = run_guard({
    "claim": {"active": True},
    "test": {"command": "python3 check.py", "passed": True},
    "evidence": full_evidence,
    "submission": {"submitted_onchain": True, "bounty_settled": True},
})
check("settled -> allow", res.get("decision") == "allow")

print("\n=== fixture expectations are internally consistent ===")
for name in ("claimable", "unfunded", "verifier-unready", "submitted-not-paid"):
    path = FIXTURES / f"{name}.json"
    if not path.is_file():
        continue
    data = json.loads(path.read_text(encoding="utf-8"))
    check(f"{name} declares expected_action", bool(data.get("expected_action")))
    check(f"{name} declares one exact next_action", bool(str(data.get("next_action", "")).strip()))

if failures:
    print(f"\n{len(failures)} check(s) failed:")
    for item in failures:
        print(f"  - {item}")
    raise SystemExit(1)

print("\nOpenHands integration smoke passed")
