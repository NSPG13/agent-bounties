"""
Tests for MCP child bounty specification and verification check.
"""
import subprocess
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class McpChildBountyTests(unittest.TestCase):
    """
    Validates the MCP child bounty markdown template and runner benchmark.
    """

    def test_mcp_child_bounty_markdown_exists_and_conforms(self) -> None:
        """
        Ensure bounties/mcp-child-bounty.md exists and contains required sections.
        """
        bounty_file = ROOT / "bounties" / "mcp-child-bounty.md"
        self.assertTrue(bounty_file.exists(), "bounties/mcp-child-bounty.md must exist")
        content = bounty_file.read_text(encoding="utf-8")
        self.assertIn("0.90 USDC", content)
        self.assertIn("0.80 USDC", content)
        self.assertIn("0.10 USDC", content)
        self.assertIn("sandboxed_regression_v1", content)
        self.assertIn("0xbe6292b9e465f549e2363b918d6dd9187038431e", content)
        self.assertIn("0xb7c2ce6430b66fb986e27b6140b29309550d487a", content)
        self.assertIn("check-agent-bounties-discovery.mjs", content)
        self.assertIn("/agent-bounty register 0xYourBaseWallet", content)
        self.assertIn("BountySettled", content)

    def test_mcp_discovery_benchmark_passes(self) -> None:
        """
        Execute the pinned mcp-discovery test runner against the current workspace.
        """
        runner = ROOT / "benchmarks" / "standing-meta-v2" / "mcp-discovery" / "test.mjs"
        result = subprocess.run(
            ["node", str(runner), str(ROOT)],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(
            result.returncode,
            0,
            f"mcp-discovery test runner failed:\nstdout: {result.stdout}\nstderr: {result.stderr}",
        )
        self.assertIn("mcp_discovery_benchmark=passed", result.stdout)


if __name__ == "__main__":
    unittest.main()
