#!/usr/bin/env python3
"""Tests for private handling of the owner funding authorization."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from scripts.serve_open_competition_v2_gmv_confirmation import store_verified_signature


class GmvConfirmationTests(unittest.TestCase):
    def test_signature_is_written_once_and_not_returned(self) -> None:
        signature = "0x" + "11" * 65
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "owner-authorization.json"
            digest = store_verified_signature(output, "0x" + "22" * 20, signature)
            self.assertRegex(digest, r"^0x[0-9a-f]{64}$")
            stored = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(stored["signature"], signature)
            self.assertNotIn(signature, digest)
            with self.assertRaises(FileExistsError):
                store_verified_signature(output, stored["owner"], signature)


if __name__ == "__main__":
    unittest.main()
