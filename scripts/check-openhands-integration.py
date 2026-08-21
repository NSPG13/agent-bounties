#!/usr/bin/env python3
"""Smoke test for the OpenHands Agent Bounties earning integration.

Validates the things production actually depends on:

  1. hooks.json matches the OFFICIAL OpenHands schema exactly --
     stop[] -> {matcher, hooks[] -> {command, timeout}} -- and registers the guard
     through an EXPLICIT PYTHON INTERPRETER, so it runs on Windows and Linux alike
     rather than depending on the +x bit and a shebang.
  2. The guard honours the real EXIT CONTRACT: 0 allows, 2 BLOCKS.
  3. stdin is treated as the OpenHands EVENT payload, and claim state comes from an
     authoritative producer.
  4. Claim IDENTITY comes from an operator-owned binding, NOT from the
     session-writable workfile: deleting or editing the workfile cannot disable
     the guard.
  5. Payment evidence must be LIVE-CANONICAL and BOUND to network, bounty,
     contract, round and solver. Offline snapshots are test-only and can never
     settle; forged or cross-bounty evidence cannot produce paid language.
  6. Unrelated sessions exit 0.

PORTABILITY NOTE. Everything here runs through `sys.executable` and `pathlib`,
with no shell, no `chmod +x` dependency, and no POSIX-only path assumptions, so
the suite behaves identically on Windows and Linux. The one POSIX-only assertion
(binding file permission bits) is explicitly skipped on Windows, which has no
equivalent mode bits.

Run: python -B scripts/check-openhands-integration.py
"""

from __future__ import annotations

import base64
import json
import os
import shlex
import shutil
import socket
import stat
import subprocess
import sys
import tempfile
import threading
from datetime import datetime, timedelta, timezone
from concurrent.futures import ThreadPoolExecutor
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SKILL = ROOT / ".agents/skills/agent-bounties/SKILL.md"
HOOKS = ROOT / ".openhands/hooks.json"
GUARD = ROOT / ".openhands/hooks/agent-bounties-evidence.py"
PRODUCER = ROOT / "integrations/openhands/state_producer.py"
FIXTURES = ROOT / "integrations/openhands/fixtures"

IS_WINDOWS = os.name == "nt"
INTERPRETERS = {"python", "python3", "py", "python.exe", "python3.exe", "py.exe"}

failures: list[str] = []
TEMPDIRS: list[str] = []


def check(label: str, ok: bool, detail: str = "") -> None:
    print(f"  {'PASS' if ok else 'FAIL'}  {label}")
    if not ok:
        if detail:
            print(f"        {detail}")
        failures.append(label)


def tmpdir() -> Path:
    path = tempfile.mkdtemp(prefix="ab-openhands-")
    TEMPDIRS.append(path)
    return Path(path)


def in_parallel(jobs):
    """Run independent subprocess cases concurrently, reporting in stable order.

    Each case spawns the guard (and often the producer), and process creation on
    Windows is several times more expensive than on Linux. Running the mutually
    independent cases concurrently keeps the suite comfortably inside the
    immutable benchmark's subprocess timeout on both platforms. Order-dependent
    groups -- anything sharing the live HTTP fixture or mutating the workfile --
    are deliberately NOT batched.
    """
    with ThreadPoolExecutor(max_workers=min(8, (os.cpu_count() or 2) * 2)) as pool:
        return list(pool.map(lambda job: job(), jobs))


NETWORK = "base-mainnet"
BOUNTY_ID = "0x" + "7a" * 32
CONTRACT = "0x" + "ab" * 20
SOLVER = "0x" + "11" * 20
OTHER_SOLVER = "0x" + "22" * 20
ROUND = 2

FULL_EVIDENCE = {
    "repository": "https://github.com/example/repo",
    "commit": "0" * 40,
    "test_command": "python -B /benchmark/check.py",
    "source_snapshot_digest": "f" * 64,
    "discovery_source": "canonical opportunities feed",
    "participation_reason": "needed this tool",
    "improvement_feedback": "surface claim expiry inline",
}
PASSED = {"command": "python -B /benchmark/check.py", "passed": True}
# The claim shape the producer emits, reused by the direct-guard cases so that
# receipt binding is exercised against a realistic claim.
CLAIMED = {
    "active": True,
    "network": NETWORK,
    "bounty_id": BOUNTY_ID,
    "bounty_contract": CONTRACT,
    "round": ROUND,
    "solver": SOLVER,
}


def now_iso(delta_minutes: int = -10) -> str:
    return (datetime.now(timezone.utc) + timedelta(minutes=delta_minutes)).isoformat()


def event(kind: str, *, bounty_id: str = BOUNTY_ID, contract: str = CONTRACT,
          rnd: int = ROUND, solver: str = SOLVER, block: int = 12_345_678,
          tx: str | None = None, log_key: str = "12345678:3",
          occurred_at: str | None = None, extra: dict | None = None) -> dict:
    """Build one canonical event in the exact shape crates/chain-base emits."""
    data = {"round": rnd, "solver": solver}
    data.update(extra or {})
    return {
        "id": "3f8f0d1a-0000-4000-8000-000000000001",
        "log_key": log_key,
        "tx_hash": tx or ("0x" + "cd" * 32),
        "block_number": block,
        "log_index": 3,
        "contract_address": contract,
        "bounty_id": bounty_id,
        "kind": kind,
        "data": data,
        "occurred_at": occurred_at or now_iso(),
    }


CLAIM_EVENT = event("bounty_claimed", extra={"bond": 100000})
SETTLE_EVENT = event("bounty_settled", extra={"solver_payout": 1_100_000})


# ---------------------------------------------------------------------------

print("=== files present ===")
for path in (SKILL, HOOKS, GUARD, PRODUCER):
    check(str(path.relative_to(ROOT)).replace("\\", "/"), path.is_file())

print("\n=== skill content ===")
if SKILL.is_file():
    raw = SKILL.read_text(encoding="utf-8")
    low = raw.lower()
    for phrase in ("canonical", "claimable", "bountysettled", "one exact next action",
                   "post your own bounty"):
        check(f"skill mentions '{phrase}'", phrase in low)
    # AgentSkills frontmatter is required by the SDK (docs.openhands.dev/sdk/guides/skill).
    check("skill has YAML frontmatter", raw.startswith("---"))
    front = raw.split("---")[1] if raw.startswith("---") else ""
    check("frontmatter declares name", "\nname:" in front)
    check("frontmatter declares description", "description:" in front)

print("\n=== hooks.json matches the OFFICIAL schema ===")
registered_cmd = None
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
            registered_cmd = str(h.get("command", "")) if isinstance(h, dict) else ""
            check("command points at the evidence guard",
                  ".openhands/hooks/agent-bounties-evidence" in (registered_cmd or ""))
            # A Stop hook must never be async: async hooks run in the background
            # and can never block, per the docs.
            check("stop hook is NOT async (async hooks can never block)",
                  isinstance(h, dict) and h.get("async") is not True, f"got {h}")

