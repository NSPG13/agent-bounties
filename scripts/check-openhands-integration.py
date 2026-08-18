#!/usr/bin/env python3
"""Smoke test for the OpenHands Agent Bounties earning integration.

Validates the things production actually depends on, which the previous version of
this checker did not:

  1. hooks.json matches the OFFICIAL OpenHands schema exactly —
     stop[] -> {matcher, hooks[] -> {command, timeout}} — not a flat entry.
  2. The guard honours the real EXIT CONTRACT: 0 allows, 2 BLOCKS. A guard that
     always exits 0 can never block anything, however good its JSON looks.
  3. stdin is treated as the OpenHands EVENT payload, and claim state comes from an
     authoritative producer.
  4. It FAILS CLOSED when a configured state source is unreadable or malformed.
  5. Forged local settlement state cannot produce paid language.
  6. Unrelated sessions (no state source configured) exit 0.

Run: python -B scripts/check-openhands-integration.py
"""

from __future__ import annotations

import json
import os
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


def run_guard(state=None, event=None, state_mode="file"):
    """Invoke the guard as OpenHands would: event JSON on stdin, state via env.

    Returns (exit_code, parsed_json_or_None, raw_stdout).
    """
    env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "OPENHANDS_SESSION_ID": "sess-test"}
    tmp = None
    # NOTE: state_mode="missing" must be honoured even when state is None — that is
    # precisely the "configured but unreadable" case the guard has to fail closed on.
    if state_mode == "missing":
        env["AGENT_BOUNTIES_STATE"] = "/nonexistent/path/state.json"
    elif state is not None:
        tmp = tempfile.NamedTemporaryFile("w", suffix=".json", delete=False)
        if isinstance(state, str):
            tmp.write(state)              # deliberately malformed payloads
        else:
            json.dump(state, tmp)
        tmp.close()
        env["AGENT_BOUNTIES_STATE"] = tmp.name

    payload = json.dumps(event if event is not None else
                         {"type": "Stop", "session_id": "sess-test"})
    done = subprocess.run(
        [sys.executable, "-B", str(GUARD)],
        cwd=ROOT, env=env, input=payload, text=True,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=40, check=False,
    )
    try:
        parsed = json.loads(done.stdout)
    except json.JSONDecodeError:
        parsed = None
    return done.returncode, parsed, done.stdout


FULL_EVIDENCE = {
    "repository": "https://github.com/example/repo",
    "commit": "0" * 40,
    "test_command": "python -B /benchmark/check.py",
    "source_snapshot_digest": "f" * 64,
    "discovery_source": "canonical opportunities feed",
    "participation_reason": "needed this tool",
    "improvement_feedback": "surface claim expiry inline",
}
CLAIMED = {"active": True, "bounty_contract": "0x" + "ab" * 20}
PASSED = {"command": "python -B /benchmark/check.py", "passed": True}

print("=== files present ===")
for path in (SKILL, HOOKS, GUARD):
    check(str(path.relative_to(ROOT)), path.is_file())

print("\n=== skill content ===")
if SKILL.is_file():
    raw = SKILL.read_text(encoding="utf-8")
    low = raw.lower()
    for phrase in ("canonical", "claimable", "bountysettled", "one exact next action",
                   "post your own bounty"):
        check(f"skill mentions '{phrase}'", phrase in low)
    # AgentSkills frontmatter is required by the SDK.
    check("skill has YAML frontmatter", raw.startswith("---"))
    check("frontmatter declares name", "\nname:" in raw.split("---")[1] if raw.startswith("---") else False)
    check("frontmatter declares description",
          "description:" in raw.split("---")[1] if raw.startswith("---") else False)

print("\n=== hooks.json matches the OFFICIAL schema ===")
if HOOKS.is_file():
    hooks = json.loads(HOOKS.read_text(encoding="utf-8"))
    stop = hooks.get("stop")
    check("stop is a non-empty list", isinstance(stop, list) and bool(stop))
    if isinstance(stop, list) and stop:
        entry = stop[0]
        check("stop entry has 'matcher'", isinstance(entry, dict) and "matcher" in entry,
              f"got keys {list(entry) if isinstance(entry, dict) else type(entry)}")
        inner = entry.get("hooks") if isinstance(entry, dict) else None
        check("stop entry nests a 'hooks' list", isinstance(inner, list) and bool(inner),
              "flat name/command entries are NOT the OpenHands schema")
        if isinstance(inner, list) and inner:
            h = inner[0]
            check("nested hook has 'command'", isinstance(h, dict) and "command" in h)
            check("nested hook has 'timeout'", isinstance(h, dict) and "timeout" in h)
            check("command points at the evidence guard",
                  ".openhands/hooks/agent-bounties-evidence" in str(h.get("command", "")))

