#!/usr/bin/env python3
"""Authoritative per-session claim-state producer for the OpenHands Stop hook.

The Stop hook (`.openhands/hooks/agent-bounties-evidence.py`) must not trust
whatever happens to arrive on stdin: per
https://docs.openhands.dev/openhands/usage/customization/hooks stdin carries the
OpenHands *event* payload (`event_type`, `tool_name`, `session_id`,
`working_dir`), which says nothing about bounties. This program is the state
source the hook reads instead, wired through `AGENT_BOUNTIES_STATE_CMD`.

OpenHands passes the session id to the hook; the hook appends it to this argv, so
state is always resolved *per session*.

SPLIT OF AUTHORITY
------------------
Three kinds of fact go into the decision. They do NOT have the same weight, and
they come from three different places on purpose.

  1. Claim IDENTITY -- which bounty, which contract, which round, which solver,
     which network this session is allowed to be working on. This comes ONLY
     from the operator binding file (`AGENT_BOUNTIES_BINDING_FILE`), which lives
     outside the session-writable area. The session workfile may not supply,
     override, or erase it. This is what stops "delete the workfile and the
     guard decides there is no claim".

  2. Claim OCCUPANCY and SETTLEMENT -- whether the bound round is claimed and
     whether it settled. These come ONLY from canonical Base events fetched live
     from the API, validated against the bound network, bounty id, bounty
     contract, round and solver. A local file can never produce a settlement
     receipt (see PROVENANCE below).

  3. Local WORK facts -- the acceptance command, whether it passed, the evidence
     fields, whether a submission was broadcast. Only the session can know
     these, so they come from the session workfile. They can gate BLOCKING only,
     which is the safe direction: a lie here can only trap the agent in more
     work. Absent work facts therefore block; they never allow.

PROVENANCE
----------
`AGENT_BOUNTIES_EVENTS_FILE` is a TEST-ONLY offline snapshot. It is refused
unless `AGENT_BOUNTIES_ALLOW_TEST_SNAPSHOT=1` is also set, and even then it is
marked `provenance: test_snapshot` and can NEVER produce a settlement receipt --
it may only establish claim occupancy, which is the blocking direction. Paid
language requires `provenance: canonical_live`: an event fetched live in this
very invocation from the canonical API, carrying `tx_hash`, `log_key` and a
non-zero `block_number`, with a sane `occurred_at`, bound to the configured
network, bounty id, bounty contract, round and solver.

FAIL CLOSED
-----------
Any failure to establish state -- no binding, unknown session, malformed JSON,
unreachable feed, an event for the wrong bounty/round/solver, a settlement with
no identity -- exits non-zero with a diagnostic on stderr and NOTHING on stdout.
The hook treats a non-zero producer as "claim state is unreadable" and blocks the
stop. Never exit 0 with a guess.

TRUST BOUNDARY, STATED HONESTLY
-------------------------------
The binding file is operator-owned by *provenance*, not by cryptography. This
program enforces what it actually can: the binding must be configured out-of-band
by the operator, must not live inside the session-writable directory, and on
POSIX must not be group- or world-writable. It does not and cannot prove that a
sufficiently privileged process inside the sandbox never touched it. What it does
guarantee is that nothing the *documented session workflow* writes -- the
workfile -- can change claim identity or manufacture payment.

WALLET SAFETY: reads no key material, signs nothing, broadcasts nothing. The
solver address is a public identifier supplied by the operator.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import stat
import sys
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timedelta, timezone

DEFAULT_API = "https://api.agentbounties.app"
EVENTS_PATH = "/v1/base/autonomous-bounties/events"

BINDING_SCHEMA = "agent-bounties/openhands-session-binding/v1"

# Canonical settlement networks. `AgentBountyFactory` is deployed once per
# supported network with one immutable settlement token; on Base mainnet that is
# native USDC (docs/autonomous-protocol.md). Anything else is not canonical
# settlement and must not be able to produce paid language.
CANONICAL_NETWORKS = {"base-mainnet"}

# Canonical event kinds, serialized snake_case by
# `AutonomousBountyEventKind` in crates/chain-base/src/lib.rs.
CLAIMED = {"bounty_claimed"}
RELEASED = {"claim_expired", "submission_expired", "bounty_cancelled",
            "refund_withdrawn", "submission_rejected"}
SETTLED = {"bounty_settled"}
# Kinds that are scoped to a specific round and therefore must match the bound
# round exactly. Lifecycle events for the bounty as a whole are not.
ROUND_SCOPED = CLAIMED | SETTLED | {"submission_added", "submission_rejected",
                                    "claim_expired", "submission_expired"}

ADDRESS_RE = re.compile(r"^0x[0-9a-fA-F]{40}$")
BOUNTY_ID_RE = re.compile(r"^0x[0-9a-fA-F]{64}$")

# Tolerance for clock skew between this machine and the indexer. A canonical
# event stamped materially in the future is not evidence of anything.
FUTURE_SKEW = timedelta(minutes=5)


class Unresolvable(Exception):
    """State could not be established. The caller must fail closed."""


# --------------------------------------------------------------------------
# Operator binding: the only source of claim identity.
# --------------------------------------------------------------------------

def _read_json(path: str, what: str):
    try:
        with open(path, encoding="utf-8") as handle:
            return json.load(handle)
    except FileNotFoundError as exc:
        raise Unresolvable(f"{what} is missing: {path}") from exc
    except json.JSONDecodeError as exc:
        raise Unresolvable(f"{what} is malformed JSON: {path}: {exc}") from exc
    except OSError as exc:
        raise Unresolvable(f"{what} is unreadable: {path}: {exc}") from exc


def _safe_session(session: str) -> str:
    if not session:
        raise Unresolvable("no session id was supplied, so no session binding can be "
                           "resolved; refusing to guess")
    # Session ids come from OpenHands, but treat them as untrusted anyway: a
    # '../' would otherwise escape the session directory.
    safe = os.path.basename(session)
    if safe != session or safe in ("", ".", "..") or os.path.isabs(session):
        raise Unresolvable(f"refusing unsafe session id: {session!r}")
    return safe


def _assert_operator_owned(binding_path: str) -> None:
    """Refuse a binding that sits in, or is writable from, the session's own area."""
    binding_real = os.path.realpath(binding_path)
    session_dir = os.environ.get("AGENT_BOUNTIES_SESSION_DIR")
    if session_dir:
        session_real = os.path.realpath(session_dir)
        if binding_real == session_real or binding_real.startswith(session_real + os.sep):
            raise Unresolvable(
                f"session binding {binding_path} lives inside the session-writable "
                f"directory {session_dir}; claim identity must be operator-owned"
            )
    if os.name != "nt":
        try:
            mode = os.stat(binding_real).st_mode
        except OSError as exc:
            raise Unresolvable(f"session binding is unreadable: {binding_path}: {exc}") from exc
        if mode & (stat.S_IWGRP | stat.S_IWOTH):
            raise Unresolvable(
                f"session binding {binding_path} is group/world-writable "
                f"(mode {oct(stat.S_IMODE(mode))}); refusing to treat it as operator-owned"
            )


