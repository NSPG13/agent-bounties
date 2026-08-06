#!/usr/bin/env python3
"""Immutable acceptance check for the A2A Agent Card bounty."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path


ROOT = Path(os.environ.get("WORKSPACE_ROOT", "/workspace"))


def require(path: str) -> Path:
    candidate = ROOT / path
    if not candidate.is_file():
        raise SystemExit(f"missing required file: {path}")
    return candidate


def text(path: str) -> str:
    return require(path).read_text(encoding="utf-8")


fixture = json.loads(text("fixtures/a2a-agent-card.json"))
if fixture.get("name") != "Agent Bounties":
    raise SystemExit("Agent Card must use the canonical product name")
if not str(fixture.get("description", "")).strip():
    raise SystemExit("Agent Card description is required")
if not str(fixture.get("version", "")).strip():
    raise SystemExit("Agent Card version is required")
if not isinstance(fixture.get("capabilities"), dict):
    raise SystemExit("Agent Card capabilities are required")
for field in ("defaultInputModes", "defaultOutputModes"):
    value = fixture.get(field)
    if not isinstance(value, list) or not value:
        raise SystemExit(f"Agent Card {field} must be a non-empty array")
interfaces = fixture.get("supportedInterfaces")
if not isinstance(interfaces, list) or not interfaces:
    raise SystemExit("Agent Card must declare at least one interface")
if not all(
    str(item.get("url", "")).startswith("https://api.agentbounties.app/")
    for item in interfaces
):
    raise SystemExit("Agent Card interfaces must use the canonical HTTPS API")
if not all(item.get("protocolVersion") == "1.0" for item in interfaces):
    raise SystemExit("every Agent Card interface must declare A2A 1.0")
binding = "https://agentbounties.app/docs/a2a-direct-api-binding-v1"
if not all(item.get("protocolBinding") == binding for item in interfaces):
    raise SystemExit(
        "interfaces must identify the documented Agent Bounties custom binding"
    )
if "protocolVersion" in fixture:
    raise SystemExit("protocolVersion belongs on interfaces, not the Agent Card root")

skills = fixture.get("skills")
if not isinstance(skills, list):
    raise SystemExit("Agent Card skills must be an array")
for skill in skills:
    for field in ("id", "name", "description", "tags"):
        value = skill.get(field)
        if value is None or value == "" or value == []:
            raise SystemExit(f"Agent Card skill is missing {field}")
skill_ids = {str(item.get("id", "")) for item in skills}
required_skills = {
    "discover-funded-work",
    "plan-bounty-claim",
    "submit-bounty-evidence",
    "check-bounty-settlement",
    "post-bounty",
}
missing = required_skills - skill_ids
if missing:
    raise SystemExit(f"Agent Card is missing skills: {sorted(missing)}")

serialized = json.dumps(fixture, sort_keys=True).lower()
for forbidden in ("private_key", "private key", "seed phrase", "api_key", "secret"):
    if forbidden in serialized:
        raise SystemExit(f"Agent Card exposes forbidden material: {forbidden}")
for required_phrase in ("canonical", "claimable", "bountysettled"):
    if required_phrase not in serialized:
        raise SystemExit(
            f"Agent Card must preserve the {required_phrase} evidence boundary"
        )

api = text("crates/api/src/main.rs")
public = text("crates/web-public/src/lib.rs")
quickstart = text("docs/agent-quickstart.md")
binding_doc = text("docs/a2a-direct-api-binding-v1.md")
for source, label in (
    (api, "API"),
    (public, "public manifest"),
    (quickstart, "quickstart"),
):
    if "/.well-known/agent-card.json" not in source:
        raise SystemExit(f"{label} does not expose the Agent Card URL")
for phrase in ("not a2a http+json", "canonical", "bountysettled"):
    if phrase not in binding_doc.lower():
        raise SystemExit(f"custom binding documentation is missing {phrase}")
if "agent_card" not in api.lower() or "agent_card" not in public.lower():
    raise SystemExit("implementation lacks focused Agent Card code and tests")
if "etag" not in api.lower() or "cache-control" not in api.lower():
    raise SystemExit("Agent Card response must implement explicit caching")

checker = require("scripts/check-a2a-agent-card.py")
completed = subprocess.run(
    [sys.executable, str(checker)],
    cwd=ROOT,
    text=True,
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
    timeout=30,
    check=False,
)
if completed.returncode != 0:
    raise SystemExit(f"A2A Agent Card smoke failed:\n{completed.stdout[-4000:]}")

print("A2A Agent Card acceptance checks passed")
