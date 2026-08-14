#!/usr/bin/env python3
"""Offline characterization tests for the failover JSON-RPC transport."""

from __future__ import annotations

import unittest
from unittest.mock import patch

from _shared.rpc import (
    FailoverJsonRpcTransport,
    RpcExecutionError,
    RetriableRpcError,
)


class FailoverRpcTest(unittest.TestCase):
    def setUp(self) -> None:
        patcher = patch("_shared.rpc.rpc")
        self.mock_rpc = patcher.start()
        self.addCleanup(patcher.stop)

    def _transport(self, endpoints: list[str]) -> FailoverJsonRpcTransport:
        return FailoverJsonRpcTransport(
            endpoints, max_retries=2, base_backoff_ms=0, max_backoff_ms=0, sleep=lambda _s: None
        )

    def test_429_retries_then_advances_to_next_endpoint(self) -> None:
        def fake(url: str, method: str, _params, _request_id=1):
            if method == "eth_chainId":
                return "0x2105"
            if url == "https://a.example":
                raise RetriableRpcError(429)
            return "0x1b4"

        self.mock_rpc.side_effect = fake
        transport = self._transport(["https://a.example", "https://b.example"])
        self.assertEqual(transport.post("eth_blockNumber", []), "0x1b4")

    def test_wrong_chain_endpoint_is_skipped(self) -> None:
        def fake(url: str, method: str, _params, _request_id=1):
            if method == "eth_chainId":
                return "0x1" if url == "https://wrong.example" else "0x2105"
            return "0x2a"

        self.mock_rpc.side_effect = fake
        transport = self._transport(["https://wrong.example", "https://good.example"])
        self.assertEqual(transport.post("eth_blockNumber", []), "0x2a")

    def test_execution_error_is_not_retried(self) -> None:
        def fake(url: str, method: str, _params, _request_id=1):
            if method == "eth_chainId":
                return "0x2105"
            raise RpcExecutionError(method, {"code": -32000, "message": "execution reverted"})

        self.mock_rpc.side_effect = fake
        transport = self._transport(["https://a.example", "https://b.example"])
        with self.assertRaises(RpcExecutionError) as raised:
            transport.post("eth_call", [])
        self.assertEqual(raised.exception.payload["code"], -32000)

    def test_exhaustion_raises_after_all_endpoints(self) -> None:
        def fake(url: str, method: str, _params, _request_id=1):
            if method == "eth_chainId":
                return "0x2105"
            raise RetriableRpcError(503)

        self.mock_rpc.side_effect = fake
        transport = self._transport(["https://a.example", "https://b.example"])
        with self.assertRaises(RuntimeError) as raised:
            transport.post("eth_blockNumber", [])
        self.assertIn("exhausted", str(raised.exception))

    def test_credentialed_url_is_rejected(self) -> None:
        with self.assertRaises(ValueError):
            FailoverJsonRpcTransport(["https://user:pass@rpc.example"])
        with self.assertRaises(ValueError):
            FailoverJsonRpcTransport(["http://rpc.example"])  # non-HTTPS


if __name__ == "__main__":
    unittest.main()
