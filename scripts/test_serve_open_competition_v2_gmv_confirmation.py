#!/usr/bin/env python3
"""Tests for private handling of the owner funding authorization."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from scripts.serve_open_competition_v2_gmv_confirmation import HTML, store_verified_signature


class GmvConfirmationTests(unittest.TestCase):
    def test_wallet_chain_query_fallback_remains_narrow_and_base_bound(self) -> None:
        self.assertIn("code === -32601 || code === 4200", HTML)
        self.assertIn("message.includes('method is not supported')", HTML)
        self.assertIn(
            "message.includes('request method') && message.includes('is not supported')",
            HTML,
        )
        self.assertIn("if (!methodUnavailable) throw error", HTML)
        self.assertIn("String(chain).toLowerCase() !== '0x2105'", HTML)
        self.assertIn("method: 'eth_signTypedData_v4'", HTML)

    def test_wallet_selector_prefers_metamask_without_mislabeling_other_wallets(self) -> None:
        self.assertIn("eip6963:announceProvider", HTML)
        self.assertIn("eip6963:requestProvider", HTML)
        self.assertIn("!isCoinbase && !isBrave", HTML)
        self.assertIn("rdns === 'io.metamask'", HTML)
        self.assertIn("option.textContent", HTML)
        self.assertIn("const provider = selection.provider", HTML)
        self.assertNotIn("const provider = window.ethereum;", HTML)

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
