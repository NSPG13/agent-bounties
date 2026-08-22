#!/usr/bin/env python3
"""Safety contract for reclaiming the prover's unused secondary swap."""

from __future__ import annotations

from pathlib import Path
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "reclaim-open-competition-v2-prover-capacity.yml"


class ProverCapacityReclaimWorkflowTests(unittest.TestCase):
    def test_is_manual_protected_and_pinned(self) -> None:
        value = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))
        triggers = value.get("on", value.get(True))
        self.assertEqual(triggers, {"workflow_dispatch": None})
        job = value["jobs"]["reclaim-unused-secondary-swap"]
        self.assertEqual(job["environment"], "v2-beta2-mainnet")
        self.assertEqual(
            job["runs-on"],
            ["self-hosted", "Linux", "X64", "ram-256gb", "open-competition-v2-prover"],
        )

    def test_requires_exact_safe_preconditions(self) -> None:
        text = WORKFLOW.read_text(encoding="utf-8")
        for required in (
            "expected_workspace=/home/pooln/actions-runner/_work/agent-bounties/agent-bounties",
            "primary=/swapfile",
            "secondary=/swapfile2",
            "primary_size=68719476736",
            "secondary_size=51539607552",
            'test "$primary_used_bytes" = 0',
            'test "$secondary_used_bytes" = 0',
            "minimum_available_memory_kib=209715200",
            "minimum_recovered_bytes=50000000000",
            "/etc/fstab",
        ):
            self.assertIn(required, text)

    def test_preserves_primary_and_removes_only_secondary(self) -> None:
        text = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn('sudo -n swapoff -- "$secondary"', text)
        self.assertIn("sudo -n awk '$1 != \"/swapfile2\"' /etc/fstab", text)
        self.assertIn('sudo -n rm -f -- "$secondary"', text)
        self.assertIn('test -f "$primary"', text)
        for forbidden in (
            "rm -rf",
            "$HOME",
            "find ",
            "/mnt/agent-bounties-artifacts",
            "docker system prune",
            "secrets.",
            "actions/checkout",
        ):
            self.assertNotIn(forbidden, text)


if __name__ == "__main__":
    unittest.main()