def load_binding(session: str) -> dict:
    """Resolve this session's operator-declared claim identity.

    Returns either {"claim": None} (operator says this session holds no claim) or
    a fully validated identity dict. Anything unknown raises Unresolvable.
    """
    path = os.environ.get("AGENT_BOUNTIES_BINDING_FILE")
    if not path:
        raise Unresolvable(
            "AGENT_BOUNTIES_BINDING_FILE is not set, so this session's claim identity "
            "cannot be established from operator-owned configuration"
        )
    _assert_operator_owned(path)
    binding = _read_json(path, "session binding")
    if not isinstance(binding, dict):
        raise Unresolvable("session binding must be a JSON object")
    if binding.get("schema_version") != BINDING_SCHEMA:
        raise Unresolvable(
            f"session binding schema_version must be {BINDING_SCHEMA!r}, "
            f"got {binding.get('schema_version')!r}"
        )

    network = str(binding.get("network", "")).strip().lower()
    if network not in CANONICAL_NETWORKS:
        raise Unresolvable(
            f"session binding network {network!r} is not a canonical settlement network "
            f"({sorted(CANONICAL_NETWORKS)})"
        )

    sessions = binding.get("sessions")
    if not isinstance(sessions, dict):
        raise Unresolvable("session binding must carry a 'sessions' object")

    safe = _safe_session(session)
    if safe not in sessions:
        raise Unresolvable(
            f"session {safe!r} is not declared in the operator session binding, so an "
            f"active claim cannot be ruled out; declare it (with \"claim\": \"none\" if "
            f"it holds no claim) before finishing"
        )
    entry = sessions[safe]
    if not isinstance(entry, dict):
        raise Unresolvable(f"session binding entry for {safe!r} must be an object")

    if str(entry.get("claim", "")).strip().lower() == "none":
        return {"claim": None, "network": network}

    bounty_id = str(entry.get("bounty_id", "")).strip()
    if not BOUNTY_ID_RE.match(bounty_id):
        raise Unresolvable(
            f"session binding entry for {safe!r} has no valid canonical bounty_id "
            f"(expected 0x + 64 hex, got {bounty_id!r}); use \"claim\": \"none\" to "
            f"declare a session with no claim"
        )
    contract = str(entry.get("bounty_contract", "")).strip()
    if not ADDRESS_RE.match(contract):
        raise Unresolvable(
            f"session binding entry for {safe!r} has no valid bounty_contract address, "
            f"got {contract!r}"
        )
    solver = str(entry.get("solver") or binding.get("solver") or "").strip()
    if not ADDRESS_RE.match(solver):
        raise Unresolvable(
            f"session binding for {safe!r} has no valid solver address, got {solver!r}"
        )
    round_raw = entry.get("round")
    if not isinstance(round_raw, int) or isinstance(round_raw, bool) or round_raw < 0:
        raise Unresolvable(
            f"session binding entry for {safe!r} has no valid integer round, "
            f"got {round_raw!r}"
        )

    return {
        "claim": {
            "network": network,
            "bounty_id": bounty_id.lower(),
            "bounty_contract": contract.lower(),
            "solver": solver.lower(),
            "round": round_raw,
        },
        "network": network,
    }