print("\n=== the hook is invoked CROSS-PLATFORM through an explicit interpreter ===")
cmd_tokens = shlex.split(registered_cmd) if registered_cmd else []
check("registered command parses into tokens", len(cmd_tokens) >= 2, f"got {cmd_tokens}")
interpreter_token = Path(cmd_tokens[0]).name.lower() if cmd_tokens else ""
check("first token is an explicit Python interpreter, not the bare script path",
      interpreter_token in INTERPRETERS,
      f"got {interpreter_token!r}; a bare './script.py' relies on the +x bit and a "
      f"shebang, neither of which exists on Windows")
check("the script is passed as an argument to that interpreter",
      len(cmd_tokens) >= 2 and cmd_tokens[1].endswith("agent-bounties-evidence.py"),
      f"got {cmd_tokens[1:] if len(cmd_tokens) > 1 else []}")
check("the script path uses forward slashes (portable in JSON and on both OSes)",
      "\\" not in (cmd_tokens[1] if len(cmd_tokens) > 1 else ""), registered_cmd or "")
check("the registered script resolves from the repository root",
      (ROOT / cmd_tokens[1]).is_file() if len(cmd_tokens) > 1 else False)
# Informational: python3 must exist wherever OpenHands runs the hook (its sandbox
# is Linux). This suite itself never depends on it -- see run_registered below.
print(f"        note: 'python3' on this host -> {shutil.which('python3') or 'not on PATH'}")


def hook_argv() -> list[str]:
    """The registered command, run portably.

    The interpreter token from hooks.json is replaced with THIS interpreter so the
    suite passes on a Windows box where `python3` may not be on PATH, while every
    other token -- crucially the script path -- comes straight out of hooks.json.
    Nothing here relies on a shell, a shebang, or the executable bit.
    """
    return [sys.executable, "-B", str(ROOT / cmd_tokens[1]), *cmd_tokens[2:]]


# ---------------------------------------------------------------------------
# Direct guard cases: state supplied as a file, so decision logic is isolated.
# ---------------------------------------------------------------------------

def run_guard(state=None, event_payload=None, state_mode="file"):
    """Invoke the guard as OpenHands would: event JSON on stdin, state via env."""
    env = {"PATH": os.environ.get("PATH", ""), "OPENHANDS_SESSION_ID": "sess-test",
           "SYSTEMROOT": os.environ.get("SYSTEMROOT", "")}
    if state_mode == "missing":
        env["AGENT_BOUNTIES_STATE"] = str(tmpdir() / "definitely-not-here.json")
    elif state is not None:
        path = tmpdir() / "state.json"
        path.write_text(state if isinstance(state, str) else json.dumps(state), encoding="utf-8")
        env["AGENT_BOUNTIES_STATE"] = str(path)

    payload = json.dumps(event_payload) if isinstance(event_payload, (dict, list)) else (
        event_payload if isinstance(event_payload, str) else
        json.dumps({"event_type": "Stop", "tool_name": None,
                    "session_id": "sess-test", "working_dir": str(ROOT)}))
    done = subprocess.run(
        hook_argv(), cwd=ROOT, env=env, input=payload, text=True,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60, check=False,
    )
    try:
        parsed = json.loads(done.stdout)
    except json.JSONDecodeError:
        parsed = None
    return done.returncode, parsed, done.stderr


print("\n=== EXIT CONTRACT: 0 allows, 2 blocks ===")
code, out, _ = run_guard(state=None)
check("no state source configured -> exit 0", code == 0, f"exit={code}")
check("  and decision is allow", (out or {}).get("decision") == "allow")

code, out, _ = run_guard({"claim": CLAIMED})
check("active claim, no test -> exit 2 (BLOCK)", code == 2, f"exit={code}")
check("  and decision is deny", (out or {}).get("decision") == "deny")

code, out, _ = run_guard({"claim": CLAIMED, "test": {"command": "pytest", "passed": False}})
check("failing test -> exit 2", code == 2, f"exit={code}")

code, out, _ = run_guard({"claim": CLAIMED, "test": PASSED,
                          "evidence": {"repository": "x", "commit": "y"}})
check("incomplete evidence -> exit 2", code == 2, f"exit={code}")
check("  reason names a missing field", "test_command" in (out or {}).get("reason", ""))

code, out, _ = run_guard({"claim": CLAIMED, "test": PASSED, "evidence": FULL_EVIDENCE})
check("not submitted on-chain -> exit 2", code == 2, f"exit={code}")
check("  reason names submit(bytes32,bytes32)",
      "submit(bytes32,bytes32)" in (out or {}).get("reason", ""))

SUBMITTED = {"claim": CLAIMED, "test": PASSED, "evidence": FULL_EVIDENCE,
             "submission": {"submitted_onchain": True}}
code, out, _ = run_guard(SUBMITTED)
reason = (out or {}).get("reason", "")
check("submitted, unsettled -> exit 0", code == 0, f"exit={code}")
check("  reason says payment NOT confirmed", "not confirmed" in reason.lower())
check("  reason requires reporting $0.00", "$0.00" in reason)

print("\n=== FAIL CLOSED on unreadable configured state ===")
code, _, _ = run_guard(state=None, state_mode="missing")
check("configured state file missing -> exit 2", code == 2, f"exit={code}")
code, _, _ = run_guard("{not valid json", state_mode="file")
check("malformed state file -> exit 2", code == 2, f"exit={code}")
code, _, _ = run_guard({"claim": {"bounty_contract": CONTRACT}})
check("claim.active absent -> exit 2", code == 2, f"exit={code}")
code, _, _ = run_guard({})
check("no 'claim' section -> exit 2", code == 2, f"exit={code}")

print("\n=== a receipt only pays when it is LIVE-CANONICAL and BOUND to this claim ===")


def good_receipt(**over) -> dict:
    receipt = {
        "kind": "BountySettled",
        "provenance": "canonical_live",
        "network": NETWORK,
        "bounty_id": BOUNTY_ID,
        "bounty_contract": CONTRACT,
        "round": ROUND,
        "solver": SOLVER,
        "tx_hash": "0x" + "cd" * 32,
        "log_key": "12345678:3",
        "block_number": 12_345_678,
        "occurred_at": now_iso(),
    }
    receipt.update(over)
    return receipt


def with_receipt(receipt) -> dict:
    state = dict(SUBMITTED)
    state["settlement"] = {"canonical_event": receipt}
    return state


code, out, _ = run_guard(with_receipt(good_receipt()))
reason = (out or {}).get("reason", "")
check("bound live-canonical receipt -> exit 0", code == 0, f"exit={code}")
check("  and reason says paid", "paid" in reason.lower(), reason)
check("  and reason cites the tx hash", "0x" + "cd" * 32 in reason, reason)

