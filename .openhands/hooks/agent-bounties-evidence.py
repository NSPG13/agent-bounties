#!/usr/bin/env python3
"""Agent Bounties evidence guard — an OpenHands Stop hook.

Runs when the agent tries to finish. Blocks completion while a claimed bounty
still lacks settle-ready evidence, so a session cannot end with the bond posted,
the work done, and nothing submitted.

EXIT CONTRACT (per https://docs.openhands.dev/openhands/usage/customization/hooks):
    exit 0 -> allow. The operation proceeds.
    exit 2 -> BLOCK. The operation is denied.
    other  -> non-blocking error.
JSON on stdout carries the human-readable decision alongside the exit code.

INVOCATION: registered in `.openhands/hooks.json` through an explicit Python
interpreter (`python3 .openhands/hooks/agent-bounties-evidence.py`) rather than as
a bare executable path, so it does not depend on the +x bit or on a shebang being
honoured by the host shell.

STDIN is the OpenHands event payload, NOT bounty state. Claim state is read from
an authoritative producer configured by AGENT_BOUNTIES_STATE_CMD (preferred) or
AGENT_BOUNTIES_STATE (a file path). The session id from the event payload is
passed through so state is resolved per session.

FAIL-CLOSED RULE: if a claim-state source is CONFIGURED but unreadable, malformed,
or dimensionally invalid, this guard BLOCKS (exit 2). Only the genuinely
unconfigured case — no state source at all, i.e. a repo not doing bounty work —
allows completion, and unrelated sessions exit 0.

Local input can never assert payment. A `bounty_settled` boolean in local state is
treated as UNVERIFIED. Paid language requires a settlement receipt in
`settlement.canonical_event` that is LIVE-CANONICAL (`provenance:
canonical_live`), carries real chain identity (`tx_hash`, `log_key`, a positive
`block_number`), and is BOUND to the same canonical network, bounty id, bounty
contract, round and solver as the active claim. A forged snapshot, a receipt for
another bounty or round, or a receipt on a non-canonical network cannot say paid.
This mirrors the producer's own rule on purpose: defence in depth, not one line.

WALLET SAFETY: never reads, stores, logs, or transmits secret key material, and
never broadcasts a transaction.
"""

from __future__ import annotations

import json
import os
import shlex
import subprocess
import sys
import traceback

# Exit contract, per the official OpenHands hooks docs:
#   0 = allow (operation proceeds), 2 = block (operation denied).
# Any OTHER code is treated as an error and the operation still PROCEEDS, so
# there is deliberately no "error" exit code here -- every refusal path uses
# BLOCK. See the fail-closed crash handler at the bottom of this file.
ALLOW, BLOCK = 0, 2

REQUIRED_EVIDENCE = (
    "repository",
    "commit",
    "test_command",
    "source_snapshot_digest",
    "discovery_source",
    "participation_reason",
    "improvement_feedback",
)

# A canonical settlement receipt must carry real chain identity, not a boolean.
RECEIPT_IDENTITY = ("tx_hash", "log_key")

# Only a network with a canonical AgentBountyFactory deployment and its immutable
# settlement token can settle. See docs/autonomous-protocol.md.
CANONICAL_NETWORKS = {"base-mainnet"}

# The only provenance that may produce paid language: an event read live from the
# canonical feed in this very invocation. Offline snapshots are test-only.
LIVE_PROVENANCE = "canonical_live"


def emit(decision: str, reason: str, code: int) -> int:
    print(json.dumps({"decision": decision, "reason": reason}))
    return code


def read_event() -> dict:
    """Parse the OpenHands event payload from stdin. Never raises."""
    if sys.stdin.isatty():
        return {}
    try:
        raw = sys.stdin.read().strip()
    except OSError:
        return {}
    if not raw:
        return {}
    try:
        parsed = json.loads(raw)
        return parsed if isinstance(parsed, dict) else {}
    except json.JSONDecodeError:
        return {}


def session_id(event: dict) -> str:
    return str(
        event.get("session_id")
        or event.get("sessionId")
        or os.environ.get("OPENHANDS_SESSION_ID")
        or ""
    )


def parse_state_cmd(cmd: str) -> list[str]:
    """Turn AGENT_BOUNTIES_STATE_CMD into argv, portably.

    A JSON array is the unambiguous form and is recommended on Windows:
        ["C:\\\\Python311\\\\python.exe", "-B", "C:\\\\repo\\\\state_producer.py"]

    A plain string is also accepted. POSIX-mode shlex treats a backslash as an
    escape character, which would silently mangle an unquoted Windows path such
    as C:\\repo\\state_producer.py into C:reposstate_producer.py, so on Windows
    the string is split in non-POSIX mode and the surrounding quotes stripped.
    """
    text = cmd.strip()
    if text.startswith("["):
        parsed = json.loads(text)
        if not isinstance(parsed, list) or not all(isinstance(x, str) for x in parsed):
            raise ValueError("AGENT_BOUNTIES_STATE_CMD JSON must be an array of strings")
        return parsed
    if os.name == "nt":
        return [tok.strip('"') for tok in shlex.split(text, posix=False)]
    return shlex.split(text)


