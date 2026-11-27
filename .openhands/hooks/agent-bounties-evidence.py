#!/usr/bin/env python3
"""Agentizable evidence guard — an OpenHands Stop hook.

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
            "offline snapshots cannot prove payment"
        )

    for field in RECEIPT_IDENTITY:
        val = event.get(field)
        if not val or not str(val).strip():
            return f"receipt missing identity field: {field}"

    block_num = event.get("block_number")
    try:
        if block_num is None or int(block_num) <= 0:
            return f"invalid block_number: {block_num}"
    except (ValueError, TypeError):
        return f"invalid block_number: {block_num}"

    network = str(event.get("network") or event.get("chain") or "").strip().lower()
    if network not in CANONICAL_NETWORKS:
        return f"network {network!r} is not in canonical networks {CANONICAL_NETWORKS}"

    # Check bindings with claim
    bounty_id = claim.get("bounty_id") or claim.get("bountyId")
    event_bounty_id = event.get("bounty_id") or event.get("bountyId")
    if bounty_id and event_bounty_id and str(bounty_id) != str(event_bounty_id):
        return f"receipt bounty_id {event_bounty_id!r} does not match claim bounty_id {bounty_id!r}"

    contract = claim.get("bounty_contract") or claim.get("contract_address") or claim.get("contractAddress")
    event_contract = event.get("bounty_contract") or event.get("contract_address") or event.get("contractAddress")
    if contract and event_contract and str(contract).lower() != str(event_contract).lower():
        return f"receipt contract {event_contract!r} does not match claim contract {contract!r}"

    round_id = claim.get("round") or claim.get("round_id") or claim.get("roundId")
    event_round_id = event.get("round") or event.get("round_id") or event.get("roundId")
    if round_id is not None and event_round_id is not None and str(round_id) != str(event_round_id):
        return f"receipt round {event_round_id!r} does not match claim round {round_id!r}"

    solver = claim.get("solver") or claim.get("solver_address") or claim.get("solverAddress")
    event_solver = event.get("solver") or event.get("solver_address") or event.get("solverAddress")
    if solver and event_solver and str(solver).lower() != str(event_solver).lower():
        return f"receipt solver {event_solver!r} does not match claim solver {solver!r}"

    return None


def get_active_claim(state: dict) -> dict | None:
    if not isinstance(state, dict):
        return None
    claim = state.get("claim")
    if isinstance(claim, dict):
        if claim.get("active") is False or claim.get("status") in ("closed", "inactive", "released"):
            return None
        return claim
    if state.get("active") is False or state.get("status") in ("closed", "inactive", "released"):
        return None
    if any(k in state for k in ("bounty_id", "bountyId", "claim_id", "claimId", "evidence", "repository")):
        return state
    return None


def missing_evidence_fields(state: dict, claim: dict) -> list[str]:
    evidence = {}
    if isinstance(state.get("evidence"), dict):
        evidence.update(state["evidence"])
    if isinstance(claim.get("evidence"), dict):
        evidence.update(claim["evidence"])

    missing = []
    for key in REQUIRED_EVIDENCE:
        val = evidence.get(key)
        if val is None:
            val = claim.get(key)
        if val is None:
            val = state.get(key)
        if val is None or (isinstance(val, str) and not val.strip()):
            missing.append(key)
    return missing


def main() -> int:
    try:
        event = read_event()
        sid = session_id(event)
        state, configured, err_msg = load_claim_state(sid)

        if not configured:
            return emit("allow", "no bounty claim state configured", ALLOW)

        if state is None:
            return emit("block", err_msg or "configured claim state is unreadable or malformed", BLOCK)

        if not isinstance(state, dict):
            return emit("block", "claim state must be a JSON object", BLOCK)

        claim = get_active_claim(state)
        if claim is None:
            return emit("allow", "no active bounty claim for session", ALLOW)

        rejection = receipt_rejection(state, claim)
        if rejection is None:
            return emit("allow", "bounty claim is settled with valid on-chain receipt", ALLOW)

        missing = missing_evidence_fields(state, claim)
        if missing:
            return emit(
                "block",
                f"active claim missing required evidence fields: {', '.join(missing)}",
                BLOCK,
            )

        return emit("allow", "active claim has all required evidence", ALLOW)

    except Exception as exc:
        sys.stderr.write(f"agent-bounties-evidence hook error: {exc}\n")
        traceback.print_exc(file=sys.stderr)
        return emit("block", f"internal hook error: {type(exc).__name__}: {exc}", BLOCK)


if __name__ == "__main__":
    sys.exit(main())
