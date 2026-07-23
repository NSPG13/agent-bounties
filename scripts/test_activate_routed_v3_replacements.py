#!/usr/bin/env python3

from __future__ import annotations

import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock


SCRIPTS = Path(__file__).resolve().parent
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))

import activate_routed_v3_dynamic as DYNAMIC
import activate_routed_v3_replacements as MODULE
import check_routed_v3_activation_readiness as READINESS


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

    def test_legacy_manifest_parser_still_requires_exact_evidence_shape(self) -> None:
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

    def test_bootstrap_event_parser_derives_policy_adapter_and_code_hash(self) -> None:
        policy = "0x" + "21" * 32
        adapter = "0x" + "22" * 20
        runtime = "0x" + "23" * 32
        transaction = "0x" + "24" * 32
        raw = json.dumps([
            {
                "topics": [
                    "0x" + "20" * 32,
                    policy,
                    "0x" + "00" * 12 + adapter[2:],
                ],
                "data": runtime,
                "transactionHash": transaction,
                "blockNumber": "0x1234",
            }
        ])
        value = DYNAMIC.parse_bootstrap_logs(raw)
        self.assertEqual(value["policy_hash"], policy)
        self.assertEqual(value["adapter"], adapter)
        self.assertEqual(value["adapter_runtime_code_hash"], runtime)
        self.assertEqual(value["bootstrap_transaction"], transaction)
        self.assertEqual(value["bootstrap_block"], 0x1234)

    def test_bootstrap_event_parser_rejects_ambiguous_history(self) -> None:
        with self.assertRaisesRegex(MODULE.ActivationError, "exactly one"):
            DYNAMIC.parse_bootstrap_logs("[]")

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
                "contract": "0x" + "31" * 20,
                "transaction_hash": "0x" + "32" * 32,
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
        with mock.patch.object(DYNAMIC, "discover_deployment", side_effect=RuntimeError("attestation failed")):
            report = READINESS.inspect("https://mainnet.base.org", "cast")
        self.assertFalse(report["ready"])
        self.assertFalse(report["financial_action_taken"])
        self.assertIn("attestation failed", report["reason"])

    def test_economics_and_scope_are_exact(self) -> None:
        self.assertEqual(MODULE.TARGET, 2_010_000)
        self.assertEqual(MODULE.TOTAL, 8_040_000)
        self.assertEqual(sorted(MODULE.ISSUES), [333, 334, 335, 336])
        self.assertEqual(MODULE.UINT64_MAX, (1 << 64) - 1)
        self.assertEqual(DYNAMIC.ROUTER, "0x380c1af742593dd88b6f20387e9ee693a0536731")
        self.assertEqual(DYNAMIC.ACTIVATION_DELAY, 604_800)


if __name__ == "__main__":
    unittest.main()
