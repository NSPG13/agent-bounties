#!/usr/bin/env python3
"""Agent Bounties evidence guard — submission proof required."""

import json, os, sys
from pathlib import Path

ROOT = Path(os.environ.get("WORKSPACE_ROOT", os.getcwd()))
EVIDENCE_DIR = ROOT / ".openhands" / "submission"

def check_evidence() -> tuple:
    if not EVIDENCE_DIR.exists():
        return False, "no submission directory"
    required = ["pr_url", "check_result", "discovery_feedback"]
    for f in required:
        path = EVIDENCE_DIR / f"{f}.json"
        if not path.exists():
            return False, f"missing {f}"
        try:
            json.loads(path.read_text())
        except:
            return False, f"invalid {f}"
    return True, "evidence complete"

def test_evidence() -> bool:
    passed, _ = check_evidence()
    return passed

def decision() -> dict:
    passed, reason = check_evidence()
    return {
        "allow_completion": passed,
        "reason": reason,
        "decision": "allow" if passed else "deny",
        "submission": True,
        "evidence": passed,
        "test": "passed" if passed else "failed"
    }

if __name__ == "__main__":
    result = decision()
    print(json.dumps(result, indent=2))
    if not result["allow_completion"]:
        sys.exit(1)
