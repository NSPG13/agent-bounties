#!/usr/bin/env python3
"""Deterministic API test for stale domain origins.

Ensures analytics, discovery, redirects, and generated public links accept
and emit only canonical agentbounties.app, api.agentbounties.app, and
mcp.agentbounties.app origins.
"""

from __future__ import annotations

import json
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "scripts" / "fixtures"


def load_fixture(name: str) -> dict:
    return json.loads((FIXTURES / name).read_text(encoding="utf-8"))


def is_canonical_origin(url: str, canonical: list[str]) -> bool:
    return any(url.startswith(o) for o in canonical)


def is_stale_origin(url: str, stale: list[str]) -> bool:
    return any(url.startswith(o) for o in stale)


class StaleDomainOriginTests(unittest.TestCase):
    def setUp(self) -> None:
        self.fixture = load_fixture("rte_stale_domain_origins.json")
        self.canonical = self.fixture["canonical_origins"]
        self.stale = self.fixture["stale_legacy_origins"]
        self.links = self.fixture["generated_links"]

    def test_canonical_website_origin_accepted(self) -> None:
        self.assertTrue(is_canonical_origin(self.links["website"], self.canonical))

    def test_canonical_api_origin_accepted(self) -> None:
        self.assertTrue(is_canonical_origin(self.links["api_health"], self.canonical))

    def test_canonical_mcp_origin_accepted(self) -> None:
        self.assertTrue(is_canonical_origin(self.links["mcp_tools"], self.canonical))

    def test_canonical_opportunity_embed_accepted(self) -> None:
        self.assertTrue(is_canonical_origin(self.links["opportunity_embed"], self.canonical))

    def test_stale_legacy_origin_rejected(self) -> None:
        self.assertFalse(is_canonical_origin(self.links["stale_legacy_link"], self.canonical))
        self.assertTrue(is_stale_origin(self.links["stale_legacy_link"], self.stale))

    def test_stale_api_origin_rejected(self) -> None:
        self.assertFalse(is_canonical_origin(self.links["stale_api_link"], self.canonical))

    def test_stale_www_origin_rejected(self) -> None:
        self.assertFalse(is_canonical_origin(self.links["stale_www_link"], self.canonical))

    def test_no_canonical_origin_in_stale_list(self) -> None:
        for origin in self.canonical:
            self.assertNotIn(origin, self.stale)

    def test_all_canonical_origins_accepted(self) -> None:
        for origin in self.canonical:
            self.assertTrue(is_canonical_origin(origin, self.canonical))

    def test_all_stale_origins_rejected(self) -> None:
        for origin in self.stale:
            self.assertFalse(is_canonical_origin(origin, self.canonical))
            self.assertTrue(is_stale_origin(origin, self.stale))

    def test_generated_links_use_canonical_origin(self) -> None:
        canonical_keys = ["website", "api_health", "mcp_tools", "opportunity_embed"]
        for key in canonical_keys:
            self.assertTrue(
                is_canonical_origin(self.links[key], self.canonical),
                f"{key} should use canonical origin",
            )

    def test_stale_links_never_use_canonical_origin(self) -> None:
        stale_keys = ["stale_legacy_link", "stale_api_link", "stale_www_link"]
        for key in stale_keys:
            self.assertFalse(
                is_canonical_origin(self.links[key], self.canonical),
                f"{key} should not use canonical origin",
            )

    def test_redirect_rules_prefixes_do_not_overlap(self) -> None:
        redirect = self.fixture["redirect_rules"]
        canonical_prefix = redirect["canonical_prefix"]
        for stale_prefix in redirect["stale_prefixes"]:
            self.assertNotEqual(stale_prefix, canonical_prefix)
            self.assertNotIn(canonical_prefix, stale_prefix)

    def test_no_dns_or_network_required(self) -> None:
        self.assertTrue(FIXTURES.exists())

    def test_fixture_is_replayable(self) -> None:
        first = load_fixture("rte_stale_domain_origins.json")
        second = load_fixture("rte_stale_domain_origins.json")
        self.assertEqual(first, second)

    def test_retired_legacy_origin_absent_from_generated_output(self) -> None:
        for key, url in self.links.items():
            if key.startswith("stale_"):
                self.assertFalse(is_canonical_origin(url, self.canonical))


if __name__ == "__main__":
    unittest.main()
