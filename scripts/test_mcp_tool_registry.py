#!/usr/bin/env python3
"""Characterize the MCP descriptor registry consumed by docs-contract checks."""

from __future__ import annotations

import json
import pathlib
import re
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
CODEX_PLUGIN_ROOT = ROOT / "plugins/agent-bounties"


class McpToolRegistryTests(unittest.TestCase):
    def test_registry_matches_descriptor_order_and_names(self) -> None:
        registry = json.loads(
            (ROOT / "crates/mcp-server/fixtures/tool-registry.json").read_text(encoding="utf-8")
        )
        self.assertEqual(registry["schema_version"], "agent-bounties/mcp-tool-registry-v1")
        source = (ROOT / "crates/mcp-server/src/main.rs").read_text(encoding="utf-8")
        self.assertNotRegex(source, r"(?m)^struct \w+Args\b")
        descriptor_source = source[
            source.index("async fn tools()") : source.index("const OPERATOR_TOKEN_REQUIRED")
        ]
        names = re.findall(r'\b(?:operator_)?tool\(\s*"([a-z0-9_]+)"', descriptor_source)
        self.assertEqual(len(names), 106)
        self.assertEqual(len(names), len(set(names)))
        self.assertEqual(names, registry["tools"])


class CodexPluginDistributionTests(unittest.TestCase):
    def test_manifest_routes_funded_posting_and_earning_intents_to_hosted_mcp(self) -> None:
        manifest = json.loads(
            (CODEX_PLUGIN_ROOT / ".codex-plugin/plugin.json").read_text(encoding="utf-8")
        )
        mcp = json.loads((CODEX_PLUGIN_ROOT / ".mcp.json").read_text(encoding="utf-8"))

        self.assertEqual(manifest["name"], "agent-bounties")
        self.assertEqual(manifest["mcpServers"], "./.mcp.json")
        self.assertEqual(
            mcp["mcpServers"]["agent-bounties"],
            {"type": "http", "url": "https://mcp.agentbounties.app/mcp"},
        )

        interface = manifest["interface"]
        prompts = interface["defaultPrompt"]
        self.assertEqual(len(prompts), 3)
        self.assertTrue(all(0 < len(prompt) <= 128 for prompt in prompts))
        self.assertTrue(any("goal" in prompt and "draft" in prompt for prompt in prompts))
        self.assertTrue(any("funded" in prompt and "earn USDC" in prompt for prompt in prompts))
        self.assertTrue(any("do not claim, sign, or move funds" in prompt for prompt in prompts))

        discovery_copy = " ".join(
            [
                manifest["description"],
                interface["shortDescription"],
                interface["longDescription"],
                *prompts,
            ]
        ).lower()
        for required_phrase in (
            "personal",
            "professional",
            "public-good",
            "funded",
            "verification-ready",
            "earn usdc",
            "bountysettled",
        ):
            self.assertIn(required_phrase, discovery_copy)
        self.assertNotIn("private key", discovery_copy)
        self.assertNotIn("seed phrase", discovery_copy)
        self.assertNotIn("unfunded", discovery_copy)


if __name__ == "__main__":
    unittest.main()
