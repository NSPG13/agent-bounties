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
        self.assertEqual(
            env["PLONK_BEST_A_FILE_SHA256"],
            "deab624396414fb95e958ed01d91745cb12b9bda2c85ffdd9d25763316167537",
        )
        self.assertEqual(
            env["PLONK_BEST_B_FILE_SHA256"],
            "e278f6d3d17b8c8e385accb3f2c5b9e7d967be3485b36c537a91cb1f60964191",
        )
        self.assertIn("sha256sum --check --strict", self.text)
        self.assertIn('.self_verified == true', self.text)

    def test_symlinks_are_rejected_and_every_file_is_hashed(self):
        self.assertIn("Download the isolated quarantine artifact", self.text)
        self.assertIn("run-id: ${{ inputs.quarantine_run_id }}", self.text)
        self.assertIn("-type l -print -quit", self.text)
        self.assertIn("find . -type f ! -name files.sha256 -print0", self.text)
        self.assertIn("xargs -0 sha256sum", self.text)
        self.assertIn("if: always()", self.text)
        self.assertIn("if-no-files-found: error", self.text)


if __name__ == "__main__":
    unittest.main()
