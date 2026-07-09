#!/usr/bin/env python3
"""Tests for bounty_inventory_guard.py (no network)."""

from __future__ import annotations

import json
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = Path(__file__).resolve().parent / "bounty_inventory_guard.py"
FIXTURES = Path(__file__).resolve().parent / "fixtures"


def run_guard(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        cwd=str(ROOT),
        capture_output=True,
        text=True,
        check=False,
    )


class BountyInventoryGuardTests(unittest.TestCase):
    def test_above_threshold(self) -> None:
        proc = run_guard(
            "--fixture",
            str(FIXTURES / "bounty_inventory_above.json"),
            "--threshold",
            "5",
            "--repository",
            "example/repo",
        )
        self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
        self.assertIn("OK", proc.stdout)
        # JSON section
        payload = json.loads(proc.stdout.split("--- JSON ---", 1)[1])
        self.assertEqual(payload["open_bounty_count"], 6)
        self.assertFalse(payload["below_threshold"])
        self.assertEqual(payload["missing_count"], 0)
        self.assertIn("does not imply", payload["disclaimer"].lower())

    def test_below_threshold_fail_below(self) -> None:
        proc = run_guard(
            "--fixture",
            str(FIXTURES / "bounty_inventory_below.json"),
            "--threshold",
            "5",
            "--fail-below",
        )
        self.assertEqual(proc.returncode, 2, proc.stderr + proc.stdout)
        payload = json.loads(proc.stdout.split("--- JSON ---", 1)[1])
        self.assertEqual(payload["open_bounty_count"], 2)
        self.assertTrue(payload["below_threshold"])
        self.assertEqual(payload["missing_count"], 3)

    def test_noisy_excludes_non_actionable(self) -> None:
        proc = run_guard(
            "--fixture",
            str(FIXTURES / "bounty_inventory_noisy.json"),
            "--threshold",
            "5",
        )
        self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
        payload = json.loads(proc.stdout.split("--- JSON ---", 1)[1])
        # 1,7,8,9 actionable = 4; 2 duplicate, 3 closed, 4 no bounty, 5 PR, 6 invalid
        self.assertEqual(payload["open_bounty_count"], 4)
        self.assertTrue(payload["below_threshold"])
        self.assertEqual(payload["missing_count"], 1)
        self.assertEqual(len(payload["issue_urls"]), 4)


if __name__ == "__main__":
    unittest.main()
