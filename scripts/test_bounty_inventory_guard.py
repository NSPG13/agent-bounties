#!/usr/bin/env python3
"""Tests for bounty_inventory_guard.py (no network)."""

from __future__ import annotations

import json
import subprocess
import sys
import unittest
from datetime import datetime, timezone
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = Path(__file__).resolve().parent / "bounty_inventory_guard.py"
FIXTURES = Path(__file__).resolve().parent / "fixtures"
PRIVATE_REPORT = ROOT / "target" / "tmp" / "inventory-guard-private.json"
if str(SCRIPT.parent) not in sys.path:
    sys.path.insert(0, str(SCRIPT.parent))

import bounty_inventory_guard as GUARD


def run_guard(
    *args: str, use_meta_defaults: bool = False
) -> subprocess.CompletedProcess[str]:
    resolved = list(args)
    if not use_meta_defaults and "--meta-threshold" not in resolved:
        resolved.extend(["--meta-threshold", "0"])
    if not use_meta_defaults and "--meta-replenishment-target" not in resolved:
        resolved.extend(["--meta-replenishment-target", "0"])
    if "--private-v2-floor" not in resolved:
        resolved.extend(["--private-v2-floor", "0"])
    if "--private-v2-target" not in resolved:
        resolved.extend(["--private-v2-target", "0"])
    if "--private-json-out" not in resolved:
        resolved.extend(["--private-json-out", str(PRIVATE_REPORT)])
    return subprocess.run(
        [sys.executable, str(SCRIPT), *resolved],
        cwd=str(ROOT),
        capture_output=True,
        text=True,
        check=False,
    )


def private_payload() -> dict[str, object]:
    return json.loads(PRIVATE_REPORT.read_text(encoding="utf-8"))


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


def standing_meta_report(
    *, count: int = 1, corrupt_code_hash: bool = False
) -> Path:
    data = json.loads(
        (FIXTURES / "bounty_inventory_claimable_above.json").read_text(
            encoding="utf-8"
        )
    )
    data["observed_at"] = datetime.now(timezone.utc).isoformat()
    items = data["verified_claimable_bounties"]
    if count < 0 or count > len(items):
        raise ValueError("standing meta fixture count is out of range")
    for index, item in enumerate(items[:count]):
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
                        if corrupt_code_hash and index == 0
                        else "0xe3b6e82880edee69b1f30560506ac80a46b4ebcc6c083cfa8207e3673eede26c"
                    ),
                    "acceptance_criteria_hash": "0x25c41d7d51e2c807754b901733de17cdb1778dbd353f86347ff33e10289fcb54",
                    "requires_funded_canonical_child": True,
                    "requires_different_solver_wallet": True,
                    "required_child_status": "settled",
                    "observed_block_number": 74565 + index,
                    "observed_block_hash": "0x" + f"{221 + index:02x}" * 32,
                },
            }
        )
    target = ROOT / "target" / "tmp" / (
        "standing-meta-corrupt.json"
        if corrupt_code_hash
        else f"standing-meta-{count}.json"
    )
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(data), encoding="utf-8")
    return target


def open_competition_v2_report(*, count: int = 5) -> Path:
    data = json.loads(
        (FIXTURES / "bounty_inventory_claimable_below.json").read_text(
            encoding="utf-8"
        )
    )
    data["observed_at"] = datetime.now(timezone.utc).isoformat()
    data["verified_claimable_bounties"] = []
    data["open_competition_v2_status"] = "verified"
    data["open_competition_v2_observed_safe_block"] = 50_223_549
    data["open_competition_v2_release_hash"] = "0x" + "ab" * 32
    data["open_competition_v2_factory_contract"] = "0x" + "cd" * 20
    data["verified_open_competition_v2_bounties"] = [
        {
            "id": "0x" + f"{index + 1:02x}" * 32,
            "contract": "0x" + f"{index + 17:02x}" * 20,
            "solver_reward_minor": 3_000_000,
            "hosted_net_prize_if_win_minor": 2_890_000,
            "claim_bond_minor": 0,
            "currency": "usdc",
            "status": "claimable",
            "winner_mode": "first_proven",
            "proof_system": "groth16",
            "proof_deadline": int(datetime.now(timezone.utc).timestamp()) + 86_400,
            "accepted_entries": 0,
            "evidence": "confirmed_canonical_open_competition_v2",
            "observed_safe_block": 50_223_549,
            "source_url": "https://api.agentbounties.app/v1/base/open-competition-v2-beta3/inventory",
            "proof_quote_url": "https://api.agentbounties.app/v1/base/open-competition-v2-beta3/proof-quotes",
        }
        for index in range(count)
    ]
    target = ROOT / "target" / "tmp" / f"open-competition-v2-{count}.json"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(data), encoding="utf-8")
    return target


