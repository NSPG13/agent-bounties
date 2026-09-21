```python
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
# settlement token can settle, plus nano mainnet for cross-operator Nano/XNO settlements. See docs/autonomous-protocol.md.
CANONICAL_NETWORKS = {"base-mainnet", "nano:mainnet"}

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
        or event.get("id")
        or ""
    ).strip()
```
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
# settlement token can settle, plus nano mainnet for cross-operator Nano/XNO settlements. See docs/autonomous-protocol.md.
CANONICAL_NETWORKS = {"base-mainnet", "nano:mainnet"}

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
        or event.get("id")
        or ""
    ).strip()
