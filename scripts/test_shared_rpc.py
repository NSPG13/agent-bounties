#!/usr/bin/env python3
"""Characterization tests for the shared JSON-RPC transport."""

from __future__ import annotations

import io
import unittest
from unittest.mock import patch
from urllib.error import URLError

from _shared.rpc import rpc


class RpcTest(unittest.TestCase):
    def test_result_and_rpc_error_contracts(self) -> None:
        for body, expected, message in (
            (b'{"result":"0x2105"}', "0x2105", None),
            (b'{"error":{"code":-1,"message":"bad"}}', None, 'RPC eth_chainId failed: {"code": -1, "message": "bad"}'),
            (b'{}', None, None),
        ):
            with self.subTest(body=body), patch("_shared.rpc.urlopen", return_value=io.BytesIO(body)):
                if message:
                    with self.assertRaises(RuntimeError) as raised:
                        rpc("http://localhost", "eth_chainId", [], 7)
                    self.assertEqual(str(raised.exception), message)
                else:
                    self.assertEqual(rpc("http://localhost", "eth_chainId", [], 7), expected)

    def test_transport_error_contract(self) -> None:
        with patch("_shared.rpc.urlopen", side_effect=URLError("offline")), self.assertRaisesRegex(
            RuntimeError, "^RPC transport failed for eth_call:"
        ):
            rpc("http://localhost", "eth_call", [])


if __name__ == "__main__":
    unittest.main()
