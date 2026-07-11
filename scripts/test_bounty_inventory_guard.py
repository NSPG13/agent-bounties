#!/usr/bin/env python3
"""Tests for bounty_inventory_guard.py (no network)."""

from __future__ import annotations

import json
import subprocess
import sys
import unittest
from datetime import datetime, timezone
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


def current_claimable_report(name: str, *, bom: bool = False) -> Path:
    data = json.loads((FIXTURES / name).read_text(encoding="utf-8"))
    data["observed_at"] = datetime.now(timezone.utc).isoformat()
    target = ROOT / "target" / "tmp" / f"current-{name}"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(
        json.dumps(data),
        encoding="utf-8-sig" if bom else "utf-8",
    )
    return target


class BountyInventoryGuardTests(unittest.TestCase):
    def test_claimable_report_accepts_utf8_bom(self) -> None:
        bom_report = current_claimable_report(
            "bounty_inventory_claimable_above.json", bom=True
        )
        proc = run_guard(
            "--fixture",
            str(FIXTURES / "bounty_inventory_above.json"),
            "--claimable-report",
            str(bom_report),
            "--threshold",
            "5",
            "--fail-below",
        )
        self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)

    def test_above_threshold(self) -> None:
        proc = run_guard(
            "--fixture",
            str(FIXTURES / "bounty_inventory_above.json"),
            "--threshold",
            "5",
            "--claimable-report",
            str(current_claimable_report("bounty_inventory_claimable_above.json")),
            "--repository",
            "example/repo",
        )
        self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
        self.assertIn("OK", proc.stdout)
        # JSON section
        payload = json.loads(proc.stdout.split("--- JSON ---", 1)[1])
        self.assertEqual(payload["open_bounty_count"], 6)
        self.assertEqual(payload["verified_claimable_count"], 5)
        self.assertTrue(payload["inventory_evidence_valid"])
        self.assertFalse(payload["below_threshold"])
        self.assertEqual(payload["missing_count"], 0)
        self.assertIn("does not imply", payload["disclaimer"].lower())

    def test_below_threshold_fail_below(self) -> None:
        proc = run_guard(
            "--fixture",
            str(FIXTURES / "bounty_inventory_below.json"),
            "--threshold",
            "5",
            "--claimable-report",
            str(current_claimable_report("bounty_inventory_claimable_below.json")),
            "--fail-below",
        )
        self.assertEqual(proc.returncode, 2, proc.stderr + proc.stdout)
        payload = json.loads(proc.stdout.split("--- JSON ---", 1)[1])
        self.assertEqual(payload["open_bounty_count"], 2)
        self.assertEqual(payload["verified_claimable_count"], 2)
        self.assertTrue(payload["below_threshold"])
        self.assertEqual(payload["missing_count"], 3)

    def test_noisy_excludes_non_actionable(self) -> None:
        proc = run_guard(
            "--fixture",
            str(FIXTURES / "bounty_inventory_noisy.json"),
            "--threshold",
            "5",
            "--claimable-report",
            str(current_claimable_report("bounty_inventory_claimable_unavailable.json")),
        )
        self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
        payload = json.loads(proc.stdout.split("--- JSON ---", 1)[1])
        # 1,7,8,9 actionable = 4; 10 is activation-blocked.
        self.assertEqual(payload["open_bounty_count"], 4)
        self.assertEqual(payload["verified_claimable_count"], 0)
        self.assertFalse(payload["inventory_evidence_valid"])
        self.assertTrue(payload["below_threshold"])
        self.assertEqual(payload["missing_count"], 5)
        self.assertEqual(len(payload["issue_urls"]), 4)
        self.assertEqual(payload["excluded_count"], 6)

    def test_malformed_claimable_entry_fails_closed(self) -> None:
        proc = run_guard(
            "--fixture",
            str(FIXTURES / "bounty_inventory_above.json"),
            "--threshold",
            "1",
            "--claimable-report",
            str(current_claimable_report("bounty_inventory_claimable_malformed.json")),
            "--fail-below",
        )
        self.assertEqual(proc.returncode, 2, proc.stderr + proc.stdout)
        payload = json.loads(proc.stdout.split("--- JSON ---", 1)[1])
        self.assertEqual(payload["verified_claimable_count"], 0)
        self.assertFalse(payload["inventory_evidence_valid"])

    def test_stale_claimable_report_fails_closed(self) -> None:
        report = json.loads(
            (FIXTURES / "bounty_inventory_claimable_above.json").read_text(
                encoding="utf-8"
            )
        )
        report["observed_at"] = "2000-01-01T00:00:00+00:00"
        stale = ROOT / "target" / "tmp" / "stale-claimable-inventory.json"
        stale.parent.mkdir(parents=True, exist_ok=True)
        stale.write_text(json.dumps(report), encoding="utf-8")
        proc = run_guard(
            "--fixture",
            str(FIXTURES / "bounty_inventory_above.json"),
            "--threshold",
            "1",
            "--claimable-report",
            str(stale),
            "--fail-below",
        )
        self.assertEqual(proc.returncode, 2, proc.stderr + proc.stdout)
        payload = json.loads(proc.stdout.split("--- JSON ---", 1)[1])
        self.assertEqual(payload["verified_claimable_count"], 0)
        self.assertFalse(payload["inventory_evidence_valid"])

    def test_zero_threshold_cannot_override_invalid_evidence(self) -> None:
        proc = run_guard(
            "--fixture",
            str(FIXTURES / "bounty_inventory_above.json"),
            "--threshold",
            "0",
            "--claimable-report",
            str(current_claimable_report("bounty_inventory_claimable_unavailable.json")),
            "--fail-below",
        )
        self.assertEqual(proc.returncode, 2, proc.stderr + proc.stdout)
        payload = json.loads(proc.stdout.split("--- JSON ---", 1)[1])
        self.assertTrue(payload["below_threshold"])
        self.assertEqual(payload["missing_count"], 0)


if __name__ == "__main__":
    unittest.main()
