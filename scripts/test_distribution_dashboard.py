#!/usr/bin/env python3
"""Tests for the live distribution dashboard join."""

from __future__ import annotations

import copy
import json
import unittest
from pathlib import Path

from scripts import distribution_dashboard as dashboard
from scripts import distribution_gate as gate


ROOT = Path(__file__).resolve().parents[1]
POLICY = gate.load_json(ROOT / "ops/distribution/activation-policy-v1.json")
CONTROL = gate.load_json(ROOT / "ops/distribution/activation-observation-template.json")


def ready_control() -> dict:
    control = copy.deepcopy(CONTROL)
    control["exclusion_review"] = {
        "status": "complete",
        "required_classes_reviewed": list(POLICY["exclusions"]["required_classes"]),
        "external_wallet_proxy_disclosed": True,
        "operator_funded_development_excluded": True,
    }
    for row in control["rails"]:
        row["canary"] = {
            "dry_runs_total": 3,
            "dry_runs_joined": 3,
            "mainnet_runs_total": 1,
            "mainnet_runs_settled": 1,
            "excluded_from_external_metrics": True,
            "evidence_refs": ["dry:1", "dry:2", "dry:3", "mainnet:1"],
        }
    return control


def report() -> dict:
    rails = []
    for vendor in POLICY["budget"]["vendors"]:
        rails.append(
            {
                "rail": vendor["rail_id"],
                "acquisitions": 100,
                "assisted_acquisitions": 2,
                "mcp_requests": 120,
                "failed_mcp_requests": 2,
                "prepared_handoffs": 20,
                "attributed_terms": 15,
                "externally_funded_bounties": 12,
                "unique_external_funded_posters": 12,
                "externally_funded_claimed_bounties": 10,
                "externally_funded_submitted_bounties": 8,
                "externally_funded_settled_bounties": 6,
                "verified_settlements_with_evidence": 6,
                "wallet_reviewed_handoffs": 14,
                "verified_useful_settlements": None,
                "handoff_failure_count": 3,
                "mcp_failure_rate_basis_points": 166,
                "external_funding_base_units": "25000000",
                "settled_gmv_base_units": "12000000",
            }
        )
    return {
        "schema_version": dashboard.REPORT_SCHEMA,
        "generated_at": "2026-09-02T00:00:00Z",
        "protocol_scope": "agent-bounties/autonomous-v1",
        "excluded_wallet_classes": list(POLICY["exclusions"]["required_classes"]),
        "unavailable_metrics": ["verified_useful_settlements"],
        "rails": rails,
        "total_external_funded_bounties": 36,
        "unique_external_funded_posters": 30,
        "attributed_external_funded_bounties": 36,
        "attribution_coverage_basis_points": 10_000,
        "attribution_coverage_ready": True,
    }


class DistributionDashboardTests(unittest.TestCase):
    def test_live_report_drives_funnel_and_control_drives_usefulness(self) -> None:
        control = ready_control()
        control["rails"][0].update(
            spend_mxn="23779.28",
            verified_useful_settlements=6,
            verified_useful_evidence_refs=[f"origin-proof:{index}" for index in range(6)],
        )
        result = dashboard.build_dashboard(POLICY, control, report())
        glama = result["rails"][0]
        self.assertEqual(glama["funnel"]["claimed"], 10)
        self.assertEqual(glama["funnel"]["wallet_review"], 14)
        self.assertEqual(glama["funnel"]["verified_useful"], 6)
        self.assertEqual(glama["failures"]["handoff"], 3)
        self.assertEqual(glama["settled_gmv_usdc"], "12")
        self.assertEqual(glama["activation"]["decision"], "scale_next_tranche")
        self.assertEqual(result["metrics"]["unique_external_funded_poster_wallets"], 30)
        self.assertIsNone(result["metrics"]["ltv"])

    def test_usefulness_cannot_exceed_hash_matched_settlement_evidence(self) -> None:
        control = ready_control()
        control["rails"][1]["verified_useful_settlements"] = 7
        control["rails"][1]["verified_useful_evidence_refs"] = [f"proof:{index}" for index in range(7)]
        with self.assertRaisesRegex(gate.DistributionGateError, "exceed evidence-backed"):
            dashboard.build_dashboard(POLICY, control, report())

    def test_rejects_noncanonical_protocol_scope(self) -> None:
        invalid = report()
        invalid["protocol_scope"] = "all-protocols-unverified"
        with self.assertRaisesRegex(gate.DistributionGateError, "scope"):
            dashboard.build_dashboard(POLICY, ready_control(), invalid)

    def test_rejects_missing_required_wallet_exclusion_class(self) -> None:
        invalid = report()
        invalid["excluded_wallet_classes"].remove("related_party")
        with self.assertRaisesRegex(gate.DistributionGateError, "missing required excluded"):
            dashboard.build_dashboard(POLICY, ready_control(), invalid)

    def test_template_remains_blocked_without_claiming_activation(self) -> None:
        result = dashboard.build_dashboard(POLICY, CONTROL, report())
        self.assertEqual(
            {row["activation"]["decision"] for row in result["rails"]},
            {"blocked_exclusion_review"},
        )


if __name__ == "__main__":
    unittest.main()