def load_claim_state(sid: str):
    """Resolve authoritative claim state.

    Returns (state_dict, configured, error_message).
      configured=False -> no source at all; this repo is not doing bounty work.
      state=None with configured=True -> unreadable/malformed; caller must BLOCK.
    """
    cmd = os.environ.get("AGENT_BOUNTIES_STATE_CMD")
    if cmd:
        try:
            argv = parse_state_cmd(cmd)
            if not argv:
                return None, True, "AGENT_BOUNTIES_STATE_CMD is set but parses to no command"
            if sid:
                argv.append(sid)
            done = subprocess.run(
                argv, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                timeout=20, check=False,
            )
            if done.returncode != 0:
                return None, True, f"state producer exited {done.returncode}: {done.stderr[-200:]}"
            return json.loads(done.stdout), True, None
        except json.JSONDecodeError as exc:
            return None, True, f"state producer emitted invalid JSON: {exc}"
        except Exception as exc:  # timeout, missing binary, permissions
            return None, True, f"state producer failed: {type(exc).__name__}: {exc}"

    path = os.environ.get("AGENT_BOUNTIES_STATE")
    if path:
        try:
            with open(path, encoding="utf-8") as handle:
                return json.load(handle), True, None
        except FileNotFoundError:
            return None, True, f"configured state file is missing: {path}"
        except json.JSONDecodeError as exc:
            return None, True, f"configured state file is malformed: {exc}"
        except OSError as exc:
            return None, True, f"configured state file unreadable: {exc}"

    return None, False, None


def receipt_rejection(state: dict, claim: dict) -> str | None:
    """Return None when the receipt genuinely proves payment, else why it does not.

    A receipt is only payment evidence when it is live-canonical, carries real
    chain identity, and is bound to the exact claim this session holds. Anything
    else -- a test snapshot, a receipt for another bounty/round/solver, a
    non-canonical network -- is rejected, and the caller falls through to the
    "$0.00, not confirmed" branch.
    """
    settlement = state.get("settlement")
    if not isinstance(settlement, dict):
        return "no settlement section"
    event = settlement.get("canonical_event")
    if not isinstance(event, dict):
        return "settlement.canonical_event is not an object"
    if str(event.get("kind", "")).lower() not in ("bountysettled", "bounty_settled"):
        return f"receipt kind {event.get('kind')!r} is not BountySettled"

    provenance = str(event.get("provenance", "")).strip()
    if provenance != LIVE_PROVENANCE:
        return (
            f"receipt provenance is {provenance or '(absent)'!r}, not {LIVE_PROVENANCE!r}; "
            "offline snapshots are test-only and are never payment evidence"
        )

    network = str(event.get("network", "")).strip().lower()
    if network not in CANONICAL_NETWORKS:
        return f"receipt network {network or '(absent)'!r} is not a canonical settlement network"

    missing = [f for f in RECEIPT_IDENTITY if not str(event.get(f, "")).strip()]
    if missing:
        return f"receipt is missing chain identity: {', '.join(missing)}"
    block = event.get("block_number")
    if not isinstance(block, int) or isinstance(block, bool) or block <= 0:
        return f"receipt has no positive block_number (got {block!r})"

    # Bind the receipt to THIS claim. A settlement for a different bounty, round
    # or solver is somebody else's payment.
    for field in ("network", "bounty_id", "bounty_contract"):
        want = str(claim.get(field, "")).strip().lower()
        got = str(event.get(field, "")).strip().lower()
        if not want:
            return f"claim does not declare {field}, so the receipt cannot be bound to it"
        if got != want:
            return f"receipt {field} {got or '(absent)'!r} does not match the claim's {want!r}"
    solver_want = str(claim.get("solver", "")).strip().lower()
    solver_got = str(event.get("solver", "")).strip().lower()
    if not solver_want:
        return "claim does not declare a solver, so the receipt cannot be bound to it"
    if solver_got != solver_want:
        return f"receipt solver {solver_got or '(absent)'!r} is not the claim solver {solver_want!r}"
    if event.get("round") != claim.get("round"):
        return (
            f"receipt round {event.get('round')!r} does not match the claim round "
            f"{claim.get('round')!r}"
        )
    return None