REJECTIONS = {
    "offline test snapshot cannot pay": good_receipt(provenance="test_snapshot"),
    "receipt with no provenance cannot pay": good_receipt(provenance=""),
    "receipt for another bounty cannot pay": good_receipt(bounty_id="0x" + "99" * 32),
    "receipt for another round cannot pay": good_receipt(round=ROUND + 1),
    "receipt for another solver cannot pay": good_receipt(solver=OTHER_SOLVER),
    "receipt on another contract cannot pay": good_receipt(bounty_contract="0x" + "99" * 20),
    "receipt on a non-canonical network cannot pay": good_receipt(network="base-sepolia"),
    "receipt with no tx_hash cannot pay": good_receipt(tx_hash=""),
    "receipt with no log_key cannot pay": good_receipt(log_key=""),
    "receipt with block_number 0 cannot pay": good_receipt(block_number=0),
    "receipt with a non-integer block_number cannot pay": good_receipt(block_number="12345678"),
    "receipt of the wrong kind cannot pay": good_receipt(kind="SubmissionAdded"),
}
# Each rejection case is independent: same input shape, one receipt field poisoned.
for label, (code, out, _) in zip(
        REJECTIONS,
        in_parallel([(lambda r=r: run_guard(with_receipt(r))) for r in REJECTIONS.values()])):
    reason = (out or {}).get("reason", "")
    ok = code == 0 and "$0.00" in reason and "not confirmed" in reason.lower() \
        and "work is paid" not in reason.lower()
    check(label, ok, f"exit={code} reason={reason[:220]}")

# ISOLATING CASE for the canonical-network rule. In the table above the receipt's
# network disagrees with the claim's, so the claim-binding loop rejects it too and
# the case would still pass with CANONICAL_NETWORKS deleted. Here claim and
# receipt AGREE on a non-canonical network -- exactly what a producer pointed at a
# testnet would emit -- so only the canonical-network rule can reject it.
# Settlement on a chain without the canonical factory and its immutable USDC is
# not payment, however internally consistent it looks.
testnet_state = dict(SUBMITTED)
testnet_state["claim"] = dict(CLAIMED, network="base-sepolia")
testnet_state["settlement"] = {"canonical_event": good_receipt(network="base-sepolia")}
code, out, _ = run_guard(testnet_state)
reason = (out or {}).get("reason", "")
check("a self-consistent TESTNET settlement cannot pay (isolates the canonical-network rule)",
      code == 0 and "$0.00" in reason and "work is paid" not in reason.lower(),
      f"exit={code} reason={reason[:250]}")

print("\n=== FORGED local settlement cannot assert payment ===")
forged = dict(SUBMITTED)
forged["submission"] = {"submitted_onchain": True, "bounty_settled": True}
forged["paid"] = True
code, out, _ = run_guard(forged)
reason = (out or {}).get("reason", "")
check("forged bounty_settled -> still exit 0", code == 0, f"exit={code}")
check("  but reason does NOT claim paid", "work is paid" not in reason.lower())
check("  and reason still requires $0.00", "$0.00" in reason)
check("  and reason flags the rejected assertion",
      "rejected" in reason.lower() or "cannot prove payment" in reason.lower(), reason)

# ---------------------------------------------------------------------------
# End to end: real Stop event -> registered command -> shipped producer.
# ---------------------------------------------------------------------------
print("\n=== END TO END: a real Stop event drives the shipped state producer ===")

# ---------------------------------------------------------------------------
# Operator signing keys for the test bindings.
#
# The private half is generated here, in the CHECKER process, and never written
# to a path the simulated session can read -- mirroring the real deployment,
# where the operator signs outside the sandbox and only the public key is
# pinned into the session environment.
# ---------------------------------------------------------------------------
sys.path.insert(0, str(ROOT / "integrations/openhands"))
from ed25519_verify import canonical_payload  # noqa: E402
from ed25519_verify import verify as ed25519_verify  # noqa: E402
from ed25519_verify import _verify_pure, _verify_pycryptodome  # noqa: E402


class Operator:
    """An Ed25519 signing identity for the fixtures."""

    def __init__(self):
        from Crypto.PublicKey import ECC
        self._ECC = ECC
        self._key = ECC.generate(curve="Ed25519")
        self.public = self._key.public_key().export_key(format="raw")
        self.public_b64 = base64.b64encode(self.public).decode()

    def sign(self, payload: bytes) -> bytes:
        from Crypto.Signature import eddsa
        return eddsa.new(self._key, "rfc8032").sign(payload)

    def envelope(self, body: dict) -> dict:
        return {
            "alg": "ed25519",
            "public_key_b64": self.public_b64,
            "signature_b64": base64.b64encode(
                self.sign(canonical_payload(body))).decode(),
        }


try:
    OPERATOR = Operator()          # the pinned, legitimate operator
    ROGUE = Operator()             # a key the SESSION could generate for itself
    SIGNING_AVAILABLE = True
except ImportError:
    OPERATOR = ROGUE = None
    SIGNING_AVAILABLE = False

check("test signing key material is available (PyCryptodome, "
      "scripts/requirements-attest.txt)", SIGNING_AVAILABLE,
      "install with: pip install -r scripts/requirements-attest.txt")
if not SIGNING_AVAILABLE:
    print("\nCannot exercise the signed-binding boundary without a signer.")
    raise SystemExit(1)

print("\n=== the bundled Ed25519 verifier matches RFC 8032 ===")
# The Stop hook must work with the standard library alone, so the integration
# ships a pure-Python RFC 8032 verifier as a fallback. A homemade crypto
# primitive is only acceptable if it is pinned to the official vectors AND
# cross-checked against a real library, so do both. If these ever disagree, the
# authority boundary is not what it claims to be.
RFC8032_VECTORS = [
    # (public key, message, signature) from RFC 8032 section 7.1
    ("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a", "",
     "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590"
     "a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"),
    ("3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c", "72",
     "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e"
     "15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00"),
    ("fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025", "af82",
     "6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3ac18ff9b538d"
     "16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40a"),
]
for index, (pk_hex, msg_hex, sig_hex) in enumerate(RFC8032_VECTORS, start=1):
    pk, msg, sig = (bytes.fromhex(pk_hex), bytes.fromhex(msg_hex),
                    bytes.fromhex(sig_hex))
    check(f"RFC 8032 vector {index}: pure verifier accepts", _verify_pure(pk, msg, sig) is True)
    check(f"RFC 8032 vector {index}: library verifier agrees",
          _verify_pycryptodome(pk, msg, sig) is True)
    tampered = bytearray(sig)
    tampered[0] ^= 0x01
    check(f"RFC 8032 vector {index}: pure verifier REJECTS a flipped bit",
          _verify_pure(pk, msg, bytes(tampered)) is False)
    check(f"RFC 8032 vector {index}: pure verifier REJECTS a different message",
          _verify_pure(pk, msg + b"x", sig) is False)
# A verifier that accepts everything would pass the positive vectors above.
check("the pure verifier rejects an all-zero signature (not vacuously permissive)",
      _verify_pure(bytes.fromhex(RFC8032_VECTORS[0][0]), b"", bytes(64)) is False)


WORLD = tmpdir()
SESSIONS = WORLD / "sessions"
SESSIONS.mkdir()
BINDING = WORLD / "binding.json"   # deliberately OUTSIDE the session-writable dir


def write_binding(path: Path, sessions: dict, network: str = NETWORK,
                  schema: str = "agent-bounties/openhands-session-binding/v1",
                  signer=None, sign: bool = True) -> Path:
    """Write a binding and, by default, sign it with the pinned operator key."""
    body = {
        "schema_version": schema,
        "network": network,
        "solver": SOLVER,
        "sessions": sessions,
    }
    if sign:
        body["signature"] = (signer or OPERATOR).envelope(body)
    path.write_text(json.dumps(body), encoding="utf-8")
    if not IS_WINDOWS:
        path.chmod(0o600)
    return path


