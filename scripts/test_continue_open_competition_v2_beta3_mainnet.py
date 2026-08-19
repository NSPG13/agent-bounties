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

    def test_only_successful_frozen_sepolia_evidence_can_continue(self):
        self.assertIn("run-id: ${{ inputs.sepolia_run_id }}", self.text)
        self.assertIn("open-competition-v2-beta3-live-sepolia-resumed", self.text)
        self.assertIn("failed-x402-charge-refund.json", self.text)
        self.assertIn("failed-x402-charge-refund-2.json", self.text)
        self.assertIn("x402-canary-replacement.json", self.text)
        self.assertIn(".minimum_broker_sla_seconds == 1800", self.text)
        self.assertIn(".superseded_recovery.recovered == true", self.text)
        self.assertIn(
            "0xba73504377041ca89b5262421e7c994a40e7c955c5f71f9dc95f16d2c966d312",
            self.text,
        )
        self.assertIn(".settlement_event_id | length > 0", self.text)
        self.assertIn(".source_commit == $commit", self.text)
        self.assertNotIn('--source-commit "$GITHUB_SHA"', self.text)

    def test_prover_installs_only_the_verified_local_gnark_alias(self):
        self.assertIn('docker tag "$SP1_GNARK_IMAGE" "$SP1_GNARK_RUNTIME_IMAGE"', self.text)
        self.assertIn("expected_gnark_image_id", self.text)
        self.assertIn("expected_gnark_cli", self.text)
        self.assertNotIn('docker pull "$SP1_GNARK_RUNTIME_IMAGE"', self.text)

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
