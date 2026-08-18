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
import shlex
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

    # The exact stdin payload shape documented at
    # https://docs.openhands.dev/openhands/usage/customization/hooks
    payload = json.dumps(event if event is not None else
                         {"event_type": "Stop", "session_id": "sess-test",
                          "working_dir": str(ROOT)})
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

print("\n=== END TO END: a real OpenHands Stop event invokes the registered hook ===")
# Do NOT re-implement the invocation. Read the command straight out of hooks.json
# and run it the way OpenHands does: as a shell command from working_dir, with the
# documented OPENHANDS_* environment variables and the event payload on stdin.
# No `python -B <script>` prefix, so this also proves the shebang and the +x bit.
registered_cmd = None
if HOOKS.is_file():
    try:
        registered_cmd = json.loads(HOOKS.read_text(encoding="utf-8"))["stop"][0]["hooks"][0]["command"]
    except (KeyError, IndexError, TypeError, json.JSONDecodeError):
        registered_cmd = None
check("hooks.json yields a runnable stop command", bool(registered_cmd), f"got {registered_cmd!r}")

check("guard is executable (chmod +x, as the docs require)",
      os.access(GUARD, os.X_OK), f"mode={oct(GUARD.stat().st_mode & 0o777)}")

# The REAL state producer shipped in this repo -- not a stub reimplemented here,
# so a regression in the producer fails this gate. It is handed the session id
# and answers only for the session that actually holds a claim.
producer_dir = Path(tempfile.mkdtemp())
sessions = producer_dir / "sessions"
sessions.mkdir()
events_snapshot = producer_dir / "events.json"
events_snapshot.write_text(
    json.dumps([{"kind": "BountyClaimed", "solver": "0x" + "11" * 20, "bounty_id": "b-1"}]),
    encoding="utf-8",
)
(sessions / "sess-bounty.json").write_text(json.dumps({
    "bounty_id": "b-1",
    "solver": "0x" + "11" * 20,
    "bounty_contract": "0x" + "ab" * 20,
}), encoding="utf-8")
(sessions / "sess-unrelated.json").write_text(json.dumps({}), encoding="utf-8")

PRODUCER = ROOT / "integrations/openhands/state_producer.py"
check("state producer is present", PRODUCER.is_file(), str(PRODUCER))
PRODUCER_CMD = f"{shlex.quote(sys.executable)} -B {shlex.quote(str(PRODUCER))}"


def run_registered(session, extra_env=None):
    """Invoke the hook exactly as OpenHands would."""
    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "OPENHANDS_EVENT_TYPE": "Stop",
        "OPENHANDS_PROJECT_DIR": str(ROOT),
        "OPENHANDS_SESSION_ID": session,
        "AGENT_BOUNTIES_STATE_CMD": PRODUCER_CMD,
        "AGENT_BOUNTIES_SESSION_DIR": str(sessions),
        # Explicit offline canonical snapshot: this gate must not depend on
        # network egress, and there is deliberately no silent offline fallback.
        "AGENT_BOUNTIES_EVENTS_FILE": str(events_snapshot),
    }
    env.update(extra_env or {})
    done = subprocess.run(
        registered_cmd, shell=True, cwd=ROOT, env=env,
        input=json.dumps({"event_type": "Stop", "tool_name": None,
                          "session_id": session, "working_dir": str(ROOT)}),
        text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        timeout=60, check=False,
    )
    try:
        return done.returncode, json.loads(done.stdout), done.stderr
    except json.JSONDecodeError:
        return done.returncode, {}, done.stderr


if registered_cmd:
    rc, body, err = run_registered("sess-bounty")
    check("real Stop event on an incomplete active claim -> exit 2 (BLOCK)",
          rc == 2, f"exit={rc} stderr={err[-300:]}")
    check("  and the deny decision is on stdout", body.get("decision") == "deny", f"body={body}")

    rc, body, err = run_registered("sess-unrelated")
    check("real Stop event on an unrelated session -> exit 0",
          rc == 0, f"exit={rc} stderr={err[-300:]}")
    check("  and the decision is allow", body.get("decision") == "allow", f"body={body}")

    # An unknown session has no workfile: the producer cannot establish state, so
    # the stop must block rather than assume there is no claim.
    rc, body, err = run_registered("sess-never-seen")
    check("session with no workfile -> exit 2 (fail closed, not assumed idle)",
          rc == 2, f"exit={rc}")

    # A state producer that fails must never let the session end.
    rc, body, err = run_registered(
        "sess-bounty",
        {"AGENT_BOUNTIES_STATE_CMD": f"{shlex.quote(sys.executable)} -c \"import sys; sys.exit(3)\""},
    )
    check("state producer exiting non-zero -> exit 2 (fail closed)", rc == 2, f"exit={rc}")
    rc, body, err = run_registered(
        "sess-bounty",
        {"AGENT_BOUNTIES_STATE_CMD": f"{shlex.quote(sys.executable)} -c \"print('not json')\""},
    )
    check("state producer emitting non-JSON -> exit 2 (fail closed)", rc == 2, f"exit={rc}")

    # Unreachable canonical feed must block, not degrade to a local-only answer.
    rc, body, err = run_registered(
        "sess-bounty",
        {"AGENT_BOUNTIES_EVENTS_FILE": str(producer_dir / "does-not-exist.json")},
    )
    check("canonical feed unusable -> exit 2 (no local-only fallback)", rc == 2, f"exit={rc}")


