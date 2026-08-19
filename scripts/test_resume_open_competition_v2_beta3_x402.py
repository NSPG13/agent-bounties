from pathlib import Path
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/resume-open-competition-v2-beta3-x402.yml"
SOURCE_COMMIT = "4d09d82825c38f2bf93a8ee4375a95b302410c29"


class ResumeBeta3X402WorkflowTests(unittest.TestCase):
    def test_resume_is_bounded_to_the_preserved_canary(self) -> None:
        text = WORKFLOW.read_text(encoding="utf-8")
        workflow = yaml.safe_load(text)
        job = workflow["jobs"]["resume-sepolia-x402"]

        self.assertEqual(workflow["env"]["RELEASE_SOURCE_COMMIT"], SOURCE_COMMIT)
        self.assertEqual(
            workflow["env"]["SOURCE_ACTOR_DERIVATION_SALT"],
            "32147289466:15:sepolia",
        )
        self.assertEqual(
            workflow["env"]["EXPECTED_SEPOLIA_DEPLOYER"],
            "0xfd7be4c69541ab297aece2a674fc1418b898cc0a",
        )
        self.assertEqual(job["environment"], "v2-beta2-sepolia")
        self.assertIn("workflow_dispatch", workflow[True])
        dispatch = workflow[True]["workflow_dispatch"]
        self.assertEqual(dispatch["inputs"]["mode"]["default"], "recover-second-only")
        self.assertEqual(
            dispatch["inputs"]["mode"]["options"],
            ["recover-second-only", "finalize-success", "resume-rehearsal"],
        )
        self.assertNotIn("push", workflow[True])
        self.assertNotIn("schedule", workflow[True])
        self.assertIn("/mnt/agent-bounties-artifacts/beta3-attempt15", text)
        self.assertIn("mkdir -p target", text)
        self.assertIn("run_open_competition_v2_x402_rehearsal.py", text)
        self.assertIn("OPEN_COMPETITION_V2_INDEXER_MAX_BLOCKS_PER_QUERY=10000", text)
        checkouts = [
            step
            for step in job["steps"]
            if str(step.get("uses", "")).startswith("actions/checkout@")
        ]
        self.assertEqual(len(checkouts), 2)
        self.assertEqual(checkouts[0]["with"]["ref"], "${{ env.RELEASE_SOURCE_COMMIT }}")
        self.assertFalse(checkouts[0]["with"]["clean"])
        self.assertEqual(checkouts[1]["with"]["ref"], "${{ github.sha }}")
        self.assertEqual(checkouts[1]["with"]["path"], "target/continuation-control")
        self.assertIn("sudo rm -rf target", text)
        self.assertIn(
            "target/continuation-control/scripts/recover_open_competition_v2_x402_charge.py",
            text,
        )
        self.assertIn(
            "target/continuation-control/scripts/refresh_open_competition_v2_x402_canary.py",
            text,
        )
        self.assertIn("target/x402-canary-replacement.json", text)
        self.assertIn(".x402_canary.replacement_id == 1", text)
        self.assertIn(".recovery.recovered == true", text)
        self.assertIn("--fixture programs/public-vector-metric-v1/fixtures/rehearsal-best-score-a.json", text)
        self.assertIn("OPEN_COMPETITION_V2_PROVER_TIMEOUT_SECONDS=1200", text)
        self.assertIn("OPEN_COMPETITION_V2_BROKER_LEASE_SECONDS=1230", text)
        self.assertIn("target/failed-x402-charge-refund.json", text)
        self.assertIn(
            "0xba73504377041ca89b5262421e7c994a40e7c955c5f71f9dc95f16d2c966d312",
            text,
        )
        self.assertIn("target/failed-x402-charge-refund-2.json", text)
        self.assertIn(
            "0x53fdaf15f234cf1ab4267bde5ce602221b8ad4e81ca011f457ab365a899e1e56",
            text,
        )
        self.assertIn("target/failed-x402-charge-refund-3.json", text)
        self.assertIn(
            "0xedf4427c273df26905f3a5fe377d17bab4e2f9c8485f38f498652379ff4b622a",
            text,
        )
        self.assertIn("target/failed-x402-charge-refund-4.json", text)
        self.assertIn(
            "0x8b0b85cdd06147ae1e37fdbd4e8ea78876bb312bd234dcd49aadfa25e0b89c27",
            text,
        )
        self.assertIn("target/failed-x402-charge-refund-5.json", text)
        self.assertIn('if [[ "$RECOVERY_ONLY" == "true" ]]', text)
        self.assertIn('if [[ "$FINALIZE_SUCCESS" == "true" ]]', text)
        self.assertIn("sepolia-x402-rehearsal-success.json", text)
        self.assertIn("x402-canary-replacement-success.json", text)
        self.assertIn("chmod u+w target/release-gates.json", text)
        self.assertNotIn(
            "target/continuation-control/scripts/record_open_competition_v2_beta3_gate.py",
            text,
        )
        self.assertIn("python scripts/record_open_competition_v2_beta3_gate.py", text)
        self.assertIn("if: inputs.mode != 'recover-second-only'", text)
        self.assertIn("open-competition-v2-beta3-x402-charge-recovery-5", text)
        self.assertIn('docker tag "$SP1_GNARK_IMAGE" "$SP1_GNARK_RUNTIME_IMAGE"', text)
        self.assertIn("expected_gnark_image_id", text)
        self.assertIn("expected_gnark_cli", text)
        self.assertIn("--manifest-path target/continuation-control/Cargo.toml", text)
        self.assertIn("--release -p api -p worker", text)
        self.assertIn("target/continuation-build/release/api", text)
        self.assertIn("--worker-binary target/continuation-build/release/worker", text)
        self.assertIn(
            "target/continuation-control/scripts/run_open_competition_v2_x402_rehearsal.py",
            text,
        )
        self.assertIn("--source-commit \"$RELEASE_SOURCE_COMMIT\"", text)
        self.assertNotIn("jq -r .deployer", text)
        self.assertNotIn("deploy_open_competition_v2_beta3.py", text)
        self.assertNotIn("fund_open_competition_v2_beta3_broker.py", text)
        self.assertNotIn("python scripts/run_open_competition_v2_sepolia_rehearsal.py", text)


if __name__ == "__main__":
    unittest.main()
