#!/usr/bin/env python3
"""Deterministic A2A Agent Card smoke for Agent Bounties."""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BINDING = "https://agentbounties.app/docs/a2a-direct-api-binding-v1"
REQUIRED_SKILLS = {
    "discover-funded-work",
    "plan-bounty-claim",
    "submit-bounty-evidence",
    "check-bounty-settlement",
    "post-bounty",
}


def fail(msg: str) -> None:
    raise SystemExit(msg)


def main() -> None:
    fixture_path = ROOT / "fixtures" / "a2a-agent-card.json"
    site_path = ROOT / "site" / ".well-known" / "agent-card.json"
    if not fixture_path.is_file():
        fail("missing fixtures/a2a-agent-card.json")
    if not site_path.is_file():
        fail("missing site/.well-known/agent-card.json")

    fixture = json.loads(fixture_path.read_text(encoding="utf-8"))
    site = json.loads(site_path.read_text(encoding="utf-8"))
    if fixture != site:
        fail("site Agent Card must match fixtures/a2a-agent-card.json")

    if fixture.get("name") != "Agent Bounties":
        fail("name must be Agent Bounties")
    if "protocolVersion" in fixture:
        fail("protocolVersion must not be on Agent Card root")
    skills = {s.get("id") for s in fixture.get("skills") or []}
    missing = REQUIRED_SKILLS - skills
    if missing:
        fail(f"missing skills: {sorted(missing)}")
    for iface in fixture.get("supportedInterfaces") or []:
        if iface.get("protocolBinding") != BINDING:
            fail("interface missing custom binding")
        if iface.get("protocolVersion") != "1.0":
            fail("interface must be A2A 1.0")
        if not str(iface.get("url", "")).startswith("https://api.agentbounties.app/"):
            fail("interface URL must be canonical API")

    api = (ROOT / "crates" / "api" / "src" / "main.rs").read_text(encoding="utf-8")
    public = (ROOT / "crates" / "web-public" / "src" / "lib.rs").read_text(encoding="utf-8")
    if "/.well-known/agent-card.json" not in api:
        fail("API missing agent-card route")
    if "agent_card" not in api.lower() or "agent_card" not in public.lower():
        fail("missing agent_card implementation")
    if "etag" not in api.lower() or "cache-control" not in api.lower():
        fail("API agent card must set cache headers")

    binding_doc = (ROOT / "docs" / "a2a-direct-api-binding-v1.md").read_text(encoding="utf-8")
    for phrase in ("not a2a http+json", "canonical", "bountysettled"):
        if phrase not in binding_doc.lower():
            fail(f"binding doc missing {phrase}")

    quickstart = (ROOT / "docs" / "agent-quickstart.md").read_text(encoding="utf-8")
    if "/.well-known/agent-card.json" not in quickstart:
        fail("quickstart missing agent-card URL")

    body = json.dumps(fixture, sort_keys=True, separators=(",", ":")).encode()
    etag = hashlib.sha256(body).hexdigest()[:16]
    print(f"A2A Agent Card smoke passed etag_prefix={etag}")


if __name__ == "__main__":
    main()
