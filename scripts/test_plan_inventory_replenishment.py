#!/usr/bin/env python3
"""Unit tests for deterministic inventory replenishment planning (Issue #872)."""

from __future__ import annotations

from copy import deepcopy
from datetime import datetime, timezone
import unittest

from scripts.plan_inventory_replenishment import (
    CANDIDATE_MANIFEST_SCHEMA,
    DEFAULT_MAX_SNAPSHOT_AGE_SECONDS,
    INVENTORY_SCHEMA,
    NATIVE_BASE_USDC,
    PLAN_SCHEMA,
    generate_replenishment_plan,
)


VERIFIERS = [
    "0xbe6292b9e465f549e2363b918d6dd9187038431e",
    "0xb7c2ce6430b66fb986e27b6140b29309550d487a",
]


def _make_candidate(idx: int) -> dict:
    issue_num = 868 + idx
    digest_char = f"{idx:01x}"
    return {
        "candidate_id": f"direct-task-{idx}",
        "issue": issue_num,
        "title": f"Direct inventory task {idx}",
        "benchmark_subdirectory": f"benchmarks/direct-inventory-v1/task-{idx}",
        "benchmark_digest": "sha256:" + (digest_char * 64),
        "solver_reward": 1_000_000,
        "verifier_reward": 100_000,
        "claim_bond": 100_000,
        "initial_funding": 1_100_000,
        "threshold": 2,
        "verifiers": list(VERIFIERS),
    }


def _base_inventory(ready_count: int = 0) -> dict:
    return {
        "schema": INVENTORY_SCHEMA,
        "observed_at": "2026-10-22T20:00:00Z",
        "safe_block": 21_500_000,
        "ready_count": ready_count,
        "in_progress_count": 1,
        "submitted_count": 0,
        "paid_count": 12,
        "verification_unavailable_count": 0,
        "source_degraded": False,
        "active_issues": [],
    }


def _base_candidate_manifest(count: int = 5) -> dict:
    return {
        "schema": CANDIDATE_MANIFEST_SCHEMA,
        "chain_id": 8453,
        "wallet": "0x1eaa1c68772cf76bc5f4e4174766076e33ace662",
        "settlement_token": NATIVE_BASE_USDC,
        "policy_version": 5,
        "policy_hash": "0xac8c86836e1cab2b4707420057d414b0dfa675006bda6584df7fb9118caabe88",
        "wallet_state": {
            "wallet_balance": 10_000_000,
            "period_spent": 0,
            "max_per_period": 10_000_000,
            "lifetime_spent": 2_000_000,
            "max_lifetime_spend": 25_000_000,
            "max_per_action": 2_000_000,
            "max_bounty_target": 2_000_000,
            "revoked": False,
        },
        "candidates": [_make_candidate(i) for i in range(1, count + 1)],
    }


