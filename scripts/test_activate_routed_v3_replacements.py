#!/usr/bin/env python3

from __future__ import annotations

import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock


SCRIPTS = Path(__file__).resolve().parent
WORKFLOWS = SCRIPTS.parent / ".github" / "workflows"
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))

import activate_routed_v3_dynamic as DYNAMIC
import activate_routed_v3_replacements as MODULE
import check_routed_v3_activation_readiness as READINESS


NOW = 1_800_000_000


class PolicyCast:
    def __init__(self, **overrides: object) -> None:
        self.state = MODULE.active_wallet.expected_state()
        self.state.update(overrides)

    def chain_id(self) -> int:
        return MODULE.CHAIN_ID

    def code(self, target: str) -> str:
        return "0x6000"

    def rpc(self, *args: str, timeout: int = 300) -> str:
        if args == ("block", "latest", "--field", "timestamp"):
            return str(NOW)
        raise AssertionError(f"unexpected rpc {args}")

    def call(self, target: str, signature: str, *args: str) -> str:
        if signature == "isPolicyActive(bytes32)(bool)":
            return "true"
        if signature.startswith("policies(bytes32)"):
            return "\n".join(
                [
                    "0x" + "14" * 20,
                    "0x" + "15" * 32,
                    "1",
                    "1",
                    "1",
                    "false",
                ]
            )
        if signature.startswith("policy()"):
            return "\n".join(
                str(self.state[key])
                for key in (
                    "delegate",
                    "valid_after",
                    "valid_until",
                    "period_seconds",
                    "max_per_action",
                    "max_per_period",
                    "max_lifetime_spend",
                    "max_bounty_target",
                    "allowed_actions",
                    "allowed_verification_modes",
                    "deterministic_verifier",
                    "signed_quorum",
                    "ai_quorum",
                )
            )
        values = {
            "owner()(address)": self.state["owner"],
            "policyHash()(bytes32)": self.state["policy_hash"],
            "policyVersion()(uint64)": self.state["policy_version"],
            "periodSpent()(uint256)": 0,
            "lifetimeSpent()(uint256)": 25_050_000,
            "balanceOf(address)(uint256)": 63_950_000,
            "periodBucket()(uint256)": NOW // 86_400,
        }
        if signature in values:
            return str(values[signature])
        raise AssertionError(f"unexpected call {target} {signature} {args}")


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
        self.assertEqual(MODULE.TOTAL, 10_050_000)
        self.assertEqual(
            sorted(MODULE.ISSUES),
            [333, 335, 336, 590, 647, 648, 649, 650, 651],
        )
        self.assertNotIn(334, MODULE.ISSUES)
        self.assertEqual(set(MODULE.ISSUES), set(MODULE.durable.LANES) - {334})
        self.assertEqual(DYNAMIC.ROUTER, "0x380c1af742593dd88b6f20387e9ee693a0536731")
        self.assertEqual(DYNAMIC.ACTIVATION_DELAY, 604_800)
        self.assertEqual(DYNAMIC.BOOTSTRAP_BLOCK, 49_069_936)

    def test_exact_active_wallet_policy_is_ready(self) -> None:
        deployment = self.deployment_fixture()
        deployment.update(
            {
                "router_address": MODULE.active_wallet.DETERMINISTIC_VERIFIER,
                "adapter_address": deployment["adapter"]["address"],
                "adapter_runtime_code_hash": deployment["adapter"]["runtime_code_hash"],
                "policy_hash": deployment["policy_hash"],
            }
        )
        state = MODULE.policy_state(PolicyCast(), deployment)
        self.assertEqual(state["policy_hash"], MODULE.active_wallet.POLICY_HASH)
        self.assertEqual(state["policy_version"], 5)
        self.assertEqual(state["affordable_creations"], 4)

    def test_active_wallet_policy_drift_fails_closed(self) -> None:
        deployment = self.deployment_fixture()
        deployment.update(
            {
                "router_address": MODULE.active_wallet.DETERMINISTIC_VERIFIER,
                "adapter_address": deployment["adapter"]["address"],
                "adapter_runtime_code_hash": deployment["adapter"]["runtime_code_hash"],
                "policy_hash": deployment["policy_hash"],
            }
        )
        cases = {
            "owner": "0x" + "91" * 20,
            "delegate": "0x" + "92" * 20,
            "allowed_verification_modes": 1,
            "deterministic_verifier": "0x" + "93" * 20,
            "signed_quorum": "0x" + "94" * 32,
            "policy_hash": "0x" + "95" * 32,
            "policy_version": 6,
            "max_per_period": 11_000_000,
        }
        for field, observed in cases.items():
            with self.subTest(field=field), self.assertRaisesRegex(MODULE.ActivationError, field):
                MODULE.policy_state(PolicyCast(**{field: observed}), deployment)

    def test_router_address_is_quoted_in_activation_workflow_yaml(self) -> None:
        expected = 'ROUTER: "0x380c1af742593dd88b6f20387e9ee693a0536731"'
        for name in (
            "activate-routed-v3-replacements.yml",
            "routed-v3-activation-check.yml",
        ):
            workflow = (WORKFLOWS / name).read_text(encoding="utf-8")
            self.assertIn(expected, workflow, name)
            self.assertIn("--from-block 49069936", workflow, name)
            self.assertIn("--to-block 49069936", workflow, name)

    def test_resume_skips_planning_and_send_for_an_existing_canonical_contract(self) -> None:
        cast = mock.Mock()
        cast.call.return_value = "true"
        create = mock.Mock(return_value="0x" + "41" * 32)

        transaction, remaining = MODULE.create_if_missing(
            cast, "0x" + "31" * 20, 3, create
        )

        self.assertEqual(transaction, "already-canonical")
        self.assertEqual(remaining, 3)
        create.assert_not_called()

    def test_reconciliation_accepts_every_active_state_and_fails_closed(self) -> None:
        contract = "0x" + "31" * 20
        bounty_id = "0x" + "32" * 32
        events = [
            {"kind": "canonical_bounty_created"},
            {"kind": "funding_added"},
            {"kind": "bounty_became_claimable"},
        ]
        for status in ("claimable", "claimed", "submitted", "verifying"):
            with self.subTest(status=status), mock.patch.object(
                MODULE,
                "http_json",
                side_effect=[
                    events,
                    [
                        {
                            "bounty_contract": contract,
                            "status": status,
                            "terms_valid": True,
                            "verification_ready": True,
                        }
                    ],
                ],
            ):
                result = MODULE.reconcile("https://api.example", contract, bounty_id, 1)
                self.assertEqual(result["feed_item"]["status"], status)

        failures = [
            {"status": "claimable", "terms_valid": False, "verification_ready": True},
            {"status": "claimable", "terms_valid": True, "verification_ready": False},
            {"status": "settled", "terms_valid": True, "verification_ready": True},
        ]
        for item in failures:
            item["bounty_contract"] = contract
            with self.subTest(item=item), mock.patch.object(
                MODULE,
                "http_json",
                side_effect=[events, [item]],
            ), mock.patch.object(MODULE.time, "monotonic", side_effect=[0, 0, 2]), mock.patch.object(
                MODULE.time, "sleep"
            ):
                with self.assertRaisesRegex(MODULE.ActivationError, "did not reconcile"):
                    MODULE.reconcile("https://api.example", contract, bounty_id, 1)

        with mock.patch.object(
            MODULE,
            "http_json",
            side_effect=[events, [
                {
                    "bounty_contract": contract,
                    "status": "claimable",
                    "terms_valid": True,
                    "verification_ready": True,
                },
                {
                    "bounty_contract": contract,
                    "status": "claimed",
                    "terms_valid": True,
                    "verification_ready": True,
                },
            ]],
        ):
            with self.assertRaisesRegex(MODULE.ActivationError, "ambiguous"):
                MODULE.reconcile("https://api.example", contract, bounty_id, 1)


if __name__ == "__main__":
    unittest.main()
