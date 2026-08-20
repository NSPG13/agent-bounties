import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import diagnose_open_competition_v2_beta3_prover as diagnostic


class ProverDiagnosticTests(unittest.TestCase):
    def test_redact_removes_secret_and_url_credentials(self) -> None:
        with patch.dict(os.environ, {"EXAMPLE_SECRET": "do-not-print"}, clear=False):
            value = diagnostic.redact("do-not-print https://user:password@example.test/path")
        self.assertNotIn("do-not-print", value)
        self.assertNotIn("password", value)
        self.assertIn("[redacted]", value)

    def test_load_record_matches_exact_provider_id(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            expected = {"provider_job_id": "beta3-" + "a" * 64}
            (root / "job.json").write_text(json.dumps(expected), encoding="utf-8")
            path, record = diagnostic.load_record(root, expected["provider_job_id"])
        self.assertEqual(path.name, "job.json")
        self.assertEqual(record, expected)


if __name__ == "__main__":
    unittest.main()