def decide(state, configured, err, sid):
    # Unconfigured: nothing to protect. Unrelated sessions must exit 0.
    if not configured:
        return "allow", "no bounty claim-state source configured for this session", ALLOW

    # Configured but unreadable -> FAIL CLOSED. This was the review's Finding 2.
    if state is None:
        return (
            "deny",
            f"Claim state is configured but unreadable, so an active claim cannot be ruled out: {err}. "
            "Next action: fix the state producer (AGENT_BOUNTIES_STATE_CMD/AGENT_BOUNTIES_STATE) "
            "and re-run. Failing closed to protect a posted bond.",
            BLOCK,
        )

    if not isinstance(state, dict):
        return "deny", "Claim state is not a JSON object; failing closed.", BLOCK

    claim = state.get("claim")
    if claim is None:
        return (
            "deny",
            "Claim state is present but has no 'claim' section, so claim status is unknown. "
            "Failing closed. Next action: have the state producer emit claim.active explicitly.",
            BLOCK,
        )
    if not isinstance(claim, dict):
        return "deny", "'claim' must be an object; failing closed.", BLOCK

    active = claim.get("active")
    if active is None:
        return (
            "deny",
            "claim.active is absent, so an active claim cannot be ruled out. Failing closed.",
            BLOCK,
        )

    # A bound canonical receipt is TERMINAL and must be evaluated before the
    # "no active claim" early-out. Settlement ENDS the claim, so a genuinely paid
    # bounty arrives here with claim.active == False; checking occupancy first
    # would report "no active claim" and silently lose the payment evidence.
    rejection = receipt_rejection(state, claim)
    if rejection is None:
        receipt = state["settlement"]["canonical_event"]
        return (
            "allow",
            "Canonical BountySettled receipt is present, live-canonical, and bound to this "
            f"claim (network {receipt['network']}, bounty {receipt['bounty_id']}, round "
            f"{receipt['round']}, tx {receipt['tx_hash']}, log {receipt['log_key']}, block "
            f"{receipt['block_number']}); work is paid.",
            ALLOW,
        )

    if active is not True:
        return "allow", f"no active claim for session {sid or '(unknown)'}", ALLOW

    contract = claim.get("bounty_contract", "<bounty_contract>")

    # 1. Work must be tested.
    test = state.get("test") or {}
    if not str(test.get("command", "")).strip():
        return (
            "deny",
            "An active claim exists but no test command was recorded. Next action: run the "
            "bounty's acceptance check (python -B /benchmark/check.py) and record the exact command.",
            BLOCK,
        )
    if test.get("passed") is not True:
        return (
            "deny",
            f"The acceptance check has not passed (command: {test.get('command')}). "
            "Next action: fix the failing criterion and re-run that exact command.",
            BLOCK,
        )

    # 2. Evidence must be complete.
    evidence = state.get("evidence") or {}
    missing = [f for f in REQUIRED_EVIDENCE if not str(evidence.get(f, "")).strip()]
    if missing:
        return (
            "deny",
            f"Evidence is incomplete; missing: {', '.join(missing)}. Next action: populate every "
            "required field, computing source_snapshot_digest with "
            "`git ls-files -z | sort -z | xargs -0 sha256sum | sha256sum`.",
            BLOCK,
        )

    # 3. Submission must be on-chain.
    submission = state.get("submission") or {}
    if submission.get("submitted_onchain") is not True:
        return (
            "deny",
            "Work is tested and evidence is complete, but no submission is on-chain. Next action: "
            f"call submit(bytes32,bytes32) on {contract} with the submission and evidence hashes, "
            "then publish the evidence.",
            BLOCK,
        )

    # 4. Paid language requires a canonical receipt — never a local boolean. The
    # receipt was already evaluated above; `rejection` says why it did not count.
    claims_paid = (
        bool(submission.get("bounty_settled"))
        or bool(state.get("paid"))
        or isinstance(state.get("settlement"), dict)
    )
    note = (
        " A settlement assertion was present but REJECTED as payment evidence: "
        f"{rejection}. Local or unbound input cannot prove payment."
        if claims_paid else ""
    )
    return (
        "allow",
        "Submission is on-chain and awaiting the verifier. Payment is NOT confirmed: only a "
        "canonical BountySettled event proves payment. Report $0.00 earned until that event "
        "exists." + note,
        ALLOW,
    )


def main() -> int:
    event = read_event()
    sid = session_id(event)
    state, configured, err = load_claim_state(sid)
    decision, reason, code = decide(state, configured, err, sid)
    return emit(decision, reason, code)


if __name__ == "__main__":
    # FAIL CLOSED on an unexpected crash.
    #
    # Per https://docs.openhands.dev/openhands/usage/customization/hooks the exit
    # contract is: 0 = proceed, 2 = block, "any other code - Error. The operation
    # proceeds, but the error is logged." An uncaught exception exits 1, which
    # therefore lets the session END -- the exact fail-open outcome this guard
    # exists to prevent, and the worst case for a solver holding a posted bond.
    #
    # So a crash must deny, not error. The traceback still goes to stderr for
    # debugging, but the decision on stdout and the exit code both say BLOCK.
    try:
        raise SystemExit(main())
    except SystemExit:
        raise
    except BaseException:  # noqa: BLE001 - deliberate catch-all; see above
        traceback.print_exc(file=sys.stderr)
        raise SystemExit(
            emit(
                "deny",
                "agent-bounties evidence guard crashed, so claim state could not be "
                "verified. Failing closed: an unverified session must not end while a "
                "bond may be posted. Fix the guard, then finish. Earned: $0.00.",
                BLOCK,
            )
        )
