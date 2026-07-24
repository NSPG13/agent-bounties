#!/usr/bin/env python3
"""Characterize the MCP descriptor registry consumed by docs-contract checks."""

from __future__ import annotations

import json
import pathlib
import re
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]


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
        self.assertEqual(len(names), 113)
        self.assertEqual(len(names), len(set(names)))
        self.assertEqual(names, registry["tools"])


if __name__ == "__main__":
    unittest.main()
