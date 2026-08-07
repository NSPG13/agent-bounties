#!/usr/bin/env python3
"""Characterize the MCP descriptor registry consumed by docs-contract checks."""

from __future__ import annotations

import json
import pathlib
import re
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
CODEX_PLUGIN_ROOT = ROOT / "plugins/agent-bounties"
REQUIRED_READINESS_TOOLS = (
    "list_autonomous_bounties",
    "list_opportunities",
    "prepare_agent_to_earn",
    "prepare_bounty_post",
)


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
        self.assertEqual(len(names), 116)
        self.assertEqual(len(names), len(set(names)))
        self.assertEqual(names, registry["tools"])
        self.assertIn("list_open_competition_verifiers", names)
        self.assertIn("prepare_open_competition_creation", names)


class McpApiDiscoveryCompatibilityTests(unittest.TestCase):
    def test_discovery_manifest_and_registry_tools_align_for_readiness_workflow(self) -> None:
        registry = json.loads(
            (ROOT / "crates/mcp-server/fixtures/tool-registry.json").read_text(encoding="utf-8")
        )
        discovery = json.loads(
            (ROOT / "site/.well-known/agent-bounties.json").read_text(encoding="utf-8")
        )

        tools = discovery.get("agent_tools")
        self.assertIsInstance(tools, list)
        registry_tools = registry.get("tools")
        self.assertIsInstance(registry_tools, list)

        endpoints = discovery.get("endpoints", {})
        mcp_tools_url = endpoints.get("mcp_tools")
        mcp_transport_url = endpoints.get("mcp_streamable_http")
        if not mcp_tools_url or not mcp_transport_url:
            self.fail("discovery manifest missing MCP tool transport endpoint or tool-inventory URL")

        missing_summary = []
        for tool in REQUIRED_READINESS_TOOLS:
            if tool not in tools:
                missing_summary.append(f"discovery-missing:{tool}")
            if tool not in registry_tools:
                missing_summary.append(f"registry-missing:{tool}")

        if len(tools) != len(set(tools)):
            missing_summary.append("discovery-duplicate-tools")
        if len(registry_tools) != len(set(registry_tools)):
            missing_summary.append("registry-duplicate-tools")

        transport_drift = []
        if mcp_tools_url == mcp_transport_url:
            transport_drift.append("mcp_tools_equal_transport")
        if not str(mcp_tools_url).endswith("/tools"):
            transport_drift.append(f"mcp_tools_not_tools_path:{mcp_tools_url}")
        if not str(mcp_transport_url).endswith("/mcp"):
            transport_drift.append(f"mcp_transport_not_mcp_path:{mcp_transport_url}")

        if missing_summary or transport_drift:
            self.fail(
                "MCP/API tool-registry drift: "
                + "; ".join([*missing_summary, *transport_drift]),
            )


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
