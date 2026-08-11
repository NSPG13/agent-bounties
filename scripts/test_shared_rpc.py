#!/usr/bin/env python3
"""Characterization and acceptance tests for the shared retry-safe RPC transport.

Run with ``python -m unittest scripts.test_shared_rpc -v`` from the repository
root, matching the immutable direct-inventory-v1/rpc-failover benchmark.
"""

from __future__ import annotations

import io
import unittest
from unittest.mock import patch
from urllib.error import HTTPError, URLError

from scripts._shared.rpc import (
    BASE_CHAIN_ID,
    BASE_RPC_ENDPOINTS,
    RpcError,
    rpc,
    rpc_failover,
)


def _result(body: str) -> io.BytesIO:
    return io.BytesIO(f'{{"jsonrpc":"2.0","id":1,"result":{body}}}'.encode("utf-8"))


class RpcTest(unittest.TestCase):
    def test_result_and_rpc_error_contracts(self) -> None:
        for body, expected, message in (
            (b'{"result":"0x2105"}', "0x2105", None),
            (
                b'{"error":{"code":-1,"message":"bad"}}',
                None,
                'RPC eth_chainId failed: {"code": -1, "message": "bad"}',
            ),
            (b"{}", None, None),
        ):
            with self.subTest(body=body), patch(
                "scripts._shared.rpc.urlopen", return_value=io.BytesIO(body)
            ):
                if message:
                    with self.assertRaises(RuntimeError) as raised:
                        rpc("http://localhost", "eth_chainId", [], 7)
                    self.assertEqual(str(raised.exception), message)
                else:
                    self.assertEqual(
                        rpc("http://localhost", "eth_chainId", [], 7), expected
                    )

    def test_transport_error_contract(self) -> None:
        with patch(
            "scripts._shared.rpc.urlopen", side_effect=URLError("offline")
        ), self.assertRaisesRegex(RuntimeError, "^RPC transport failed for eth_call:"):
            rpc("http://localhost", "eth_call", [])


class RpcFailoverTest(unittest.TestCase):
    def test_https_endpoints_and_base_chain_constant(self) -> None:
        """The ordered endpoint list is HTTPS-only and pinned to chain 8453."""
        self.assertEqual(BASE_CHAIN_ID, 8453)
        self.assertTrue(BASE_RPC_ENDPOINTS)
        for endpoint in BASE_RPC_ENDPOINTS:
            self.assertTrue(endpoint.startswith("https://"))

    def test_wrong_chain_rejected_before_use(self) -> None:
        """Endpoints reporting a wrong chain are rejected and never used."""
        with patch(
            "scripts._shared.rpc.urlopen", return_value=_result('"0x1"')
        ), self.assertRaisesRegex(RuntimeError, "wrong chain"):
            rpc_failover(
                "eth_blockNumber", [], endpoints=("https://wrong.example",)
            )

    def test_rpc_error_not_retried(self) -> None:
        """Confirmed JSON-RPC execution errors (rpc error preservation) propagate
        immediately and are never retried or failed over."""
        error_body = io.BytesIO(
            b'{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"execution reverted"}}'
        )
        with patch(
            "scripts._shared.rpc.urlopen",
            side_effect=[_result('"0x2105"'), error_body],
        ) as mocked:
            with self.assertRaises(RpcError) as raised:
                rpc_failover(
                    "eth_call", [], endpoints=("https://a.example", "https://b.example")
                )
        self.assertIn("RPC eth_call failed", str(raised.exception))
        self.assertEqual(mocked.call_count, 2)

    def test_429_failover_to_next_endpoint(self) -> None:
        """HTTP 429 on one endpoint fails over to the next ordered endpoint."""
        too_many = HTTPError("https://a.example", 429, "Too Many Requests", None, None)
        with patch(
            "scripts._shared.rpc.urlopen",
            side_effect=[too_many, _result('"0x2105"'), _result('"0x1234"')],
        ) as mocked:
            result = rpc_failover(
                "eth_blockNumber",
                [],
                endpoints=("https://a.example", "https://b.example"),
            )
        self.assertEqual(result, "0x1234")
        self.assertEqual(mocked.call_count, 3)

    def test_5xx_retry_with_deterministic_backoff(self) -> None:
        """HTTP 500 is retried on the same endpoint with bounded backoff."""
        server_error = HTTPError(
            "https://a.example", 500, "Server Error", None, None
        )
        sleeps: list[float] = []
        with patch(
            "scripts._shared.rpc.urlopen",
            side_effect=[_result('"0x2105"'), server_error, _result('"0x99"')],
        ), patch(
            "scripts._shared.rpc.time.sleep", side_effect=lambda s: sleeps.append(s)
        ):
            result = rpc_failover(
                "eth_blockNumber", [], endpoints=("https://a.example",)
            )
        self.assertEqual(result, "0x99")
        self.assertEqual(sleeps, [1.0])

    def test_endpoint_exhaustion_raises(self) -> None:
        """All endpoints failing raises a bounded exhaustion error."""
        with patch(
            "scripts._shared.rpc.urlopen",
            side_effect=[_result('"0x2105"'), URLError("down"), URLError("down"), URLError("down")],
        ), self.assertRaisesRegex(RuntimeError, "exhausted"):
            rpc_failover("eth_blockNumber", [], endpoints=("https://a.example",))

    def test_non_https_endpoint_refused(self) -> None:
        """Plain-HTTP endpoints are refused before any request is made."""
        with patch(
            "scripts._shared.rpc.urlopen"
        ) as mocked, self.assertRaisesRegex(RuntimeError, "non-HTTPS"):
            rpc_failover(
                "eth_blockNumber", [], endpoints=("http://plain.example",)
            )
        mocked.assert_not_called()


if __name__ == "__main__":
    unittest.main()
