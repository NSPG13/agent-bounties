#!/usr/bin/env python3
"""Smoke-test Hermes earning integration."""
from __future__ import annotations
import json,os,sys
from pathlib import Path
ROOT=Path(os.environ.get("WORKSPACE_ROOT","/workspace"))

def check(path,msg):
 p=ROOT/path
 if not p.is_file():
  raise SystemExit(f"missing {path}: {msg}")
 return p

skill=check("skills/agent-bounties/SKILL.md","canonical skill").read_text(encoding="utf-8")
assert skill.startswith("---\n"),"YAML frontmatter required"
closing=skill.find("\n---\n",4)
assert 0<closing<2000,f"frontmatter too long: {closing}"

readme=check("integrations/hermes/README.md","Hermes README").read_text(encoding="utf-8")
cmd="hermes skills install https://raw.githubusercontent.com/NSPG13/agent-bounties/main/skills/agent-bounties/SKILL.md"
assert cmd in readme,"missing install command"
assert "--now" in readme or "/reset" in readme,"missing fresh-session activation"

for name in ["claimable.json","unfunded.json","stale.json"]:
 f=ROOT/"integrations/hermes/fixtures"/name
 assert f.is_file(),f"missing fixture: {name}"
 data=json.loads(f.read_text())
 assert "next_action" in data,f"{name} missing next_action"
 assert isinstance(data["next_action"],str) and data["next_action"],f"{name} next_action empty"

print("PASS: Hermes integration smoke test")
