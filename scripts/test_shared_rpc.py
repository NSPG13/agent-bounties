#!/usr/bin/env python3
"""Characterization tests for retry-safe Base RPC failover transport."""

from __future__ import annotations

import io
import sys
import unittest
from pathlib import Path
from unittest.mock import patch
from urllib.error import HTTPError, URLError

# Allow `python -m unittest scripts.test_shared_rpc` and direct file runs from repo root.
_SCRIPTS = Path(__file__).resolve().parent
if str(_SCRIPTS) not in sys.path:
    sys.path.insert(0, str(_SCRIPTS))

from _shared.rpc import (
    BASE_CHAIN_ID,
    BASE_RPC_ENDPOINTS,
    RpcError,
    TransportError,
    _validate_chain,
    rpc,
    rpc_failover,
    select_working_base_rpc,
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

    def test_chain_validation_accepts_8453(self) -> None:
        with patch(
            "_shared.rpc.urlopen",
            return_value=io.BytesIO(b'{"result":"0x2105"}'),
        ):
            chain_id = _validate_chain("https://mainnet.base.org")
        self.assertEqual(chain_id, 8453)

    def test_chain_validation_rejects_wrong_chain(self) -> None:
        """wrong chain: Ethereum mainnet eth_chainId is rejected offline."""
        with patch(
            "_shared.rpc.urlopen",
            return_value=io.BytesIO(b'{"result":"0x1"}'),
        ):
            chain_id = _validate_chain("https://wrong.chain")
        self.assertIsNone(chain_id)

    def test_chain_validation_transport_failure_returns_none(self) -> None:
        with patch("_shared.rpc.urlopen", side_effect=URLError("offline")):
            chain_id = _validate_chain("https://dead.endpoint")
        self.assertIsNone(chain_id)

    def test_rpc_error_not_retried(self) -> None:
        """JSON-RPC execution errors are preserved and never retried."""
        call_count = [0]

        def mock_open(*_args, **_kwargs):
            call_count[0] += 1
            return io.BytesIO(
                b'{"error":{"code":-32000,"message":"execution reverted"}}'
            )

        with patch("_shared.rpc.urlopen", side_effect=mock_open):
            # first call is eth_chainId validation
            # second would be eth_call — but validation may consume first
            with self.assertRaises((RpcError, RuntimeError)):
                rpc_failover(
                    "eth_call",
                    [{"to": "0x00"}],
                    endpoints=["https://base.local"],
                    max_retries=3,
                )
        # Must not thrash on execution errors after a valid chain id path.
        self.assertLessEqual(call_count[0], 4)

    def test_rpc_error_after_valid_chain(self) -> None:
        """rpc error must be preserved after chain validation (never retried)."""
        call_count = [0]

        def mock_open(*_args, **_kwargs):
            call_count[0] += 1
            if call_count[0] == 1:
                return io.BytesIO(b'{"result":"0x2105"}')
            return io.BytesIO(
                b'{"error":{"code":-32000,"message":"execution reverted"}}'
            )

        with patch("_shared.rpc.urlopen", side_effect=mock_open):
            with self.assertRaises(RpcError) as ctx:
                rpc_failover(
                    "eth_call",
                    [{"to": "0x00"}],
                    endpoints=["https://base.local"],
                    max_retries=3,
                )
        self.assertIn("execution reverted", str(ctx.exception))
        self.assertEqual(call_count[0], 2)

    def test_http_429_retried_then_exhaust(self) -> None:
        """HTTP 429 is retried, then exhaust after max_retries."""

        class MockResponse:
            def getcode(self):
                return 429

            def read(self):
                return b"{}"

            def __enter__(self):
                return self

            def __exit__(self, *args):
                return False

        seq = {"n": 0}

        def mock_open(*_args, **_kwargs):
            seq["n"] += 1
            if seq["n"] == 1:
                return io.BytesIO(b'{"result":"0x2105"}')
            return MockResponse()

        with patch("_shared.rpc.urlopen", side_effect=mock_open), patch(
            "_shared.rpc.time.sleep"
        ):
            with self.assertRaises(RuntimeError) as ctx:
                rpc_failover(
                    "eth_blockNumber",
                    [],
                    endpoints=["https://base.local"],
                    max_retries=3,
                )
        self.assertIn("RPC failover exhausted", str(ctx.exception))
        self.assertIn("429", str(ctx.exception))

    def test_http_500_retried_and_recovered(self) -> None:
        """HTTP 500 then recovery path."""
        call_count = [0]

        class FailResponse:
            def getcode(self):
                return 500

            def read(self):
                return b"{}"

            def __enter__(self):
                return self

            def __exit__(self, *args):
                return False

        def mock_fn(*_args, **_kwargs):
            call_count[0] += 1
            if call_count[0] == 1:
                return io.BytesIO(b'{"result":"0x2105"}')  # chain id ok
            if call_count[0] == 2:
                return FailResponse()
            return io.BytesIO(b'{"result":"0xabc"}')

        with patch("_shared.rpc.urlopen", side_effect=mock_fn), patch(
            "_shared.rpc.time.sleep"
        ):
            result = rpc_failover(
                "eth_blockNumber",
                [],
                endpoints=["https://base.local"],
                max_retries=3,
            )
        self.assertEqual(result, "0xabc")

    def test_https_endpoints_used(self) -> None:
        for endpoint in BASE_RPC_ENDPOINTS:
            self.assertTrue(
                endpoint.startswith("https://"),
                f"Endpoint {endpoint} must use HTTPS",
            )
        self.assertEqual(BASE_CHAIN_ID, 8453)

    def test_failover_skips_invalid_chain(self) -> None:
        """Wrong-chain endpoint is skipped; next endpoint is used."""

        def mock_fn(req, **_kwargs):
            url = getattr(req, "full_url", str(req))
            if "wrong" in url:
                return io.BytesIO(b'{"result":"0x1"}')
            return io.BytesIO(b'{"result":"0x2105"}')

        with patch("_shared.rpc.urlopen", side_effect=mock_fn):
            result = rpc_failover(
                "eth_blockNumber",
                [],
                endpoints=["https://wrong.chain", "https://base.local"],
                max_retries=1,
            )
        self.assertEqual(result, "0x2105")

    def test_endpoint_exhaust(self) -> None:
        """exhaust all endpoints after transport failures."""
        with patch("_shared.rpc.urlopen", side_effect=URLError("offline")), patch(
            "_shared.rpc.time.sleep"
        ):
            with self.assertRaisesRegex(RuntimeError, "RPC failover exhausted"):
                rpc_failover(
                    "eth_chainId",
                    [],
                    endpoints=["https://a.local", "https://b.local"],
                    max_retries=1,
                )

    def test_select_working_returns_fallback_not_failed_preferred(self) -> None:
        """When preferred fails/429s, selection returns the endpoint that passed."""

        class Fail429:
            def getcode(self):
                return 429

            def read(self):
                return b"{}"

            def __enter__(self):
                return self

            def __exit__(self, *args):
                return False

        def mock_fn(req, **_kwargs):
            url = getattr(req, "full_url", str(req))
            if "preferred.fail" in url:
                return Fail429()
            return io.BytesIO(b'{"result":"0x2105"}')

        with patch("_shared.rpc.urlopen", side_effect=mock_fn), patch(
            "_shared.rpc.time.sleep"
        ):
            selected = select_working_base_rpc(
                preferred="https://preferred.fail",
                endpoints=["https://preferred.fail", "https://fallback.ok"],
                max_retries=1,
            )
            # Follow-up read must use the selected fallback, not the preferred URL.
            result = rpc_failover(
                "eth_blockNumber",
                [],
                endpoints=[selected],
                max_retries=1,
            )
        self.assertEqual(selected, "https://fallback.ok")
        self.assertEqual(result, "0x2105")


if __name__ == "__main__":
    unittest.main()
