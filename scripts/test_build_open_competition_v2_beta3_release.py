import importlib.util
import json
from pathlib import Path
import unittest
from unittest import mock


SCRIPT = Path(__file__).with_name("build_open_competition_v2_beta3_release.py")
SPEC = importlib.util.spec_from_file_location("open_competition_v2_release", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class OpenCompetitionV2ReleaseTests(unittest.TestCase):
    subject_hash = "0x" + "33" * 32

    def test_release_builder_defaults_to_the_isolated_deployer(self):
        self.assertEqual(
            MODULE.DEFAULT_DEPLOYER,
            "0xfd7be4c69541ab297aece2a674fc1418b898cc0a",
        )

    def test_resumes_exact_verifiers_after_interrupted_factory_deployment(self):
        deployer = MODULE.DEFAULT_DEPLOYER
        expected = {
            MODULE.create_address(deployer, 2): "groth16",
            MODULE.create_address(deployer, 3): "plonk",
        }

        start = MODULE.resume_exact_verifier_pair(
            deployer=deployer,
            observed_nonce=4,
            code_hash=lambda address: expected.get(address),
            groth16_runtime_hash="groth16",
            plonk_runtime_hash="plonk",
        )

        self.assertEqual(start, 2)

    def test_does_not_resume_inexact_partial_deployment(self):
        deployer = MODULE.DEFAULT_DEPLOYER
        groth16 = MODULE.create_address(deployer, 2)
        plonk = MODULE.create_address(deployer, 3)
        cases = (
            {groth16: "wrong", plonk: "plonk"},
            {groth16: "groth16", plonk: "wrong"},
        )
        for observed in cases:
            with self.subTest(observed=observed):
                self.assertEqual(
                    MODULE.resume_exact_verifier_pair(
                        deployer=deployer,
                        observed_nonce=4,
                        code_hash=lambda address, values=observed: values.get(address),
                        groth16_runtime_hash="groth16",
                        plonk_runtime_hash="plonk",
                    ),
                    4,
                )

    def test_resumes_completed_exact_deployment_for_reconciliation(self):
        deployer = MODULE.DEFAULT_DEPLOYER
        observed = {
            MODULE.create_address(deployer, 2): "groth16",
            MODULE.create_address(deployer, 3): "plonk",
            MODULE.create_address(deployer, 4): "factory",
        }

        self.assertEqual(
            MODULE.resume_exact_verifier_pair(
                deployer=deployer,
                observed_nonce=5,
                code_hash=lambda address: observed.get(address),
                groth16_runtime_hash="groth16",
                plonk_runtime_hash="plonk",
            ),
            2,
        )

    def test_release_root_retargets_only_release_files(self):
        original = (MODULE.ROOT, MODULE.CONTRACT_ROOT, MODULE.OUT)
        try:
            release_root = Path("target/frozen-release")
            MODULE.configure_release_root(release_root)
            self.assertEqual(MODULE.ROOT, release_root.resolve())
            self.assertEqual(MODULE.CONTRACT_ROOT, MODULE.ROOT / "contracts/base-escrow")
            self.assertEqual(MODULE.OUT, MODULE.CONTRACT_ROOT / "out")
        finally:
            MODULE.ROOT, MODULE.CONTRACT_ROOT, MODULE.OUT = original

    def test_mainnet_continuation_uses_current_control_on_frozen_release(self):
        workflow = (
            MODULE.ROOT / ".github/workflows/continue-open-competition-v2-beta3-mainnet.yml"
        ).read_text(encoding="utf-8")
        deploy = workflow.split("  deploy-mainnet:", 1)[1].split(
            "  deploy-production-prover:", 1
        )[0]
        self.assertIn("path: .release-control", deploy)
        self.assertIn(
            "python .release-control/scripts/build_open_competition_v2_beta3_release.py",
            deploy,
        )
        self.assertIn('--release-root "$GITHUB_WORKSPACE"', deploy)
        self.assertIn(
            "python .release-control/scripts/deploy_open_competition_v2_beta3.py",
            deploy,
        )

    def test_proof_build_pins_every_bundle_to_the_configured_deployer(self):
        workflow = (
            MODULE.ROOT / ".github/workflows/open-competition-v2-beta3-release.yml"
        ).read_text(encoding="utf-8")
        build = workflow.split("  build-release-assets:", 1)[1].split(
            "  live-sepolia-rehearsal:", 1
        )[0]
        self.assertIn("BASE_DEPLOYER_ADDRESS: ${{ vars.BASE_DEPLOYER_ADDRESS }}", build)
        self.assertIn('test -n "$BASE_DEPLOYER_ADDRESS"', build)
        self.assertEqual(
            build.count(
                'build_open_competition_v2_beta3_release.py --network base-mainnet'
            ),
            3,
        )
        self.assertEqual(
            build.count('--deployer "$DEPLOYER" --source-commit "$GITHUB_SHA"'),
            3,
        )

    def test_mainnet_workflow_records_the_exact_x402_gate(self):
        workflow = (MODULE.ROOT / ".github/workflows/open-competition-v2-beta3-release.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn(
            "mainnet_canary_accounting_reconciled x402_success_and_refund_complete",
            workflow,
        )
        self.assertNotIn("x402_success_and_refund_canaries_complete", workflow)

    def test_mainnet_canaries_rebuild_cleaned_contract_artifacts(self):
        for workflow_path in (
            ".github/workflows/open-competition-v2-beta3-release.yml",
            ".github/workflows/continue-open-competition-v2-beta3-mainnet.yml",
        ):
            workflow = (MODULE.ROOT / workflow_path).read_text(encoding="utf-8")
            canary = workflow.split("  mainnet-canaries:", 1)[1].split(
                "  activate-public-beta:", 1
            )[0]
            self.assertIn(
                "foundry-rs/foundry-toolchain@b00af27efadbc7b4ca8b82abbd903b17cc874d2a",
                canary,
            )
            self.assertIn(
                "forge build --root contracts/base-escrow --force --ast",
                canary,
            )

    def test_sepolia_rehearsal_uses_an_isolated_broker(self):
        workflow = (MODULE.ROOT / ".github/workflows/open-competition-v2-beta3-release.yml").read_text(
            encoding="utf-8"
        )
        sepolia = workflow.split("  live-sepolia-rehearsal:", 1)[1].split(
            "  deploy-mainnet:", 1
        )[0]
        self.assertIn("OPEN_COMPETITION_V2_BROKER_PRIVATE_KEY", sepolia)
        self.assertIn("--network base-sepolia", sepolia)
        self.assertIn('OPEN_COMPETITION_V2_BROKER_PAYMENT_ADDRESS="$BROKER"', sepolia)
        self.assertIn('X402_RELAYER_PRIVATE_KEY="$OPEN_COMPETITION_V2_BROKER_PRIVATE_KEY"', sepolia)
        self.assertNotIn('X402_RELAYER_PRIVATE_KEY="$BASE_SEPOLIA_DEPLOYER_PRIVATE_KEY"', sepolia)

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
        setup_systems = {}
        for index, name, model in (
            (1, "groth16", "mpc_phase2"),
            (2, "plonk", "public_mpc_kzg_srs"),
        ):
            setup_systems[name] = {
                "security_model": model,
                "verification_passed": True,
                "verifier_hash": systems[name]["verifier_hash"],
                "constraint_system_sha256": f"{index:02x}" * 32,
                "proving_key_sha256": "44" * 32,
                "verifying_key_sha256": "55" * 32,
                "transcript_sha256": "66" * 32,
                "verification_evidence_sha256": "77" * 32,
                "contribution_count": 3,
            }
        return {
            "schema_version": "agent-bounties/open-competition-v2-beta3-verifier-assets-v2",
            "sp1_source_commit": MODULE.SP1_COMMIT,
            "circuit_version": MODULE.SP1_SAFE_CIRCUIT_VERSION,
            "gpu_proving_enabled": False,
            "asset_state": "self_verified",
            "setup_provenance": {
                "state": "trusted_mpc",
                "mainnet_eligible": True,
                "manifest_sha256": "0x" + "44" * 32,
                "systems": setup_systems,
            },
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
        self.assertEqual(identity["sp1_guest_rust_version"], "1.94.0-dev")
        self.assertEqual(MODULE.HOST_RUST_VERSION, "1.96.1")
        self.assertEqual(MODULE.SP1_GUEST_RUST_VERSION, "1.94.0-dev")
        self.assertEqual(identity["status"], "reproduced_beta3")
        self.assertEqual(MODULE.PROGRAM_VKEY, identity["program_vkey"])
        self.assertEqual(MODULE.SOURCE_HASH, identity["source_hash"])
        self.assertEqual(MODULE.ELF_HASH, identity["elf_keccak256"])
        self.assertEqual(MODULE.ELF_SHA256, identity["elf_sha256"])

    def test_metric_guest_and_host_builds_remain_separate(self) -> None:
        helper = (MODULE.ROOT / "scripts/build_open_competition_v2_metric_elf.sh").read_text(
            encoding="utf-8"
        )
        workflow = (MODULE.ROOT / ".github/workflows/open-competition-v2-beta3-release.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn("cargo prove build --locked", helper)
        self.assertIn("--remap-path-prefix=$repository=/agent-bounties", helper)
        self.assertIn('"$strip" --strip-all "$elf"', helper)
        self.assertIn("build_open_competition_v2_metric_elf.sh", workflow)
        self.assertIn("OPEN_COMPETITION_V2_METRIC_ELF", workflow)
        self.assertIn("SP1_SAFE_CIRCUIT_COMMIT", workflow)
        self.assertIn("SP1_SAFE_RUNTIME_COMMIT", workflow)

    def test_workflow_sp1_pins_match_metric_release_identities(self) -> None:
        workflow = (MODULE.ROOT / ".github/workflows/open-competition-v2-beta3-release.yml").read_text(
            encoding="utf-8"
        )
        identities = [
            json.loads((MODULE.ROOT / path).read_text(encoding="utf-8"))
            for path in (
                "programs/public-vector-metric-v1/release-identity.json",
                "programs/structured-artifact-metric-v1/release-identity.json",
                "programs/forward-canonical-gmv-attribution-metric-v2/release-identity.json",
            )
        ]
        circuit_commits = {identity["sp1_commit"] for identity in identities}
        runtime_commits = {identity["sp1_runtime_commit"] for identity in identities}
        self.assertEqual(len(circuit_commits), 1)
        self.assertEqual(len(runtime_commits), 1)
        circuit_commit = circuit_commits.pop()
        runtime_commit = runtime_commits.pop()
        self.assertIn(f"SP1_SAFE_CIRCUIT_COMMIT: {circuit_commit}", workflow)
        self.assertIn(f"SP1_SAFE_RUNTIME_COMMIT: {runtime_commit}", workflow)

    def test_current_manifest_fails_closed(self) -> None:
        gates = MODULE.load_gates(
            MODULE.ROOT / "deployments/open-competition-v2-beta3-release-gates.json"
        )
        self.assertFalse(gates["prelaunch_complete"])
        self.assertFalse(gates["public_beta_launch_complete"])
        self.assertFalse(gates["sepolia_broker_rehearsal_ready"])
        self.assertFalse(gates["graduation_complete"])
        self.assertRegex(gates["beta_risk_hash"], r"^0x[0-9a-f]{64}$")

    def test_mainnet_rejects_test_only_setup(self) -> None:
        assets = self.verifier_assets()
        assets["setup_provenance"] = {
            "state": "test_only_unsafe",
            "mainnet_eligible": False,
            "manifest_sha256": None,
            "systems": {
                "groth16": {"security_model": "single_party_local_setup"},
                "plonk": {"security_model": "unverified_setup_provenance"},
            },
        }
        gates = MODULE.load_gates(
            MODULE.ROOT / "deployments/open-competition-v2-beta3-release-gates.json"
        )
        preflight = {
            "number": 1,
            "hash": "0x" + "22" * 32,
            "timestamp": 1,
            "deployer_nonce": 1,
            "deployer_eth_wei": MODULE.MIN_DEPLOYER_ETH_WEI,
            "deployer_usdc_base_units": 0,
            "dependency_runtime_hashes": {},
        }
        common = dict(
            deployer=MODULE.DEFAULT_DEPLOYER,
            source_commit="a" * 40,
            repository_subject=self.subject_hash,
            preflight=preflight,
            gates=gates,
            verifier_assets=assets,
            allow_pending_metric_identity=True,
        )
        MODULE.build_bundle(network_name="base-sepolia", **common)
        with self.assertRaisesRegex(RuntimeError, "trusted setup provenance"):
            MODULE.build_bundle(network_name="base-mainnet", **common)

    def test_release_stages_do_not_require_graduation_before_beta(self) -> None:
        path = MODULE.ROOT / "target/tmp/open-competition-v2-staged-gates.json"
        evidence = {
            "source_commit": "a" * 40,
            "subject_hash": self.subject_hash,
            "evidence_hash": "0x" + "11" * 32,
            "uri": "https://example.test/evidence",
        }
        value = {
            "schema_version": "agent-bounties/open-competition-v2-beta3-release-gates-v5",
            "protocol_version": "agent-bounties/open-competition-v2-beta3",
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
        self.assertEqual(gates["subject_hash"], self.subject_hash)
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
        self.assertEqual(bundle["sp1"]["host_rust_version"], "1.96.1")
        self.assertEqual(bundle["sp1"]["guest_rust_version"], "1.94.0-dev")
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
        with mock.patch.dict(
            MODULE.METRIC_IDENTITY, {"status": "reproduced_beta3"}
        ), mock.patch.dict(
            MODULE.STRUCTURED_ARTIFACT_IDENTITY, {"status": "reproduced_beta3"}
        ), mock.patch.dict(
            MODULE.CANONICAL_GMV_IDENTITY, {"status": "reproduced_beta3"}
        ):
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

    def test_sepolia_broker_rehearsal_breaks_no_mainnet_gate(self) -> None:
        path = MODULE.ROOT / "target/tmp/open-competition-v2-sepolia-broker-gates.json"
        evidence = {
            "source_commit": "a" * 40,
            "subject_hash": self.subject_hash,
            "evidence_hash": "0x" + "11" * 32,
            "uri": "https://example.test/evidence",
        }
        value = {
            "schema_version": "agent-bounties/open-competition-v2-beta3-release-gates-v5",
            "protocol_version": "agent-bounties/open-competition-v2-beta3",
            "beta_risk_preimage": "risk",
            "gates": {name: False for name in MODULE.REQUIRED_GATE_NAMES},
            "evidence": {name: None for name in MODULE.REQUIRED_GATE_NAMES},
        }
        for name in MODULE.SEPOLIA_BROKER_REHEARSAL_GATE_NAMES:
            value["gates"][name] = True
            value["evidence"][name] = evidence
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(__import__("json").dumps(value), encoding="utf-8")
        gates = MODULE.load_gates(path, self.subject_hash)
        self.assertTrue(gates["sepolia_broker_rehearsal_ready"])
        self.assertFalse(gates["prelaunch_complete"])
        preflight = {
            "number": 1,
            "hash": "0x" + "22" * 32,
            "timestamp": 1,
            "deployer_nonce": 1,
            "deployer_eth_wei": MODULE.MIN_DEPLOYER_ETH_WEI,
            "deployer_usdc_base_units": 0,
            "dependency_runtime_hashes": {},
        }
        common = dict(
            deployer=MODULE.DEFAULT_DEPLOYER,
            source_commit="a" * 40,
            repository_subject=self.subject_hash,
            preflight=preflight,
            gates=gates,
            verifier_assets=self.verifier_assets(),
            allow_pending_metric_identity=True,
        )
        sepolia = MODULE.build_bundle(network_name="base-sepolia", **common)
        mainnet = MODULE.build_bundle(network_name="base-mainnet", **common)
        self.assertTrue(sepolia["activation"]["sepolia_broker_rehearsal_enabled"])
        self.assertFalse(mainnet["activation"]["sepolia_broker_rehearsal_enabled"])
        with mock.patch.dict(
            MODULE.METRIC_IDENTITY, {"status": "reproduced_beta3"}
        ), mock.patch.dict(
            MODULE.STRUCTURED_ARTIFACT_IDENTITY, {"status": "reproduced_beta3"}
        ), mock.patch.dict(
            MODULE.CANONICAL_GMV_IDENTITY, {"status": "reproduced_beta3"}
        ):
            self.assertTrue(MODULE.runtime_manifest(sepolia, 10)["proof_broker_enabled"])
            self.assertFalse(MODULE.runtime_manifest(mainnet, 10)["proof_broker_enabled"])

    def test_completed_gate_requires_hash_bound_evidence(self) -> None:
        path = MODULE.ROOT / "target/tmp/open-competition-v2-unevidenced-gate.json"
        gates = {name: False for name in MODULE.REQUIRED_GATE_NAMES}
        gates["repository_gate_complete"] = True
        value = {
            "schema_version": "agent-bounties/open-competition-v2-beta3-release-gates-v5",
            "protocol_version": "agent-bounties/open-competition-v2-beta3",
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
            "schema_version": "agent-bounties/open-competition-v2-beta3-release-gates-v5",
            "protocol_version": "agent-bounties/open-competition-v2-beta3",
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
            / "contracts/base-escrow/src/OpenCompetitionBountyFactoryV2Beta3.sol"
        ).read_text(encoding="utf-8").lower()
        self.assertIn("groth16verifierhash", source)
        self.assertIn("groth16runtimecodehash", source)
        self.assertIn("plonkverifierhash", source)
        self.assertIn("plonkruntimecodehash", source)
        self.assertNotIn("gateway", source)

    def test_immutable_names_are_bound_by_ast_not_sort_order(self) -> None:
        artifact = MODULE.artifact(
            "OpenCompetitionBountyFactoryV2Beta3", "OpenCompetitionBountyFactoryV2Beta3"
        )
        self.assertEqual(
            set(MODULE.immutable_names(artifact).values()),
            {"settlementToken", "groth16Adapter", "plonkAdapter", "implementation"},
        )

    def test_deployer_does_not_need_to_hold_canary_usdc(self) -> None:
        gates = MODULE.load_gates(
            MODULE.ROOT / "deployments/open-competition-v2-beta3-release-gates.json"
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
