from pathlib import Path
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/preserve-open-competition-v2-beta3-mainnet-canary-proofs.yml"


class ProofPreservationWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = WORKFLOW.read_text(encoding="utf-8")
        cls.workflow = yaml.safe_load(cls.text)

    def test_preservation_cannot_sign_or_move_funds(self):
        self.assertNotIn("actions/checkout", self.text)
        self.assertNotIn("PRIVATE_KEY", self.text)
        self.assertNotIn("eth_sendRawTransaction", self.text)
        self.assertNotIn("cast send", self.text)
        self.assertIn('"money_moved_by_preservation": False', self.text)

    def test_exact_failed_run_and_proof_hashes_are_pinned_as_strings(self):
        env = self.workflow["env"]
        self.assertEqual(env["SOURCE_RUN_ID"], "32623224167")
        self.assertEqual(env["SOURCE_RUN_ATTEMPT"], "1")
        for name in (
            "PROOF_CONTEXT_HASH",
            "PLONK_BEST_A_PROOF_HASH",
            "PLONK_BEST_A_JOURNAL_HASH",
            "PLONK_BEST_B_PROOF_HASH",
            "PLONK_BEST_B_JOURNAL_HASH",
        ):
            self.assertIsInstance(env[name], str)
            self.assertTrue(env[name].startswith("0x"))
        self.assertIn(".proof_hash == $proof and .journal_hash == $journal", self.text)

    def test_symlinks_are_rejected_and_every_file_is_hashed(self):
        self.assertIn("Stage the failed-run proof workspace before validation", self.text)
        self.assertIn('cp -aL "$source/mainnet-proofs" "$stage/"', self.text)
        self.assertIn("-type l -print -quit", self.text)
        self.assertIn("find . -type f ! -name files.sha256 -print0", self.text)
        self.assertIn("xargs -0 sha256sum", self.text)
        self.assertIn("if: always()", self.text)
        self.assertIn("if-no-files-found: error", self.text)


if __name__ == "__main__":
    unittest.main()
