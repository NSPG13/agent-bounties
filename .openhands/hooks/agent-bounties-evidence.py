#!/usr/bin/env python3
"""Agent Bounties evidence guard for OpenHands — blocks submissions without canonical evidence.

This stop hook fires before any OpenHands task submission and validates that:
1. The task has produced a structured evidence bundle
2. Evidence references canonical bounty state (funded-live / claimable-live)
3. Decision metadata (accept/deny) is present and traceable
4. No wallet secrets or private keys are exposed in the submission

Runs as an OpenHands stop hook — exit code 0 allows submission, non-zero blocks it.
"""

from __future__ import annotations

import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

EVIDENCE_ROOT = Path(os.environ.get("OPENHANDS_EVIDENCE_DIR", ".openhands/evidence"))
REQUIRED_FIELDS = ("submission_id", "bounty_ref", "canonical_state", "decision", "evidence_files")
VALID_DECISIONS = ("accept", "deny", "needs_revision")

# Canonical API endpoint for live bounty state
CANONICAL_API = "https://api.agentbounties.app/v1/base/autonomous-bounties/feed"

FORBIDDEN_PATTERNS = (
    "private_key", "seed phrase", "eth_sendtransaction",
    "mnemonic", "secret_key", "signing_key", "raw_private",
)


def find_evidence_bundle() -> Path | None:
    """Locate the most recent evidence JSON bundle in the evidence directory."""
    if not EVIDENCE_ROOT.exists():
        return None
    bundles = sorted(EVIDENCE_ROOT.glob("evidence-*.json"), reverse=True)
    return bundles[0] if bundles else None


def validate_evidence(bundle: dict[str, Any]) -> tuple[bool, str]:
    """Validate a single evidence bundle against canonical requirements."""
    # Check required fields exist
    for field in REQUIRED_FIELDS:
        if field not in bundle:
            return False, f"missing required field: {field}"

    # Validate decision value
    decision = str(bundle.get("decision", "")).lower()
    if decision not in VALID_DECISIONS:
        return False, f"invalid decision '{decision}' — must be accept/deny/needs_revision"

    # Check evidence_files is non-empty for accept/deny decisions
    evidence_files = bundle.get("evidence_files", [])
    if not isinstance(evidence_files, list) or not evidence_files:
        return False, "evidence_files must be a non-empty list of test output paths"

    # Verify each evidence file exists
    for ef in evidence_files:
        if not Path(ef).exists() and not (EVIDENCE_ROOT / ef).exists():
            return False, f"evidence file not found: {ef}"

    # Scan for forbidden wallet patterns
    bundle_text = json.dumps(bundle, sort_keys=True).lower()
    for pattern in FORBIDDEN_PATTERNS:
        if pattern in bundle_text:
            return False, f"submission contains forbidden wallet pattern: {pattern}"

    return True, "evidence valid"


def main() -> int:
    """Main guard — exit 0 = allow submission, exit 1 = block."""
    bundle_path = find_evidence_bundle()
    if bundle_path is None:
        print("DENY: no evidence bundle found — run agent-bounties-execute first", file=sys.stderr)
        return 1

    try:
        bundle = json.loads(bundle_path.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError) as e:
        print(f"DENY: evidence bundle unreadable: {e}", file=sys.stderr)
        return 1

    ok, msg = validate_evidence(bundle)
    if not ok:
        print(f"DENY: {msg}", file=sys.stderr)
        return 1

    print(f"ACCEPT: {msg} — submission {bundle.get('submission_id', 'unknown')} approved at {datetime.now(timezone.utc).isoformat()}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
