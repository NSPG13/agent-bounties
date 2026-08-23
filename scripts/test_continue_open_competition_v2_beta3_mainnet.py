from pathlib import Path
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/continue-open-competition-v2-beta3-mainnet.yml"
RELEASE_WORKFLOW = ROOT / ".github/workflows/open-competition-v2-beta3-release.yml"
RELEASE_SOURCE_COMMIT = "5a351f3e373691be58a9575b4374812b494b6086"


class UniqueKeyLoader(yaml.SafeLoader):
    pass


def construct_mapping(loader, node, deep=False):
    mapping = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=deep)
        if key in mapping:
            raise ValueError(f"duplicate YAML key: {key}")
        mapping[key] = loader.construct_object(value_node, deep=deep)
    return mapping


UniqueKeyLoader.add_constructor(
    yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, construct_mapping
)


class MainnetContinuationWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = WORKFLOW.read_text(encoding="utf-8")
        cls.workflow = yaml.load(cls.text, Loader=UniqueKeyLoader)

    def test_self_hosted_prover_uses_available_python3(self):
        prover = self.text.split("  deploy-production-prover:", 1)[1].split(
            "  deploy-production-control-plane:", 1
        )[0]
        self.assertIn("python3 - <<'PY'", prover)
        self.assertNotIn("python - <<'PY'", prover)

    def test_release_and_protected_environment_are_pinned(self):
        self.assertEqual(
            self.workflow["env"]["RELEASE_SOURCE_COMMIT"], RELEASE_SOURCE_COMMIT
        )
        for name, expected in (
            ("RELEASE_HASH", "0x46008fb819726a43209e55ee7e58c92700a8fc3435f76be282bfdef710ced594"),
            ("COMPETITION_FACTORY", "0x29d0e39e0c03797c690633535722e6b34a69a78a"),
            ("BOUNDED_RESERVE_FACTORY", "0xad0765eac772ff6cf696f2416751269d97a5419f"),
            ("BOUNDED_RESERVE_IMPLEMENTATION", "0x9c62e1ab727909a18a830744eb244645ee91b0eb"),
        ):
            self.assertIsInstance(self.workflow["env"][name], str)
            self.assertEqual(self.workflow["env"][name], expected)
        self.assertEqual(self.workflow["permissions"]["actions"], "read")
        for job in self.workflow["jobs"].values():
            self.assertEqual(job["environment"], "v2-beta2-mainnet")
            checkout = next(
                step for step in job["steps"] if str(step.get("uses", "")).startswith("actions/checkout@")
            )
            self.assertEqual(checkout["with"]["ref"], "${{ env.RELEASE_SOURCE_COMMIT }}")

    def test_dependency_chain_is_fail_closed(self):
        jobs = self.workflow["jobs"]
        self.assertEqual(jobs["deploy-production-prover"]["needs"], "deploy-mainnet")
        self.assertEqual(
            jobs["deploy-production-control-plane"]["needs"],
            ["deploy-mainnet", "deploy-production-prover"],
        )
        self.assertEqual(
            jobs["mainnet-canaries"]["needs"],
            ["deploy-mainnet", "deploy-production-control-plane"],
        )
        self.assertEqual(
            jobs["activate-public-beta"]["needs"],
            ["deploy-mainnet", "deploy-production-control-plane", "mainnet-canaries"],
        )

    def test_render_recovery_uses_current_controller_with_frozen_release(self):
        for job_name, expected_calls in (
            ("deploy-production-control-plane", 2),
            ("activate-public-beta", 1),
        ):
            job = self.workflow["jobs"][job_name]
            checkouts = [
                step
                for step in job["steps"]
                if str(step.get("uses", "")).startswith("actions/checkout@")
            ]
            self.assertEqual(checkouts[0]["with"]["ref"], "${{ env.RELEASE_SOURCE_COMMIT }}")
            self.assertEqual(checkouts[1]["with"]["ref"], "${{ github.sha }}")
            self.assertEqual(checkouts[1]["with"]["path"], ".release-control")
            run = "\n".join(str(step.get("run", "")) for step in job["steps"])
            self.assertEqual(
                run.count(
                    "python .release-control/scripts/configure_open_competition_v2_beta3_render.py"
                ),
                expected_calls,
            )

    def test_runtime_readiness_uses_render_origin_and_current_controller(self):
        self.assertEqual(
            self.workflow["env"]["OPEN_COMPETITION_V2_RUNTIME_READINESS_URL"],
            "https://agent-bounties-api.onrender.com/v1/base/open-competition-v2-beta3/release",
        )
        self.assertEqual(
            self.text.count(
                "python .release-control/scripts/wait_open_competition_v2_beta3_runtime.py"
            ),
            3,
        )
        self.assertEqual(
            self.text.count('--url "$OPEN_COMPETITION_V2_RUNTIME_READINESS_URL"'),
            3,
        )
        self.assertEqual(
            self.text.count(
                "python .release-control/scripts/promote_open_competition_v2_beta3_release.py"
            ),
            2,
        )
        self.assertNotIn(
            "python scripts/promote_open_competition_v2_beta3_release.py",
            self.text,
        )

    def test_canonical_interfaces_are_proved_on_the_self_hosted_canary_runner(self):
        canaries = self.text.split("  mainnet-canaries:", 1)[1].split(
            "  activate-public-beta:", 1
        )[0]
        activation = self.text.split("  activate-public-beta:", 1)[1]
        self.assertIn(
            "python scripts/verify_open_competition_v2_beta3_interfaces.py",
            canaries,
        )
        self.assertIn("target/mainnet-canonical-interfaces.json", canaries)
        self.assertIn(
            "cp target/mainnet-canaries/mainnet-canonical-interfaces.json target/production-interfaces.json",
            activation,
        )
        self.assertNotIn(
            "python scripts/verify_open_competition_v2_beta3_interfaces.py",
            activation,
        )

    def test_only_exact_completed_mainnet_artifacts_can_continue(self):
        triggers = self.workflow.get("on", self.workflow.get(True))
        self.assertEqual(
            set(triggers["workflow_dispatch"]["inputs"].keys()),
            {"release_run_id", "preserved_canary_proof_run_id"},
        )
        self.assertEqual(self.text.count("run-id: ${{ inputs.release_run_id }}"), 2)
        self.assertIn(
            "open-competition-v2-beta3-mainnet-deployment-${{ env.RELEASE_SOURCE_COMMIT }}",
            self.text,
        )
        self.assertIn(
            "open-competition-v2-beta3-release-assets-${{ env.RELEASE_SOURCE_COMMIT }}",
            self.text,
        )
        self.assertIn(".preflight_safe_block.observed_deployer_nonce == 38", self.text)
        self.assertIn(".preflight_safe_block.resuming_exact_verifiers == true", self.text)
        self.assertIn(".transaction.transaction_hash == null", self.text)
        self.assertIn(".transaction.recovered_exact_deployment == true", self.text)
        self.assertIn("bounded-open-competition-v2-wallet-deployment-evidence.json", self.text)
        self.assertIn("sha256sum --check --strict", self.text)
        self.assertIn(
            "f10ad7c23fe73be6d428b061ba3a1f36281d0cdaa2dc85fa402c5c0d1c9c3aa9",
            self.text,
        )
        self.assertNotIn("BASE_MAINNET_DEPLOYER_PRIVATE_KEY", self.text.split(
            "  deploy-production-prover:", 1
        )[0])

    def test_exact_preserved_proofs_can_resume_only_with_pristine_actors(self):
        self.assertIn(
            "open-competition-v2-beta3-mainnet-canary-proofs-32623224167-1",
            self.text,
        )
        self.assertIn(
            'BETA3_ACTOR_DERIVATION_SALT="$PRESERVED_CANARY_SOURCE_RUN_ID:$PRESERVED_CANARY_SOURCE_RUN_ATTEMPT:mainnet"',
            self.text,
        )
        self.assertIn("PRESERVED_PLONK_BEST_A_PROOF_HASH", self.text)
        self.assertIn("PRESERVED_PLONK_BEST_B_PROOF_HASH", self.text)
        self.assertIn("--require-pristine-derived-actors", self.text)
        self.assertGreaterEqual(self.text.count('--shadow-rpc-url "$BASE_MAINNET_SHADOW_RPC_URL"'), 2)

    def test_prover_installs_all_three_reviewed_profiles(self):
        prover = self.text.split("  deploy-production-prover:", 1)[1].split(
            "  deploy-production-control-plane:", 1
        )[0]
        for profile in (
            "public-vector-metric-v1",
            "structured-artifact-metric-v1",
            "forward-canonical-gmv-attribution-metric-v2",
        ):
            self.assertIn(f"{profile}-script", prover)
        self.assertIn(
            '"profiles": ["public-vector-metric-v1", "structured-artifact-metric-v1", "forward-canonical-gmv-attribution-metric-v2"]',
            prover,
        )

    def test_primary_release_uses_python3_on_the_self_hosted_prover(self):
        release = RELEASE_WORKFLOW.read_text(encoding="utf-8")
        prover = release.split("  deploy-production-prover:", 1)[1].split(
            "  deploy-production-control-plane:", 1
        )[0]
        self.assertIn("python3 - <<'PY'", prover)
        self.assertNotIn("python - <<'PY'", prover)

    def test_prover_installs_only_the_verified_local_gnark_alias(self):
        self.assertIn('docker tag "$SP1_GNARK_IMAGE" "$SP1_GNARK_RUNTIME_IMAGE"', self.text)
        self.assertIn("expected_gnark_image_id", self.text)
        self.assertIn(
            ".release-control/scripts/install_open_competition_v2_prover_assets.py",
            self.text,
        )
        self.assertIn(
            "SP1_GROTH16_CIRCUIT_PATH=/var/lib/agent-bounties-prover/circuits/groth16",
            self.text,
        )
        self.assertIn(
            "SP1_PLONK_CIRCUIT_PATH=/var/lib/agent-bounties-prover/circuits/plonk",
            self.text,
        )
        self.assertNotIn(
            "SP1_GROTH16_CIRCUIT_PATH=$OPEN_COMPETITION_V2_TRUSTED_SETUP_ROOT/groth16",
            self.text,
        )
        self.assertIn("expected_gnark_cli", self.text)
        self.assertNotIn('docker pull "$SP1_GNARK_RUNTIME_IMAGE"', self.text)

    def test_canaries_use_current_control_binaries_with_frozen_release_assets(self):
        self.assertIn("ref: ${{ github.sha }}", self.text)
        self.assertIn("path: target/continuation-control", self.text)
        self.assertIn("--manifest-path target/continuation-control/Cargo.toml", self.text)
        self.assertIn("target/continuation-build/release/api", self.text)
        self.assertIn("--worker-binary target/continuation-build/release/worker", self.text)
        self.assertEqual(
            self.text.count(
                "target/continuation-control/scripts/run_open_competition_v2_x402_rehearsal.py"
            ),
            2,
        )
        self.assertEqual(
            self.text.count('sudo rm -rf "$GITHUB_WORKSPACE/target"'),
            2,
        )

    def test_canaries_and_activation_remain_mandatory(self):
        for evidence in (
            "mainnet-x402-success.json",
            "mainnet-x402-refund.json",
            "mainnet-accounting.json",
            "mainnet-fresh-agent-flow.json",
            "mainnet_plonk_canary_complete",
            "mainnet_groth16_canary_complete",
            "owner_public_beta_activation_approved",
        ):
            self.assertIn(evidence, self.text)
        self.assertIn(
            ".public_creation_enabled == true and .proof_broker_enabled == true",
            self.text,
        )


if __name__ == "__main__":
    unittest.main()
