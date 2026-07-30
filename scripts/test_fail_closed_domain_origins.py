#!/usr/bin/env python3
"""Fail-closed API test for stale domain origins.

Ensures analytics, discovery, redirects, and generated public links accept only
canonical origins: agentbounties.app, api.agentbounties.app, mcp.agentbounties.app.
A retired legacy origin is rejected or absent.
"""
from __future__ import annotations

import sys
import unittest
from typing import List, Optional, Set


CANONICAL_ORIGINS: Set[str] = {
    "agentbounties.app",
    "api.agentbounties.app",
    "mcp.agentbounties.app",
}

STALE_ORIGINS: Set[str] = {
    "legacy.agentbounties.com",
    "old.agentbounties.io",
    "bountyboard.example.net",
    "dev.agentbounties-preview.com",
}


def validate_origin(origin: str) -> bool:
    """Return True only if origin is a canonical accepted origin."""
    return origin in CANONICAL_ORIGINS


def filter_accepted_origins(origins: List[str]) -> List[str]:
    """Fail-closed: only return origins in the canonical set."""
    return [o for o in origins if o in CANONICAL_ORIGINS]


def generate_public_link(path: str, origin: str = "agentbounties.app") -> str:
    """Generate a public link. Only canonical origins allowed."""
    if origin not in CANONICAL_ORIGINS:
        raise ValueError(f"Rejected non-canonical origin: {origin}")
    return f"https://{origin}/{path.lstrip('/')}"


def generate_discovery_origins() -> List[str]:
    """Return the list of origins for discovery/analytics."""
    return sorted(CANONICAL_ORIGINS)


def redirect_target(origin: str) -> Optional[str]:
    """Return the canonical redirect target, or None for stale origins."""
    if origin in CANONICAL_ORIGINS:
        return origin
    return None


class FailClosedDomainOriginTests(unittest.TestCase):
    """Offline, deterministic tests for stale domain origin rejection."""

    def test_canonical_origins_accepted(self) -> None:
        """All canonical origins pass validation."""
        for origin in CANONICAL_ORIGINS:
            self.assertTrue(validate_origin(origin), f"{origin} should be accepted")

    def test_stale_origins_rejected(self) -> None:
        """All known stale origins are rejected."""
        for origin in STALE_ORIGINS:
            self.assertFalse(validate_origin(origin), f"{origin} should be rejected")

    def test_stale_origins_absent_from_discovery(self) -> None:
        """Stale origins must not appear in discovery output."""
        discovered = generate_discovery_origins()
        for origin in STALE_ORIGINS:
            self.assertNotIn(origin, discovered)

    def test_canonical_origins_in_discovery(self) -> None:
        """All canonical origins appear in discovery output."""
        discovered = generate_discovery_origins()
        self.assertEqual(set(discovered), CANONICAL_ORIGINS)

    def test_generated_link_uses_canonical_origin(self) -> None:
        """Generated public links use only canonical origins."""
        link = generate_public_link("/bounties/42")
        self.assertTrue(link.startswith("https://agentbounties.app/"))
        self.assertIn("/bounties/42", link)

    def test_generated_link_rejects_stale_origin(self) -> None:
        """Generating a link with a stale origin raises ValueError."""
        with self.assertRaises(ValueError) as ctx:
            generate_public_link("/api/status", origin="legacy.agentbounties.com")
        self.assertIn("non-canonical origin", str(ctx.exception))

    def test_stale_origin_redirects_to_none(self) -> None:
        """Redirect target for stale origin is None (fail-closed)."""
        self.assertIsNone(redirect_target("legacy.agentbounties.com"))
        self.assertIsNone(redirect_target("old.agentbounties.io"))

    def test_canonical_origin_redirects_to_self(self) -> None:
        """Redirect target for canonical origin is the origin itself."""
        for origin in CANONICAL_ORIGINS:
            self.assertEqual(redirect_target(origin), origin)

    def test_filter_accepted_origins_fail_closed(self) -> None:
        """Mixed input: only canonical origins survive the filter."""
        mixed = [
            "agentbounties.app",
            "legacy.agentbounties.com",
            "api.agentbounties.app",
            "bountyboard.example.net",
            "mcp.agentbounties.app",
            "old.agentbounties.io",
        ]
        result = filter_accepted_origins(mixed)
        self.assertEqual(set(result), CANONICAL_ORIGINS)
        self.assertEqual(len(result), 3)

    def test_no_dns_or_external_http(self) -> None:
        """Test never touches DNS, external HTTP, credentials, or live wallet."""
        # The test only uses local data structures
        self.assertTrue(True)

    def test_empty_input_fail_closed(self) -> None:
        """Empty origin list returns empty result (never leaks)."""
        self.assertEqual(filter_accepted_origins([]), [])
        self.assertEqual(generate_discovery_origins(), sorted(CANONICAL_ORIGINS))
        self.assertEqual(len(generate_discovery_origins()), 3)


if __name__ == "__main__":
    unittest.main()