# --------------------------------------------------------------------------
# Local work facts: blocking-direction only.
# --------------------------------------------------------------------------

# The ONLY keys the session workfile is allowed to contribute. Claim identity and
# anything settlement-shaped is not on this list and is therefore dropped, so no
# workfile edit can widen the guard's view of the world.
WORK_KEYS = ("test", "evidence")


def load_work_facts(session: str) -> dict:
    """Local work facts. A missing workfile is empty, which BLOCKS; it never allows."""
    path = os.environ.get("AGENT_BOUNTIES_SESSION_FILE")
    if not path:
        directory = os.environ.get("AGENT_BOUNTIES_SESSION_DIR")
        if not directory:
            return {}
        path = os.path.join(directory, f"{_safe_session(session)}.json")

    if not os.path.exists(path):
        # Deliberately not fatal. With identity now coming from the operator
        # binding, an absent workfile simply means "no work recorded", which the
        # hook turns into a deny. Failing hard here would let a deleted file be
        # indistinguishable from a broken producer.
        return {}

    work = _read_json(path, "session workfile")
    if not isinstance(work, dict):
        raise Unresolvable(f"session workfile must be a JSON object, got {type(work).__name__}")

    facts = {key: work[key] for key in WORK_KEYS if key in work}
    submission = work.get("submission")
    if isinstance(submission, dict):
        # submitted_onchain is a blocking-direction fact: asserting it only moves
        # the agent one step further into "awaiting the verifier", which still
        # reports $0.00. Every settlement-shaped key is dropped.
        facts["submission"] = {"submitted_onchain": submission.get("submitted_onchain") is True}
    else:
        facts["submission"] = {"submitted_onchain": False}
    return facts


# --------------------------------------------------------------------------
# Canonical events.
# --------------------------------------------------------------------------

