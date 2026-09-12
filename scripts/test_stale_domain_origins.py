#!/usr/bin/env python3
"""Fail-closed API test for stale domain origins (Issue #684)."""

from __future__ import annotations

import json
import pathlib
import unittest
from urllib.parse import urlparse

ROOT = pathlib.Path(__file__).resolve().parents[1]

CANONICAL_HOSTS = {
    "agentbounties.app",
    "api.agentbounties.app",
    "mcp.agentbounties.app",
}

RETIRED_HOSTS = {
    "agentbounties.io",
    "agentbounties.net",
    "legacy.agentbounties.app",
    "agent-bounties-legacy.render.com",
    "agent-bounties.herokuapp.com",
}


def validate_origin(origin_url: str) -> bool:
    """Validate if an origin URL is canonical and valid."""
    if not origin_url or not origin_url.startswith("https://"):
        return False
    parsed = urlparse(origin_url)
    return parsed.hostname in CANONICAL_HOSTS


def generate_canonical_redirect(
    target_path: str, base_origin: str = "https://agentbounties.app"
) -> str:
    """Generate a canonical redirect URL from base origin and path."""
    if not validate_origin(base_origin):
        raise ValueError(f"Invalid or stale domain origin: {base_origin}")
    clean_path = target_path if target_path.startswith("/") else "/" + target_path
    return f"{base_origin}{clean_path}"


class StaleDomainOriginTests(unittest.TestCase):
    def test_canonical_origins_accepted(self) -> None:
        for host in CANONICAL_HOSTS:
            url = f"https://{host}"
            self.assertTrue(validate_origin(url), f"Canonical origin {url} should be accepted")

    def test_retired_origins_rejected(self) -> None:
        for host in RETIRED_HOSTS:
            url = f"https://{host}"
            self.assertFalse(validate_origin(url), f"Retired origin {url} must be rejected")
            http_url = f"http://{host}"
            self.assertFalse(
                validate_origin(http_url), f"Non-HTTPS origin {http_url} must be rejected"
            )

    def test_protocol_json_uses_canonical_origins(self) -> None:
        protocol = json.loads((ROOT / "site/protocol.json").read_text(encoding="utf-8"))
        api_url = protocol.get("api_base_url", "")
        mcp_url = protocol.get("mcp_base_url", "")

        self.assertTrue(
            validate_origin(api_url), f"API base URL {api_url} is not a valid canonical origin"
        )
        self.assertTrue(
            validate_origin(mcp_url), f"MCP base URL {mcp_url} is not a valid canonical origin"
        )

        protocol_str = json.dumps(protocol)
        for retired in RETIRED_HOSTS:
            self.assertNotIn(retired, protocol_str, f"Retired host {retired} found in protocol.json")

    def test_redirect_generation_fails_closed_on_stale_origin(self) -> None:
        # Valid canonical origin succeeds
        link = generate_canonical_redirect("/earn.html", "https://agentbounties.app")
        self.assertEqual(link, "https://agentbounties.app/earn.html")

        # Retired origin fails closed
        with self.assertRaises(ValueError):
            generate_canonical_redirect("/earn.html", "https://agentbounties.io")

        # Non-HTTPS origin fails closed
        with self.assertRaises(ValueError):
            generate_canonical_redirect("/earn.html", "http://agentbounties.app")


if __name__ == "__main__":
    unittest.main()
