#!/usr/bin/env python3
"""Authoritative per-session claim-state producer for the OpenHands Stop hook.

The Stop hook (`.openhands/hooks/agent-bounties-evidence.py`) must not trust
whatever happens to arrive on stdin: per
https://docs.openhands.dev/openhands/usage/customization/hooks stdin carries the
OpenHands *event* payload (`event_type`, `session_id`, `working_dir`, ...), which
says nothing about bounties. This program is the state source the hook reads
instead, wired through `AGENT_BOUNTIES_STATE_CMD`:

    export AGENT_BOUNTIES_STATE_CMD="python3 -B integrations/openhands/state_producer.py"

OpenHands passes the session id to the hook; the hook appends it to this argv, so
state is always resolved *per session*.

SPLIT OF AUTHORITY
------------------
Two kinds of fact go into the decision, and they do NOT have the same weight.

  * Local work facts — which bounty this session claimed, the acceptance command
    that was run, whether it passed, the evidence fields. Only the session can
    know these, so they come from the session workfile. They can gate BLOCKING,
    which is the safe direction: a lie here can only trap the agent in more work.

  * Payment facts — whether the bounty is claimed and whether it settled. These
    come ONLY from the canonical Base event feed. A local `bounty_settled: true`
    is dropped on the floor before the hook ever sees it, so no amount of local
    tampering can produce paid language. The hook independently enforces the same
    rule, so this is defence in depth rather than the only line.

FAIL CLOSED
-----------
Any failure to establish state — missing workfile, malformed JSON, unreachable
feed, HTTP error, non-dict payload — exits non-zero with a diagnostic on stderr
and NOTHING on stdout. The hook treats a non-zero producer as "claim state is
unreadable" and blocks the stop. Never exit 0 with a guess.

WALLET SAFETY: reads no key material, signs nothing, broadcasts nothing. The
solver address is a public identifier supplied by the operator.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request

DEFAULT_API = "https://api.agentbounties.app"
EVENTS_PATH = "/v1/base/autonomous-bounties/events"
NETWORK = "base-mainnet"

# Canonical event kinds, per docs/autonomous-protocol.md. Only BountySettled
# proves payment; the others only describe claim occupancy.
CLAIMED = {"bountyclaimed", "bounty_claimed"}
RELEASED = {"claimexpired", "claim_expired", "bountyrefunded", "bounty_refunded",
            "claimreleased", "claim_released"}
SETTLED = {"bountysettled", "bounty_settled"}

# Fields that may only ever come from a canonical event, never from local input.
CANONICAL_ONLY = ("settlement", "paid")


class Unresolvable(Exception):
    """State could not be established. The caller must fail closed."""


def load_workfile(session: str) -> dict:
    """Local work facts for this session. Missing/malformed is unresolvable."""
    path = os.environ.get("AGENT_BOUNTIES_SESSION_FILE")
    if not path:
        directory = os.environ.get("AGENT_BOUNTIES_SESSION_DIR")
        if not directory:
            raise Unresolvable(
                "neither AGENT_BOUNTIES_SESSION_FILE nor AGENT_BOUNTIES_SESSION_DIR is set, "
                "so this session's work facts cannot be located"
            )
        if not session:
            raise Unresolvable(
                "AGENT_BOUNTIES_SESSION_DIR is set but no session id was supplied, so the "
                "correct workfile cannot be chosen; refusing to guess"
            )
        # Session ids come from OpenHands, but treat them as untrusted anyway:
        # a '../' would otherwise read outside the session directory.
        safe = os.path.basename(session)
        if safe != session or safe in ("", ".", ".."):
            raise Unresolvable(f"refusing unsafe session id: {session!r}")
        path = os.path.join(directory, f"{safe}.json")

    try:
        with open(path, encoding="utf-8") as handle:
            work = json.load(handle)
    except FileNotFoundError as exc:
        raise Unresolvable(f"session workfile is missing: {path}") from exc
    except json.JSONDecodeError as exc:
        raise Unresolvable(f"session workfile is malformed JSON: {path}: {exc}") from exc
    except OSError as exc:
        raise Unresolvable(f"session workfile unreadable: {path}: {exc}") from exc

    if not isinstance(work, dict):
        raise Unresolvable(f"session workfile must be a JSON object, got {type(work).__name__}")
    return work


def fetch_events(bounty_id: str) -> list:
    """Canonical Base events for one bounty. Any failure is unresolvable.

    AGENT_BOUNTIES_EVENTS_FILE points at a pre-fetched canonical snapshot for
    sandboxed runs with no egress. It is opt-in and explicit; there is no silent
    offline fallback, because silently skipping the canonical feed is exactly how
    a guard starts trusting local input.
    """
    snapshot = os.environ.get("AGENT_BOUNTIES_EVENTS_FILE")
    if snapshot:
        try:
            with open(snapshot, encoding="utf-8") as handle:
                payload = json.load(handle)
        except (OSError, json.JSONDecodeError) as exc:
            raise Unresolvable(f"canonical event snapshot unusable: {snapshot}: {exc}") from exc
    else:
        base = os.environ.get("AGENT_BOUNTIES_API", DEFAULT_API).rstrip("/")
        query = urllib.parse.urlencode({"network": NETWORK, "bounty_id": bounty_id})
        url = f"{base}{EVENTS_PATH}?{query}"
        try:
            with urllib.request.urlopen(url, timeout=15) as response:  # noqa: S310 - fixed https API
                payload = json.loads(response.read().decode("utf-8"))
        except urllib.error.HTTPError as exc:
            raise Unresolvable(f"canonical event feed returned HTTP {exc.code} for {url}") from exc
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            raise Unresolvable(f"canonical event feed unreachable: {url}: {exc}") from exc
        except json.JSONDecodeError as exc:
            raise Unresolvable(f"canonical event feed returned non-JSON: {url}: {exc}") from exc

    if isinstance(payload, dict):
        payload = payload.get("events", payload.get("data"))
    if not isinstance(payload, list):
        raise Unresolvable("canonical event feed did not return an events list")
    return payload


def receipt_of(event: dict) -> dict | None:
    """Extract a settlement receipt with real event identity, or None."""
    identity = {
        key: str(event[key]).strip()
        for key in ("event_id", "log_key", "tx_hash")
        if str(event.get(key, "")).strip()
    }
    if not identity:
        # A settlement event with no identity is not a receipt. Treat it as
        # unresolvable rather than quietly downgrading to "unpaid": the feed is
        # behaving in a way this producer does not understand.
        raise Unresolvable("canonical settlement event carries no event_id/log_key/tx_hash")
    identity["kind"] = "BountySettled"
    return identity


def reduce_events(events: list, solver: str) -> tuple[bool, dict | None]:
    """Fold canonical events into (claim_active, settlement_receipt).

    Events are applied in feed order. A settlement always ends the claim.
    """
    active = False
    receipt = None
    wanted = solver.lower() if solver else ""
    for event in events:
        if not isinstance(event, dict):
            raise Unresolvable("canonical event feed contains a non-object entry")
        kind = str(event.get("kind") or event.get("type") or event.get("event") or "").lower()
        who = str(event.get("solver") or event.get("claimant") or "").lower()
        if wanted and who and who != wanted:
            continue  # someone else's claim on the same bounty
        if kind in CLAIMED:
            active = True
        elif kind in RELEASED:
            active = False
        elif kind in SETTLED:
            active = False
            receipt = receipt_of(event)
    return active, receipt


def build_state(session: str) -> dict:
    work = load_workfile(session)

    # Local input can never inject settlement. Strip it before anything else so
    # there is no path, however convoluted, from the workfile to paid language.
    for field in CANONICAL_ONLY:
        work.pop(field, None)
    submission = dict(work.get("submission") or {})
    submission.pop("bounty_settled", None)

    bounty_id = str(work.get("bounty_id", "")).strip()
    if not bounty_id:
        # No bounty in this session's workfile means no claim to protect. Say so
        # explicitly rather than omitting claim.active, which the hook blocks on.
        return {"claim": {"active": False}, "source": "session workfile: no bounty_id"}

    solver = str(work.get("solver") or os.environ.get("AGENT_BOUNTIES_SOLVER", "")).strip()
    active, receipt = reduce_events(fetch_events(bounty_id), solver)

    state: dict = {
        "claim": {
            "active": active,
            "bounty_id": bounty_id,
            "bounty_contract": str(work.get("bounty_contract", "")).strip(),
        },
        "test": work.get("test") or {},
        "evidence": work.get("evidence") or {},
        "submission": submission,
        "source": "claim/settlement from canonical Base events; work facts from session workfile",
    }
    if receipt:
        state["settlement"] = {"canonical_event": receipt}
    return state


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
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