write_binding(BINDING, {
    "sess-bounty": {"bounty_id": BOUNTY_ID, "bounty_contract": CONTRACT, "round": ROUND},
    "sess-unrelated": {"claim": "none"},
})
(SESSIONS / "sess-bounty.json").write_text(json.dumps({}), encoding="utf-8")

SNAPSHOT = WORLD / "events.json"
SNAPSHOT.write_text(json.dumps([CLAIM_EVENT]), encoding="utf-8")

# ONE PORTABLE ARGV REPRESENTATION, END TO END.
#
# The previous version built this with shlex.quote(), which emits POSIX SINGLE
# quotes. parse_state_cmd's Windows branch splits in non-POSIX mode and strips
# only DOUBLE quotes, so on Windows the interpreter path arrived still wrapped
# in literal single quotes and the producer raised FileNotFoundError -- nine
# assertions failed on the platform the suite claims to support.
#
# The documented JSON-array form has no quoting rules to disagree about: it is
# argv verbatim on every platform. It is therefore what the checker registers,
# what the README documents, and what the shell-string branch is measured
# against for equivalence further down.
def state_cmd(*argv: str) -> str:
    return json.dumps(list(argv))


PRODUCER_CMD = state_cmd(sys.executable, "-B", str(PRODUCER))


def base_env(session: str) -> dict:
    return {
        "PATH": os.environ.get("PATH", ""),
        "SYSTEMROOT": os.environ.get("SYSTEMROOT", ""),
        "OPENHANDS_EVENT_TYPE": "Stop",
        "OPENHANDS_PROJECT_DIR": str(ROOT),
        "OPENHANDS_SESSION_ID": session,
        "AGENT_BOUNTIES_STATE_CMD": PRODUCER_CMD,
        "AGENT_BOUNTIES_BINDING_FILE": str(BINDING),
        "AGENT_BOUNTIES_OPERATOR_PUBKEY": OPERATOR.public_b64,
        "AGENT_BOUNTIES_SESSION_DIR": str(SESSIONS),
        "AGENT_BOUNTIES_EVENTS_FILE": str(SNAPSHOT),
        "AGENT_BOUNTIES_ALLOW_TEST_SNAPSHOT": "1",
    }


def run_registered(session, extra_env=None, drop=()):
    env = base_env(session)
    env.update(extra_env or {})
    for key in drop:
        env.pop(key, None)
    done = subprocess.run(
        hook_argv(), cwd=ROOT, env=env,
        input=json.dumps({"event_type": "Stop", "tool_name": None,
                          "session_id": session, "working_dir": str(ROOT)}),
        text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=90, check=False,
    )
    try:
        return done.returncode, json.loads(done.stdout), done.stderr
    except json.JSONDecodeError:
        return done.returncode, {}, done.stderr


rc, body, err = run_registered("sess-bounty")
check("real Stop event on an incomplete active claim -> exit 2 (BLOCK)",
      rc == 2, f"exit={rc} stderr={err[-300:]}")
check("  and the deny decision is on stdout", body.get("decision") == "deny", f"body={body}")

rc, body, err = run_registered("sess-unrelated")
check("session the operator declared claim-free -> exit 0", rc == 0,
      f"exit={rc} stderr={err[-300:]}")
check("  and the decision is allow", body.get("decision") == "allow", f"body={body}")

rc, body, err = run_registered("sess-never-declared")
check("session absent from the operator binding -> exit 2 (not assumed idle)", rc == 2,
      f"exit={rc}")

rc, body, err = run_registered("sess-bounty", drop=("AGENT_BOUNTIES_BINDING_FILE",))
check("no operator binding configured -> exit 2 (identity cannot be established)",
      rc == 2, f"exit={rc}")

rc, body, err = run_registered(
    "sess-bounty",
    {"AGENT_BOUNTIES_STATE_CMD": state_cmd(sys.executable, "-c",
                                           "import sys; sys.exit(3)")})
check("state producer exiting non-zero -> exit 2 (fail closed)", rc == 2, f"exit={rc}")

rc, body, err = run_registered(
    "sess-bounty",
    {"AGENT_BOUNTIES_STATE_CMD": state_cmd(sys.executable, "-c", "print('not json')")})
check("state producer emitting non-JSON -> exit 2 (fail closed)", rc == 2, f"exit={rc}")

rc, body, err = run_registered(
    "sess-bounty", {"AGENT_BOUNTIES_EVENTS_FILE": str(WORLD / "nope.json")})
check("canonical feed unusable -> exit 2 (no local-only fallback)", rc == 2, f"exit={rc}")

print("\n=== the session-writable workfile CANNOT disable the guard ===")
# Finding (2) from the review: deleting the workfile used to make the guard report
# claim.active=false. Identity now comes from the operator binding, so it cannot.
workfile = SESSIONS / "sess-bounty.json"
workfile.unlink()
rc, body, err = run_registered("sess-bounty")
check("deleting the workfile still BLOCKS (identity is operator-owned)", rc == 2,
      f"exit={rc} body={body} stderr={err[-300:]}")
check("  and the reason is missing work, not 'no claim'",
      "no active claim" not in body.get("reason", "").lower(), f"body={body}")

# A workfile that lies about which bounty it is working on changes nothing.
workfile.write_text(json.dumps({
    "bounty_id": "0x" + "00" * 32, "bounty_contract": "0x" + "00" * 20,
    "solver": OTHER_SOLVER, "claim": {"active": False},
    "test": PASSED, "evidence": FULL_EVIDENCE,
    "submission": {"submitted_onchain": True, "bounty_settled": True},
    "settlement": {"canonical_event": good_receipt()},
    "paid": True,
}), encoding="utf-8")
rc, body, err = run_registered("sess-bounty")
reason = body.get("reason", "")
check("a workfile claiming 'no claim' + 'paid' cannot say paid", rc == 0 and "$0.00" in reason,
      f"exit={rc} reason={reason[:250]}")
check("  and it certainly does not report the work as paid",
      "work is paid" not in reason.lower(), reason[:250])

print("\n=== offline snapshots are TEST-ONLY and can never settle ===")
rc, body, err = run_registered("sess-bounty", drop=("AGENT_BOUNTIES_ALLOW_TEST_SNAPSHOT",))
check("snapshot without the explicit test opt-in -> exit 2", rc == 2, f"exit={rc}")
# The producer writes its diagnostic to ITS stderr, which the hook captures and
# folds into the decision reason -- it is not the hook's own stderr.
check("  and the decision explains the snapshot is test-only",
      "test-only" in (body.get("reason", "") + err).lower(),
      f"reason={body.get('reason', '')[:300]} stderr={err[-200:]}")

settled_snapshot = WORLD / "settled.json"
settled_snapshot.write_text(json.dumps([CLAIM_EVENT, SETTLE_EVENT]), encoding="utf-8")
rc, body, err = run_registered("sess-bounty", {"AGENT_BOUNTIES_EVENTS_FILE": str(settled_snapshot)})
check("a BountySettled inside a test snapshot -> exit 2, never paid", rc == 2, f"exit={rc}")
check("  and the reason never says paid", "work is paid" not in body.get("reason", "").lower())

cross = WORLD / "cross-bounty.json"
cross.write_text(json.dumps([event("bounty_settled", bounty_id="0x" + "99" * 32)]),
                 encoding="utf-8")
