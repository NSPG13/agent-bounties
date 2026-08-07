#!/usr/bin/env python3
"""Deterministic smoke check for the A2A Agent Card implementation.

Validates the Agent Card fixture, the canonical endpoint wiring in the API and
public manifest crates, and the custom binding documentation. Pure stdlib so it
runs inside the precommitted sandbox without network access.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

CARD_URL = "/.well-known/agent-card.json"
BINDING = "https://agentbounties.app/docs/a2a-direct-api-binding-v1"
REQUIRED_SKILLS = {
    "discover-funded-work",
    "plan-bounty-claim",
    "submit-bounty-evidence",
    "check-bounty-settlement",
    "post-bounty",
}


def fail(msg: str) -> None:
    print(f"A2A Agent Card smoke FAILED: {msg}")
    sys.exit(1)


def main() -> None:
    fixture_path = ROOT / "fixtures" / "a2a-agent-card.json"
    if not fixture_path.is_file():
        fail("missing fixtures/a2a-agent-card.json")
    fixture = json.loads(fixture_path.read_text(encoding="utf-8"))

    if fixture.get("name") != "Agent Bounties":
        fail("canonical product name missing")
    if not str(fixture.get("description", "")).strip():
        fail("description required")
    if not str(fixture.get("version", "")).strip():
        fail("version required")
    if not isinstance(fixture.get("capabilities"), dict):
        fail("capabilities must be an object")
    for field in ("defaultInputModes", "defaultOutputModes"):
        if not isinstance(fixture.get(field), list) or not fixture[field]:
            fail(f"{field} must be a non-empty array")

    interfaces = fixture.get("supportedInterfaces")
    if not isinstance(interfaces, list) or not interfaces:
        fail("supportedInterfaces must declare at least one interface")
    for item in interfaces:
        url = str(item.get("url", ""))
        if not url.startswith("https://api.agentbounties.app/"):
            fail("interfaces must use the canonical HTTPS API")
        if item.get("protocolVersion") != "1.0":
            fail("every interface must declare A2A 1.0")
        if item.get("protocolBinding") != BINDING:
            fail("interfaces must identify the documented custom binding")
    if "protocolVersion" in fixture:
        fail("protocolVersion belongs on interfaces, not the card root")

    skills = fixture.get("skills")
    if not isinstance(skills, list):
        fail("skills must be an array")
    for skill in skills:
        for field in ("id", "name", "description", "tags"):
            value = skill.get(field)
            if value is None or value == "" or value == []:
                fail(f"skill is missing {field}")
    skill_ids = {str(item.get("id", "")) for item in skills}
    missing = REQUIRED_SKILLS - skill_ids
    if missing:
        fail(f"missing skills: {sorted(missing)}")

    serialized = json.dumps(fixture, sort_keys=True).lower()
    for forbidden in ("private_key", "private key", "seed phrase", "api_key", "secret"):
        if forbidden in serialized:
            fail(f"forbidden material present: {forbidden}")
    for required_phrase in ("canonical", "claimable", "bountysettled"):
        if required_phrase not in serialized:
            fail(f"missing evidence phrase: {required_phrase}")

    api = (ROOT / "crates/api/src/main.rs").read_text(encoding="utf-8")
    public = (ROOT / "crates/web-public/src/lib.rs").read_text(encoding="utf-8")
    quickstart = (ROOT / "docs/agent-quickstart.md").read_text(encoding="utf-8")
    binding_doc = (ROOT / "docs/a2a-direct-api-binding-v1.md").read_text(encoding="utf-8")

    for source, label in ((api, "API"), (public, "public manifest"), (quickstart, "quickstart")):
        if CARD_URL not in source:
            fail(f"{label} does not expose the Agent Card URL")
    for phrase in ("not a2a http+json", "canonical", "bountysettled"):
        if phrase not in binding_doc.lower():
            fail(f"binding documentation is missing {phrase}")
    if "agent_card" not in api.lower() or "agent_card" not in public.lower():
        fail("implementation lacks focused Agent Card code and tests")
    if "etag" not in api.lower() or "cache-control" not in api.lower():
        fail("Agent Card response must implement explicit caching")

    print("A2A Agent Card smoke passed")


if __name__ == "__main__":
    main()