print("\n=== TAMPERING: the workfile cannot manufacture payment ===")


def run_producer(session, work, events, env_extra=None):
    """Run the shipped producer directly and return (exit_code, state, stderr)."""
    tmpdir = Path(tempfile.mkdtemp())
    sess_dir = tmpdir / "sessions"
    sess_dir.mkdir()
    (sess_dir / f"{session}.json").write_text(json.dumps(work), encoding="utf-8")
    ev = tmpdir / "events.json"
    ev.write_text(json.dumps(events), encoding="utf-8")
    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "AGENT_BOUNTIES_SESSION_DIR": str(sess_dir),
        "AGENT_BOUNTIES_EVENTS_FILE": str(ev),
    }
    env.update(env_extra or {})
    done = subprocess.run(
        [sys.executable, "-B", str(PRODUCER), session],
        env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        timeout=60, check=False,
    )
    try:
        return done.returncode, json.loads(done.stdout), done.stderr
    except json.JSONDecodeError:
        return done.returncode, {}, done.stderr


SOLVER = "0x" + "11" * 20
COMPLETE_WORK = {
    "bounty_id": "b-1", "solver": SOLVER, "bounty_contract": "0x" + "ab" * 20,
    "test": PASSED, "evidence": FULL_EVIDENCE,
    "submission": {"submitted_onchain": True},
}
CLAIM_EVENT = {"kind": "BountyClaimed", "solver": SOLVER, "bounty_id": "b-1"}

# A workfile screaming that it was paid, with a canonical feed that says otherwise.
forged_work = dict(COMPLETE_WORK)
forged_work["submission"] = {"submitted_onchain": True, "bounty_settled": True}
forged_work["paid"] = True
forged_work["settlement"] = {"canonical_event": {"kind": "BountySettled", "tx_hash": "0xdead"}}
rc, state, err = run_producer("s", forged_work, [CLAIM_EVENT])
check("forged workfile settlement is stripped by the producer", rc == 0 and "settlement" not in state,
      f"exit={rc} state={state}")
check("  forged submission.bounty_settled is stripped",
      "bounty_settled" not in (state.get("submission") or {}), f"submission={state.get('submission')}")
check("  forged top-level paid is stripped", "paid" not in state, f"state keys={list(state)}")

# Genuine canonical settlement DOES produce a receipt, with real event identity.
settle_event = {"kind": "BountySettled", "solver": SOLVER, "bounty_id": "b-1",
                "tx_hash": "0x" + "cd" * 32, "log_key": "0xabc:12"}
rc, state, err = run_producer("s", COMPLETE_WORK, [CLAIM_EVENT, settle_event])
receipt = ((state.get("settlement") or {}).get("canonical_event") or {})
check("canonical BountySettled event yields a receipt", rc == 0 and receipt.get("tx_hash") == settle_event["tx_hash"],
      f"exit={rc} receipt={receipt}")
check("  and the claim is no longer active", (state.get("claim") or {}).get("active") is False,
      f"claim={state.get('claim')}")

# A settlement belonging to a DIFFERENT solver must not pay this agent.
other = {"kind": "BountySettled", "solver": "0x" + "22" * 20, "bounty_id": "b-1",
         "tx_hash": "0x" + "ee" * 32}
rc, state, err = run_producer("s", COMPLETE_WORK, [CLAIM_EVENT, other])
check("another solver's settlement is not our receipt", rc == 0 and "settlement" not in state,
      f"exit={rc} state={state}")

# Claim occupancy follows canonical events, not the workfile.
rc, state, err = run_producer("s", COMPLETE_WORK, [CLAIM_EVENT])
check("canonical claim -> claim.active true", (state.get("claim") or {}).get("active") is True)
rc, state, err = run_producer(
    "s", COMPLETE_WORK, [CLAIM_EVENT, {"kind": "ClaimExpired", "solver": SOLVER, "bounty_id": "b-1"}])