class BountyInventoryGuardTests(unittest.TestCase):
    def test_rpc_probe_uses_failover_pool_and_redacts_credentials(self) -> None:
        credentialed = "https://user:pass@rpc.example/v2/secret?api_key=hidden"
        with mock.patch.object(
            GUARD,
            "resolve_inventory_base_rpc",
            return_value=credentialed,
        ), mock.patch.object(
            GUARD,
            "rpc_failover",
            return_value="0xabc",
        ) as failover:
            result = GUARD.probe_inventory_rpc("https://preferred.example")

        self.assertEqual(
            result["base_rpc_preferred_endpoint"],
            "https://rpc.example",
        )
        self.assertTrue(result["failover_enabled"])
        self.assertEqual(result["eth_blockNumber"], "0xabc")
        failover.assert_called_once_with(
            "eth_blockNumber",
            [],
            preferred=credentialed,
            max_retries=2,
        )

    def test_default_standing_meta_floor_is_one_with_two_item_buffer(self) -> None:
        passing = run_guard(
            "--fixture",
            str(FIXTURES / "bounty_inventory_above.json"),
            "--threshold",
            "5",
            "--claimable-report",
            str(standing_meta_report(count=1)),
            "--fail-below",
            use_meta_defaults=True,
        )
        self.assertEqual(passing.returncode, 0, passing.stderr + passing.stdout)
        payload = private_payload()
        self.assertEqual(payload["meta_threshold"], 1)
        self.assertEqual(payload["meta_replenishment_target"], 2)
        self.assertEqual(payload["verified_meta_claimable_count"], 1)

        below = run_guard(
            "--fixture",
            str(FIXTURES / "bounty_inventory_above.json"),
            "--threshold",
            "5",
            "--claimable-report",
            str(standing_meta_report(count=0)),
            "--fail-below",
            use_meta_defaults=True,
        )
        self.assertEqual(below.returncode, 0, below.stderr + below.stdout)
        payload = private_payload()
        self.assertEqual(payload["verified_meta_claimable_count"], 0)
        self.assertTrue(payload["meta_below_threshold"])
        self.assertFalse(payload["below_threshold"])

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
        self.assertEqual(payload["active_funded_opportunities"], 5)
        self.assertTrue(payload["inventory_evidence_valid"])
        self.assertFalse(payload["below_public_floor"])
        self.assertEqual(payload["missing_to_public_floor"], 0)
        self.assertNotIn("claimable_bounty_ids", payload)
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
        payload = private_payload()
        self.assertEqual(payload["verified_meta_claimable_count"], 1)
        self.assertFalse(payload["meta_below_threshold"])
        self.assertTrue(payload["meta_replenishment_required"])
        self.assertEqual(payload["meta_replenishment_count"], 1)
        self.assertFalse(payload["below_threshold"])

    def test_general_inventory_reports_missing_meta_without_failing_liquidity(self) -> None:
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
        self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
        public_payload = json.loads(proc.stdout.split("--- JSON ---", 1)[1])
        payload = private_payload()
        self.assertNotIn("verified_meta_claimable_count", public_payload)
        self.assertEqual(payload["verified_claimable_count"], 5)
        self.assertEqual(payload["verified_meta_claimable_count"], 0)
        self.assertTrue(payload["meta_below_threshold"])
        self.assertFalse(payload["below_threshold"])

    def test_open_competition_v2_satisfies_general_inventory_floor(self) -> None:
        private_report = ROOT / "target" / "tmp" / "private-v2-inventory.json"
        proc = run_guard(
            "--fixture",
            str(FIXTURES / "bounty_inventory_above.json"),
            "--threshold",
            "5",
            "--claimable-report",
            str(open_competition_v2_report()),
            "--private-v2-floor",
            "5",
            "--private-v2-target",
            "10",
            "--private-json-out",
            str(private_report),
            "--fail-below",
        )
        self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
        payload = json.loads(proc.stdout.split("--- JSON ---", 1)[1])
        self.assertEqual(payload["active_funded_opportunities"], 5)
        self.assertNotIn("verified_open_competition_v2_count", payload)
        self.assertNotIn("private_v2_floor", payload)
        private_payload = json.loads(private_report.read_text(encoding="utf-8"))
        self.assertEqual(private_payload["verified_open_competition_v2_count"], 5)
        self.assertEqual(private_payload["private_v2_missing_to_target"], 5)
        self.assertTrue(private_payload["private_v2_replenishment_required"])
        self.assertTrue(payload["inventory_evidence_valid"])
        self.assertFalse(payload["below_public_floor"])

    def test_private_v2_floor_fails_closed_when_evidence_is_missing(self) -> None:
        proc = run_guard(
            "--fixture",
            str(FIXTURES / "bounty_inventory_above.json"),
            "--threshold",
            "5",
            "--claimable-report",
            str(current_claimable_report("bounty_inventory_claimable_above.json")),
            "--private-v2-floor",
            "5",
            "--private-v2-target",
            "10",
            "--fail-below",
        )
        self.assertEqual(proc.returncode, 2, proc.stderr + proc.stdout)
        payload = json.loads(proc.stdout.split("--- JSON ---", 1)[1])
        self.assertNotIn("verified_open_competition_v2_count", payload)
        self.assertIn("BELOW LIQUIDITY POLICY", proc.stdout)

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
        payload = private_payload()
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
        payload = private_payload()
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
        self.assertEqual(payload["active_funded_opportunities"], 1)
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
        self.assertEqual(payload["active_funded_opportunities"], 0)
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
        self.assertEqual(payload["active_funded_opportunities"], 0)
        self.assertTrue(payload["inventory_evidence_valid"])
        self.assertTrue(payload["below_public_floor"])
        self.assertEqual(payload["missing_to_public_floor"], 5)

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
        self.assertEqual(payload["active_funded_opportunities"], 0)
        self.assertFalse(payload["inventory_evidence_valid"])
        self.assertTrue(payload["below_public_floor"])
        self.assertEqual(payload["missing_to_public_floor"], 5)
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
        self.assertEqual(payload["active_funded_opportunities"], 0)
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
        self.assertEqual(payload["active_funded_opportunities"], 0)
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
        self.assertTrue(payload["below_public_floor"])
        self.assertEqual(payload["missing_to_public_floor"], 0)


if __name__ == "__main__":
    unittest.main()
