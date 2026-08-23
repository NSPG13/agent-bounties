#!/usr/bin/env python3
"""Deterministic, network-free tests for the private V2 replenishment planner."""

from __future__ import annotations

import copy
import hashlib
import json
import sys
import unittest
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / "scripts"
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))

import plan_open_competition_v2_replenishment as PLANNER

SPECS_PATH = ROOT / "ops" / "open-competition-v2-forward-gmv-candidate-pool-v2.json"
LEDGER_PATH = ROOT / "ops" / "open-competition-v2-replenishment-ledger-v1.example.json"
NOW = datetime(2026, 8, 23, 8, 10, tzinfo=timezone.utc)
RELEASE_HASH = "0x46008fb819726a43209e55ee7e58c92700a8fc3435f76be282bfdef710ced594"
FACTORY = "0x29d0e39e0c03797c690633535722e6b34a69a78a"


def load_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def inventory(active: int, *, observed_at: str = "2026-08-23T08:09:00Z") -> dict:
    return {
        "inventory_evidence_valid": True,
        "private_gmv_meta_floor": 5,
        "private_gmv_meta_target": 10,
        "verified_gmv_meta_competition_count": active,
        "private_v2_observed_safe_block": 50_266_417,
        "private_v2_release_hash": RELEASE_HASH,
        "private_v2_factory_contract": FACTORY,
        "private_v2_gmv_profile": dict(PLANNER.REQUIRED_LIVE_GMV_PROFILE),
        "private_inventory_observed_at": observed_at,
    }


def reviewed_specs(specs: dict) -> dict:
    """Bind the scheduled pool to a deterministic reviewed live profile."""
    result = copy.deepcopy(specs)
    profile = result["profile_release"]
    profile["status"] = "reviewed"
    for key in ("program_vkey", "source_hash", "elf_hash"):
        profile[key] = PLANNER.REQUIRED_LIVE_GMV_PROFILE[key]
    return result


def synthetic_private_ranking(specs: dict) -> dict:
    """Build a non-production ranking fixture without publishing operator scores."""
    ranked = []
    for index, candidate in enumerate(specs["candidates"]):
        ranked.append(
            {
                "candidate_id": candidate["candidate_id"],
                "launch_role": "initial" if index < 10 else "standby",
                "scores": {
                    "real_user_evidence": 80,
                    "gmv_impact": 80,
                    "evidence_quality": 80,
                },
            }
        )
    return {
        "schema_version": PLANNER.PRIVATE_RANKING_SCHEMA,
        "ranking_weights": {
            "real_user_evidence": 50,
            "gmv_impact": 30,
            "evidence_quality": 20,
        },
        "ranked_candidates": ranked,
    }


def execution(
    index: int,
    *,
    status: str = "planned",
    occurred_at: str = "2026-08-23T08:05:00Z",
    candidate_id: str | None = None,
    amount: int | float = PLANNER.TOTAL_PER_COMPETITION_BASE_UNITS,
) -> dict:
    return {
        "idempotency_key": "0x" + hashlib.sha256(f"execution-{index}".encode()).hexdigest(),
        "candidate_id": candidate_id or f"historical-{index}-v1",
        "status": status,
        "occurred_at": occurred_at,
        "amount_base_units": amount,
    }


