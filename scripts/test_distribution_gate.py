#!/usr/bin/env python3
"""Deterministic tests for the event-driven distribution gate."""

from __future__ import annotations

import copy
import json
import unittest
from pathlib import Path

from scripts import distribution_gate as gate


ROOT = Path(__file__).resolve().parents[1]
POLICY = json.loads((ROOT / "ops/distribution/activation-policy-v1.json").read_text(encoding="utf-8"))
ORDERS = json.loads((ROOT / "ops/distribution/vendor-orders-v1.json").read_text(encoding="utf-8"))


def observation() -> dict:
    required = POLICY["exclusions"]["required_classes"]
    rails = []
    for vendor in POLICY["budget"]["vendors"]:
        rails.append(
            {
                "rail_id": vendor["rail_id"],
                "spend_mxn": "0.00",
                "unique_external_funded_poster_wallets": 0,
                "external_funded_canonical_settlements": 0,
                "verified_useful_settlements": 0,
                "verified_useful_evidence_refs": [],
                "canary": {
                    "dry_runs_total": 3,
                    "dry_runs_joined": 3,
                    "mainnet_runs_total": 1,
                    "mainnet_runs_settled": 1,
                    "excluded_from_external_metrics": True,
                    "evidence_refs": ["fixture:dry:1", "fixture:dry:2", "fixture:dry:3", "fixture:mainnet:1"],
                },
            }
        )
    return {
        "schema_version": gate.OBSERVATION_SCHEMA,
        "evidence_scope": "deterministic fixture",
        "exclusion_review": {
            "status": "complete",
            "required_classes_reviewed": list(required),
            "external_wallet_proxy_disclosed": True,
            "operator_funded_development_excluded": True,
        },
        "attribution": {
            "eligible_external_funded_bounties": 20,
            "attributed_external_funded_bounties": 20,
        },
        "safety": {"open_critical_incidents": 0},
        "rails": rails,
    }


