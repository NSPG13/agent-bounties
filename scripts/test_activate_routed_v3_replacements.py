#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("activate_routed_v3_replacements.py")
SPEC = importlib.util.spec_from_file_location("activate_routed_v3_replacements", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)

READY_SCRIPT = Path(__file__).with_name("check_routed_v3_activation_readiness.py")
READY_SPEC = importlib.util.spec_from_file_location("check_routed_v3_activation_readiness", READY_SCRIPT)
assert READY_SPEC and READY_SPEC.loader
READINESS = importlib.util.module_from_spec(READY_SPEC)
READY_SPEC.loader.exec_module(READINESS)


class ActivateRoutedV3Tests(unittest.TestCase):
    def deployment_fixture(self) -> dict:
        return {
            "schema": "agent-bounties/durable-verifier-router-deployment-v1",
            "network": "base-mainnet",
            "chain_id": 8453,
            "router": {
                "address": "0x" + "11" * 20,
                "runtime_code_hash": "0x" + "12" * 32,
            },
            "policy_hash": "0x" + "13" * 32,
            "adapter": {
                "address": "0x" + "14" * 20,
                "runtime_code_hash": "0x" + "15" * 32,
                "acceptance_criteria_hash": "0x" + "16" * 32,
            },
        }

    def test_load_deployment_requires_exact_evidence_shape(self) -> None:
        original = MODULE.DEPLOYMENT_PATH
        try:
            with tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "deployment.json"
                path.write_text(json.dumps(self.deployment_fixture()), encoding="utf-8")
                MODULE.DEPLOYMENT_PATH = path
                value = MODULE.load_deployment()
            self.assertEqual(value["router_address"], "0x" + "11" * 20)
            self.assertEqual(value["policy_hash"], "0x" + "13" * 32)
            self.assertEqual(value["adapter_address"], "0x" + "14" * 20)
        finally:
            MODULE.DEPLOYMENT_PATH = original

    def test_issue_body_advertises_routed_profit_and_payment_boundary(self) -> None:
        deployment = self.deployment_fixture()
        deployment.update({
            "router_address": deployment["router"]["address"],
            "policy_hash": deployment["policy_hash"],
            "adapter_address": deployment["adapter"]["address"],
        })
        body = MODULE.issue_body(
            333,
            "CLI",
            MODULE.ISSUES[333]["old"],
            {
                "contract": "0x" + "21" * 20,
                "transaction_hash": "0x" + "22" * 32,
            },
            deployment,
        )
        self.assertIn("2.00 USDC", body)
        self.assertIn("0.01 USDC", body)
        self.assertIn("1 USDC gross profit", body)
        self.assertIn(deployment["router_address"], body)
        self.assertIn(deployment["policy_hash"], body)
        self.assertIn("Only canonical `BountySettled` proves earnings", body)

    def test_redaction_hides_private_key_and_rpc(self) -> None:
        value = MODULE.redact_command(
            ["cast", "send", "--private-key", "secret", "--rpc-url", "credentialed"]
        )
        self.assertNotIn("secret", value)
        self.assertNotIn("credentialed", value)
        self.assertEqual(value.count("***"), 2)

    def test_readiness_probe_fails_closed_without_raising(self) -> None:
        original = MODULE.DEPLOYMENT_PATH
        try:
            MODULE.DEPLOYMENT_PATH = Path("/definitely/missing/deployment.json")
            report = READINESS.inspect("https://mainnet.base.org", "cast")
        finally:
            MODULE.DEPLOYMENT_PATH = original
        self.assertFalse(report["ready"])
        self.assertFalse(report["financial_action_taken"])
        self.assertIn("manifest is missing", report["reason"])

    def test_economics_and_scope_are_exact(self) -> None:
        self.assertEqual(MODULE.TARGET, 2_010_000)
        self.assertEqual(MODULE.TOTAL, 8_040_000)
        self.assertEqual(sorted(MODULE.ISSUES), [333, 334, 335, 336])
        self.assertEqual(MODULE.UINT64_MAX, (1 << 64) - 1)


if __name__ == "__main__":
    unittest.main()
