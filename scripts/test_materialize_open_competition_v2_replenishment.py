#!/usr/bin/env python3
"""Tests for deterministic V2 replenishment request materialization."""

from __future__ import annotations

import copy
import json
import sys
import unittest
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / "scripts"
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))

import materialize_open_competition_v2_replenishment as MATERIALIZER
import plan_open_competition_v2_replenishment as PLANNER
from test_plan_open_competition_v2_replenishment import (
    LEDGER_PATH,
    NOW,
    SPECS_PATH,
    inventory,
    load_json,
    reviewed_specs,
    synthetic_private_ranking,
)


class ReplenishmentMaterializerTests(unittest.TestCase):
    def setUp(self) -> None:
        specs = reviewed_specs(load_json(SPECS_PATH))
        self.plan = PLANNER.build_plan(
            inventory(4),
            specs,
            synthetic_private_ranking(specs),
            load_json(LEDGER_PATH),
            now=NOW,
        )
        self.assertEqual(self.plan["status"], "ready")

    def test_materializes_exact_bounded_request(self) -> None:
        request = MATERIALIZER.materialize(self.plan)
        self.assertEqual(len(request["creations"]), 6)
        self.assertTrue(request["signer_revalidation_required"])
        self.assertEqual(request["policy"]["required_spend_base_units"], 18_240_000)
        for creation in request["creations"]:
            self.assertEqual(creation["economics"]["solver_reward_base_units"], 3_000_000)
            self.assertEqual(creation["economics"]["keeper_reward_base_units"], 40_000)
            self.assertEqual(creation["profile_id"], MATERIALIZER.PROFILE_ID)
            self.assertEqual(creation["meta_bounty"]["objective"], "highest_external_canonical_gmv")
            self.assertEqual(creation["settlement"]["winner_mode"], "best_score")
            self.assertEqual(creation["settlement"]["score_direction"], "higher_is_better")
            self.assertEqual(creation["settlement"]["score_threshold_base_units"], 1)
            self.assertEqual(creation["meta_bounty"]["snapshot"]["status"], "ready")
            self.assertEqual(
                creation["meta_bounty"]["excluded_wallets"],
                MATERIALIZER.REQUIRED_EXCLUDED_WALLETS,
            )
            self.assertEqual(
                creation["meta_bounty"]["excluded_bounty_contracts"],
                MATERIALIZER.REQUIRED_EXCLUDED_BOUNTY_CONTRACTS,
            )
            self.assertNotIn("artifact_template", creation)
            self.assertNotIn("scores", json.dumps(creation))
            self.assertNotIn("launch_role", json.dumps(creation))

    def test_materialization_is_content_addressed_and_deterministic(self) -> None:
        first = MATERIALIZER.materialize(self.plan)
        second = MATERIALIZER.materialize(self.plan)
        self.assertEqual(first, second)
        self.assertRegex(first["request_hash"], r"^0x[0-9a-f]{64}$")

    def test_non_ready_plan_is_rejected(self) -> None:
        plan = copy.deepcopy(self.plan)
        plan["status"] = "blocked"
        with self.assertRaises(MATERIALIZER.MaterializeError):
            MATERIALIZER.materialize(plan)

    def test_amount_drift_and_duplicate_candidates_are_rejected(self) -> None:
        amount_drift = copy.deepcopy(self.plan)
        amount_drift["policy"]["per_candidate_base_units"] += 1
        duplicate = copy.deepcopy(self.plan)
        duplicate["selected_candidates"][1] = copy.deepcopy(
            duplicate["selected_candidates"][0]
        )
        for plan in (amount_drift, duplicate):
            with self.subTest():
                with self.assertRaises(MATERIALIZER.MaterializeError):
                    MATERIALIZER.materialize(plan)

    def test_noop_plan_is_rejected(self) -> None:
        specs = reviewed_specs(load_json(SPECS_PATH))
        plan = PLANNER.build_plan(
            inventory(10),
            specs,
            synthetic_private_ranking(specs),
            load_json(LEDGER_PATH),
            now=datetime(2026, 8, 22, 4, 35, tzinfo=timezone.utc),
        )
        with self.assertRaises(MATERIALIZER.MaterializeError):
            MATERIALIZER.materialize(plan)

    def test_materializer_rejects_exclusion_drift_after_planning(self) -> None:
        plan = copy.deepcopy(self.plan)
        plan["selected_candidates"][0]["eligibility_policy"]["excluded_wallets"].pop()
        with self.assertRaises(MATERIALIZER.MaterializeError):
            MATERIALIZER.materialize(plan)


if __name__ == "__main__":
    unittest.main()
