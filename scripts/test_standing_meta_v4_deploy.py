#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest import mock


SCRIPT = Path(__file__).with_name("standing_meta_v4_deploy.py")
SPEC = importlib.util.spec_from_file_location("standing_meta_v4_deploy", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class FakeSubscriptionFoundry:
    def keccak_text(self, value: str) -> str:
        self.value = value
        return "0x" + "11" * 32


class FakeCoordinatorFoundry:
    def __init__(
        self,
        *,
        minimum_confirmations: int = 0,
        maximum_callback_gas: int = 2_500_000,
        reentrancy_lock: bool = False,
        proving_key_marker: str = "0x" + "00" * 19 + "01",
    ) -> None:
        self.minimum_confirmations = minimum_confirmations
        self.maximum_callback_gas = maximum_callback_gas
        self.reentrancy_lock = reentrancy_lock
        self.proving_key_marker = proving_key_marker

    def call(self, address: str, signature: str, *args: str) -> str:
        if signature == "s_config()(uint16,uint32,bool,uint32,uint32,uint32,uint32,uint8,uint8)":
            return "\n".join(
                (
                    str(self.minimum_confirmations),
                    str(self.maximum_callback_gas),
                    str(self.reentrancy_lock).lower(),
                    "172800",
                    "42500",
                    "0",
                    "0",
                    "60",
                    "50",
                )
            )
        if signature == "s_provingKeys(bytes32)(address)":
            return self.proving_key_marker
        raise AssertionError((address, signature, args))


class FakeOwnerWithdrawalFoundry:
    def __init__(self, *, owner: str | None = None, wallet_balance: int = 37_000_000) -> None:
        self.owner = owner or MODULE.EXPECTED_BOUNDED_WALLET_OWNER
        self.wallet_balance = wallet_balance

    def chain_id(self) -> int:
        return MODULE.BASE_MAINNET_CHAIN_ID

    def code(self, address: str) -> str:
        return "0x6001"

    def call(self, address: str, signature: str, *args: str) -> str:
        if signature == "owner()(address)":
            return self.owner
        if signature == "balanceOf(address)(uint256)":
            return str(self.wallet_balance)
        raise AssertionError((address, signature, args))

    def command(self, *args: str, timeout: int = 300) -> str:
        self.calldata_args = args
        return "0x" + "12" * (4 + 32 * 3)

    def rpc(self, *args: str, timeout: int = 300) -> str:
        if args == ("block-number",):
            return "123456"
        raise AssertionError(args)

    def keccak_text(self, value: str) -> str:
        return "0x" + "ab" * 32


class FakeCanonicalComponentFoundry:
    def call(self, address: str, signature: str, *args: str) -> str:
        if signature == "termsRegistry()(address)":
            return "0x" + "09" * 20
        if signature == "verifierModule()(address)":
            return "0x" + "0a" * 20
        raise AssertionError((address, signature, args))


class FakePlanFoundry:
    def __init__(self, repo: Path) -> None:
        self.repo = repo

    def chain_id(self) -> int:
        return MODULE.BASE_SEPOLIA_CHAIN_ID

    def code(self, address: str) -> str:
        return "0x6001"

    def compiler_evidence(self) -> dict:
        return {**MODULE.EXPECTED_COMPILER_CONFIGURATION, "forge_version": "forge 1.4.0"}

    def block_number(self) -> int:
        return 123

    def block_timestamp(self, block_number: int) -> int:
        self.timestamp_block = block_number
        return 456

    def balance_at(self, address: str, block_number: int) -> int:
        self.balance_observation = (address, block_number)
        return 789


class StandingMetaV4DeployTests(unittest.TestCase):
    def readiness(self, r4_evidence: dict[str, bool]) -> dict:
        r4 = dict(r4_evidence)
        r4["independent_review_evidence"] = {
            "source_commit": None,
            "source_tree": None,
            "reviewer_identity": None,
            "review_url": None,
            "report_sha256": None,
            "findings_resolved_or_accepted": False,
        }
        return {
            "schema": "agent-bounties/standing-meta-v4-deployment-readiness-v1",
            "protocol_version": "standing-meta-v4",
            "latency_policy_status": MODULE.EXPECTED_LATENCY_POLICY_STATUS,
            "latency_policy_decision": MODULE.EXPECTED_LATENCY_POLICY_DECISION,
            "configuration": dict(MODULE.EXPECTED_CONFIGURATION),
            "monitoring_policy": dict(MODULE.EXPECTED_MONITORING_POLICY),
            "required_components": list(MODULE.EXPECTED_CANONICAL_COMPONENTS),
            "networks": {
                "base-mainnet": {
                    "sponsorship_intent": {
                        "maximum_source_amount_base_units": MODULE.MAINNET_SOURCE_USDC_CAP
                    }
                }
            },
            "r4_evidence": r4,
        }

    def test_release_errors_redact_signer_and_rpc_credentials(self) -> None:
        rendered = MODULE.redacted_command(
            [
                "cast",
                "send",
                "--private-key",
                "0xsupersecret",
                "--rpc-url",
                "https://rpc.example/private-token",
            ]
        )
        self.assertNotIn("supersecret", rendered)
        self.assertNotIn("private-token", rendered)
        self.assertEqual(rendered.count("[redacted]"), 2)

    def test_networks_pin_official_vrf_configuration(self) -> None:
        mainnet = MODULE.network_config(MODULE.BASE_MAINNET_CHAIN_ID)
        sepolia = MODULE.network_config(MODULE.BASE_SEPOLIA_CHAIN_ID)
        self.assertEqual(mainnet["vrf_coordinator"], MODULE.BASE_MAINNET_VRF)
        self.assertEqual(mainnet["key_hash"], MODULE.BASE_MAINNET_KEY_HASH)
        self.assertEqual(sepolia["vrf_coordinator"], MODULE.BASE_SEPOLIA_VRF)
        self.assertEqual(sepolia["key_hash"], MODULE.BASE_SEPOLIA_KEY_HASH)
        with self.assertRaises(MODULE.DeploymentError):
            MODULE.network_config(1)

    def test_coordinator_configuration_is_live_and_compatible(self) -> None:
        config = MODULE.coordinator_configuration(
            FakeCoordinatorFoundry(), MODULE.BASE_SEPOLIA_VRF, MODULE.BASE_SEPOLIA_KEY_HASH
        )
        self.assertEqual(config["requested_confirmations"], 3)
        self.assertEqual(config["requested_callback_gas_limit"], 150_000)
        self.assertEqual(config["maximum_callback_gas_limit"], 2_500_000)
        self.assertTrue(config["proving_key_registered"])

    def test_coordinator_configuration_fails_closed_on_incompatible_state(self) -> None:
        cases = (
            (FakeCoordinatorFoundry(minimum_confirmations=4), "minimum confirmations"),
            (FakeCoordinatorFoundry(maximum_callback_gas=149_999), "maximum callback gas"),
            (FakeCoordinatorFoundry(reentrancy_lock=True), "reentrancy lock"),
            (FakeCoordinatorFoundry(proving_key_marker="0x" + "00" * 20), "not registered"),
        )
        for foundry, message in cases:
            with self.subTest(message=message), self.assertRaisesRegex(MODULE.DeploymentError, message):
                MODULE.coordinator_configuration(
                    foundry, MODULE.BASE_SEPOLIA_VRF, MODULE.BASE_SEPOLIA_KEY_HASH
                )

    def test_mainnet_source_cap_is_exactly_seven_usdc(self) -> None:
        self.assertEqual(MODULE.MAINNET_SOURCE_USDC_CAP, 7_000_000)
        self.assertEqual(MODULE.EIP170_RUNTIME_LIMIT, 24_576)
        self.assertEqual(MODULE.EIP3860_INITCODE_LIMIT, 49_152)
        self.assertEqual(MODULE.BOUNDED_WALLET, "0x1eaa1c68772cf76bc5f4e4174766076e33ace662")

    def test_owner_withdrawal_request_is_unsigned_exact_and_capped(self) -> None:
        foundry = FakeOwnerWithdrawalFoundry()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "readiness.json"
            MODULE.write_object(path, self.readiness({}))
            request = MODULE.prepare_owner_withdrawal(foundry, path, 7_000_000)

            self.assertEqual(request["wallet_owner"], MODULE.EXPECTED_BOUNDED_WALLET_OWNER)
            self.assertEqual(request["recipient"], MODULE.EXPECTED_KEEPER)
            self.assertEqual(request["amount_base_units"], 7_000_000)
            self.assertEqual(request["status"], "unsigned_not_authorized")
            self.assertFalse(request["ready_to_submit"])
            self.assertNotIn("signature", request["unsigned_transaction"])
            self.assertNotIn("private_key", request["unsigned_transaction"])
            self.assertEqual(foundry.calldata_args[1], "withdrawToken(address,address,uint256)")

            with self.assertRaisesRegex(MODULE.DeploymentError, "seven USDC cap"):
                MODULE.prepare_owner_withdrawal(foundry, path, 7_000_001)

    def test_owner_withdrawal_request_rejects_owner_or_balance_drift(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "readiness.json"
            MODULE.write_object(path, self.readiness({}))
            with self.assertRaisesRegex(MODULE.DeploymentError, "owner drift"):
                MODULE.prepare_owner_withdrawal(
                    FakeOwnerWithdrawalFoundry(owner="0x" + "11" * 20), path, 1
                )
            with self.assertRaisesRegex(MODULE.DeploymentError, "balance is below"):
                MODULE.prepare_owner_withdrawal(FakeOwnerWithdrawalFoundry(wallet_balance=1), path, 2)

    def test_latency_policy_is_fast_and_fail_closed(self) -> None:
        self.assertEqual(MODULE.EXPECTED_CONFIGURATION["per_bounty_solver_enrollment_seconds"], 0)
        self.assertEqual(MODULE.EXPECTED_CONFIGURATION["solver_assignment_seconds"], 120)
        self.assertEqual(MODULE.EXPECTED_CONFIGURATION["primary_response_seconds"], 1_800)
        self.assertEqual(MODULE.EXPECTED_CONFIGURATION["appeal_filing_seconds"], 14_400)
        self.assertEqual(MODULE.EXPECTED_CONFIGURATION["appeal_voting_seconds"], 7_200)
        self.assertEqual(MODULE.EXPECTED_CONFIGURATION["bounty_verification_seconds"], 86_400)
        self.assertEqual(MODULE.EXPECTED_CONFIGURATION["minimum_request_confirmations"], 3)
        self.assertEqual(MODULE.EXPECTED_CONFIGURATION["fulfillment_deadline_seconds"], 7_200)

    def test_source_revision_requires_exact_commit_and_reports_dirty_state(self) -> None:
        commit = "ab" * 20
        tree = "cd" * 20
        with mock.patch.object(MODULE, "run", side_effect=[commit, tree, ""]):
            clean = MODULE.source_revision_evidence(Path("repo"), "git")
        self.assertEqual(clean, {"commit": commit, "tree": tree, "clean": True})

        with mock.patch.object(MODULE, "run", side_effect=[commit, tree, " M src/X.sol"]):
            dirty = MODULE.source_revision_evidence(Path("repo"), "git")
        self.assertFalse(dirty["clean"])

        with mock.patch.object(MODULE, "run", return_value="short"):
            with self.assertRaisesRegex(MODULE.DeploymentError, "40-character"):
                MODULE.source_revision_evidence(Path("repo"), "git")

    def test_compiler_evidence_is_exact_and_fails_on_drift(self) -> None:
        metadata = {
            "compiler": {"version": "0.8.26+commit.8a97fa7a"},
            "settings": {
                "optimizer": {"enabled": True, "runs": 200},
                "metadata": {"bytecodeHash": "ipfs"},
                "evmVersion": "cancun",
            },
        }
        foundry = MODULE.Foundry(Path("repo"), "https://rpc.invalid", "forge", "cast")
        artifact = {"metadata": metadata}
        with (
            mock.patch.object(foundry, "build_artifacts"),
            mock.patch.object(foundry, "artifact", return_value=artifact),
            mock.patch.object(MODULE, "run", return_value="forge Version: 1.4.0"),
        ):
            evidence = foundry.compiler_evidence()
        for name, expected in MODULE.EXPECTED_COMPILER_CONFIGURATION.items():
            self.assertEqual(evidence[name], expected)

        metadata["settings"]["optimizer"]["runs"] = 201
        with (
            mock.patch.object(foundry, "build_artifacts"),
            mock.patch.object(foundry, "artifact", return_value=artifact),
            mock.patch.object(MODULE, "run", return_value="forge Version: 1.4.0"),
        ):
            with self.assertRaisesRegex(MODULE.DeploymentError, "optimizer_runs"):
                foundry.compiler_evidence()

    def test_compiled_bytecode_rejects_placeholders_and_odd_lengths(self) -> None:
        self.assertEqual(MODULE.compiled_bytecode({"object": "0x6001"}, "code"), "0x6001")
        for value in ({"object": "0x1"}, {"object": "0x60__"}, {}, None):
            with self.subTest(value=value), self.assertRaises(MODULE.DeploymentError):
                MODULE.compiled_bytecode(value, "code")

    def test_plan_binds_clean_source_compiler_manifest_and_observation_block(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            readiness_path = repo / "deployments" / "standing-meta-v4-config.json"
            MODULE.write_object(readiness_path, self.readiness({}))
            foundry = FakePlanFoundry(repo)
            source = {"commit": "12" * 20, "tree": "23" * 20, "clean": True}
            with (
                mock.patch.object(MODULE, "source_revision_evidence", return_value=source),
                mock.patch.object(MODULE, "artifact_evidence", return_value={"component": {}}),
                mock.patch.object(
                    MODULE,
                    "coordinator_configuration",
                    return_value={"requested_confirmations": 3},
                ) as coordinator,
            ):
                plan = MODULE.build_plan(foundry, readiness_path)

            self.assertEqual(plan["source_revision"], source)
            self.assertEqual(plan["observed_block"], 123)
            self.assertEqual(plan["observed_block_timestamp"], 456)
            self.assertEqual(plan["keeper_native_balance_wei"], 789)
            self.assertTrue(plan["release_candidate_clean"])
            self.assertTrue(plan["selected_network_deploy_allowed"])
            self.assertFalse(plan["mainnet_deploy_allowed"])
            self.assertEqual(
                plan["readiness_manifest"]["repository_path"],
                "deployments/standing-meta-v4-config.json",
            )
            self.assertEqual(plan["content_sha256"], MODULE.content_sha256(plan))
            coordinator.assert_called_once_with(
                foundry,
                MODULE.BASE_SEPOLIA_VRF,
                MODULE.BASE_SEPOLIA_KEY_HASH,
                123,
            )

    def test_readiness_rejects_required_component_schema_drift(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "readiness.json"
            value = self.readiness({})
            value["required_components"][-1] = "lookalike_bundle"
            MODULE.write_object(path, value)
            with self.assertRaisesRegex(MODULE.DeploymentError, "component schema drift"):
                MODULE.validate_readiness_manifest(path)

    def test_readiness_rejects_unfrozen_latency_policy(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "readiness.json"
            value = self.readiness({})
            value["latency_policy_status"] = "draft"
            MODULE.write_object(path, value)
            with self.assertRaisesRegex(MODULE.DeploymentError, "not review-frozen"):
                MODULE.validate_readiness_manifest(path)

    def test_readiness_rejects_monitoring_policy_drift(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "readiness.json"
            value = self.readiness({})
            value["monitoring_policy"]["maximum_snapshot_age_seconds"] = 301
            MODULE.write_object(path, value)
            with self.assertRaisesRegex(MODULE.DeploymentError, "monitoring policy drift"):
                MODULE.validate_readiness_manifest(path)

    def test_subscription_event_parser_requires_one_matching_log(self) -> None:
        foundry = FakeSubscriptionFoundry()
        coordinator = MODULE.BASE_MAINNET_VRF
        receipt = {
            "logs": [
                {
                    "address": coordinator,
                    "topics": ["0x" + "11" * 32, "0x" + (123).to_bytes(32, "big").hex()],
                }
            ]
        }
        self.assertEqual(MODULE.subscription_created_id(foundry, receipt, coordinator), 123)
        self.assertEqual(foundry.value, MODULE.SUBSCRIPTION_CREATED_EVENT)
        with self.assertRaises(MODULE.DeploymentError):
            MODULE.subscription_created_id(foundry, {"logs": []}, coordinator)

    def test_mainnet_release_gate_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "readiness.json"
            MODULE.write_object(
                path,
                self.readiness(
                    {
                        "independent_review_complete": False,
                        "base_sepolia_rehearsal_complete": False,
                        "base_mainnet_fork_test_complete": True,
                    }
                ),
            )
            with self.assertRaises(MODULE.DeploymentError):
                MODULE.require_mainnet_release_gate(path, True)
            with self.assertRaises(MODULE.DeploymentError):
                MODULE.require_mainnet_release_gate(path, False)

            MODULE.write_object(
                path,
                self.readiness(
                    {
                        name: True
                        for name in MODULE.REQUIRED_R4_GATES
                        if name != "repository_environment_protection_complete"
                    }
                ),
            )
            with self.assertRaisesRegex(MODULE.DeploymentError, "repository_environment_protection_complete"):
                MODULE.require_mainnet_release_gate(path, True)

            value = MODULE.load_object(path)
            value["r4_evidence"]["repository_environment_protection_complete"] = True
            MODULE.write_object(path, value)
            with self.assertRaisesRegex(MODULE.DeploymentError, "review source commit"):
                MODULE.require_mainnet_release_gate(path, True)

            value["r4_evidence"]["independent_review_evidence"] = {
                "source_commit": "12" * 20,
                "source_tree": "23" * 20,
                "reviewer_identity": "external-reviewer",
                "review_url": "https://example.test/reviews/v4",
                "report_sha256": "34" * 32,
                "findings_resolved_or_accepted": True,
            }
            MODULE.write_object(path, value)
            MODULE.require_mainnet_release_gate(path, True)
            MODULE.validated_independent_review_evidence(
                value, {"commit": "12" * 20, "tree": "23" * 20}
            )
            MODULE.validated_independent_review_evidence(
                value, {"commit": "56" * 20, "tree": "23" * 20}
            )
            with self.assertRaisesRegex(MODULE.DeploymentError, "differs"):
                MODULE.validated_independent_review_evidence(
                    value, {"commit": "56" * 20, "tree": "67" * 20}
                )

    def test_manifest_commitment_is_canonical_across_line_endings(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            path = repo / "deployments" / "standing-meta-v4-config.json"
            value = self.readiness({})
            MODULE.write_object(path, value)
            first = MODULE.readiness_manifest_evidence(path, value, repo)
            path.write_bytes(path.read_bytes().replace(b"\n", b"\r\n"))
            second = MODULE.readiness_manifest_evidence(path, value, repo)
            self.assertEqual(first["content_sha256"], second["content_sha256"])

    def test_component_constructor_graph_uses_two_distinct_sortitions(self) -> None:
        report = {
            "settlement_token": MODULE.BASE_MAINNET_USDC,
            "vrf_coordinator": MODULE.BASE_MAINNET_VRF,
            "key_hash": MODULE.BASE_MAINNET_KEY_HASH,
            "subscription_id": 99,
            "deployer": MODULE.EXPECTED_KEEPER,
            "components": {
                "base_child_factory": {"address": MODULE.BASE_MAINNET_FACTORY},
                "controller": {"address": "0x" + "01" * 20},
                "verifier_sortition": {"address": "0x" + "02" * 20},
                "solver_sortition": {"address": "0x" + "03" * 20},
                "appealable_verifier": {"address": "0x" + "04" * 20},
                "standing_meta_child_factory": {"address": "0x" + "05" * 20},
                "standing_meta_parent_factory": {"address": "0x" + "06" * 20},
                "stake_pool": {"address": "0x" + "07" * 20},
            },
        }
        verifier_args = MODULE.component_args("verifier_sortition", report)
        solver_args = MODULE.component_args("solver_sortition", report)
        self.assertEqual(verifier_args, solver_args)
        bundle_args = MODULE.component_args("standing_meta_v4_bundle", report)
        self.assertIn(report["components"]["verifier_sortition"]["address"], bundle_args)
        self.assertIn(report["components"]["solver_sortition"]["address"], bundle_args)
        self.assertNotEqual(
            report["components"]["verifier_sortition"]["address"],
            report["components"]["solver_sortition"]["address"],
        )

    def test_canonical_component_evidence_includes_factory_created_contracts(self) -> None:
        report_names = (
            "controller",
            "stake_pool",
            "verifier_sortition",
            "solver_sortition",
            "appealable_verifier",
            "standing_meta_child_factory",
            "standing_meta_parent_factory",
            "standing_meta_v4_bundle",
        )
        components = {
            name: {"address": "0x" + f"{index + 1:040x}"}
            for index, name in enumerate(report_names)
        }
        addresses = MODULE.canonical_component_addresses(FakeCanonicalComponentFoundry(), components)
        self.assertEqual(set(addresses), set(MODULE.EXPECTED_CANONICAL_COMPONENTS))
        self.assertEqual(addresses["onchain_terms_registry"], "0x" + "09" * 20)
        self.assertEqual(addresses["canonical_independent_child_verifier"], "0x" + "0a" * 20)


if __name__ == "__main__":
    unittest.main()