rc, body, err = run_registered("sess-bounty", {"AGENT_BOUNTIES_EVENTS_FILE": str(cross)})
check("a cross-bounty snapshot -> exit 2 (refused, not silently applied)", rc == 2, f"exit={rc}")

# ---------------------------------------------------------------------------
# The binding itself must be operator-owned.
# ---------------------------------------------------------------------------
print("\n=== the operator binding must be operator-owned and canonical ===")

GOOD_ENTRY = {"bounty_id": BOUNTY_ID, "bounty_contract": CONTRACT, "round": ROUND}
BAD_BINDINGS = [
    ("a binding inside the session-writable dir -> exit 2",
     write_binding(SESSIONS / "binding.json", {"sess-bounty": GOOD_ENTRY})),
    ("a binding on a non-canonical network -> exit 2",
     write_binding(WORLD / "sepolia.json", {"sess-bounty": GOOD_ENTRY}, network="base-sepolia")),
    ("a binding with an unknown schema_version -> exit 2",
     write_binding(WORLD / "schema.json", {"sess-bounty": GOOD_ENTRY}, schema="v0")),
    ("a binding with a non-integer round -> exit 2",
     write_binding(WORLD / "round.json", {"sess-bounty": dict(GOOD_ENTRY, round="two")})),
    ("a binding with a non-canonical bounty id -> exit 2",
     write_binding(WORLD / "id.json", {"sess-bounty": dict(GOOD_ENTRY, bounty_id="b-1")})),
]
if IS_WINDOWS:
    print("  SKIP  world-writable binding (Windows has no POSIX mode bits)")
else:
    loose = write_binding(WORLD / "loose.json", {"sess-bounty": GOOD_ENTRY})
    loose.chmod(loose.stat().st_mode | stat.S_IWOTH)
    BAD_BINDINGS.append(("a world-writable binding -> exit 2", loose))

for (label, _), (rc, body, err) in zip(BAD_BINDINGS, in_parallel(
        [(lambda p=path: run_registered("sess-bounty",
                                        {"AGENT_BOUNTIES_BINDING_FILE": str(p)}))
         for _, path in BAD_BINDINGS])):
    check(label, rc == 2, f"exit={rc} stderr={err[-200:]}")

# ---------------------------------------------------------------------------
# THE AUTHORITY BOUNDARY, run with the SESSION's own identity.
# ---------------------------------------------------------------------------
print("\n=== a session process CANNOT forge, erase, or replace its binding ===")
# This is the review's second blocker. Previously the binding was "operator-
# owned" only by convention -- it merely sat outside the session directory and
# was not group/world-writable -- so the very same OS user running the session
# could rewrite it and change or erase the claim identity. On Windows there was
# no ownership check at all.
#
# So do exactly what the review asked: run AS the session (same process, same
# OS user, full write access to the file), mutate an outside-path owner-writable
# binding, and prove the mutation cannot alter the decision. Every case below
# succeeds at WRITING the file -- that is the point -- and still fails to move
# the guard, because the operator's private key is not in this sandbox.

SESSION_TARGET = WORLD / "session-attacked-binding.json"
write_binding(SESSION_TARGET, {
    "sess-bounty": {"bounty_id": BOUNTY_ID, "bounty_contract": CONTRACT, "round": ROUND},
})
# Pin the local work facts explicitly so the baseline is deterministic and does
# not depend on whatever an earlier section left in the workfile.
workfile.write_text(json.dumps({
    "test": PASSED, "evidence": FULL_EVIDENCE,
    "submission": {"submitted_onchain": True},
}), encoding="utf-8")

# Establish the baseline: with the correct signature the binding is fully
# ACCEPTED, identity resolves, and the guard reaches a decision on the merits
# (exit 0, awaiting the verifier, $0.00). Every attack below must turn this
# exit 0 into exit 2. Without this baseline a guard that blocked unconditionally
# would pass every assertion in this section.
rc, body, err = run_registered("sess-bounty",
                               {"AGENT_BOUNTIES_BINDING_FILE": str(SESSION_TARGET)})
baseline_reason = body.get("reason", "")
check("ATTACK BASELINE: the correctly signed binding is ACCEPTED (exit 0, merits reached)",
      rc == 0 and "$0.00" in baseline_reason,
      f"exit={rc} reason={baseline_reason[:250]} stderr={err[-200:]}")
check("  and the accepted baseline still does not claim payment",
      "work is paid" not in baseline_reason.lower(), baseline_reason[:200])

# The session really can write this file. If it could not, the whole exercise
# would be theatre.
check("the session process really can write the binding "
      "(so the boundary is not just file permissions)",
      os.access(SESSION_TARGET, os.W_OK), f"{SESSION_TARGET} is not writable here")

signed_body = json.loads(SESSION_TARGET.read_text())


def attack(label: str, document, expect_reason: str = "") -> None:
    """Overwrite the binding as the session would, then re-run the guard."""
    SESSION_TARGET.write_text(json.dumps(document) if document is not None else "",
                              encoding="utf-8")
    rc, body, err = run_registered("sess-bounty",
                                   {"AGENT_BOUNTIES_BINDING_FILE": str(SESSION_TARGET)})
    blob = (body.get("reason", "") + err).lower()
    check(label, rc == 2, f"exit={rc} reason={body.get('reason', '')[:200]}")
    if expect_reason:
        check(f"  {label}: refused for the right reason",
              expect_reason in blob, f"reason={blob[:300]}")


# EDIT: point the claim at a different bounty the session would rather work on.
attack("EDIT the bounty_id under a valid signature -> exit 2",
       {**signed_body, "sessions": {
           "sess-bounty": {"bounty_id": "0x" + "99" * 32,
                           "bounty_contract": CONTRACT, "round": ROUND}}},
       "does not verify")
# EDIT: the subtlest one -- flip the session to "no claim" to switch the guard off.
attack("EDIT the session to \"claim\": \"none\" to disable the guard -> exit 2",
       {**signed_body, "sessions": {"sess-bounty": {"claim": "none"}}},
       "does not verify")
# EDIT: swap in the attacker's own solver address.
attack("EDIT the solver address -> exit 2",
       {**signed_body, "solver": "0x" + "44" * 20}, "does not verify")
# ERASE: drop the signature envelope entirely.
attack("ERASE the signature envelope -> exit 2",
       {k: v for k, v in signed_body.items() if k != "signature"},
       "no signature envelope")
# REPLACE: re-sign the mutated document with a key the SESSION generated. The
# signature is perfectly valid -- it is simply not the operator's. This is why
# verification alone is insufficient and the key must be PINNED.
rogue_body = {
    "schema_version": "agent-bounties/openhands-session-binding/v1",
    "network": NETWORK,
    "solver": SOLVER,
    "sessions": {"sess-bounty": {"claim": "none"}},
}
rogue_body["signature"] = ROGUE.envelope(rogue_body)
check("  (the rogue document is internally VALID, so only key pinning can catch it)",
      ed25519_verify(ROGUE.public, canonical_payload(rogue_body),
                     base64.b64decode(rogue_body["signature"]["signature_b64"])),
      "the rogue signature should verify against the rogue key")
