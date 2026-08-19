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
        self.assertEqual(job["environment"], "v2-beta2-sepolia")
        self.assertIn("workflow_dispatch", workflow[True])
        self.assertNotIn("push", workflow[True])
        self.assertNotIn("schedule", workflow[True])
        self.assertIn("/mnt/agent-bounties-artifacts/beta3-attempt15", text)
        self.assertIn("mkdir -p target", text)
        self.assertIn("run_open_competition_v2_x402_rehearsal.py", text)
        self.assertIn("--source-commit \"$RELEASE_SOURCE_COMMIT\"", text)
        self.assertNotIn("deploy_open_competition_v2_beta3.py", text)
        self.assertNotIn("fund_open_competition_v2_beta3_broker.py", text)
        self.assertNotIn("run_open_competition_v2_sepolia_rehearsal.py", text)


if __name__ == "__main__":
    unittest.main()