class DistributionGateTests(unittest.TestCase):
    def test_frozen_policy_is_valid(self) -> None:
        gate.validate_policy(POLICY)

    def test_frozen_vendor_orders_match_policy(self) -> None:
        gate.validate_orders(POLICY, ORDERS)

    def test_vendor_order_cannot_raise_spend_ceiling(self) -> None:
        orders = copy.deepcopy(ORDERS)
        orders["orders"][0]["maximum_initial_spend_mxn"] = "999999.00"
        with self.assertRaisesRegex(gate.DistributionGateError, "maximum spend"):
            gate.validate_orders(POLICY, orders)

    def test_vendor_order_inventory_must_match_public_rate_card_budget(self) -> None:
        orders = copy.deepcopy(ORDERS)
        orders["orders"][1]["planned_inventory"][0]["public_price_usd_per_unit"] = "39.00"
        with self.assertRaisesRegex(gate.DistributionGateError, "inventory does not match"):
            gate.validate_orders(POLICY, orders)

    def test_vendor_order_must_fail_closed(self) -> None:
        orders = copy.deepcopy(ORDERS)
        orders["orders"][0]["purchase_state"] = "purchased"
        with self.assertRaisesRegex(gate.DistributionGateError, "fail closed"):
            gate.validate_orders(POLICY, orders)

    def test_vendor_order_requires_current_deployed_destination_and_future_alias(self) -> None:
        orders = copy.deepcopy(ORDERS)
        orders["orders"][0]["install_destination"] = "https://install.agentbounties.app/glama"
        with self.assertRaisesRegex(gate.DistributionGateError, "deployed rail-specific"):
            gate.validate_orders(POLICY, orders)

    def test_zero_spend_with_canaries_authorizes_initial_placement(self) -> None:
        result = gate.evaluate(POLICY, observation())
        self.assertTrue(result["exclusion_gate_passed"])
        self.assertEqual(
            [row["decision"] for row in result["decisions"]],
            ["activate_initial_placement"] * 3,
        )
        self.assertEqual(result["decisions"][0]["proposed_next_tranche_mxn"], "23779.28")
        self.assertTrue(result["decisions"][0]["owner_purchase_approval_required"])

    def test_incomplete_canary_blocks_activation(self) -> None:
        sample = observation()
        sample["rails"][0]["canary"]["mainnet_runs_settled"] = 0
        result = gate.evaluate(POLICY, sample)
        self.assertEqual(result["decisions"][0]["decision"], "blocked_canary")
        self.assertIn("mainnet_settlement_canary_incomplete", result["decisions"][0]["reason_codes"])

    def test_incomplete_exclusion_review_blocks_all_vendors(self) -> None:
        sample = observation()
        sample["exclusion_review"]["required_classes_reviewed"].pop()
        result = gate.evaluate(POLICY, sample)
        self.assertEqual({row["decision"] for row in result["decisions"]}, {"blocked_exclusion_review"})

    def test_passing_outcomes_scale_the_next_tranche(self) -> None:
        sample = observation()
        row = sample["rails"][0]
        row.update(
            spend_mxn="23779.28",
            unique_external_funded_poster_wallets=12,
            external_funded_canonical_settlements=6,
            verified_useful_settlements=6,
            verified_useful_evidence_refs=[f"proof:glama:{index}" for index in range(6)],
        )
        result = gate.evaluate(POLICY, sample)
        decision = result["decisions"][0]
        self.assertEqual(decision["decision"], "scale_next_tranche")
        self.assertEqual(decision["proposed_next_tranche_mxn"], "47558.56")
        self.assertEqual(decision["funded_poster_cac_mxn"], "1981.61")
        self.assertEqual(decision["settled_bounty_cac_mxn"], "3963.21")

    def test_minimum_sample_without_cac_efficiency_does_not_renew(self) -> None:
        sample = observation()
        row = sample["rails"][1]
        row.update(
            spend_mxn="15000.00",
            unique_external_funded_poster_wallets=6,
            external_funded_canonical_settlements=3,
            verified_useful_settlements=3,
            verified_useful_evidence_refs=[f"proof:mcp-so:{index}" for index in range(3)],
        )
        result = gate.evaluate(POLICY, sample)
        decision = result["decisions"][1]
        self.assertEqual(decision["decision"], "do_not_renew")
        self.assertIn("funded_poster_cac_above_cap", decision["reason_codes"])
        self.assertIn("settled_bounty_cac_above_cap", decision["reason_codes"])

    def test_low_attribution_coverage_prevents_scaling(self) -> None:
        sample = observation()
        sample["attribution"] = {
            "eligible_external_funded_bounties": 20,
            "attributed_external_funded_bounties": 18,
        }
        sample["rails"][2].update(
            spend_mxn="10191.12",
            unique_external_funded_poster_wallets=6,
            external_funded_canonical_settlements=3,
            verified_useful_settlements=3,
            verified_useful_evidence_refs=[f"proof:mcpservers:{index}" for index in range(3)],
        )
        result = gate.evaluate(POLICY, sample)
        self.assertEqual(result["decisions"][2]["decision"], "hold_attribution_coverage")

    def test_low_attribution_coverage_prevents_initial_purchase(self) -> None:
        sample = observation()
        sample["attribution"] = {
            "eligible_external_funded_bounties": 20,
            "attributed_external_funded_bounties": 18,
        }
        result = gate.evaluate(POLICY, sample)
        self.assertEqual(
            {row["decision"] for row in result["decisions"]},
            {"hold_attribution_coverage"},
        )

    def test_unverified_settlement_prevents_scaling(self) -> None:
        sample = observation()
        sample["rails"][2].update(
            spend_mxn="10191.12",
            unique_external_funded_poster_wallets=6,
            external_funded_canonical_settlements=4,
            verified_useful_settlements=3,
            verified_useful_evidence_refs=[f"proof:mcpservers:{index}" for index in range(3)],
        )
        result = gate.evaluate(POLICY, sample)
        self.assertEqual(result["decisions"][2]["decision"], "do_not_renew")
        self.assertIn("settlement_without_complete_usefulness_evidence", result["decisions"][2]["reason_codes"])

    def test_useful_settlement_count_requires_inspectable_evidence_refs(self) -> None:
        sample = observation()
        sample["rails"][1]["external_funded_canonical_settlements"] = 1
        sample["rails"][1]["verified_useful_settlements"] = 1
        with self.assertRaisesRegex(gate.DistributionGateError, "unique evidence reference"):
            gate.evaluate(POLICY, sample)

    def test_critical_incident_halts_every_vendor(self) -> None:
        sample = observation()
        sample["safety"]["open_critical_incidents"] = 1
        result = gate.evaluate(POLICY, sample)
        self.assertEqual({row["decision"] for row in result["decisions"]}, {"halt_critical_incident"})

    def test_policy_rejects_tampered_budget_conversion(self) -> None:
        policy = copy.deepcopy(POLICY)
        policy["budget"]["vendors"][0]["initial_spend_mxn"] = "1.00"
        with self.assertRaisesRegex(gate.DistributionGateError, "frozen FX"):
            gate.validate_policy(policy)


if __name__ == "__main__":
    unittest.main()
