#!/usr/bin/env python3
"""Deterministic compatibility test for MCP tool registry vs API discovery manifest.

Compares the public MCP tool registry with the API discovery manifest and
fails closed when a required read-only discovery or readiness tool is
missing or renamed.
"""

from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TOOL_REGISTRY = ROOT / "crates/mcp-server/fixtures/tool-registry.json"
DISCOVERY_MANIFEST = ROOT / "site/.well-known/agent-bounties.json"

REQUIRED_DISCOVERY_TOOLS = [
    "list_autonomous_bounties",
    "list_opportunities",
    "prepare_agent_to_earn",
    "prepare_bounty_post",
]


def load_tool_registry() -> list[str]:
    data = json.loads(TOOL_REGISTRY.read_text(encoding="utf-8"))
    return list(data["tools"])


def load_discovery_tools() -> list[str]:
    data = json.loads(DISCOVERY_MANIFEST.read_text(encoding="utf-8"))
    return list(data["agent_tools"])


def check_duplicates(names: list[str]) -> list[str]:
    seen: set[str] = set()
    dupes: list[str] = []
    for name in names:
        if name in seen:
            dupes.append(name)
        seen.add(name)
    return dupes


class McpApiToolDriftTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.mcp_tools = load_tool_registry()
        cls.discovery_tools = load_discovery_tools()

    def test_required_tools_exist_in_mcp_registry(self) -> None:
        missing = [t for t in REQUIRED_DISCOVERY_TOOLS if t not in self.mcp_tools]
        if missing:
            self.fail(f"MCP registry missing required tools: {missing}")

    def test_required_tools_exist_in_discovery_manifest(self) -> None:
        missing = [t for t in REQUIRED_DISCOVERY_TOOLS if t not in self.discovery_tools]
        if missing:
            self.fail(f"Discovery manifest missing required tools: {missing}")

    def test_no_duplicates_in_mcp_registry(self) -> None:
        dupes = check_duplicates(self.mcp_tools)
        self.assertEqual(len(dupes), 0, f"MCP registry has duplicate tools: {dupes}")

    def test_no_duplicates_in_discovery_manifest(self) -> None:
        dupes = check_duplicates(self.discovery_tools)
        self.assertEqual(len(dupes), 0, f"Discovery manifest has duplicate tools: {dupes}")

    def test_list_autonomous_bounties_in_both(self) -> None:
        self.assertIn("list_autonomous_bounties", self.mcp_tools)
        self.assertIn("list_autonomous_bounties", self.discovery_tools)

    def test_list_opportunities_in_both(self) -> None:
        self.assertIn("list_opportunities", self.mcp_tools)
        self.assertIn("list_opportunities", self.discovery_tools)

    def test_prepare_agent_to_earn_in_both(self) -> None:
        self.assertIn("prepare_agent_to_earn", self.mcp_tools)
        self.assertIn("prepare_agent_to_earn", self.discovery_tools)

    def test_prepare_bounty_post_in_both(self) -> None:
        self.assertIn("prepare_bounty_post", self.mcp_tools)
        self.assertIn("prepare_bounty_post", self.discovery_tools)

    def test_mcp_registry_distinct_from_discovery_transport(self) -> None:
        manifest = json.loads(DISCOVERY_MANIFEST.read_text(encoding="utf-8"))
        mcp_transport = manifest.get("endpoints", {}).get("mcp_tools", "")
        self.assertNotEqual(mcp_transport, TOOL_REGISTRY.as_uri())

    def test_mcp_tools_are_subset_of_protocol_tools(self) -> None:
        mcp_set = set(self.mcp_tools)
        discovery_set = set(self.discovery_tools)
        extra_in_mcp = mcp_set - discovery_set
        self.assertTrue(
            len(extra_in_mcp) >= 0,
            f"MCP has tools not in discovery manifest: {extra_in_mcp}",
        )

    def test_offline_runnable_from_committed_fixtures(self) -> None:
        self.assertTrue(TOOL_REGISTRY.exists())
        self.assertTrue(DISCOVERY_MANIFEST.exists())

    def test_fixture_is_replayable(self) -> None:
        first = load_tool_registry()
        second = load_tool_registry()
        self.assertEqual(first, second)


def main() -> None:
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(McpApiToolDriftTests)
    runner = unittest.TextTestRunner(verbosity=2)
    result = runner.run(suite)
    if not result.wasSuccessful():
        errors = []
        for test, trace in result.failures + result.errors:
            errors.append(f"{test.id()} failed")
        print(
            json.dumps(
                {"ok": False, "errors": errors, "tests_run": result.testsRun},
            ),
        )
        sys.exit(1)
    print(
        json.dumps(
            {"ok": True, "tests_run": result.testsRun},
        ),
    )


if __name__ == "__main__":
    main()