attack("REPLACE the binding, re-signed with the session's OWN key -> exit 2",
       rogue_body, "not by the pinned operator key")
# Truncate to nothing.
attack("TRUNCATE the binding to empty -> exit 2", None)

# Restore, and prove the boundary is not simply "always block": the legitimate
# signed document is accepted again afterwards. Without this, a guard that
# rejected everything unconditionally would pass every assertion above.
SESSION_TARGET.write_text(json.dumps(signed_body), encoding="utf-8")
rc, body, err = run_registered("sess-bounty",
                               {"AGENT_BOUNTIES_BINDING_FILE": str(SESSION_TARGET)})
check("restoring the operator-signed binding is accepted again "
      "(the guard is not merely always-deny)",
      rc == 0 and "$0.00" in body.get("reason", ""),
      f"exit={rc} reason={body.get('reason', '')[:250]} stderr={err[-200:]}")

# And an unsigned binding cannot be used at all, even at the right path with
# perfect permissions -- the old "outside the session dir + 0600" story.
unsigned = write_binding(WORLD / "unsigned.json", {
    "sess-bounty": {"bounty_id": BOUNTY_ID, "bounty_contract": CONTRACT, "round": ROUND},
}, sign=False)
rc, body, err = run_registered("sess-bounty",
                               {"AGENT_BOUNTIES_BINDING_FILE": str(unsigned)})
check("an UNSIGNED binding outside the session dir with mode 0600 -> exit 2",
      rc == 2, f"exit={rc}")

# Without a pinned key there is no boundary to enforce, so refuse to proceed
# rather than silently degrading to the old provenance-by-location behaviour.
rc, body, err = run_registered("sess-bounty", drop=("AGENT_BOUNTIES_OPERATOR_PUBKEY",))
check("no pinned operator public key -> exit 2 (no silent downgrade)", rc == 2,
      f"exit={rc}")
rc, body, err = run_registered("sess-bounty",
                               {"AGENT_BOUNTIES_OPERATOR_PUBKEY": "not-base64!!"})
check("a malformed pinned operator key -> exit 2", rc == 2, f"exit={rc}")
rc, body, err = run_registered(
    "sess-bounty", {"AGENT_BOUNTIES_OPERATOR_PUBKEY": base64.b64encode(b"short").decode()})
check("a pinned key of the wrong length -> exit 2", rc == 2, f"exit={rc}")
rc, body, err = run_registered("sess-bounty",
                               {"AGENT_BOUNTIES_OPERATOR_PUBKEY": ROGUE.public_b64})
check("pinning a DIFFERENT key than the signer -> exit 2", rc == 2, f"exit={rc}")

# The signing tool must round-trip with the shipped verifier: a signature the
# operator tool produces has to be one the guard accepts, or the documented
# workflow is broken.
print("\n=== the operator signing tool round-trips with the shipped verifier ===")
SIGN_TOOL = ROOT / "integrations/openhands/sign_binding.py"
check("integrations/openhands/sign_binding.py ships", SIGN_TOOL.is_file())
keydir = tmpdir()
keyfile = keydir / "operator.key"
gen = subprocess.run([sys.executable, "-B", str(SIGN_TOOL), "keygen", "--out", str(keyfile)],
                     text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                     timeout=90, check=False)
check("keygen produces an operator key", gen.returncode == 0 and keyfile.is_file(),
      f"exit={gen.returncode} stderr={gen.stderr[-300:]}")
if keyfile.is_file():
    tool_binding = keydir / "binding.json"
    tool_binding.write_text(json.dumps({
        "schema_version": "agent-bounties/openhands-session-binding/v1",
        "network": NETWORK, "solver": SOLVER,
        "sessions": {"sess-bounty": {"bounty_id": BOUNTY_ID,
                                     "bounty_contract": CONTRACT, "round": ROUND}},
    }), encoding="utf-8")
    signed = subprocess.run(
        [sys.executable, "-B", str(SIGN_TOOL), "sign", "--key", str(keyfile),
         "--binding", str(tool_binding)],
        text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=90, check=False)
    check("sign succeeds", signed.returncode == 0, f"stderr={signed.stderr[-300:]}")
    tool_pub = json.loads(keyfile.read_text())["public_key_b64"]
    rc, body, err = run_registered("sess-bounty", {
        "AGENT_BOUNTIES_BINDING_FILE": str(tool_binding),
        "AGENT_BOUNTIES_OPERATOR_PUBKEY": tool_pub})
    check("  a tool-signed binding is ACCEPTED by the shipped verifier",
          rc == 0 and "$0.00" in body.get("reason", ""),
          f"exit={rc} reason={body.get('reason', '')[:250]} stderr={err[-200:]}")
    # ... and tampering with it afterwards is caught.
    tampered_doc = json.loads(tool_binding.read_text())
    tampered_doc["sessions"]["sess-bounty"]["round"] = ROUND + 1
    tool_binding.write_text(json.dumps(tampered_doc), encoding="utf-8")
    rc, body, err = run_registered("sess-bounty", {
        "AGENT_BOUNTIES_BINDING_FILE": str(tool_binding),
        "AGENT_BOUNTIES_OPERATOR_PUBKEY": tool_pub})
    check("  editing a tool-signed binding afterwards -> exit 2", rc == 2, f"exit={rc}")
    check("  the private seed never leaves the operator key file",
          json.loads(keyfile.read_text()).get("private_seed_b64")
          not in tool_binding.read_text(),
          "the private seed must never appear in a binding document")


print("\n=== AGENT_BOUNTIES_STATE_CMD parses portably (Windows paths included) ===")
# REGRESSION PIN for the review's first blocker. The suite used to register a
# shlex.quote()'d shell string, whose POSIX single quotes the Windows branch of
# parse_state_cmd does not strip -- so the command the suite actually shipped
# raised FileNotFoundError on Windows and nine assertions failed there while
# every one of them passed on Linux.
#
# The fix is one portable representation (the documented JSON array). This
# asserts the property that makes it portable: the argv the checker registers
# must parse to the SAME tokens under BOTH branches of parse_state_cmd,
# including the Windows branch, on whatever platform the suite is running.
both = subprocess.run(
    [sys.executable, "-B", "-c",
     "import importlib.util,json,os,sys;"
     "spec=importlib.util.spec_from_file_location('g',sys.argv[1]);"
     "m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);"
     "posix=m.parse_state_cmd(sys.argv[2]);"
     "os.name='nt';"
     "windows=m.parse_state_cmd(sys.argv[2]);"
     "print(json.dumps({'posix':posix,'windows':windows}))",
     str(GUARD), PRODUCER_CMD],
    text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60, check=False)
try:
    parsed = json.loads(both.stdout)
except json.JSONDecodeError:
    parsed = {}
want_argv = [sys.executable, "-B", str(PRODUCER)]
check("the REGISTERED state command parses identically on POSIX and Windows",
      parsed.get("posix") == parsed.get("windows") == want_argv,
      f"posix={parsed.get('posix')} windows={parsed.get('windows')} "
      f"want={want_argv} stderr={both.stderr[-200:]}")
