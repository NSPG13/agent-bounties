import importlib.util
from pathlib import Path
import unittest
from unittest import mock


SCRIPT = Path(__file__).with_name("build_open_competition_v2_beta2_release.py")
SPEC = importlib.util.spec_from_file_location("open_competition_v2_release", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class OpenCompetitionV2ReleaseTests(unittest.TestCase):
    subject_hash = "0x" + "33" * 32

    @staticmethod
    def verifier_assets() -> dict:
        systems = {}
        for index, name in enumerate(("groth16", "plonk"), start=1):
            creation = bytes([0x60, index, 0x60, 0x00])
            runtime = bytes([0x60, index])
            systems[name] = {
                "verifier_hash": "0x" + f"{index:02x}" * 32,
                "creation_code": "0x" + creation.hex(),
                "creation_code_hash": MODULE.keccak256(creation),
                "runtime_code": "0x" + runtime.hex(),
                "runtime_code_hash": MODULE.keccak256(runtime),
            }
        return {
            "schema_version": "agent-bounties/open-competition-v2-beta2-verifier-assets-v1",
            "sp1_source_commit": MODULE.SP1_COMMIT,
            "circuit_version": MODULE.SP1_SAFE_CIRCUIT_VERSION,
            "gpu_proving_enabled": False,
            "asset_state": "self_verified",
            "proof_systems": systems,
            "proof_evidence": {
                "groth16_self_verified": "0x" + "11" * 32,
                "plonk_self_verified_1": "0x" + "22" * 32,
                "plonk_self_verified_2": "0x" + "33" * 32,
            },
        }

    def test_metric_identity_is_single_release_source_of_truth(self) -> None:
        identity = MODULE.METRIC_IDENTITY
        self.assertEqual(
            identity["schema"],
            "agent-bounties/open-competition-v2-metric-release-identity-v1",
        )
        self.assertEqual(identity["rust_version"], "1.96.1")
        self.assertEqual(identity["status"], "pending_patched_rebuild")
        self.assertEqual(MODULE.PROGRAM_VKEY, identity["program_vkey"])
        self.assertEqual(MODULE.SOURCE_HASH, identity["source_hash"])
        self.assertEqual(MODULE.ELF_HASH, identity["elf_keccak256"])
        self.assertEqual(MODULE.ELF_SHA256, identity["elf_sha256"])

    def test_current_manifest_fails_closed(self) -> None:
        gates = MODULE.load_gates(
            MODULE.ROOT / "deployments/open-competition-v2-beta2-release-gates.json"
        )
        self.assertFalse(gates["prelaunch_complete"])
        self.assertFalse(gates["public_beta_launch_complete"])
        self.assertFalse(gates["graduation_complete"])
        self.assertRegex(gates["beta_risk_hash"], r"^0x[0-9a-f]{64}$")

    def test_release_stages_do_not_require_graduation_before_beta(self) -> None:
        path = MODULE.ROOT / "target/tmp/open-competition-v2-staged-gates.json"
        evidence = {
            "source_commit": "a" * 40,
            "subject_hash": self.subject_hash,
            "evidence_hash": "0x" + "11" * 32,
            "uri": "https://example.test/evidence",
        }
        value = {
            "schema_version": "agent-bounties/open-competition-v2-beta2-release-gates-v3",
            "protocol_version": "agent-bounties/open-competition-v2-beta2",
            "beta_risk_preimage": "risk",
            "gates": {name: False for name in MODULE.REQUIRED_GATE_NAMES},
            "evidence": {name: None for name in MODULE.REQUIRED_GATE_NAMES},
        }
        path.parent.mkdir(parents=True, exist_ok=True)
        for name in MODULE.PRELAUNCH_GATE_NAMES:
            value["gates"][name] = True
            value["evidence"][name] = evidence
        path.write_text(__import__("json").dumps(value), encoding="utf-8")
        gates = MODULE.load_gates(path, self.subject_hash)
        self.assertTrue(gates["prelaunch_complete"])
        self.assertFalse(gates["public_beta_launch_complete"])
        self.assertFalse(gates["graduation_complete"])
        preflight = {
            "number": 1,
            "hash": "0x" + "22" * 32,
            "timestamp": 1,
            "deployer_nonce": 1,
            "deployer_eth_wei": MODULE.MIN_DEPLOYER_ETH_WEI,
            "deployer_usdc_base_units": 0,
            "dependency_runtime_hashes": {},
        }
        bundle = MODULE.build_bundle(
            network_name="base-mainnet",
            deployer=MODULE.DEFAULT_DEPLOYER,
            source_commit="a" * 40,
            repository_subject=self.subject_hash,
            preflight=preflight,
            gates=gates,
            verifier_assets=self.verifier_assets(),
            allow_pending_metric_identity=True,
        )
        self.assertTrue(bundle["activation"]["mainnet_signing_allowed"])
        self.assertFalse(bundle["activation"]["broker_canary_enabled"])
        self.assertFalse(bundle["activation"]["public_creation_enabled"])
        self.assertFalse(bundle["activation"]["default_protocol_enabled"])

        for name in MODULE.BROKER_CANARY_GATE_NAMES:
            value["gates"][name] = True
            value["evidence"][name] = evidence
        path.write_text(__import__("json").dumps(value), encoding="utf-8")
        gates = MODULE.load_gates(path, self.subject_hash)
        self.assertTrue(gates["broker_canary_ready"])
        self.assertFalse(gates["public_beta_launch_complete"])
        bundle = MODULE.build_bundle(
            network_name="base-mainnet",
            deployer=MODULE.DEFAULT_DEPLOYER,
            source_commit="a" * 40,
            repository_subject=self.subject_hash,
            preflight=preflight,
            gates=gates,
            verifier_assets=self.verifier_assets(),
            allow_pending_metric_identity=True,
        )
        self.assertTrue(bundle["activation"]["broker_canary_enabled"])
        with mock.patch.dict(MODULE.METRIC_IDENTITY, {"status": "reproduced_beta2"}):
            runtime = MODULE.runtime_manifest(bundle, 10)
        self.assertTrue(runtime["proof_broker_enabled"])
        self.assertFalse(runtime["public_creation_enabled"])

        for name in MODULE.PUBLIC_BETA_GATE_NAMES:
            value["gates"][name] = True
            value["evidence"][name] = evidence
        path.write_text(__import__("json").dumps(value), encoding="utf-8")
        gates = MODULE.load_gates(path, self.subject_hash)
        self.assertTrue(gates["public_beta_launch_complete"])
        self.assertFalse(gates["graduation_complete"])
        bundle = MODULE.build_bundle(
            network_name="base-mainnet",
            deployer=MODULE.DEFAULT_DEPLOYER,
            source_commit="a" * 40,
            repository_subject=self.subject_hash,
            preflight=preflight,
            gates=gates,
            verifier_assets=self.verifier_assets(),
            allow_pending_metric_identity=True,
        )
        self.assertTrue(bundle["activation"]["public_creation_enabled"])
        self.assertFalse(bundle["activation"]["default_protocol_enabled"])

        for name in MODULE.GRADUATION_GATE_NAMES:
            value["gates"][name] = True
            value["evidence"][name] = evidence
        path.write_text(__import__("json").dumps(value), encoding="utf-8")
        gates = MODULE.load_gates(path, self.subject_hash)
        self.assertTrue(gates["graduation_complete"])
        bundle = MODULE.build_bundle(
            network_name="base-mainnet",
            deployer=MODULE.DEFAULT_DEPLOYER,
            source_commit="a" * 40,
            repository_subject=self.subject_hash,
            preflight=preflight,
            gates=gates,
            verifier_assets=self.verifier_assets(),
            allow_pending_metric_identity=True,
        )
        self.assertTrue(bundle["activation"]["default_protocol_enabled"])

    def test_completed_gate_requires_hash_bound_evidence(self) -> None:
        path = MODULE.ROOT / "target/tmp/open-competition-v2-unevidenced-gate.json"
        gates = {name: False for name in MODULE.REQUIRED_GATE_NAMES}
        gates["repository_gate_complete"] = True
        value = {
            "schema_version": "agent-bounties/open-competition-v2-beta2-release-gates-v3",
            "protocol_version": "agent-bounties/open-competition-v2-beta2",
            "beta_risk_preimage": "risk",
            "gates": gates,
            "evidence": {name: None for name in MODULE.REQUIRED_GATE_NAMES},
        }
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(__import__("json").dumps(value), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "lacks evidence"):
            MODULE.load_gates(path)

    def test_completed_gate_must_target_exact_repository_subject(self) -> None:
        path = MODULE.ROOT / "target/tmp/open-competition-v2-wrong-subject.json"
        value = {
            "schema_version": "agent-bounties/open-competition-v2-beta2-release-gates-v3",
            "protocol_version": "agent-bounties/open-competition-v2-beta2",
            "beta_risk_preimage": "risk",
            "gates": {name: name == "repository_gate_complete" for name in MODULE.REQUIRED_GATE_NAMES},
            "evidence": {name: None for name in MODULE.REQUIRED_GATE_NAMES},
        }
        value["evidence"]["repository_gate_complete"] = {
            "source_commit": "a" * 40,
            "subject_hash": "0x" + "44" * 32,
            "evidence_hash": "0x" + "11" * 32,
            "uri": "https://example.test/evidence",
        }
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(__import__("json").dumps(value), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "another repository subject"):
            MODULE.load_gates(path, self.subject_hash)

    def test_repository_subject_excludes_only_gate_manifest(self) -> None:
        manifest = MODULE.GATE_MANIFEST_RELATIVE.encode()
        first = (
            b"100644 blob aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\tREADME.md\0"
            + b"100644 blob bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\t"
            + manifest
            + b"\0"
        )
        manifest_only_change = first.replace(b"b" * 40, b"c" * 40)
        source_change = first.replace(b"a" * 40, b"d" * 40)
        with mock.patch.object(MODULE.subprocess, "check_output", return_value=first):
            subject = MODULE.repository_subject_hash("a" * 40)
        with mock.patch.object(
            MODULE.subprocess, "check_output", return_value=manifest_only_change
        ):
            self.assertEqual(MODULE.repository_subject_hash("b" * 40), subject)
        with mock.patch.object(
            MODULE.subprocess, "check_output", return_value=source_change
        ):
            self.assertNotEqual(MODULE.repository_subject_hash("c" * 40), subject)

    def test_exact_checkout_rejects_mismatch_and_tracked_changes(self) -> None:
        with mock.patch.object(
            MODULE.subprocess, "check_output", return_value="b" * 40 + "\n"
        ):
            with self.assertRaisesRegex(ValueError, "checked-out Git HEAD"):
                MODULE.verify_exact_checkout("a" * 40)

        clean = mock.Mock(returncode=0)
        dirty = mock.Mock(returncode=1)
        with mock.patch.object(
            MODULE.subprocess, "check_output", return_value="a" * 40 + "\n"
        ), mock.patch.object(MODULE.subprocess, "run", side_effect=[clean, dirty]):
            with self.assertRaisesRegex(ValueError, "tracked worktree changes"):
                MODULE.verify_exact_checkout("a" * 40)

    def test_factory_requires_project_owned_verifier_identities(self) -> None:
        source = (
            MODULE.ROOT
            / "contracts/base-escrow/src/OpenCompetitionBountyFactoryV2Beta2.sol"
        ).read_text(encoding="utf-8").lower()
        self.assertIn("groth16verifierhash", source)
        self.assertIn("groth16runtimecodehash", source)
        self.assertIn("plonkverifierhash", source)
        self.assertIn("plonkruntimecodehash", source)
        self.assertNotIn("gateway", source)

    def test_immutable_names_are_bound_by_ast_not_sort_order(self) -> None:
        artifact = MODULE.artifact(
            "OpenCompetitionBountyFactoryV2Beta2", "OpenCompetitionBountyFactoryV2Beta2"
        )
        self.assertEqual(
            set(MODULE.immutable_names(artifact).values()),
            {"settlementToken", "groth16Adapter", "plonkAdapter", "implementation"},
        )

    def test_deployer_does_not_need_to_hold_canary_usdc(self) -> None:
        gates = MODULE.load_gates(
            MODULE.ROOT / "deployments/open-competition-v2-beta2-release-gates.json"
        )
        preflight = {
            "number": 1,
            "hash": "0x" + "11" * 32,
            "timestamp": 1,
            "deployer_nonce": 1,
            "deployer_eth_wei": MODULE.MIN_DEPLOYER_ETH_WEI,
            "deployer_usdc_base_units": MODULE.CANARY_BUDGET - 1,
            "dependency_runtime_hashes": {},
        }
        bundle = MODULE.build_bundle(
            network_name="base-mainnet",
            deployer=MODULE.DEFAULT_DEPLOYER,
            source_commit="a" * 40,
            repository_subject=self.subject_hash,
            preflight=preflight,
            gates=gates,
            verifier_assets=self.verifier_assets(),
            allow_pending_metric_identity=True,
        )
        self.assertTrue(bundle["canary_budget"]["deployer_is_not_required_to_fund"])
        self.assertEqual(
            bundle["canary_budget"]["required_usdc_base_units"], MODULE.CANARY_BUDGET
        )
        self.assertEqual(bundle["compiler"]["solc"], "0.8.26+commit.8a97fa7a")
        self.assertEqual(bundle["compiler"]["image"], MODULE.SOLC_IMAGE)
        self.assertRegex(MODULE.SOLC_IMAGE, r"^docker\.io/ethereum/solc@sha256:[0-9a-f]{64}$")


if __name__ == "__main__":
    unittest.main()