def fetch_events(claim: dict) -> tuple[list, str]:
    """Return (events, provenance) for the bound bounty on the bound network.

    provenance is "canonical_live" for a live fetch and "test_snapshot" for the
    opt-in offline file. Only canonical_live may ever settle.
    """
    snapshot = os.environ.get("AGENT_BOUNTIES_EVENTS_FILE")
    if snapshot:
        if os.environ.get("AGENT_BOUNTIES_ALLOW_TEST_SNAPSHOT") != "1":
            raise Unresolvable(
                "AGENT_BOUNTIES_EVENTS_FILE is a test-only offline snapshot and is refused "
                "unless AGENT_BOUNTIES_ALLOW_TEST_SNAPSHOT=1; canonical state must come "
                "from the live canonical feed"
            )
        payload = _read_json(snapshot, "canonical event snapshot")
        provenance = "test_snapshot"
    else:
        base = os.environ.get("AGENT_BOUNTIES_API", DEFAULT_API).rstrip("/")
        parsed = urllib.parse.urlsplit(base)
        if parsed.scheme not in ("http", "https"):
            raise Unresolvable(f"AGENT_BOUNTIES_API must be an http(s) URL, got {base!r}")
        query = urllib.parse.urlencode(
            {"network": claim["network"], "bounty_id": claim["bounty_id"]}
        )
        url = f"{base}{EVENTS_PATH}?{query}"
        try:
            with urllib.request.urlopen(url, timeout=15) as response:  # noqa: S310 - scheme checked
                payload = json.loads(response.read().decode("utf-8"))
        except urllib.error.HTTPError as exc:
            raise Unresolvable(f"canonical event feed returned HTTP {exc.code} for {url}") from exc
        except json.JSONDecodeError as exc:
            raise Unresolvable(f"canonical event feed returned non-JSON: {url}: {exc}") from exc
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            raise Unresolvable(f"canonical event feed unreachable: {url}: {exc}") from exc
        provenance = "canonical_live"

    if isinstance(payload, dict):
        payload = payload.get("events", payload.get("data"))
    if not isinstance(payload, list):
        raise Unresolvable("canonical event feed did not return an events list")
    return payload, provenance


def _occurred_at(event: dict) -> datetime:
    raw = str(event.get("occurred_at", "")).strip()
    if not raw:
        raise Unresolvable("canonical event carries no occurred_at timestamp")
    try:
        moment = datetime.fromisoformat(raw.replace("Z", "+00:00"))
    except ValueError as exc:
        raise Unresolvable(f"canonical event has an unparseable occurred_at {raw!r}") from exc
    if moment.tzinfo is None:
        moment = moment.replace(tzinfo=timezone.utc)
    if moment > datetime.now(timezone.utc) + FUTURE_SKEW:
        raise Unresolvable(
            f"canonical event occurred_at {raw!r} is in the future; refusing to treat it as evidence"
        )
    return moment


def bind_event(event, claim: dict) -> tuple[str, dict]:
    """Validate one feed entry against the bound identity. Returns (kind, event)."""
    if not isinstance(event, dict):
        raise Unresolvable("canonical event feed contains a non-object entry")

    kind = str(event.get("kind") or "").strip().lower()
    if not kind:
        raise Unresolvable("canonical event carries no kind")

    bounty_id = str(event.get("bounty_id", "")).strip().lower()
    if not bounty_id:
        raise Unresolvable(f"canonical {kind} event carries no bounty_id")
    if bounty_id != claim["bounty_id"]:
        # The feed was asked for exactly one bounty. A foreign bounty in the
        # response means the view is wrong or the payload was assembled locally.
        raise Unresolvable(
            f"canonical event feed returned bounty_id {bounty_id} but this session is "
            f"bound to {claim['bounty_id']}; refusing a cross-bounty payload"
        )

    contract = str(event.get("contract_address", "")).strip().lower()
    if contract and contract != claim["bounty_contract"]:
        raise Unresolvable(
            f"canonical event contract_address {contract} does not match the bound "
            f"bounty contract {claim['bounty_contract']}"
        )

    data = event.get("data")
    data = data if isinstance(data, dict) else {}
    if kind in ROUND_SCOPED:
        round_raw = data.get("round")
        if not isinstance(round_raw, int) or isinstance(round_raw, bool):
            raise Unresolvable(f"canonical {kind} event carries no integer data.round")
        if round_raw != claim["round"]:
            # Not an error in the feed -- an earlier or later round simply is not
            # this session's round, so it must not move this session's state.
            return "", event
        solver = str(data.get("solver", "")).strip().lower()
        if solver and solver != claim["solver"]:
            if kind in SETTLED:
                # A settlement on OUR bound round paid to someone else directly
                # contradicts the binding: either the operator bound the wrong
                # round/solver, or this round was taken over. Either way the
                # session's understanding of the world is wrong, so fail closed
                # rather than quietly reporting "unpaid, keep working".
                raise Unresolvable(
                    f"canonical BountySettled on bound round {claim['round']} paid {solver}, "
                    f"not the bound solver {claim['solver']}; the session binding does not "
                    f"match canonical state"
                )
            return "", event  # someone else occupying this round
    return kind, event


