from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/resume-open-competition-v2-beta3-mainnet.yml"


class MainnetResumeWorkflowTests(unittest.TestCase):
    def test_github_yaml_parser_keeps_hex_constants_as_strings(self):
        text = WORKFLOW.read_text(encoding="utf-8")
        for key in (
            "PLONK_COMPETITION",
            "PLONK_SETTLEMENT_TRANSACTION",
            "FORCED_REFUND_PAYMENT_TRANSACTION",
            "FORCED_REFUND_TRANSACTION",
            "BASE_USDC",
        ):
            self.assertRegex(text, rf'(?m)^  {re.escape(key)}: "0x[0-9a-f]+"$')

    def test_resume_is_protected_and_replaces_the_terminal_canary(self):
        text = WORKFLOW.read_text(encoding="utf-8")
        self.assertEqual(text.count("environment: v2-beta2-mainnet"), 2)
        self.assertIn('SOURCE_RUN_ID: "32346174505"', text)
        self.assertIn("open-competition-v2-beta3-mainnet-deployment-continuation", text)
        self.assertIn("open-competition-v2-beta3-production-control-plane-continuation", text)
        self.assertIn("CANARY_TEMPLATE_JOB_ID", text)
        self.assertNotIn("RESUME_X402_JOB_ID", text)
        self.assertNotIn("--proof-job-id", text)
        self.assertIn("recovery_nonce = str(time.time_ns())", text)
        self.assertIn('"solver_nonce": recovery_nonce', text)
        self.assertIn('sleep "$((proof_deadline - now + 2))"', text)
        self.assertIn("--replacement-id 2", text)
        self.assertNotRegex(text, r'"solver_nonce": "\d+"')
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
