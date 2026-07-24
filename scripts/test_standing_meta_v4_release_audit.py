#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).with_name("standing_meta_v4_release_audit.py")
SPEC = importlib.util.spec_from_file_location("standing_meta_v4_release_audit", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def environment_payload(*, reviewer_id: int | None, author_id: int = 10) -> tuple[dict, dict]:
    rules: list[dict] = [{"type": "branch_policy"}]
    if reviewer_id is not None:
        rules.append(
            {
                "type": "required_reviewers",
                "prevent_self_review": True,
                "reviewers": [
                    {
                        "type": "User",
                        "reviewer": {"id": reviewer_id, "login": "reviewer"},
                    }
                ],
            }
        )
    return (
        {
            "can_admins_bypass": False,
            "protection_rules": rules,
            "deployment_branch_policy": {
                "protected_branches": False,
                "custom_branch_policies": True,
            },
        },
        {"branch_policies": [{"name": "main", "type": "branch"}]},
    )


def manifest() -> dict:
    network = {
        "subscription_id": None,
        "consumers_authorized": False,
        "native_subscription_reserve_funded": False,
        "minimum_native_subscription_reserve_wei": None,
        "gas_sponsorship_available": False,
        "minimum_gas_sponsorship_reserve_wei": None,
        "components": None,
    }
    return {
        "schema": "agent-bounties/standing-meta-v4-deployment-readiness-v1",
        "protocol_version": "standing-meta-v4",
        "status": "not_deployed",
        "latency_policy_status": MODULE.EXPECTED_LATENCY_POLICY_STATUS,
        "latency_policy_decision": MODULE.EXPECTED_LATENCY_POLICY_DECISION,
        "configuration": dict(MODULE.LATENCY_POLICY),
        "monitoring_policy": dict(MODULE.MONITORING_POLICY),
        "required_components": list(MODULE.EXPECTED_CANONICAL_COMPONENTS),
        "r4_evidence": {
            **{name: False for name in MODULE.REQUIRED_R4_GATES},
            "independent_review_evidence": {
                "source_commit": None,
                "reviewer_identity": None,
                "review_url": None,
                "report_sha256": None,
                "findings_resolved_or_accepted": False,
            },
        },
        "networks": {"base-sepolia": dict(network), "base-mainnet": dict(network)},
    }


class StandingMetaV4ReleaseAuditTests(unittest.TestCase):
    def test_environment_requires_independent_reviewer(self) -> None:
        environment, policies = environment_payload(reviewer_id=None)
        result = MODULE.evaluate_environment("standing-meta-v4-mainnet", environment, policies, 10)
        self.assertFalse(result["complete"])
        self.assertIn("required reviewers are absent", result["blockers"])

        environment, policies = environment_payload(reviewer_id=10)
        result = MODULE.evaluate_environment("standing-meta-v4-mainnet", environment, policies, 10)
        self.assertFalse(result["complete"])
        self.assertIn("no required user reviewer is independent of the maintainer author", result["blockers"])

        environment, policies = environment_payload(reviewer_id=11)
        result = MODULE.evaluate_environment("standing-meta-v4-mainnet", environment, policies, 10)
        self.assertTrue(result["complete"])

    def test_environment_requires_main_only_and_no_admin_bypass(self) -> None:
        environment, policies = environment_payload(reviewer_id=11)
        environment["can_admins_bypass"] = True
        policies["branch_policies"].append({"name": "release/*", "type": "branch"})
        result = MODULE.evaluate_environment("standing-meta-v4-mainnet", environment, policies, 10)
        self.assertFalse(result["complete"])
        self.assertIn("environment administrator bypass is enabled", result["blockers"])
        self.assertIn("deployment branch policy is not exactly main", result["blockers"])

    def test_manifest_audit_is_fail_closed_without_live_evidence(self) -> None:
        result = MODULE.audit_manifest(manifest(), None)
        self.assertFalse(result["ready_for_mainnet"])
        self.assertFalse(result["manifest_status_valid"])
        self.assertFalse(result["environment_evidence_complete"])
        self.assertEqual(result["latency_policy_mismatches"], {})
        self.assertIn("R4 gate incomplete: independent_review_complete", result["blockers"])
        self.assertIn("base-mainnet incomplete: subscription_id", result["blockers"])

    def test_manifest_requires_commit_bound_independent_review_evidence(self) -> None:
        value = manifest()
        value["r4_evidence"]["independent_review_complete"] = True
        result = MODULE.audit_manifest(value, None)
        self.assertFalse(result["independent_review_evidence"]["complete"])
        self.assertIn(
            "independent review evidence incomplete: source_commit",
            result["blockers"],
        )

        value["r4_evidence"]["independent_review_evidence"] = {
            "source_commit": "12" * 20,
            "reviewer_identity": "External Security Reviewer",
            "review_url": "https://example.test/review/v4",
            "report_sha256": "34" * 32,
            "findings_resolved_or_accepted": True,
        }
        result = MODULE.audit_manifest(value, None)
        self.assertTrue(result["independent_review_evidence"]["complete"])

    def test_manifest_audit_requires_exact_named_component_addresses(self) -> None:
        value = manifest()
        value["networks"]["base-mainnet"]["components"] = {
            name: "0x" + f"{index + 1:040x}"
            for index, name in enumerate(MODULE.EXPECTED_CANONICAL_COMPONENTS)
        }
        value["networks"]["base-mainnet"]["components"].pop("standing_meta_v4_bundle")
        value["networks"]["base-mainnet"]["components"]["lookalike_bundle"] = "0x" + "22" * 20
        result = MODULE.audit_manifest(value, None)
        checks = result["network_readiness"]["base-mainnet"]["checks"]
        self.assertFalse(checks["exact_component_set"])
        self.assertFalse(checks["all_component_addresses_valid"])

        value["networks"]["base-mainnet"]["components"].pop("lookalike_bundle")
        value["networks"]["base-mainnet"]["components"]["standing_meta_v4_bundle"] = "not-an-address"
        result = MODULE.audit_manifest(value, None)
        checks = result["network_readiness"]["base-mainnet"]["checks"]
        self.assertTrue(checks["exact_component_set"])
        self.assertFalse(checks["all_component_addresses_valid"])

    def test_manifest_audit_rejects_latency_drift(self) -> None:
        value = manifest()
        value["configuration"]["solver_assignment_seconds"] = 600
        result = MODULE.audit_manifest(value, None)
        self.assertEqual(
            result["latency_policy_mismatches"]["solver_assignment_seconds"],
            {"expected": 120, "actual": 600},
        )

    def test_manifest_audit_requires_review_frozen_latency_decision(self) -> None:
        value = manifest()
        value["latency_policy_status"] = "draft"
        result = MODULE.audit_manifest(value, None)
        self.assertFalse(result["ready_for_mainnet"])
        self.assertFalse(result["latency_policy_status_valid"])
        self.assertIn("latency policy is not review-frozen", result["blockers"])

    def test_manifest_audit_rejects_monitoring_policy_drift(self) -> None:
        value = manifest()
        value["monitoring_policy"]["minimum_eligible_verifier_wallets"] = 7
        result = MODULE.audit_manifest(value, None)
        self.assertFalse(result["monitoring_policy_valid"])
        self.assertIn("monitoring policy drift", result["blockers"])


if __name__ == "__main__":
    unittest.main()