class PlanInventoryReplenishmentTests(unittest.TestCase):
    def test_sufficient_inventory_plans_zero_tasks(self) -> None:
        inventory = _base_inventory(ready_count=5)
        manifest = _base_candidate_manifest(count=5)
        plan = generate_replenishment_plan(
            inventory, manifest, now="2026-10-22T20:05:00Z", target_ready_count=5
        )
        self.assertEqual(plan["schema"], PLAN_SCHEMA)
        self.assertEqual(plan["status"], "no_replenishment_needed")
        self.assertEqual(plan["next_authorized_action"], "NONE")
        self.assertEqual(plan["inventory"]["deficit"], 0)
        self.assertEqual(plan["economics"]["candidate_count"], 0)
        self.assertEqual(plan["economics"]["total_funding"], 0)
        self.assertEqual(plan["candidates"], [])
        self.assertEqual(plan["blockers"], [])
        self.assertFalse(plan["financial_action_taken"])

    def test_exact_five_task_replenishment_is_deterministic_and_idempotent(self) -> None:
        inventory = _base_inventory(ready_count=0)
        manifest = _base_candidate_manifest(count=6)
        plan_a = generate_replenishment_plan(
            inventory, manifest, now="2026-10-22T20:05:00Z", target_ready_count=5
        )
        # Shuffle candidate order to verify deterministic selection and idempotency key
        shuffled_manifest = deepcopy(manifest)
        shuffled_manifest["candidates"] = list(reversed(shuffled_manifest["candidates"]))
        plan_b = generate_replenishment_plan(
            inventory, shuffled_manifest, now="2026-10-22T20:05:00Z", target_ready_count=5
        )

        self.assertEqual(plan_a["status"], "ready_to_plan")
        self.assertEqual(plan_a["next_authorized_action"], "DISPATCH_CANDIDATE_TASKS_NON_FINANCIAL")
        self.assertEqual(plan_a["verifier_readiness"]["status"], "READY")
        self.assertEqual(plan_a["inventory"]["deficit"], 5)
        self.assertEqual(plan_a["economics"]["candidate_count"], 5)
        self.assertEqual(plan_a["economics"]["per_task_funding"], 1_100_000)
        self.assertEqual(plan_a["economics"]["total_funding"], 5_500_000)
        self.assertEqual(plan_a["economics"]["min_solver_margin"], 900_000)
        self.assertEqual(
            [c["issue"] for c in plan_a["candidates"]],
            [869, 870, 871, 872, 873],
        )
        self.assertEqual(plan_a["idempotency_key"], plan_b["idempotency_key"])
        self.assertFalse(plan_a["financial_action_taken"])
        self.assertFalse(plan_a["broadcasted"])
        self.assertEqual(plan_a["labels_applied"], [])

    def test_insufficient_wallet_balance_blocks_plan(self) -> None:
        inventory = _base_inventory(ready_count=0)
        manifest = _base_candidate_manifest(count=5)
        manifest["wallet_state"]["wallet_balance"] = 5_499_999
        plan = generate_replenishment_plan(
            inventory, manifest, now="2026-10-22T20:02:00Z", target_ready_count=5
        )
        self.assertEqual(plan["status"], "blocked")
        self.assertEqual(plan["next_authorized_action"], "RESOLVE_BLOCKERS")
        self.assertIn("insufficient_wallet_balance", plan["blockers"])
        self.assertFalse(plan["sufficiency"]["wallet_sufficient"])
        self.assertEqual(plan["candidates"], [])
        self.assertFalse(plan["financial_action_taken"])

    def test_period_and_lifetime_cap_exhaustion_blocks_plan(self) -> None:
        inventory = _base_inventory(ready_count=0)
        manifest = _base_candidate_manifest(count=5)
        manifest["wallet_state"]["period_spent"] = 5_000_000
        manifest["wallet_state"]["max_per_period"] = 10_000_000  # only 5.0 USDC remaining < 5.5 USDC
        manifest["wallet_state"]["lifetime_spent"] = 20_000_000
        manifest["wallet_state"]["max_lifetime_spend"] = 25_000_000  # only 5.0 USDC remaining < 5.5 USDC
        plan = generate_replenishment_plan(
            inventory, manifest, now="2026-10-22T20:02:00Z", target_ready_count=5
        )
        self.assertEqual(plan["status"], "blocked")
        self.assertIn("period_cap_exceeded", plan["blockers"])
        self.assertIn("lifetime_cap_exceeded", plan["blockers"])
        self.assertFalse(plan["sufficiency"]["period_sufficient"])
        self.assertFalse(plan["sufficiency"]["lifetime_sufficient"])
        self.assertEqual(plan["candidates"], [])

    def test_stale_inventory_evidence_is_rejected(self) -> None:
        inventory = _base_inventory(ready_count=0)
        manifest = _base_candidate_manifest(count=5)
        # 25 minutes old (> 900s max_snapshot_age_seconds)
        plan = generate_replenishment_plan(
            inventory,
            manifest,
            now="2026-10-22T20:25:00Z",
            target_ready_count=5,
            max_snapshot_age_seconds=900,
        )
        self.assertEqual(plan["status"], "blocked")
        self.assertIn("stale_inventory_snapshot", plan["blockers"])
        self.assertEqual(plan["candidates"], [])

    def test_duplicate_candidates_and_non_positive_margin_are_rejected(self) -> None:
        inventory = _base_inventory(ready_count=0)
        manifest = _base_candidate_manifest(count=5)
        # Make candidate 2 a duplicate of candidate 1
        manifest["candidates"][1] = deepcopy(manifest["candidates"][0])
        # Give candidate 3 a non-positive solver margin (claim_bond >= solver_reward)
        manifest["candidates"][2]["claim_bond"] = 1_000_000
        manifest["candidates"][2]["verifier_reward"] = 1_000_000
        manifest["candidates"][2]["initial_funding"] = 2_000_000

        plan = generate_replenishment_plan(
            inventory, manifest, now="2026-10-22T20:01:00Z", target_ready_count=5
        )
        self.assertEqual(plan["status"], "blocked")
        self.assertTrue(any(b.startswith("duplicate_candidate:") for b in plan["blockers"]))
        self.assertTrue(any(b.startswith("non_positive_solver_margin:") for b in plan["blockers"]))
        self.assertEqual(plan["candidates"], [])

    def test_stale_snapshot_rejected_against_current_clock(self) -> None:
        """Verify that omitting `now` ages a historical snapshot against the real current clock."""
        inventory = _base_inventory(ready_count=0)
        inventory["observed_at"] = "2020-01-01T00:00:00Z"
        manifest = _base_candidate_manifest(count=5)
        plan = generate_replenishment_plan(inventory, manifest, target_ready_count=5)
        self.assertEqual(plan["status"], "blocked")
        self.assertIn("stale_inventory_snapshot", plan["blockers"])
        self.assertGreater(plan["inventory"]["snapshot_age_seconds"], DEFAULT_MAX_SNAPSHOT_AGE_SECONDS)
        self.assertEqual(plan["candidates"], [])

    def test_changing_benchmark_or_verifiers_changes_idempotency_key(self) -> None:
        """Verify that altering benchmark digest, subdirectory, or verifiers set alters idempotency key."""
        inventory = _base_inventory(ready_count=0)
        base_manifest = _base_candidate_manifest(count=5)
        plan_base = generate_replenishment_plan(
            inventory, base_manifest, now="2026-10-22T20:05:00Z", target_ready_count=5
        )
        self.assertEqual(plan_base["status"], "ready_to_plan")

        # 1. Alter benchmark_digest
        manifest_alt_digest = deepcopy(base_manifest)
        manifest_alt_digest["candidates"][0]["benchmark_digest"] = "sha256:" + ("f" * 64)
        plan_alt_digest = generate_replenishment_plan(
            inventory, manifest_alt_digest, now="2026-10-22T20:05:00Z", target_ready_count=5
        )
        self.assertNotEqual(
            plan_base["idempotency_key"],
            plan_alt_digest["idempotency_key"],
            "Changing benchmark_digest must produce a distinct idempotency key",
        )

        # 2. Alter benchmark_subdirectory
        manifest_alt_subdir = deepcopy(base_manifest)
        manifest_alt_subdir["candidates"][0]["benchmark_subdirectory"] = "benchmarks/direct-inventory-v1/task-alt"
        plan_alt_subdir = generate_replenishment_plan(
            inventory, manifest_alt_subdir, now="2026-10-22T20:05:00Z", target_ready_count=5
        )
        self.assertNotEqual(
            plan_base["idempotency_key"],
            plan_alt_subdir["idempotency_key"],
            "Changing benchmark_subdirectory must produce a distinct idempotency key",
        )

        # 3. Alter verifiers set
        manifest_alt_verifiers = deepcopy(base_manifest)
        manifest_alt_verifiers["candidates"][0]["verifiers"] = [
            "0x0000000000000000000000000000000000000001",
            "0x0000000000000000000000000000000000000002",
        ]
        plan_alt_verifiers = generate_replenishment_plan(
            inventory, manifest_alt_verifiers, now="2026-10-22T20:05:00Z", target_ready_count=5
        )
        self.assertNotEqual(
            plan_base["idempotency_key"],
            plan_alt_verifiers["idempotency_key"],
            "Changing verifiers set must produce a distinct idempotency key",
        )

    def test_missing_identity_policy_fields_and_zero_caps_are_rejected(self) -> None:
        """Verify that missing identity, missing policy fields, and zero caps are rejected."""
        inventory = _base_inventory(ready_count=0)

        # Missing wallet identity
        manifest_no_wallet = _base_candidate_manifest(count=5)
        del manifest_no_wallet["wallet"]
        plan = generate_replenishment_plan(inventory, manifest_no_wallet, now="2026-10-22T20:05:00Z")
        self.assertEqual(plan["status"], "blocked")
        self.assertIn("missing_wallet_identity", plan["blockers"])

        # Zero-address wallet
        manifest_zero_wallet = _base_candidate_manifest(count=5)
        manifest_zero_wallet["wallet"] = "0x" + "00" * 20
        plan = generate_replenishment_plan(inventory, manifest_zero_wallet, now="2026-10-22T20:05:00Z")
        self.assertEqual(plan["status"], "blocked")
        self.assertIn("missing_wallet_identity", plan["blockers"])

        # Missing policy_version
        manifest_no_pv = _base_candidate_manifest(count=5)
        del manifest_no_pv["policy_version"]
        plan = generate_replenishment_plan(inventory, manifest_no_pv, now="2026-10-22T20:05:00Z")
        self.assertEqual(plan["status"], "blocked")
        self.assertIn("missing_policy_version", plan["blockers"])

        # Missing policy_hash
        manifest_no_ph = _base_candidate_manifest(count=5)
        del manifest_no_ph["policy_hash"]
        plan = generate_replenishment_plan(inventory, manifest_no_ph, now="2026-10-22T20:05:00Z")
        self.assertEqual(plan["status"], "blocked")
        self.assertIn("missing_policy_hash", plan["blockers"])

        # Missing chain_id
        manifest_no_chain = _base_candidate_manifest(count=5)
        del manifest_no_chain["chain_id"]
        plan = generate_replenishment_plan(inventory, manifest_no_chain, now="2026-10-22T20:05:00Z")
        self.assertEqual(plan["status"], "blocked")
        self.assertIn("missing_or_wrong_chain_id", plan["blockers"])

        # Missing settlement_token
        manifest_no_token = _base_candidate_manifest(count=5)
        del manifest_no_token["settlement_token"]
        plan = generate_replenishment_plan(inventory, manifest_no_token, now="2026-10-22T20:05:00Z")
        self.assertEqual(plan["status"], "blocked")
        self.assertIn("missing_settlement_token", plan["blockers"])

        # Missing safe_block in inventory
        inv_no_safe_block = _base_inventory(ready_count=0)
        del inv_no_safe_block["safe_block"]
        plan = generate_replenishment_plan(inv_no_safe_block, _base_candidate_manifest(count=5), now="2026-10-22T20:05:00Z")
        self.assertEqual(plan["status"], "blocked")
        self.assertIn("missing_safe_block", plan["blockers"])

        # Zero max_per_period cap
        manifest_zero_period = _base_candidate_manifest(count=5)
        manifest_zero_period["wallet_state"]["max_per_period"] = 0
        plan = generate_replenishment_plan(inventory, manifest_zero_period, now="2026-10-22T20:05:00Z")
        self.assertEqual(plan["status"], "blocked")
        self.assertIn("zero_period_cap", plan["blockers"])

        # Zero max_lifetime_spend cap
        manifest_zero_lifetime = _base_candidate_manifest(count=5)
        manifest_zero_lifetime["wallet_state"]["max_lifetime_spend"] = 0
        plan = generate_replenishment_plan(inventory, manifest_zero_lifetime, now="2026-10-22T20:05:00Z")
        self.assertEqual(plan["status"], "blocked")
        self.assertIn("zero_lifetime_cap", plan["blockers"])

        # Zero max_per_action cap
        manifest_zero_action = _base_candidate_manifest(count=5)
        manifest_zero_action["wallet_state"]["max_per_action"] = 0
        plan = generate_replenishment_plan(inventory, manifest_zero_action, now="2026-10-22T20:05:00Z")
        self.assertEqual(plan["status"], "blocked")
        self.assertIn("zero_action_cap", plan["blockers"])

        # Task exceeding per-action cap
        manifest_exceed_action = _base_candidate_manifest(count=5)
        manifest_exceed_action["wallet_state"]["max_per_action"] = 1_000_000  # task initial_funding is 1.1 USDC
        plan = generate_replenishment_plan(inventory, manifest_exceed_action, now="2026-10-22T20:05:00Z")
        self.assertEqual(plan["status"], "blocked")
        self.assertTrue(any(b.startswith("task_exceeds_action_cap:") for b in plan["blockers"]))


if __name__ == "__main__":
    unittest.main()
