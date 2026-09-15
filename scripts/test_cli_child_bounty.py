"""
Tests for CLI child bounty specification and verification check.
"""
import subprocess
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class CliChildBountyTests(unittest.TestCase):
    """
    Validates the CLI child bounty markdown template and runner benchmark.
    """

    def test_cli_child_bounty_markdown_exists_and_conforms(self) -> None:
        """
        Ensure bounties/cli-child-bounty.md exists and contains required sections.
        """
        bounty_file = ROOT / "bounties" / "cli-child-bounty.md"
        self.assertTrue(bounty_file.exists(), "bounties/cli-child-bounty.md must exist")
        content = bounty_file.read_text(encoding="utf-8")
        self.assertIn("0.90 USDC", content)
        self.assertIn("0.80 USDC", content)
        self.assertIn("0.10 USDC", content)
        self.assertIn("sandboxed_regression_v1", content)
        self.assertIn("0xbe6292b9e465f549e2363b918d6dd9187038431e", content)
        self.assertIn("0xb7c2ce6430b66fb986e27b6140b29309550d487a", content)
        self.assertIn("check-agent-bounties-cli.mjs", content)
        self.assertIn("/agent-bounty register 0xYourBaseWallet", content)
        self.assertIn("BountySettled", content)

    def test_cli_benchmark_passes(self) -> None:
        """
        Execute the pinned cli test runner against the current workspace.
        """
        runner = ROOT / "benchmarks" / "standing-meta-v2" / "cli" / "test.mjs"
        result = subprocess.run(
            ["node", str(runner), str(ROOT)],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(
            result.returncode,
            0,
            f"cli test runner failed:\nstdout: {result.stdout}\nstderr: {result.stderr}",
        )
        self.assertIn("cli_benchmark=passed", result.stdout)

    def test_cli_self_test_passes(self) -> None:
        """
        Execute the benchmark harness self-test.
        """
        runner = ROOT / "benchmarks" / "standing-meta-v2" / "cli" / "self-test.mjs"
        result = subprocess.run(
            ["node", str(runner)],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(
            result.returncode,
            0,
            f"cli self-test failed:\nstdout: {result.stdout}\nstderr: {result.stderr}",
        )
        self.assertIn("cli_benchmark_self_test=passed", result.stdout)


if __name__ == "__main__":
    unittest.main()
