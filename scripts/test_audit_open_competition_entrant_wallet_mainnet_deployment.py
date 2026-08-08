#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("audit_open_competition_entrant_wallet_mainnet_deployment.py")
SPEC = importlib.util.spec_from_file_location("entrant_wallet_mainnet_deployment_audit", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class DeploymentAuditHexValidationTests(unittest.TestCase):
    def test_accepts_exact_32_byte_transaction_hash(self) -> None:
        self.assertTrue(MODULE.is_hex_bytes("0x" + "ab" * 32, 32))

    def test_rejects_wrong_length_or_non_hex_transaction_hash(self) -> None:
        self.assertFalse(MODULE.is_hex_bytes("0x" + "ab" * 31, 32))
        self.assertFalse(MODULE.is_hex_bytes("0x" + "zz" * 32, 32))
        self.assertFalse(MODULE.is_hex_bytes(None, 32))


if __name__ == "__main__":
    unittest.main()
