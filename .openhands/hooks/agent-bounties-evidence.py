#!/usr/bin/env python3
"""Agent Bounties evidence guard — blocks completion until submission evidence exists."""
from __future__ import annotations

import json
import sys
from pathlib import Path

EVIDENCE_FILE = Path("/workspace/evidence.json")
REQUIRED_FIELDS = {"submission", "evidence", "test", "decision"}


def check_evidence() -> dict:
    """Check if all required evidence fields are present."""
    if not EVIDENCE_FILE.is_file():
        return {"decision": "deny", "reason": "No evidence file found — run focused checks first"}

    try:
        data = json.loads(EVIDENCE_FILE.read_text(encoding="utf-8"))
    except json.JSONDecodeError as e:
        return {"decision": "deny", "reason": f"Invalid evidence JSON: {e}"}

    missing = REQUIRED_FIELDS - set(data.keys())
    if missing:
        return {
            "decision": "deny",
            "reason": f"Missing evidence fields: {', '.join(sorted(missing))}",
            "provided": sorted(data.keys())
        }

    # Validate submission evidence
    submission = data.get("submission", {})
    if not isinstance(submission, dict):
        return {"decision": "deny", "reason": "submission must be an object"}

    required_sub = {"repository_url", "commit_hash", "snapshot_digest"}
    missing_sub = required_sub - set(submission.keys())
    if missing_sub:
        return {
            "decision": "deny",
            "reason": f"Missing submission fields: {', '.join(sorted(missing_sub))}"
        }

    # Validate test evidence
    test = data.get("test", {})
    if not test.get("passed", False):
        return {"decision": "deny", "reason": "Tests have not passed — fix and re-run"}

    return {
        "decision": "allow",
        "reason": "All evidence checks passed",
        "submission": submission,
        "test_summary": test.get("summary", "")
    }


def main() -> int:
    result = check_evidence()
    print(json.dumps(result, indent=2))
    if result["decision"] == "deny":
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