class ReplenishmentPlannerTests(unittest.TestCase):
    def setUp(self) -> None:
        checked_in_specs = load_json(SPECS_PATH)
        self.pending_specs = copy.deepcopy(checked_in_specs)
        for candidate in self.pending_specs["candidates"]:
            candidate["snapshot"]["status"] = "pending"
        self.specs = reviewed_specs(checked_in_specs)
        self.ranking = synthetic_private_ranking(self.specs)
        self.ledger = load_json(LEDGER_PATH)

    def plan(self, active: int, **overrides: object) -> dict:
        return PLANNER.build_plan(
            overrides.get("inventory_report", inventory(active)),
            overrides.get("candidate_specs", self.specs),
            overrides.get("private_ranking", self.ranking),
            overrides.get("ledger", self.ledger),
            now=overrides.get("now", NOW),
        )

    def test_inventory_states_restore_exact_target(self) -> None:
        cases = {
            10: ("noop", 0, "none"),
            9: ("ready", 1, "warning"),
            5: ("ready", 5, "warning"),
            4: ("ready", 6, "critical"),
            0: ("ready", 10, "critical"),
        }
        for active, (status, selected, severity) in cases.items():
            with self.subTest(active=active):
                plan = self.plan(active)
                self.assertEqual(plan["status"], status)
                self.assertEqual(len(plan["selected_candidates"]), selected)
                self.assertEqual(plan["severity"], severity)
                if status == "ready":
                    self.assertEqual(
                        plan["policy"]["required_spend_base_units"],
                        selected * PLANNER.TOTAL_PER_COMPETITION_BASE_UNITS,
                    )

    def test_reviewed_fixture_has_twenty_meta_campaigns_and_private_ranking_is_separate(self) -> None:
        self.assertEqual(len(self.specs["candidates"]), 20)
        serialized = json.dumps(self.specs)
        self.assertNotIn("ranking_weights", serialized)
        self.assertNotIn("launch_role", serialized)
        self.assertNotIn('"scores"', serialized)
        self.assertEqual(self.plan(0)["status"], "ready")

    def test_pool_fails_closed_when_forward_campaign_policy_is_not_scheduled(self) -> None:
        plan = self.plan(
            0,
            candidate_specs=self.pending_specs,
            private_ranking=synthetic_private_ranking(self.pending_specs),
        )
        self.assertEqual(plan["status"], "blocked")
        self.assertFalse(any("profile" in blocker for blocker in plan["blockers"]))
        self.assertTrue(any("scheduled" in blocker for blocker in plan["blockers"]))

    def test_checked_in_ready_pool_restores_target_when_every_live_gate_matches(self) -> None:
        specs = reviewed_specs(load_json(SPECS_PATH))
        current = datetime(2026, 8, 23, 8, 15, tzinfo=timezone.utc)
        report = inventory(0, observed_at="2026-08-23T08:14:00Z")
        plan = PLANNER.build_plan(
            report,
            specs,
            synthetic_private_ranking(specs),
            self.ledger,
            now=current,
        )
        self.assertEqual(plan["status"], "ready", plan)
        self.assertEqual(len(plan["selected_candidates"]), 10)
        self.assertEqual(plan["blockers"], [])

    def test_same_inputs_and_clock_produce_identical_plan(self) -> None:
        first = self.plan(4)
        second = self.plan(4)
        self.assertEqual(first, second)
        self.assertEqual(first["idempotency_key"], second["idempotency_key"])

    def test_missing_or_invalid_inventory_fails_closed(self) -> None:
        missing = inventory(4)
        missing.pop("verified_gmv_meta_competition_count")
        invalid = inventory(4)
        invalid["inventory_evidence_valid"] = False
        for report in (missing, invalid, None):
            with self.subTest(report=report):
                self.assertEqual(
                    self.plan(4, inventory_report=report)["status"], "blocked"
                )

    def test_missing_or_drifted_live_gmv_profile_fails_closed(self) -> None:
        missing = inventory(4)
        missing.pop("private_v2_gmv_profile")
        drifted = inventory(4)
        drifted["private_v2_gmv_profile"]["program_vkey"] = "0x" + "11" * 32
        for report in (missing, drifted):
            with self.subTest():
                plan = self.plan(4, inventory_report=report)
                self.assertEqual(plan["status"], "blocked")
                self.assertIn("not live", plan["blockers"][0])

    def test_stale_and_future_inventory_fail_closed(self) -> None:
        for observed_at in ("2026-08-23T07:54:59Z", "2026-08-23T08:11:01Z"):
            with self.subTest(observed_at=observed_at):
                plan = self.plan(4, inventory_report=inventory(4, observed_at=observed_at))
                self.assertEqual(plan["status"], "blocked")
                self.assertIn("stale", plan["blockers"][0])

    def test_release_and_economics_mismatches_fail_closed(self) -> None:
        mismatched_release = copy.deepcopy(self.specs)
        mismatched_release["release_hash"] = "0x" + "1" * 64
        mismatched_economics = copy.deepcopy(self.specs)
        mismatched_economics["economics"]["total_per_competition_base_units"] = 3_040_001
        for specs in (mismatched_release, mismatched_economics):
            with self.subTest(specs=specs["economics"]):
                plan = self.plan(4, candidate_specs=specs)
                self.assertEqual(plan["status"], "blocked")
                self.assertEqual(plan["severity"], "critical")

    def test_candidate_profile_must_equal_the_live_release(self) -> None:
        drifted = copy.deepcopy(self.specs)
        drifted["profile_release"]["program_vkey"] = "0x" + "11" * 32
        plan = self.plan(4, candidate_specs=drifted)
        self.assertEqual(plan["status"], "blocked")
        self.assertIn("differs from the live release", plan["blockers"][0])

    def test_missing_or_drifted_gmv_exclusions_fail_closed(self) -> None:
        missing = copy.deepcopy(self.specs)
        missing.pop("eligibility_policy")
        omitted_operator = copy.deepcopy(self.specs)
        omitted_operator["eligibility_policy"]["excluded_wallets"].pop()
        reordered_contracts = copy.deepcopy(self.specs)
        reordered_contracts["eligibility_policy"]["excluded_bounty_contracts"].reverse()
        for specs in (missing, omitted_operator, reordered_contracts):
            with self.subTest():
                self.assertEqual(self.plan(4, candidate_specs=specs)["status"], "blocked")

    def test_duplicate_candidates_and_ranking_mismatches_fail_closed(self) -> None:
        duplicate_specs = copy.deepcopy(self.specs)
        duplicate_specs["candidates"][1]["candidate_id"] = duplicate_specs["candidates"][0]["candidate_id"]
        duplicate_ranking = copy.deepcopy(self.ranking)
        duplicate_ranking["ranked_candidates"][1]["candidate_id"] = duplicate_ranking["ranked_candidates"][0]["candidate_id"]
        wrong_weights = copy.deepcopy(self.ranking)
        wrong_weights["ranking_weights"]["real_user_evidence"] = 49
        for key, value in (
            ("candidate_specs", duplicate_specs),
            ("private_ranking", duplicate_ranking),
            ("private_ranking", wrong_weights),
        ):
            with self.subTest(key=key):
                self.assertEqual(self.plan(4, **{key: value})["status"], "blocked")

    def test_candidate_exhaustion_blocks_without_partial_plan(self) -> None:
        reserved_ids = [item["candidate_id"] for item in self.specs["candidates"][:11]]
        ledger = {
            "schema_version": PLANNER.LEDGER_SCHEMA,
            "executions": [
                execution(
                    index,
                    status="activated",
                    occurred_at="2026-08-20T12:00:00Z",
                    candidate_id=candidate_id,
                )
                for index, candidate_id in enumerate(reserved_ids)
            ],
        }
        plan = self.plan(0, ledger=ledger)
        self.assertEqual(plan["status"], "blocked")
        self.assertEqual(plan["selected_candidates"], [])
        self.assertIn("fewer unused", plan["blockers"][0])

    def test_daily_cap_and_later_utc_day_recovery(self) -> None:
        ledger = {
            "schema_version": PLANNER.LEDGER_SCHEMA,
            "executions": [execution(index, status="activated") for index in range(9)],
        }
        blocked = self.plan(8, ledger=ledger)
        self.assertEqual(blocked["status"], "blocked")
        recovered = self.plan(
            8,
            ledger=ledger,
            now=datetime(2026, 8, 24, 0, 5, tzinfo=timezone.utc),
            inventory_report=inventory(8, observed_at="2026-08-24T00:04:00Z"),
        )
        self.assertEqual(recovered["status"], "ready")

    def test_lifetime_cap_blocks_full_restoration(self) -> None:
        ledger = {
            "schema_version": PLANNER.LEDGER_SCHEMA,
            "executions": [
                execution(
                    index,
                    status="activated",
                    occurred_at="2026-08-20T12:00:00Z",
                )
                for index in range(25)
            ],
        }
        plan = self.plan(8, ledger=ledger)
        self.assertEqual(plan["status"], "blocked")
        self.assertIn("lifetime", plan["blockers"][0])

    def test_ledger_duplicates_future_entries_and_bad_amounts_fail_closed(self) -> None:
        base = execution(1)
        duplicate_key = copy.deepcopy(base)
        duplicate_key["candidate_id"] = "different-candidate-v1"
        cases = [
            [base, duplicate_key],
            [base, execution(2, candidate_id=base["candidate_id"])],
            [execution(3, occurred_at="2026-08-23T08:10:01Z")],
            [execution(4, amount=3_040_000.0)],
            [execution(5, amount=3_040_001)],
        ]
        for entries in cases:
            with self.subTest(entries=entries):
                ledger = {"schema_version": PLANNER.LEDGER_SCHEMA, "executions": entries}
                self.assertEqual(self.plan(4, ledger=ledger)["status"], "blocked")

    def test_pending_execution_blocks_new_plans_until_reconciled(self) -> None:
        for status in ("planned", "broadcast"):
            with self.subTest(status=status):
                ledger = {
                    "schema_version": PLANNER.LEDGER_SCHEMA,
                    "executions": [execution(1, status=status)],
                }
                plan = self.plan(4, ledger=ledger)
                self.assertEqual(plan["status"], "blocked")
                self.assertIn("unreconciled signer", plan["blockers"][0])

    def test_invalid_feedback_and_analysis_evidence_fail_closed(self) -> None:
        bad_feedback = copy.deepcopy(self.specs)
        bad_feedback["candidates"][0]["feedback_sources"] = []
        bad_analysis = copy.deepcopy(self.specs)
        bad_analysis["candidates"][0]["analysis_sources"][0]["kind"] = "ai_opinion"
        for specs in (bad_feedback, bad_analysis):
            with self.subTest():
                self.assertEqual(
                    self.plan(4, candidate_specs=specs)["status"], "blocked"
                )


if __name__ == "__main__":
    unittest.main()
