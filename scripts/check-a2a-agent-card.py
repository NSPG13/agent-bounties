#!/usr/bin/env python3
"""Smoke test for the A2A Agent Card endpoint.

Validates that `/.well-known/agent-card.json` returns a valid A2A 1.0 Agent Card
with all required skills, canonical evidence boundaries, and proper caching headers.
"""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path
from urllib.request import Request, urlopen
from urllib.error import URLError


API_BASE = os.environ.get("API_BASE_URL", "http://127.0.0.1:8080")
CARD_URL = f"{API_BASE}/.well-known/agent-card.json"


def fetch_card(url: str) -> tuple[dict, dict]:
    """Fetch the agent card and return (body, headers)."""
    req = Request(url)
    req.add_header("Accept", "application/json")
    try:
        with urlopen(req, timeout=10) as resp:
            headers = dict(resp.headers)
            body = json.loads(resp.read().decode("utf-8"))
            return body, headers
    except URLError as e:
        raise SystemExit(f"Failed to fetch Agent Card from {url}: {e}")
    except json.JSONDecodeError as e:
        raise SystemExit(f"Agent Card is not valid JSON: {e}")


def validate_card(card: dict, headers: dict) -> None:
    """Run all A2A Agent Card acceptance checks."""
    errors = []

    # Basic fields
    if card.get("name") != "Agent Bounties":
        errors.append("Agent Card must use the canonical product name 'Agent Bounties'")
    if not str(card.get("description", "")).strip():
        errors.append("Agent Card description is required")
    if not str(card.get("version", "")).strip():
        errors.append("Agent Card version is required")
    if not isinstance(card.get("capabilities"), dict):
        errors.append("Agent Card capabilities are required")

    # Input/output modes
    for field in ("defaultInputModes", "defaultOutputModes"):
        value = card.get(field)
        if not isinstance(value, list) or not value:
            errors.append(f"Agent Card {field} must be a non-empty array")

    # Interfaces
    interfaces = card.get("supportedInterfaces")
    if not isinstance(interfaces, list) or not interfaces:
        errors.append("Agent Card must declare at least one interface")
    else:
        for item in interfaces:
            if not str(item.get("url", "")).startswith("https://api.agentbounties.app/"):
                errors.append("Agent Card interfaces must use the canonical HTTPS API")
            if item.get("protocolVersion") != "1.0":
                errors.append("every Agent Card interface must declare A2A 1.0")
            binding = "https://agentbounties.app/docs/a2a-direct-api-binding-v1"
            if item.get("protocolBinding") != binding:
                errors.append("interfaces must identify the documented Agent Bounties custom binding")

    # protocolVersion at root level should not exist
    if "protocolVersion" in card and card["protocolVersion"] is not None:
        errors.append("protocolVersion belongs on interfaces, not the Agent Card root")

    # Skills
    skills = card.get("skills")
    if not isinstance(skills, list):
        errors.append("Agent Card skills must be an array")
    else:
        for skill in skills:
            for field in ("id", "name", "description", "tags"):
                value = skill.get(field)
                if value is None or value == "" or value == []:
                    errors.append(f"Agent Card skill is missing {field}")
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
            errors.append(f"Agent Card is missing skills: {sorted(missing)}")

    # Forbidden material
    serialized = json.dumps(card, sort_keys=True).lower()
    for forbidden in ("private_key", "private key", "seed phrase", "api_key", "secret"):
        if forbidden in serialized:
            errors.append(f"Agent Card exposes forbidden material: {forbidden}")

    # Required phrases
    for required_phrase in ("canonical", "claimable", "bountysettled"):
        if required_phrase not in serialized:
            errors.append(f"Agent Card must preserve the {required_phrase} evidence boundary")

    # Cache headers
    etag = headers.get("etag", headers.get("ETag", ""))
    cache_control = headers.get("cache-control", headers.get("Cache-Control", ""))
    if not etag:
        errors.append("Agent Card response missing ETag header")
    if not cache_control or "max-age" not in cache_control.lower():
        errors.append("Agent Card response missing Cache-Control with max-age")

    if errors:
        for e in errors:
            print(f"FAIL: {e}", file=sys.stderr)
        raise SystemExit(f"{len(errors)} validation errors")
    print("A2A Agent Card acceptance checks passed")


if __name__ == "__main__":
    card, headers = fetch_card(CARD_URL)
    validate_card(card, headers)
