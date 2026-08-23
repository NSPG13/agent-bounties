from pathlib import Path
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/recover-open-competition-v2-beta3-mainnet-deployment.yml"
SOURCE_COMMIT = "5a351f3e373691be58a9575b4374812b494b6086"
RELEASE_RUN_ID = "32606926043"


class MainnetDeploymentRecoveryWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.text = WORKFLOW.read_text(encoding="utf-8")
        cls.workflow = yaml.safe_load(cls.text)

    def test_recovery_is_manual_main_only_and_protected(self) -> None:
        self.assertIn("workflow_dispatch", self.workflow[True])
        self.assertNotIn("push", self.workflow[True])
        self.assertNotIn("schedule", self.workflow[True])
        job = self.workflow["jobs"]["recover-mainnet-deployment"]
        self.assertEqual(job["if"], "github.ref == 'refs/heads/main'")
        self.assertEqual(job["environment"], "v2-beta2-mainnet")
        self.assertEqual(self.workflow["permissions"], {"actions": "read", "contents": "read"})

    def test_exact_release_artifacts_and_addresses_are_pinned(self) -> None:
        environment = self.workflow["env"]
        self.assertEqual(environment["RELEASE_SOURCE_COMMIT"], SOURCE_COMMIT)
        self.assertEqual(str(environment["RELEASE_RUN_ID"]), RELEASE_RUN_ID)
        self.assertEqual(environment["EXPECTED_START_NONCE"], "34")
        self.assertEqual(environment["EXPECTED_OBSERVED_NONCE"], "35")
        self.assertEqual(
            environment["EXPECTED_GROTH16_VERIFIER"],
            "0x6788e13954e7e27f8d2c62ab8ce86b96b8d9169f",
        )
        self.assertEqual(
            environment["EXPECTED_PLONK_VERIFIER"],
            "0xa2549e89b7d56a99ddabcbece342a211e5ef340a",
        )
        self.assertEqual(
            environment["EXPECTED_FACTORY"],
            "0x29d0e39e0c03797c690633535722e6b34a69a78a",
        )
        self.assertIn("run-id: ${{ env.RELEASE_RUN_ID }}", self.text)
        self.assertIn(
            "open-competition-v2-beta3-release-assets-${{ env.RELEASE_SOURCE_COMMIT }}",
            self.text,
        )
        self.assertIn(
            "open-competition-v2-beta3-live-sepolia-${{ env.RELEASE_SOURCE_COMMIT }}",
            self.text,
        )

    def test_frozen_source_and_current_recovery_controls_are_separate(self) -> None:
        job = self.workflow["jobs"]["recover-mainnet-deployment"]
        checkouts = [
            step
            for step in job["steps"]
            if str(step.get("uses", "")).startswith("actions/checkout@")
        ]
        self.assertEqual(checkouts[0]["with"]["ref"], "${{ env.RELEASE_SOURCE_COMMIT }}")
        self.assertEqual(checkouts[1]["with"]["ref"], "${{ github.sha }}")
        self.assertEqual(checkouts[1]["with"]["path"], ".release-control")
        self.assertIn("HEAD:contracts/base-escrow", self.text)
        self.assertIn("--release-root \"$GITHUB_WORKSPACE\"", self.text)

    def test_recovery_reuses_exact_prefix_and_dual_rpc_submission(self) -> None:
        self.assertIn(".preflight_safe_block.resuming_exact_verifiers == true", self.text)
        self.assertIn(".transactions[0].recovered_exact_deployment == true", self.text)
        self.assertEqual(self.text.count('--shadow-rpc-url "$BASE_MAINNET_SHADOW_RPC_URL"'), 2)
        self.assertIn("deploy_open_competition_v2_beta3.py", self.text)
        self.assertIn("deploy_bounded_open_competition_v2_wallet_factory.py", self.text)
        self.assertNotIn("fund_open_competition_v2_beta3_broker.py", self.text)

    def test_recovery_does_not_authorize_owner_funds(self) -> None:
        self.assertIn("does not authorize owner USDC or bounty settlement", self.text)
        self.assertNotIn("OWNER_PRIVATE_KEY", self.text)
        self.assertNotIn("77.668098", self.text)


if __name__ == "__main__":
    unittest.main()