# And prove that assertion is load-bearing, deterministically on ANY platform.
#
# Using sys.executable here would be useless: shlex.quote() is a no-op for a
# path containing no shell-unsafe characters, so on a typical POSIX checkout the
# "broken" legacy form and the fixed form are byte-identical and the pin would
# pass for the wrong reason. A real Windows interpreter path contains
# backslashes, which shlex.quote DOES wrap in single quotes and the Windows
# splitter (which strips only double quotes) then leaves attached -- producing
# the literal FileNotFoundError the review hit. Use that path explicitly.
WINDOWS_PYTHON = r"C:\Python311\python.exe"
WINDOWS_PRODUCER = r"C:\repo\integrations\openhands\state_producer.py"
legacy = subprocess.run(
    [sys.executable, "-B", "-c",
     "import importlib.util,json,os,sys;"
     "spec=importlib.util.spec_from_file_location('g',sys.argv[1]);"
     "m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);"
     "os.name='nt';"
     "print(json.dumps(m.parse_state_cmd(sys.argv[2])))",
     str(GUARD),
     f"{shlex.quote(WINDOWS_PYTHON)} -B {shlex.quote(WINDOWS_PRODUCER)}"],
    text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60, check=False)
try:
    legacy_tokens = json.loads(legacy.stdout)
except json.JSONDecodeError:
    legacy_tokens = []
check("  the OLD shlex.quote form leaves stray quotes on a Windows path "
      "(this is the bug that was shipped)",
      bool(legacy_tokens) and legacy_tokens[0] != WINDOWS_PYTHON,
      f"legacy tokens={legacy_tokens}; if token 0 is already the clean path, "
      f"this pin is vacuous")
# The JSON array carries the same Windows argv through both branches untouched.
array_probe = subprocess.run(
    [sys.executable, "-B", "-c",
     "import importlib.util,json,os,sys;"
     "spec=importlib.util.spec_from_file_location('g',sys.argv[1]);"
     "m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);"
     "posix=m.parse_state_cmd(sys.argv[2]);"
     "os.name='nt';"
     "print(json.dumps({'posix':posix,'windows':m.parse_state_cmd(sys.argv[2])}))",
     str(GUARD), state_cmd(WINDOWS_PYTHON, "-B", WINDOWS_PRODUCER)],
    text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60, check=False)
try:
    array_parsed = json.loads(array_probe.stdout)
except json.JSONDecodeError:
    array_parsed = {}
want_windows = [WINDOWS_PYTHON, "-B", WINDOWS_PRODUCER]
check("  the JSON-array form carries a Windows argv intact through BOTH branches",
      array_parsed.get("posix") == array_parsed.get("windows") == want_windows,
      f"posix={array_parsed.get('posix')} windows={array_parsed.get('windows')} "
      f"want={want_windows} stderr={array_probe.stderr[-200:]}")

# Equivalence: a plain shell string must reach the producer and reach the same
# decision as the array form. Use a path with no spaces or backslashes so the
# string form is unambiguous under either splitter -- the point here is that the
# two REPRESENTATIONS agree, not that shell quoting is solved.
array_rc, array_body, _ = run_registered("sess-bounty")
plain = f"{sys.executable} -B {PRODUCER}"
if " " in plain:
    print("  SKIP  shell-string equivalence (interpreter or repo path contains a space)")
else:
    rc, body, err = run_registered("sess-bounty", {"AGENT_BOUNTIES_STATE_CMD": plain})
    check("shell-string state command yields the same decision as the JSON-array form",
          (rc, body.get("decision")) == (array_rc, array_body.get("decision")),
          f"string=({rc}, {body.get('decision')}) array=({array_rc}, "
          f"{array_body.get('decision')}) stderr={err[-200:]}")
rc, body, err = run_registered("sess-bounty", {"AGENT_BOUNTIES_STATE_CMD": json.dumps(
    {"cmd": "nope"})})
check("non-array JSON state command -> exit 2 (fail closed)", rc == 2, f"exit={rc}")
rc, body, err = run_registered("sess-bounty", {"AGENT_BOUNTIES_STATE_CMD": "   "})
check("empty state command -> exit 2 (fail closed)", rc == 2, f"exit={rc}")

# Prove the Windows string-splitting branch keeps backslash paths intact. The
# branch is OS-gated, so exercise the function directly rather than only on nt.
# Tokens are printed one per line: comparing against a repr() would compare
# against DOUBLED backslashes and quietly pass for the wrong reason.
probe = subprocess.run(
    [sys.executable, "-B", "-c",
     "import importlib.util,os,sys;"
     "spec=importlib.util.spec_from_file_location('g',sys.argv[1]);"
     "m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);"
     "os.name='nt';"
     "print('\\n'.join(m.parse_state_cmd(sys.argv[2])))",
     str(GUARD), r'C:\Python311\python.exe -B C:\repo\state_producer.py'],
    text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60, check=False)
tokens = probe.stdout.splitlines()
check("a Windows backslash command survives parsing (no escape mangling)",
      tokens == [r"C:\Python311\python.exe", "-B", r"C:\repo\state_producer.py"],
      f"tokens={tokens} stderr={probe.stderr[-200:]}")
# And prove the naive POSIX split really would have corrupted it, so the test
# above is not vacuously true on a platform where both branches agree.
check("  (POSIX-mode split would have mangled it, so the branch is load-bearing)",
      shlex.split(r'C:\Python311\python.exe -B C:\repo\state_producer.py')[0]
      != r"C:\Python311\python.exe")

# Session ids are untrusted input.
traversal = subprocess.run(
    [sys.executable, "-B", str(PRODUCER), os.path.join("..", "..", "etc", "passwd")],
    env={"PATH": os.environ.get("PATH", ""), "SYSTEMROOT": os.environ.get("SYSTEMROOT", ""),
         "AGENT_BOUNTIES_BINDING_FILE": str(BINDING),
         "AGENT_BOUNTIES_SESSION_DIR": str(SESSIONS)},
    text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60, check=False)
check("path-traversal session id is refused", traversal.returncode != 0,
      f"exit={traversal.returncode}")

# ---------------------------------------------------------------------------
# LIVE canonical settlement, served over real HTTP, is the only thing that pays.
# ---------------------------------------------------------------------------
print("\n=== a LIVE canonical BountySettled is the only thing that pays ===")

SERVED: dict = {"events": [CLAIM_EVENT], "status": 200}


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):  # noqa: N802 - BaseHTTPRequestHandler API
        if "/v1/base/autonomous-bounties/events" not in self.path:
            self.send_response(404)
            self.end_headers()
            return
        status = SERVED["status"]
        body = json.dumps(SERVED["events"]).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format, *args):  # noqa: A002 - BaseHTTPRequestHandler API
        return  # silence the default stderr access log


httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
httpd.socket.settimeout(5)
threading.Thread(target=httpd.serve_forever, daemon=True).start()
API = f"http://127.0.0.1:{httpd.server_address[1]}"
with socket.create_connection(("127.0.0.1", httpd.server_address[1]), timeout=5):
    pass  # the server is genuinely accepting connections before any assertion runs

# Complete local work, so only settlement is in question.
workfile.write_text(json.dumps({
    "test": PASSED, "evidence": FULL_EVIDENCE, "submission": {"submitted_onchain": True},
}), encoding="utf-8")
LIVE = {"AGENT_BOUNTIES_API": API}


