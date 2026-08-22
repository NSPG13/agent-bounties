#!/usr/bin/env python3
"""Safety contract for the scoped self-hosted prover cleanup."""

from __future__ import annotations

from pathlib import Path
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "clean-open-competition-v2-prover-builds.yml"
EXPECTED_WORKSPACE = "/home/pooln/actions-runner/_work/agent-bounties/agent-bounties"


class ProverCleanupWorkflowTests(unittest.TestCase):
    def test_is_manual_protected_and_pinned_to_the_prover_runner(self) -> None:
        value = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))
        triggers = value.get("on", value.get(True))
        self.assertEqual(triggers, {"workflow_dispatch": None})
        job = value["jobs"]["clean-generated-output"]
        self.assertEqual(job["environment"], "v2-beta2-mainnet")
        self.assertEqual(
            job["runs-on"],
            ["self-hosted", "Linux", "X64", "ram-256gb", "open-competition-v2-prover"],
        )

    def test_cleanup_is_exactly_scoped_to_generated_outputs(self) -> None:
        text = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn(f"expected_workspace={EXPECTED_WORKSPACE}", text)
        self.assertIn('test "$workspace" = "$expected_workspace"', text)
        self.assertIn('"$workspace"/*', text)
        for relative in (
            ".sp1-safe/target",
            "target",
            "contracts/base-escrow/out",
            "contracts/base-escrow/cache",
        ):
            self.assertIn(relative, text)
        for forbidden in ("$HOME", "find ", "/mnt/agent-bounties-artifacts", "docker system prune"):
            self.assertNotIn(forbidden, text)

    def test_requires_recovered_capacity_and_uses_no_secret(self) -> None:
        text = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn('test "$after_bytes" -ge 16106127360', text)
        self.assertIn("df -h", text)
        self.assertIn("/home/pooln/actions-runner/_work", text)
        self.assertIn("du -x -h --max-depth=2", text)
        self.assertNotIn("secrets.", text)
        self.assertNotIn("actions/checkout", text)


if __name__ == "__main__":
    unittest.main()