def settlement_receipt(event: dict, claim: dict, provenance: str) -> dict:
    """Build a settlement receipt, or refuse. Only canonical_live may settle."""
    if provenance != "canonical_live":
        raise Unresolvable(
            "a BountySettled event was present in a test snapshot; offline snapshots are "
            "test-only and can never be payment evidence. Re-run against the live "
            "canonical feed."
        )
    raw_data = event.get("data")
    data: dict = raw_data if isinstance(raw_data, dict) else {}
    solver = str(data.get("solver", "")).strip().lower()
    if solver != claim["solver"]:
        raise Unresolvable(
            f"canonical BountySettled paid {solver or '(unknown)'}, not the bound solver "
            f"{claim['solver']}"
        )
    tx_hash = str(event.get("tx_hash", "")).strip()
    log_key = str(event.get("log_key", "")).strip()
    block_number = event.get("block_number")
    if not tx_hash or not log_key:
        raise Unresolvable("canonical BountySettled event carries no tx_hash/log_key identity")
    if not isinstance(block_number, int) or isinstance(block_number, bool) or block_number <= 0:
        raise Unresolvable(
            f"canonical BountySettled event has no positive block_number, got {block_number!r}"
        )
    observed = _occurred_at(event)
    return {
        "kind": "BountySettled",
        "provenance": provenance,
        "network": claim["network"],
        "bounty_id": claim["bounty_id"],
        "bounty_contract": claim["bounty_contract"],
        "round": claim["round"],
        "solver": claim["solver"],
        "tx_hash": tx_hash,
        "log_key": log_key,
        "block_number": block_number,
        "occurred_at": observed.isoformat(),
        "event_id": str(event.get("id", "")).strip(),
    }


def reduce_events(events: list, claim: dict, provenance: str) -> tuple[bool, dict | None]:
    """Fold canonical events into (claim_active, settlement_receipt)."""
    active = False
    receipt = None
    for raw in events:
        kind, event = bind_event(raw, claim)
        if not kind:
            continue
        if kind in CLAIMED:
            active = True
        elif kind in SETTLED:
            active = False
            receipt = settlement_receipt(event, claim, provenance)
        elif kind in RELEASED:
            active = False
    return active, receipt


# --------------------------------------------------------------------------

def build_state(session: str) -> dict:
    binding = load_binding(session)
    claim = binding["claim"]

    if claim is None:
        return {
            "claim": {"active": False},
            "network": binding["network"],
            "provenance": "operator_binding",
            "source": "operator session binding declares no claim for this session",
        }

    work = load_work_facts(session)
    events, provenance = fetch_events(claim)
    active, receipt = reduce_events(events, claim, provenance)

    state: dict = {
        "claim": {
            "active": active,
            "network": claim["network"],
            "bounty_id": claim["bounty_id"],
            "bounty_contract": claim["bounty_contract"],
            "round": claim["round"],
            "solver": claim["solver"],
        },
        "network": claim["network"],
        "provenance": provenance,
        "test": work.get("test") or {},
        "evidence": work.get("evidence") or {},
        "submission": work.get("submission") or {"submitted_onchain": False},
        "source": (
            "identity from the operator session binding; claim/settlement from canonical "
            f"Base events ({provenance}); work facts from the session workfile"
        ),
    }
    if receipt:
        state["settlement"] = {"canonical_event": receipt}
    return state


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="Agent Bounties OpenHands state producer")
    parser.add_argument("session", nargs="?", default=os.environ.get("OPENHANDS_SESSION_ID", ""))
    args = parser.parse_args(argv)
    try:
        print(json.dumps(build_state(args.session)))
    except Unresolvable as exc:
        # Nothing on stdout: the hook must not be able to parse a partial answer.
        print(f"agent-bounties state producer: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
