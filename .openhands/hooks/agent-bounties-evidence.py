#!/usr/bin/env python3
"""Agent Bounties evidence guard — an OpenHands stop hook.

Runs when the agent tries to finish. It blocks completion while a claimed bounty
still lacks the evidence needed to settle, so a session cannot end with the bond
posted, the work done, and nothing submitted.

Decision protocol (stdout JSON):
    {"decision": "allow"}                     nothing to block on
    {"decision": "deny", "reason": "..."}     stop; the reason states one exact next action

Reads optional state from AGENT_BOUNTIES_STATE (path to JSON) or stdin.
Fails OPEN on unreadable input: a broken guard must not wedge every session.

WALLET SAFETY: this guard never reads, stores or transmits secret key material,
and never broadcasts a transaction.
"""

from __future__ import annotations

import json
import os
import sys

# Evidence fields the canonical submission schema requires.
REQUIRED_EVIDENCE = (
    "repository",
    "commit",
    "test_command",
    "source_snapshot_digest",
    "discovery_source",
    "participation_reason",
    "improvement_feedback",
)


def load_state():
    """Load session state from env-pointed file or stdin. Never raises."""
    path = os.environ.get("AGENT_BOUNTIES_STATE")
    if path:
        try:
            with open(path, encoding="utf-8") as handle:
                return json.load(handle)
        except (OSError, json.JSONDecodeError):
            return None
    if not sys.stdin.isatty():
        try:
            raw = sys.stdin.read().strip()
            return json.loads(raw) if raw else None
        except (json.JSONDecodeError, OSError):
            return None
    return None


def decide(state):
    """Return an allow/deny decision with one exact next action on deny."""
    if not isinstance(state, dict):
        # Fail open: no state means no claimed bounty to protect.
        return {"decision": "allow", "reason": "no bounty state available"}

    claim = state.get("claim") or {}
    if not claim.get("active"):
        return {"decision": "allow", "reason": "no active claim"}

    contract = claim.get("bounty_contract", "<bounty_contract>")

    # 1. Work must actually be tested before it can be submitted.
    test = state.get("test") or {}
    if not test.get("command"):
        return {
            "decision": "deny",
            "reason": (
                "An active claim exists but no test command was recorded. "
                "Next action: run the bounty's acceptance check "
                "(python /benchmark/check.py) and record the exact command."
            ),
        }
    if test.get("passed") is not True:
        return {
            "decision": "deny",
            "reason": (
                f"The acceptance check has not passed (command: {test.get('command')}). "
                "Next action: fix the failing criterion and re-run that exact command "
                "before submitting."
            ),
        }

    # 2. Evidence must be complete, or the submission cannot be verified.
    evidence = state.get("evidence") or {}
    missing = [field for field in REQUIRED_EVIDENCE if not str(evidence.get(field, "")).strip()]
    if missing:
        return {
            "decision": "deny",
            "reason": (
                f"Evidence is incomplete; missing: {', '.join(missing)}. "
                "Next action: populate every required evidence field, computing "
                "source_snapshot_digest with "
                "`git ls-files -z | sort -z | xargs -0 sha256sum | sha256sum`."
            ),
        }

    # 3. The submission must be recorded on-chain.
    submission = state.get("submission") or {}
    if not submission.get("submitted_onchain"):
        return {
            "decision": "deny",
            "reason": (
                "Work is tested and evidence is complete, but no submission is on-chain. "
                f"Next action: call submit(bytes32,bytes32) on {contract} with the "
                "submission and evidence hashes, then publish the evidence."
            ),
        }

    # 4. Never let the session claim payment without canonical proof.
    if not submission.get("bounty_settled"):
        return {
            "decision": "allow",
            "reason": (
                "Submission is on-chain and awaiting the verifier. Payment is NOT "
                "confirmed: only a canonical BountySettled event proves payment. "
                "Report $0.00 earned until that event exists."
            ),
        }

    return {"decision": "allow", "reason": "BountySettled confirmed; work is paid"}


def main():
    result = decide(load_state())
    print(json.dumps(result))
    # Exit 0 always: the decision travels in the payload, not the exit code, so a
    # deny is a normal, parseable outcome rather than a crash.
    return 0


if __name__ == "__main__":
    sys.exit(main())