check("canonical claim expiry -> claim.active false", (state.get("claim") or {}).get("active") is False)

# Structural failures are unresolvable, never a cheerful default.
rc, state, err = run_producer("s", COMPLETE_WORK, {"events": "not-a-list"})
check("non-list event payload -> producer exits non-zero", rc != 0, f"exit={rc}")
check("  and prints nothing on stdout", state == {}, f"state={state}")
rc, state, err = run_producer("s", COMPLETE_WORK,
                              [{"kind": "BountySettled", "solver": SOLVER}])
check("settlement with no event identity -> producer exits non-zero", rc != 0, f"exit={rc}")

# Session ids are untrusted input: no path traversal out of the session dir.
rc, state, err = run_producer("s", COMPLETE_WORK, [CLAIM_EVENT],
                              {"AGENT_BOUNTIES_SESSION_DIR": str(Path(tempfile.mkdtemp()))})
check("missing workfile -> producer exits non-zero", rc != 0, f"exit={rc}")
traversal = subprocess.run(
    [sys.executable, "-B", str(PRODUCER), "../../etc/passwd"],
    env={"PATH": os.environ.get("PATH", "/usr/bin:/bin"),
         "AGENT_BOUNTIES_SESSION_DIR": str(sessions)},
    text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30, check=False,
)
check("path-traversal session id is refused", traversal.returncode != 0,
      f"exit={traversal.returncode}")

print("\n=== event payload handling ===")
code, out, _ = run_guard(state=None, event={"event_type": "Stop", "session_id": "other-session"})
check("unrelated session exits 0", code == 0, f"exit={code}")
code, out, raw = run_guard(state=None, event="not-json-at-all")
check("non-JSON stdin does not crash the guard", code in (0, 2), f"exit={code}")

print("\n=== an UNHANDLED CRASH must block, never fail open ===")
# Per the official docs, exit 0 allows and exit 2 blocks, but ANY OTHER exit code
# is "Error. The operation proceeds, but the error is logged." An uncaught Python
# exception exits 1, so a crashing guard would silently let a session with a live
# bond end -- the exact failure this guard exists to prevent.
#
# This must be a crash the ordinary handling in load_claim_state does NOT catch,
# otherwise the test proves nothing. A directory path, for instance, raises
# IsADirectoryError, which IS an OSError and is already handled above. So use
# deeply nested JSON: json.load raises RecursionError, which is not a
# JSONDecodeError and not an OSError, and therefore escapes to the top level.
crash_state = Path(tempfile.mkdtemp()) / "recursion-bomb.json"
crash_state.write_text("[" * 200_000 + "]" * 200_000, encoding="utf-8")
crash_env = {
    "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
    "OPENHANDS_SESSION_ID": "sess-crash",
    "AGENT_BOUNTIES_STATE": str(crash_state),
}
crashed = subprocess.run(
    [sys.executable, "-B", str(GUARD)],
    cwd=ROOT, env=crash_env,
    input=json.dumps({"event_type": "Stop", "session_id": "sess-crash"}),
    text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    timeout=60, check=False,
)
# Prove the test actually reached the crash path instead of a handled branch.
check(
    "the bomb really does raise an unhandled exception",
    "RecursionError" in crashed.stderr,
    f"stderr tail={crashed.stderr[-200:]!r}",
)
check(
    "  crash never exits 1 (exit 1 would ALLOW the stop)",
    crashed.returncode != 1,
    f"exit={crashed.returncode}",
)
check(
    "  crash blocks with exit 2",
    crashed.returncode == 2,
    f"exit={crashed.returncode}",
)
try:
    crash_out = json.loads(crashed.stdout)
except (json.JSONDecodeError, ValueError):
    crash_out = {}
check("  crash emits a deny decision", crash_out.get("decision") == "deny",
      f"stdout={crashed.stdout[:200]}")
check("  crash reason still reports $0.00",
      "$0.00" in crash_out.get("reason", ""), f"reason={crash_out.get('reason')!r}")

# The guard must also never define a non-blocking 'error' exit code, because per
# the docs any code other than 0/2 lets the operation proceed anyway.
guard_src = GUARD.read_text(encoding="utf-8")
check(
    "guard defines no fail-open ERROR exit code",
    "ERROR = 1" not in guard_src and "ERROR=1" not in guard_src,
)

print("\n=== fixtures present ===")
for name in ("claimable", "unfunded", "verifier-unready", "submitted-not-paid"):
    check(f"fixtures/{name}.json", (FIXTURES / f"{name}.json").is_file())

if failures:
    print(f"\n{len(failures)} check(s) failed:")
    for item in failures:
        print(f"  - {item}")
    raise SystemExit(1)

print("\nOpenHands integration smoke passed")
