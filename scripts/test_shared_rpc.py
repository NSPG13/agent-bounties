#!/usr/bin/env python3
"""Characterization tests for retry-safe Base RPC failover transport."""

from __future__ import annotations

import io
import unittest
from unittest.mock import patch
from urllib.error import URLError

from _shared.rpc import (
    rpc,
    rpc_failover,
    _TransportError,
    _RpcError,
    _validate_chain,
    BASE_CHAIN_ID,
    BASE_RPC_ENDPOINTS,
)


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
                "_shared.rpc.urlopen", return_value=io.BytesIO(body)
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
            "_shared.rpc.urlopen", side_effect=URLError("offline")
        ), self.assertRaisesRegex(
            RuntimeError, "^RPC transport failed for eth_call:"
        ):
            rpc("http://localhost", "eth_call", [])

    # ── RPC failover tests ──

    def test_chain_validation_accepts_8453(self) -> None:
        """eth_chainId returns 0x2105 (8453) → validation passes."""
        with patch(
            "_shared.rpc.urlopen",
            return_value=io.BytesIO(b'{"result":"0x2105"}'),
        ):
            chain_id = _validate_chain("https://mainnet.base.org")
        self.assertEqual(chain_id, 8453)

    def test_chain_validation_rejects_wrong_chain(self) -> None:
        """eth_chainId returns 0x1 (Ethereum mainnet) → validation fails (wrong chain)."""
        with patch(
            "_shared.rpc.urlopen",
            return_value=io.BytesIO(b'{"result":"0x1"}'),
        ):
            chain_id = _validate_chain("https://wrong.chain")
        self.assertIsNone(chain_id)

    def test_chain_validation_transport_failure_returns_none(self) -> None:
        """Transport error during chain validation → skip endpoint."""
        with patch("_shared.rpc.urlopen", side_effect=URLError("offline")):
            chain_id = _validate_chain("https://dead.endpoint")
        self.assertIsNone(chain_id)

    def test_rpc_error_not_retried(self) -> None:
        """JSON-RPC execution errors are NOT retried — they propagate immediately."""
        call_count = [0]

        def mock_open(*args, **kwargs):
            call_count[0] += 1
            return io.BytesIO(
                b'{"error":{"code":-32000,"message":"execution reverted"}}'
            )

        with patch("_shared.rpc.urlopen", side_effect=mock_open):
            with self.assertRaises(_RpcError):
                rpc_failover("eth_call", [{"to": "0x00"}], endpoints=["https://base.local"])
        # Only 1 call — RPC errors are never retried
        self.assertEqual(call_count[0], 1)

    def test_http_429_retried_then_exhaust(self) -> None:
        """HTTP 429 is retried, then exhaust after max_retries."""
        call_count = [0]

        class MockResponse:
            def getcode(self):
                return 429

            def read(self):
                return b"{}"

            def close(self):
                pass

        with patch("_shared.rpc.urlopen", side_effect=lambda *a, **kw: MockResponse()):
            with self.assertRaises(RuntimeError) as ctx:
                rpc_failover(
                    "eth_chainId",
                    [],
                    endpoints=["https://base.local"],
                    max_retries=3,
                )
        self.assertIn("RPC failover exhausted", str(ctx.exception))

    def test_http_500_retried_and_recovered(self) -> None:
        """HTTP 500 on attempt 1 → retry → gets valid chain ID on attempt 2."""
        call_count = [0]

        def mock_fn(*args, **kwargs):
            call_count[0] += 1
            if call_count[0] == 1:
                # 500 error
                resp = type("Resp", (), {
                    "getcode": lambda s: 500,
                    "read": lambda s: b"{}",
                    "close": lambda s: None,
                })()
                return resp
            else:
                # Recover: valid chain ID
                return io.BytesIO(b'{"result":"0x2105"}')

        with patch("_shared.rpc.urlopen", side_effect=mock_fn), patch(
            "_shared.rpc.time.sleep"
        ):
            result = rpc_failover(
                "eth_blockNumber",
                [],
                endpoints=["https://base.local"],
                max_retries=3,
            )
        self.assertEqual(result, "0x2105")

    def test_https_endpoints_used(self) -> None:
        """All configured endpoints use HTTPS scheme."""
        for endpoint in BASE_RPC_ENDPOINTS:
            self.assertTrue(
                endpoint.startswith("https://"),
                f"Endpoint {endpoint} must use HTTPS",
            )

    def test_failover_skips_invalid_chain(self) -> None:
        """If first endpoint is wrong chain, failover tries next endpoint."""
        call_order = []

        def mock_fn(req, **kwargs):
            url = req.full_url if hasattr(req, "full_url") else str(req)
            call_order.append(url)
            if "wrong" in url:
                return io.BytesIO(b'{"result":"0x1"}')  # wrong chain
            return io.BytesIO(b'{"result":"0x2105"}')  # correct chain

        with patch("_shared.rpc.urlopen", side_effect=mock_fn):
            result = rpc_failover(
                "eth_blockNumber",
                [],
                endpoints=["https://wrong.chain", "https://base.local"],
                max_retries=1,
            )
        self.assertEqual(result, "0x2105")


if __name__ == "__main__":
    unittest.main()
