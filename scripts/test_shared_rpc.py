#!/usr/bin/env python3
"""Characterization tests for the shared JSON-RPC transport."""

from __future__ import annotations

import io
import unittest
from unittest.mock import patch
from urllib.error import HTTPError, URLError

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
            rpc("http://localhost", "eth_call", [], retry_delay=0)

    def test_retries_transient_timeout_then_returns_result(self) -> None:
        responses = [TimeoutError("slow"), io.BytesIO(b'{"result":"0x14a34"}')]
        with patch("_shared.rpc.urlopen", side_effect=responses) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept:
            self.assertEqual(
                rpc("http://localhost", "eth_chainId", [], retry_delay=0.25),
                "0x14a34",
            )
        self.assertEqual(opened.call_count, 2)
        slept.assert_called_once_with(0.25)

    def test_nonretryable_http_error_fails_immediately(self) -> None:
        error = HTTPError("http://localhost", 401, "unauthorized", {}, None)
        with patch("_shared.rpc.urlopen", side_effect=error) as opened, self.assertRaisesRegex(
            RuntimeError, "^RPC transport failed for eth_call:"
        ):
            rpc("http://localhost", "eth_call", [], retry_delay=0)
        self.assertEqual(opened.call_count, 1)

    def test_rejects_invalid_retry_policy(self) -> None:
        with self.assertRaisesRegex(ValueError, "attempts must be positive"):
            rpc("http://localhost", "eth_call", [], attempts=0)


if __name__ == "__main__":
    unittest.main()
