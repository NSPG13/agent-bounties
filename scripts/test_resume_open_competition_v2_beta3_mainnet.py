from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/resume-open-competition-v2-beta3-mainnet.yml"


class MainnetResumeWorkflowTests(unittest.TestCase):
    def test_resume_is_protected_and_reuses_canonical_funded_competition(self):
        text = WORKFLOW.read_text(encoding="utf-8")
        self.assertEqual(text.count("environment: v2-beta2-mainnet"), 2)
        self.assertIn('SOURCE_RUN_ID: "32346174505"', text)
        self.assertIn("open-competition-v2-beta3-mainnet-deployment-continuation", text)
        self.assertIn("open-competition-v2-beta3-production-control-plane-continuation", text)
        self.assertIn("EXISTING_X402_JOB_ID", text)
        self.assertIn('"solver_nonce": "4"', text)
        self.assertNotIn("createCompetition(", text)
        self.assertNotIn("approve(address", text)

    def test_resume_deploys_retry_fix_before_spending_and_activates_exact_release(self):
        text = WORKFLOW.read_text(encoding="utf-8")
        deploy = text.index("deploy-retryable-payment-api:")
        resume = text.index("resume-canary-and-activate:")
        pay = text.index("run_open_competition_v2_x402_rehearsal.py")
        self.assertLess(deploy, resume)
        self.assertLess(resume, pay)
        self.assertIn("needs: deploy-retryable-payment-api", text)
        self.assertIn("verify_open_competition_v2_beta3_mainnet_recovery.py", text)
        self.assertIn("mainnet_canary_accounting_reconciled", text)
        self.assertIn("owner_public_beta_activation_approved", text)
        self.assertIn("--expect-public true", text)


if __name__ == "__main__":
    unittest.main()
