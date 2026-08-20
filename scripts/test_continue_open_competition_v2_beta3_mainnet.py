from pathlib import Path
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/continue-open-competition-v2-beta3-mainnet.yml"
RELEASE_SOURCE_COMMIT = "4d09d82825c38f2bf93a8ee4375a95b302410c29"


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

    def test_only_successful_frozen_sepolia_evidence_can_continue(self):
        self.assertIn("run-id: ${{ inputs.sepolia_run_id }}", self.text)
        self.assertIn("open-competition-v2-beta3-live-sepolia-resumed", self.text)
        self.assertIn("failed-x402-charge-refund.json", self.text)
        self.assertIn("failed-x402-charge-refund-2.json", self.text)
        self.assertIn("failed-x402-charge-refund-3.json", self.text)
        self.assertIn("failed-x402-charge-refund-4.json", self.text)
        self.assertIn("failed-x402-charge-refund-5.json", self.text)
        self.assertIn("x402-canary-replacement.json", self.text)
        self.assertIn(".minimum_broker_sla_seconds == 1800", self.text)
        self.assertIn(".superseded_recovery.recovered == true", self.text)
        self.assertIn(
            "0xba73504377041ca89b5262421e7c994a40e7c955c5f71f9dc95f16d2c966d312",
            self.text,
        )
        self.assertIn(
            "0x53fdaf15f234cf1ab4267bde5ce602221b8ad4e81ca011f457ab365a899e1e56",
            self.text,
        )
        self.assertIn(
            "0xedf4427c273df26905f3a5fe377d17bab4e2f9c8485f38f498652379ff4b622a",
            self.text,
        )
        self.assertIn(
            "0x8b0b85cdd06147ae1e37fdbd4e8ea78876bb312bd234dcd49aadfa25e0b89c27",
            self.text,
        )
        self.assertIn(".settlement_event_id | length > 0", self.text)
        self.assertIn(
            'test "$replacement" = "$(jq -r .competition target/live-sepolia/sepolia-x402-rehearsal.json)"',
            self.text,
        )
        self.assertNotIn(
            'test "$replacement" = "$(jq -r .x402_canary.competition',
            self.text,
        )
        self.assertIn(".source_commit == $commit", self.text)
        self.assertNotIn('--source-commit "$GITHUB_SHA"', self.text)

    def test_prover_installs_only_the_verified_local_gnark_alias(self):
        self.assertIn('docker tag "$SP1_GNARK_IMAGE" "$SP1_GNARK_RUNTIME_IMAGE"', self.text)
        self.assertIn("expected_gnark_image_id", self.text)
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