def run_live(events, status=200, extra=None):
    SERVED["events"] = events
    SERVED["status"] = status
    env = dict(LIVE)
    env.update(extra or {})
    return run_registered("sess-bounty", env, drop=("AGENT_BOUNTIES_EVENTS_FILE",
                                                    "AGENT_BOUNTIES_ALLOW_TEST_SNAPSHOT"))


rc, body, err = run_live([CLAIM_EVENT])
reason = body.get("reason", "")
check("live feed, claimed but unsettled -> exit 0 and $0.00", rc == 0 and "$0.00" in reason,
      f"exit={rc} reason={reason[:250]} stderr={err[-200:]}")
check("  and it does NOT say paid", "work is paid" not in reason.lower(), reason[:250])

rc, body, err = run_live([CLAIM_EVENT, SETTLE_EVENT])
reason = body.get("reason", "")
check("live canonical BountySettled -> exit 0 and PAID", rc == 0 and "work is paid" in reason.lower(),
      f"exit={rc} reason={reason[:300]} stderr={err[-200:]}")
check("  and the reason carries the real tx hash", SETTLE_EVENT["tx_hash"] in reason, reason[:300])

rc, body, err = run_live([CLAIM_EVENT, event("bounty_settled", solver=OTHER_SOLVER)])
check("live settlement paid to another solver -> exit 2 (refused, never ours)", rc == 2,
      f"exit={rc}")

rc, body, err = run_live([CLAIM_EVENT, event("bounty_settled", rnd=ROUND + 5)])
reason = body.get("reason", "")
check("live settlement for another round -> not our payment", "work is paid" not in reason.lower(),
      f"exit={rc} reason={reason[:250]}")

rc, body, err = run_live([CLAIM_EVENT, event("bounty_settled", block=0)])
check("live settlement with block_number 0 -> exit 2", rc == 2, f"exit={rc}")

rc, body, err = run_live([CLAIM_EVENT, event("bounty_settled", tx="", log_key="")])
check("live settlement with no chain identity -> exit 2", rc == 2, f"exit={rc}")

rc, body, err = run_live([CLAIM_EVENT, event("bounty_settled", occurred_at=now_iso(600))])
check("live settlement stamped in the future -> exit 2 (not fresh evidence)", rc == 2,
      f"exit={rc}")

rc, body, err = run_live([CLAIM_EVENT, event("bounty_settled", contract="0x" + "99" * 20)])
check("live settlement on a foreign contract -> exit 2", rc == 2, f"exit={rc}")

# ISOLATING CASE for the cross-bounty rule. The generic cross-bounty snapshot
# above is also refused by the snapshot-cannot-settle rule, so on its own it does
# not prove the bounty_id binding exists. Here the settlement is live-canonical
# and valid in EVERY other respect -- only the bounty_id is foreign -- so this
# case fails if and only if the cross-bounty check is removed.
rc, body, err = run_live([CLAIM_EVENT, event("bounty_settled", bounty_id="0x" + "99" * 32)])
reason = body.get("reason", "")
check("live settlement for a FOREIGN BOUNTY -> exit 2 (isolates the bounty binding)",
      rc == 2, f"exit={rc} reason={reason[:250]}")
check("  and it never says paid", "work is paid" not in reason.lower(), reason[:250])

rc, body, err = run_live([CLAIM_EVENT, event("claim_expired")])
reason = body.get("reason", "")
check("live claim expiry -> claim released, exit 0", rc == 0, f"exit={rc} reason={reason[:200]}")
check("  and it does not say paid", "work is paid" not in reason.lower())

rc, body, err = run_live([], status=500)
check("live feed HTTP 500 -> exit 2 (fail closed)", rc == 2, f"exit={rc}")

rc, body, err = run_registered("sess-bounty",
                               {"AGENT_BOUNTIES_API": "http://127.0.0.1:1"},
                               drop=("AGENT_BOUNTIES_EVENTS_FILE",
                                     "AGENT_BOUNTIES_ALLOW_TEST_SNAPSHOT"))
check("live feed unreachable -> exit 2 (fail closed)", rc == 2, f"exit={rc}")

httpd.shutdown()

# ---------------------------------------------------------------------------
print("\n=== event payload handling ===")
code, out, _ = run_guard(state=None, event_payload={"event_type": "Stop",
                                                    "session_id": "other-session"})
check("unrelated session exits 0", code == 0, f"exit={code}")
code, out, _ = run_guard(state=None, event_payload="not-json-at-all")
check("non-JSON stdin does not crash the guard", code in (0, 2), f"exit={code}")

print("\n=== an UNHANDLED CRASH must block, never fail open ===")
# Per the official docs, exit 0 allows and exit 2 blocks, but ANY OTHER exit code
# is "Error. The operation proceeds, but the error is logged." An uncaught Python
# exception exits 1, so a crashing guard would silently let a session with a live
# bond end -- the exact failure this guard exists to prevent.
#
# The crash must be one the ordinary handling in load_claim_state does NOT catch,
# otherwise the test proves nothing. A directory path raises IsADirectoryError,
# which IS an OSError and is already handled. Deeply nested JSON makes json.load
# raise RecursionError, which is neither a JSONDecodeError nor an OSError.
crash_state = tmpdir() / "recursion-bomb.json"
crash_state.write_text("[" * 200_000 + "]" * 200_000, encoding="utf-8")
crashed = subprocess.run(
    hook_argv(), cwd=ROOT,
    env={"PATH": os.environ.get("PATH", ""), "SYSTEMROOT": os.environ.get("SYSTEMROOT", ""),
         "OPENHANDS_SESSION_ID": "sess-crash", "AGENT_BOUNTIES_STATE": str(crash_state)},
    input=json.dumps({"event_type": "Stop", "session_id": "sess-crash"}),
    text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=90, check=False)
check("the bomb really does raise an unhandled exception",
      "RecursionError" in crashed.stderr, f"stderr tail={crashed.stderr[-200:]!r}")
check("  crash never exits 1 (exit 1 would ALLOW the stop)", crashed.returncode != 1,
      f"exit={crashed.returncode}")
check("  crash blocks with exit 2", crashed.returncode == 2, f"exit={crashed.returncode}")
try:
    crash_out = json.loads(crashed.stdout)
except (json.JSONDecodeError, ValueError):
    crash_out = {}
check("  crash emits a deny decision", crash_out.get("decision") == "deny",
      f"stdout={crashed.stdout[:200]}")
check("  crash reason still reports $0.00", "$0.00" in crash_out.get("reason", ""),
      f"reason={crash_out.get('reason')!r}")

guard_src = GUARD.read_text(encoding="utf-8")
check("guard defines no fail-open ERROR exit code",
      "ERROR = 1" not in guard_src and "ERROR=1" not in guard_src)

print("\n=== fixtures present ===")
for name in ("claimable", "unfunded", "verifier-unready", "submitted-not-paid"):
    check(f"fixtures/{name}.json", (FIXTURES / f"{name}.json").is_file())

for path in TEMPDIRS:
    shutil.rmtree(path, ignore_errors=True)

if failures:
    print(f"\n{len(failures)} check(s) failed:")
    for item in failures:
        print(f"  - {item}")
    raise SystemExit(1)

print("\nOpenHands integration smoke passed")
