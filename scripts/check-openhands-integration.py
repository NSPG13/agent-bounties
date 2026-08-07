#!/usr/bin/env python3
"""Smoke-test OpenHands earning integration."""
from __future__ import annotations
import json,os,sys
from pathlib import Path
ROOT=Path(os.environ.get("WORKSPACE_ROOT","/workspace"))

def check(path):
 p=ROOT/path
 if not p.is_file():
  raise SystemExit(f"missing {path}")
 return p

skill=check(".agents/skills/agent-bounties/SKILL.md").read_text(encoding="utf-8").lower()
for phrase in ["canonical","claimable","bountysettled","one exact next action","post your own bounty"]:
 assert phrase in skill,f"missing: {phrase}"

hooks=json.loads(check(".openhands/hooks.json").read_text())
stop=hooks.get("stop")
assert isinstance(stop,list) and stop,"stop hook required"
cmds=json.dumps(stop,sort_keys=True)
assert ".openhands/hooks/agent-bounties-evidence" in cmds,"evidence guard not in stop hooks"

guard=check(".openhands/hooks/agent-bounties-evidence.py").read_text(encoding="utf-8").lower()
for phrase in ["submission","evidence","test","decision","deny"]:
 assert phrase in guard,f"missing in guard: {phrase}"

for forbidden in ["private_key","seed phrase","eth_sendtransaction"]:
 assert forbidden not in (skill+guard),f"forbidden: {forbidden}"

print("PASS: OpenHands integration smoke test")
