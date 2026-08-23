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
        self.assertIn('RELEASE_SOURCE_COMMIT: 5a351f3e373691be58a9575b4374812b494b6086', text)
        self.assertIn('SOURCE_RUN_ID: "32629535171"', text)
        self.assertIn('SOURCE_ACTOR_DERIVATION_SALT: "32623224167:1:mainnet"', text)
        self.assertIn("open-competition-v2-beta3-mainnet-deployment-continuation", text)
        self.assertIn("open-competition-v2-beta3-production-control-plane-continuation", text)
        self.assertIn("CANARY_TEMPLATE_JOB_ID", text)
        self.assertIn("CANARY_SUCCESS_JOB_ID", text)
        self.assertNotIn("RESUME_X402_JOB_ID", text)
        self.assertIn('--proof-job-id "$CANARY_SUCCESS_JOB_ID"', text)
        self.assertIn("else str(time.time_ns())", text)
        self.assertIn('str(old["solver_nonce"])', text)
        self.assertIn('"solver_nonce": recovery_nonce', text)
        self.assertIn('sleep "$((proof_deadline - now + 2))"', text)
        self.assertIn("--replacement-id 2", text)
        self.assertIn('[[ -z "$CANARY_SUCCESS_JOB_ID" ]]', text)
        self.assertNotRegex(text, r'"solver_nonce": "\d+"')
        self.assertNotIn("createCompetition(", text)
        self.assertNotIn("approve(address", text)
        self.assertIn('CANARY_SUCCESS_JOB_ID: eed7303a-892a-41e1-81e5-c6c9c37237bd', text)
        self.assertIn(
            'PLONK_SETTLEMENT_TRANSACTION: "0xc77a2689894c849c215885f63d6e09c1d5432d665d730f2021312994501ce259"',
            text,
        )
        self.assertIn(
            'FORCED_REFUND_PAYMENT_TRANSACTION: "0x1f35e1eb06f921d7735c29ce3ab01164d46019e60756724d5016a6aad92dec11"',
            text,
        )
        self.assertIn(
            'FORCED_REFUND_TRANSACTION: "0x2fee2a3cd0e546bf70e54d562f22fbbb810a61f2268b7aa9927b8fda14be1cac"',
            text,
        )
        self.assertIn('--shadow-rpc-url "$BASE_MAINNET_SHADOW_RPC_URL"', text)

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
