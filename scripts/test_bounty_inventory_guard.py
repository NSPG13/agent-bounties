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
    resolved = list(args)
    if "--meta-threshold" not in resolved:
        resolved.extend(["--meta-threshold", "0"])
    if "--meta-replenishment-target" not in resolved:
        resolved.extend(["--meta-replenishment-target", "0"])
    return subprocess.run(
        [sys.executable, str(SCRIPT), *resolved],
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


def standing_meta_report(*, corrupt_code_hash: bool = False) -> Path:
    data = json.loads(
        (FIXTURES / "bounty_inventory_claimable_above.json").read_text(
            encoding="utf-8"
        )
    )
    data["observed_at"] = datetime.now(timezone.utc).isoformat()
    item = data["verified_claimable_bounties"][0]
    item.update(
        {
            "verification_mode": "deterministic_module",
            "verifier_module": "0xe573cb4f471d38b5bf10ce82237251ac902c9867",
            "verification_ready": True,
            "standing_meta_bounty": {
                "schema_version": "agent-bounties/standing-meta-bounty-v2",
                "inventory_class": "post_bounty_third_party_completion",
                "verifier_protocol": "agent-bounties/independent-child-v2",
                "verifier_module": "0xe573cb4f471d38b5bf10ce82237251ac902c9867",
                "verifier_runtime_code_hash": (
                    "0x" + "66" * 32
                    if corrupt_code_hash
                    else "0xe3b6e82880edee69b1f30560506ac80a46b4ebcc6c083cfa8207e3673eede26c"
                ),
                "acceptance_criteria_hash": "0x25c41d7d51e2c807754b901733de17cdb1778dbd353f86347ff33e10289fcb54",
                "requires_funded_canonical_child": True,
                "requires_different_solver_wallet": True,
                "required_child_status": "settled",
                "observed_block_number": 74565,
                "observed_block_hash": "0x" + "dd" * 32,
            },
        }
    )
    target = ROOT / "target" / "tmp" / (
        "standing-meta-corrupt.json" if corrupt_code_hash else "standing-meta.json"
    )
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(data), encoding="utf-8")
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

    def test_standing_meta_floor_and_replenishment_buffer(self) -> None:
        proc = run_guard(
            "--fixture",
            str(FIXTURES / "bounty_inventory_above.json"),
            "--threshold",
            "5",
            "--meta-threshold",
            "1",
            "--meta-replenishment-target",
            "2",
            "--claimable-report",
            str(standing_meta_report()),
            "--fail-below",
        )
        self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
        payload = json.loads(proc.stdout.split("--- JSON ---", 1)[1])
        self.assertEqual(payload["verified_meta_claimable_count"], 1)
        self.assertFalse(payload["meta_below_threshold"])
        self.assertTrue(payload["meta_replenishment_required"])
        self.assertEqual(payload["meta_replenishment_count"], 1)
        self.assertFalse(payload["below_threshold"])

    def test_general_inventory_cannot_substitute_for_standing_meta_floor(self) -> None:
        proc = run_guard(
            "--fixture",
            str(FIXTURES / "bounty_inventory_above.json"),
            "--threshold",
            "5",
            "--meta-threshold",
            "1",
            "--meta-replenishment-target",
            "2",
            "--claimable-report",
            str(current_claimable_report("bounty_inventory_claimable_above.json")),
            "--fail-below",
        )
        self.assertEqual(proc.returncode, 2, proc.stderr + proc.stdout)
        payload = json.loads(proc.stdout.split("--- JSON ---", 1)[1])
        self.assertEqual(payload["verified_claimable_count"], 5)
        self.assertEqual(payload["verified_meta_claimable_count"], 0)
        self.assertTrue(payload["meta_below_threshold"])

    def test_spoofed_standing_meta_descriptor_invalidates_evidence(self) -> None:
        proc = run_guard(
            "--fixture",
            str(FIXTURES / "bounty_inventory_above.json"),
            "--threshold",
            "5",
            "--meta-threshold",
            "1",
            "--meta-replenishment-target",
            "2",
            "--claimable-report",
            str(standing_meta_report(corrupt_code_hash=True)),
            "--fail-below",
        )
        self.assertEqual(proc.returncode, 2, proc.stderr + proc.stdout)
        payload = json.loads(proc.stdout.split("--- JSON ---", 1)[1])
        self.assertFalse(payload["inventory_evidence_valid"])
        self.assertEqual(payload["verified_meta_claimable_count"], 0)

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

    def test_direct_safe_chain_evidence_does_not_require_hosted_health(self) -> None:
        proc = run_guard(
            "--fixture",
            str(FIXTURES / "bounty_inventory_above.json"),
            "--threshold",
            "1",
            "--claimable-report",
            str(current_claimable_report("bounty_inventory_claimable_direct.json")),
            "--fail-below",
        )
        self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
        payload = json.loads(proc.stdout.split("--- JSON ---", 1)[1])
        self.assertEqual(payload["verified_claimable_count"], 1)
        self.assertTrue(payload["inventory_evidence_valid"])

    def test_direct_latest_block_evidence_fails_closed(self) -> None:
        report = json.loads(
            (FIXTURES / "bounty_inventory_claimable_direct.json").read_text(
                encoding="utf-8"
            )
        )
        report["observed_at"] = datetime.now(timezone.utc).isoformat()
        report["direct_chain_observed_block"]["tag"] = "latest"
        target = ROOT / "target" / "tmp" / "direct-latest-inventory.json"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(report), encoding="utf-8")
        proc = run_guard(
            "--fixture",
            str(FIXTURES / "bounty_inventory_above.json"),
            "--threshold",
            "1",
            "--claimable-report",
            str(target),
            "--fail-below",
        )
        self.assertEqual(proc.returncode, 2, proc.stderr + proc.stdout)
        payload = json.loads(proc.stdout.split("--- JSON ---", 1)[1])
        self.assertEqual(payload["verified_claimable_count"], 0)
        self.assertFalse(payload["inventory_evidence_valid"])

    def test_direct_active_factory_with_no_claimable_inventory_is_valid_below(self) -> None:
        report = json.loads(
            (FIXTURES / "bounty_inventory_claimable_direct.json").read_text(
                encoding="utf-8"
            )
        )
        report["observed_at"] = datetime.now(timezone.utc).isoformat()
        report["direct_chain_status"] = "no_claimable_bounties"
        report["verified_claimable_bounties"] = []
        report["warnings"].append("no_verified_funded_bounty_is_claimable")
        target = ROOT / "target" / "tmp" / "direct-empty-inventory.json"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(report), encoding="utf-8")
        proc = run_guard(
            "--fixture",
            str(FIXTURES / "bounty_inventory_above.json"),
            "--threshold",
            "5",
            "--claimable-report",
            str(target),
            "--fail-below",
        )
        self.assertEqual(proc.returncode, 2, proc.stderr + proc.stdout)
        payload = json.loads(proc.stdout.split("--- JSON ---", 1)[1])
        self.assertEqual(payload["verified_claimable_count"], 0)
        self.assertTrue(payload["inventory_evidence_valid"])
        self.assertTrue(payload["below_threshold"])
        self.assertEqual(payload["missing_count"], 5)

    def test_direct_status_and_items_must_agree(self) -> None:
        report = json.loads(
            (FIXTURES / "bounty_inventory_claimable_direct.json").read_text(
                encoding="utf-8"
            )
        )
        report["observed_at"] = datetime.now(timezone.utc).isoformat()
        report["direct_chain_status"] = "no_claimable_bounties"
        target = ROOT / "target" / "tmp" / "direct-inconsistent-inventory.json"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(report), encoding="utf-8")
        proc = run_guard(
            "--fixture",
            str(FIXTURES / "bounty_inventory_above.json"),
            "--threshold",
            "1",
            "--claimable-report",
            str(target),
            "--fail-below",
        )
        self.assertEqual(proc.returncode, 2, proc.stderr + proc.stdout)
        payload = json.loads(proc.stdout.split("--- JSON ---", 1)[1])
        self.assertFalse(payload["inventory_evidence_valid"])

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
