#!/usr/bin/env python3
"""Agent Bounties evidence guard."""
from __future__ import annotations
import json,os,sys
from pathlib import Path

WORKSPACE=Path(os.environ.get("WORKSPACE_ROOT","/workspace"))
EVIDENCE_FILE=WORKSPACE/".agent-bounties-evidence.json"

def decision(allow,reason):
 print(json.dumps({"allow_completion":allow,"reason":reason}))
 sys.exit(0 if allow else 1)

if not EVIDENCE_FILE.is_file():
 decision(False,"deny: no submission evidence file")

try:
 evidence=json.loads(EVIDENCE_FILE.read_text())
except json.JSONDecodeError:
 decision(False,"deny: invalid JSON evidence")

required=["bounty_id","test_output","submission_url"]
missing=[k for k in required if k not in evidence]
if missing:
 decision(False,f"deny: missing evidence fields: {missing}")

test_output=evidence.get("test_output","")
if "PASS" not in test_output and "pass" not in test_output.lower():
 decision(False,"deny: tests not passed - run benchmark check.py first")

print(json.dumps({"allow_completion":True,"reason":"submission evidence verified","bounty_id":evidence["bounty_id"],"test_output":test_output[:200]}))