print("\n=== EXIT CONTRACT: 0 allows, 2 blocks ===")
# Unrelated session: no state source configured at all -> must allow.
code, out, _ = run_guard(state=None)
check("no state source configured -> exit 0", code == 0, f"exit={code}")
check("  and decision is allow", (out or {}).get("decision") == "allow")

# Active claim, nothing done -> must BLOCK with exit 2.
code, out, _ = run_guard({"claim": CLAIMED})
check("active claim, no test -> exit 2 (BLOCK)", code == 2, f"exit={code}")
check("  and decision is deny", (out or {}).get("decision") == "deny")

# Test recorded but failing -> block.
code, out, _ = run_guard({"claim": CLAIMED, "test": {"command": "pytest", "passed": False}})
check("failing test -> exit 2", code == 2, f"exit={code}")

# Evidence incomplete -> block, and the reason must name what is missing.
code, out, _ = run_guard({"claim": CLAIMED, "test": PASSED,
                          "evidence": {"repository": "x", "commit": "y"}})
check("incomplete evidence -> exit 2", code == 2, f"exit={code}")
check("  reason names a missing field", "test_command" in (out or {}).get("reason", ""))

# Not submitted -> block, naming the exact call.
code, out, _ = run_guard({"claim": CLAIMED, "test": PASSED, "evidence": FULL_EVIDENCE})
check("not submitted on-chain -> exit 2", code == 2, f"exit={code}")
check("  reason names submit(bytes32,bytes32)",
      "submit(bytes32,bytes32)" in (out or {}).get("reason", ""))

# Submitted, unsettled -> allow, but must NOT claim payment.
code, out, _ = run_guard({"claim": CLAIMED, "test": PASSED, "evidence": FULL_EVIDENCE,
                          "submission": {"submitted_onchain": True}})
reason = (out or {}).get("reason", "")
check("submitted, unsettled -> exit 0", code == 0, f"exit={code}")
check("  reason says payment NOT confirmed", "not confirmed" in reason.lower())
check("  reason requires reporting $0.00", "$0.00" in reason)

print("\n=== FAIL CLOSED on unreadable configured state ===")
code, out, _ = run_guard(state=None, state_mode="missing")
check("configured state file missing -> exit 2", code == 2, f"exit={code}")
code, out, _ = run_guard("{not valid json", state_mode="file")
check("malformed state file -> exit 2", code == 2, f"exit={code}")
code, out, _ = run_guard({"claim": {"bounty_contract": "0xabc"}})
check("claim.active absent -> exit 2", code == 2, f"exit={code}")
code, out, _ = run_guard({})
check("no 'claim' section -> exit 2", code == 2, f"exit={code}")

print("\n=== FORGED settlement cannot assert payment ===")
forged = {"claim": CLAIMED, "test": PASSED, "evidence": FULL_EVIDENCE,
          "submission": {"submitted_onchain": True, "bounty_settled": True},
          "paid": True}
code, out, _ = run_guard(forged)
reason = (out or {}).get("reason", "")
check("forged bounty_settled -> still exit 0", code == 0, f"exit={code}")
check("  but reason does NOT claim paid", "is paid" not in reason.lower())
check("  and reason still requires $0.00", "$0.00" in reason)
check("  and reason flags the unverified assertion",
      "cannot prove payment" in reason.lower() or "ignored" in reason.lower())

print("\n=== canonical receipt DOES allow paid language ===")
settled = {"claim": CLAIMED, "test": PASSED, "evidence": FULL_EVIDENCE,
           "submission": {"submitted_onchain": True},
           "settlement": {"canonical_event": {"kind": "BountySettled",
                                              "tx_hash": "0x" + "cd" * 32,
                                              "log_key": "0xabc:12"}}}
code, out, _ = run_guard(settled)
check("canonical receipt -> exit 0", code == 0, f"exit={code}")
check("  and reason says paid", "paid" in (out or {}).get("reason", "").lower())

print("\n=== event payload handling ===")
code, out, _ = run_guard(state=None, event={"type": "Stop", "session_id": "other-session"})
check("unrelated session exits 0", code == 0, f"exit={code}")
code, out, raw = run_guard(state=None, event="not-json-at-all")
check("non-JSON stdin does not crash the guard", code in (0, 2), f"exit={code}")

print("\n=== fixtures present ===")
for name in ("claimable", "unfunded", "verifier-unready", "submitted-not-paid"):
    check(f"fixtures/{name}.json", (FIXTURES / f"{name}.json").is_file())

if failures:
    print(f"\n{len(failures)} check(s) failed:")
    for item in failures:
        print(f"  - {item}")
    raise SystemExit(1)

print("\nOpenHands integration smoke passed")
