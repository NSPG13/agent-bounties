#!/usr/bin/env python3
"""Tests for retry-safe Base RPC selection and read failover."""

from __future__ import annotations

import io
import json
import sys
import traceback
import unittest
from pathlib import Path
from unittest.mock import patch
from urllib.error import HTTPError, URLError


SCRIPTS = Path(__file__).resolve().parent
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))

from _shared.rpc import (
    BASE_CHAIN_ID,
    BASE_RPC_ENDPOINTS,
    MAX_RPC_ERROR_BODY_BYTES,
    MAX_RPC_ERROR_MESSAGE_CHARS,
    RETRYABLE_HTTP_STATUS,
    RPC_ERROR_MESSAGE_TRUNCATION_INDICATOR,
    RpcError,
    TransportError,
    _redact_endpoint,
    _validate_chain,
    _without_absolute_urls,
    is_retryable_http_status,
    is_retryable_transport_output,
    rpc,
    rpc_failover,
    select_working_base_rpc,
)

SECRET_ENDPOINT = (
    "https://leaked-user:leaked-pass@rpc.example/v2/leaked-path"
    "?api_key=leaked-query#leaked-frag"
)
SECRET_FRAGMENTS = (
    SECRET_ENDPOINT,
    "leaked-user",
    "leaked-pass",
    "leaked-path",
    "leaked-query",
    "leaked-frag",
    "/v2/leaked-path",
    "api_key=leaked-query",
    "leaked-user:leaked-pass",
)


def walk_exception_chain(exc: BaseException | None) -> list[BaseException]:
    seen: set[int] = set()
    ordered: list[BaseException] = []

    def visit(node: BaseException | None) -> None:
        if node is None or id(node) in seen:
            return
        seen.add(id(node))
        ordered.append(node)
        visit(getattr(node, "__cause__", None))
        visit(getattr(node, "__context__", None))
        for inner in getattr(node, "exceptions", ()) or ():
            if isinstance(inner, BaseException):
                visit(inner)

    visit(exc)
    return ordered


def _text_value(value: object) -> str | None:
    if isinstance(value, bytes):
        return value.decode("utf-8", "replace")
    if isinstance(value, str):
        return value
    return None


def exception_chain_text(exc: BaseException) -> str:
    chunks: list[str] = ["".join(traceback.format_exception(exc))]
    for node in walk_exception_chain(exc):
        chunks.extend((str(node), repr(node), repr(getattr(node, "args", ()))))
        for name in ("filename", "url", "msg", "reason", "message"):
            text = _text_value(getattr(node, name, None))
            if text is not None:
                chunks.append(text)
        getter = getattr(node, "geturl", None)
        if callable(getter):
            try:
                chunks.append(str(getter()))
            except (AttributeError, TypeError, ValueError, OSError):
                pass
    return "\n".join(chunks)


class Response(io.BytesIO):
    def __init__(self, body: bytes, status: int = 200) -> None:
        super().__init__(body)
        self.status = status

    def getcode(self) -> int:
        return self.status

    def __enter__(self) -> "Response":
        return self

    def __exit__(self, *_args: object) -> bool:
        self.close()
        return False


def request_method(request: object) -> str:
    return str(json.loads(getattr(request, "data").decode("utf-8"))["method"])


def http_error_with_json(
    url: str,
    status: int,
    body: dict[str, object],
    reason: str = "upstream",
) -> HTTPError:
    payload = json.dumps(body).encode("utf-8")
    return HTTPError(url, status, reason, {}, io.BytesIO(payload))


def jsonrpc_http_error(
    url: str,
    status: int,
    error: dict[str, object],
    reason: str = "upstream",
    *,
    response_id: object = 1,
) -> HTTPError:
    return http_error_with_json(
        url,
        status,
        {"jsonrpc": "2.0", "id": response_id, "error": error},
        reason,
    )


def envelopeless_gateway_http_error(
    url: str,
    status: int,
    error: dict[str, object],
    reason: str = "upstream",
) -> HTTPError:
    return http_error_with_json(url, status, {"error": error}, reason)


class RpcTest(unittest.TestCase):
    def test_single_endpoint_result_and_rpc_error_contracts(self) -> None:
        cases = (
            (b'{"result":"0x2105"}', "0x2105", None),
            (
                b'{"error":{"code":-1,"message":"bad"}}',
                None,
                'RPC eth_chainId failed: {"code": -1, "message": "bad"}',
            ),
            (b"{}", None, None),
        )
        for body, expected, message in cases:
            with self.subTest(body=body), patch(
                "_shared.rpc.urlopen", return_value=Response(body)
            ):
                if message:
                    with self.assertRaisesRegex(RpcError, message):
                        rpc("https://base.local", "eth_chainId", [], 7)
                else:
                    self.assertEqual(
                        rpc("https://base.local", "eth_chainId", [], 7),
                        expected,
                    )

    def test_single_endpoint_retries_timeout_then_recovers(self) -> None:
        responses = [TimeoutError("slow"), Response(b'{"result":"0x14a34"}')]
        with patch("_shared.rpc.urlopen", side_effect=responses) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept:
            self.assertEqual(
                rpc(
                    "https://base.local",
                    "eth_chainId",
                    [],
                    retry_delay=0.25,
                ),
                "0x14a34",
            )
        self.assertEqual(opened.call_count, 2)
        slept.assert_called_once_with(0.25)

    def test_single_endpoint_nonretryable_http_error_fails_immediately(self) -> None:
        error = HTTPError("https://base.local", 401, "unauthorized", {}, None)
        with patch("_shared.rpc.urlopen", side_effect=error) as opened, self.assertRaisesRegex(
            RuntimeError, "^RPC transport failed for eth_call:"
        ):
            rpc("https://base.local", "eth_call", [], retry_delay=0)
        self.assertEqual(opened.call_count, 1)

    def test_invalid_retry_policies_are_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "attempts must be positive"):
            rpc("https://base.local", "eth_call", [], attempts=0)
        with self.assertRaisesRegex(ValueError, "max_retries must be positive"):
            select_working_base_rpc(max_retries=0)

    def test_chain_validation_preserves_observed_chain(self) -> None:
        with patch(
            "_shared.rpc.urlopen",
            return_value=Response(b'{"result":"0x2105"}'),
        ):
            self.assertEqual(_validate_chain("https://base.local"), 8453)
        with patch(
            "_shared.rpc.urlopen",
            return_value=Response(b'{"result":"0x1"}'),
        ):
            self.assertEqual(_validate_chain("https://wrong.local"), 1)

    def test_chain_validation_propagates_transport_error(self) -> None:
        with patch(
            "_shared.rpc.urlopen", side_effect=URLError("offline")
        ), self.assertRaises(TransportError):
            _validate_chain("https://dead.local")

    def test_chain_rpc_error_never_calls_a_second_endpoint(self) -> None:
        with patch(
            "_shared.rpc.urlopen",
            return_value=Response(
                b'{"error":{"code":-32000,"message":"execution reverted"}}'
            ),
        ) as opened, self.assertRaisesRegex(RpcError, "execution reverted"):
            select_working_base_rpc(
                endpoints=("https://first.local", "https://second.local")
            )
        self.assertEqual(opened.call_count, 1)

    def test_method_rpc_error_never_retries_or_fails_over(self) -> None:
        calls = 0

        def open_response(request: object, **_kwargs: object) -> Response:
            nonlocal calls
            calls += 1
            if request_method(request) == "eth_chainId":
                return Response(b'{"result":"0x2105"}')
            return Response(
                b'{"error":{"code":-32000,"message":"execution reverted"}}'
            )

        with patch(
            "_shared.rpc.urlopen", side_effect=open_response
        ), self.assertRaisesRegex(RpcError, "execution reverted"):
            rpc_failover(
                "eth_call",
                [{"to": "0x00"}],
                endpoints=("https://first.local", "https://second.local"),
            )
        self.assertEqual(calls, 2)

    def test_selection_retries_two_429s_then_uses_same_endpoint(self) -> None:
        responses = [
            Response(b"{}", 429),
            Response(b"{}", 429),
            Response(b'{"result":"0x2105"}'),
        ]
        with patch("_shared.rpc.urlopen", side_effect=responses) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept:
            selected = select_working_base_rpc(
                endpoints=("https://base.local",),
                max_retries=3,
            )
        self.assertEqual(selected, "https://base.local")
        self.assertEqual(opened.call_count, 3)
        self.assertEqual(slept.call_count, 2)

    def test_wrong_chain_is_skipped_before_requested_read(self) -> None:
        """A wrong chain endpoint is rejected before the requested read reaches it."""
        seen: list[tuple[str, str]] = []

        def open_response(request: object, **_kwargs: object) -> Response:
            url = str(getattr(request, "full_url"))
            method = request_method(request)
            seen.append((url, method))
            if "wrong" in url:
                return Response(b'{"result":"0x1"}')
            if method == "eth_chainId":
                return Response(b'{"result":"0x2105"}')
            return Response(b'{"result":"0xabc"}')

        with patch("_shared.rpc.urlopen", side_effect=open_response):
            result = rpc_failover(
                "eth_blockNumber",
                [],
                endpoints=("https://wrong.local", "https://base.local"),
                max_retries=1,
            )
        self.assertEqual(result, "0xabc")
        self.assertNotIn(("https://wrong.local", "eth_blockNumber"), seen)

    def test_failed_preferred_followup_read_uses_fallback(self) -> None:
        seen: list[tuple[str, str]] = []

        def open_response(request: object, **_kwargs: object) -> Response:
            url = str(getattr(request, "full_url"))
            method = request_method(request)
            seen.append((url, method))
            if "preferred" in url:
                return Response(b"{}", 429)
            if method == "eth_chainId":
                return Response(b'{"result":"0x2105"}')
            return Response(b'{"result":"0xabc"}')

        with patch("_shared.rpc.urlopen", side_effect=open_response), patch(
            "_shared.rpc.time.sleep"
        ):
            result = rpc_failover(
                "eth_blockNumber",
                [],
                preferred="https://preferred.local",
                endpoints=("https://fallback.local",),
                max_retries=1,
            )
        self.assertEqual(result, "0xabc")
        self.assertIn(("https://fallback.local", "eth_blockNumber"), seen)

    def test_endpoint_exhaustion_is_explicit(self) -> None:
        with patch("_shared.rpc.urlopen", side_effect=URLError("offline")), patch(
            "_shared.rpc.time.sleep"
        ), self.assertRaisesRegex(RuntimeError, "RPC failover exhausted"):
            rpc_failover(
                "eth_chainId",
                [],
                endpoints=("https://a.local", "https://b.local"),
                max_retries=1,
            )

    def test_endpoint_catalog_is_https_base_mainnet(self) -> None:
        self.assertTrue(BASE_RPC_ENDPOINTS)
        self.assertTrue(all(item.startswith("https://") for item in BASE_RPC_ENDPOINTS))
        self.assertEqual(BASE_CHAIN_ID, 8453)

    def test_endpoint_redaction_removes_all_secret_locations(self) -> None:
        value = _redact_endpoint(
            "https://user:pass@rpc.example/v2/secret?api_key=also-secret#frag"
        )
        self.assertEqual(value, "https://rpc.example")
        self.assertNotIn("secret", value)
        self.assertNotIn("user", value)
        self.assertNotIn("frag", value)

    def test_cast_transport_classifier_excludes_json_rpc_errors(self) -> None:
        self.assertTrue(is_retryable_transport_output("HTTP error 429: over rate limit"))
        self.assertTrue(is_retryable_transport_output("connection reset by peer"))
        self.assertTrue(is_retryable_transport_output("HTTP error 501: not implemented"))
        self.assertTrue(is_retryable_transport_output("HTTP 599"))
        self.assertFalse(is_retryable_transport_output("HTTP error 401: unauthorized"))
        self.assertFalse(
            is_retryable_transport_output(
                "JSON-RPC error: execution reverted"
            )
        )
        self.assertFalse(
            is_retryable_transport_output(
                "JSON-RPC error: upstream returned HTTP 501"
            )
        )
        self.assertFalse(
            is_retryable_transport_output(
                "JSON-RPC error: upstream returned HTTP 500"
            )
        )

    def test_retryable_http_status_covers_every_5xx(self) -> None:
        self.assertTrue(is_retryable_http_status(408))
        self.assertTrue(is_retryable_http_status(429))
        self.assertTrue(is_retryable_http_status(500))
        self.assertTrue(is_retryable_http_status(501))
        self.assertTrue(is_retryable_http_status(599))
        self.assertEqual(RETRYABLE_HTTP_STATUS, frozenset({408, 429, *range(500, 600)}))
        self.assertFalse(is_retryable_http_status(400))
        self.assertFalse(is_retryable_http_status(401))
        self.assertFalse(is_retryable_http_status(499))
        self.assertFalse(is_retryable_http_status(600))

    def test_http_500_jsonrpc_error_body_never_retries(self) -> None:
        error = jsonrpc_http_error(
            SECRET_ENDPOINT,
            500,
            {"code": -32000, "message": "execution reverted"},
        )
        with patch("_shared.rpc.urlopen", side_effect=error) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept, self.assertRaisesRegex(RpcError, "execution reverted") as raised:
            rpc(SECRET_ENDPOINT, "eth_call", [], attempts=3, retry_delay=0)
        self.assertEqual(opened.call_count, 1)
        slept.assert_not_called()
        self.assertTrue(error.closed)
        self.assertTrue(error.fp.closed)
        self.assertIn('"code": -32000', str(raised.exception))
        self.assertIn('"message": "execution reverted"', str(raised.exception))
        surface = exception_chain_text(raised.exception)
        for fragment in SECRET_FRAGMENTS:
            self.assertNotIn(fragment, surface)
        for node in walk_exception_chain(raised.exception):
            self.assertNotIsInstance(node, HTTPError)
            self.assertNotIsInstance(node, TransportError)

    def test_http_501_jsonrpc_error_body_never_retries(self) -> None:
        error = jsonrpc_http_error(
            SECRET_ENDPOINT,
            501,
            {"code": -32601, "message": "method not found"},
        )
        with patch("_shared.rpc.urlopen", side_effect=error) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept, self.assertRaisesRegex(RpcError, "method not found") as raised:
            rpc(SECRET_ENDPOINT, "eth_call", [], attempts=3, retry_delay=0)
        self.assertEqual(opened.call_count, 1)
        slept.assert_not_called()
        self.assertTrue(error.closed)
        self.assertTrue(error.fp.closed)
        self.assertIn('"code": -32601', str(raised.exception))
        self.assertIn('"message": "method not found"', str(raised.exception))
        surface = exception_chain_text(raised.exception)
        for fragment in SECRET_FRAGMENTS:
            self.assertNotIn(fragment, surface)
        for node in walk_exception_chain(raised.exception):
            self.assertNotIsInstance(node, HTTPError)
            self.assertNotIsInstance(node, TransportError)

    def test_http_500_jsonrpc_error_body_never_fails_over(self) -> None:
        error = jsonrpc_http_error(
            SECRET_ENDPOINT,
            500,
            {"code": -32000, "message": "execution reverted"},
        )
        with patch("_shared.rpc.urlopen", side_effect=error) as opened, patch(
            "_shared.rpc.time.sleep"
        ), self.assertRaisesRegex(RpcError, "execution reverted") as raised:
            rpc_failover(
                "eth_call",
                [{"to": "0x00"}],
                endpoints=(SECRET_ENDPOINT, "https://second.local"),
                max_retries=3,
            )
        self.assertEqual(opened.call_count, 1)
        self.assertTrue(error.closed)
        self.assertIn('"code": -32000', str(raised.exception))
        for node in walk_exception_chain(raised.exception):
            self.assertNotIsInstance(node, HTTPError)
            self.assertNotIsInstance(node, TransportError)

    def test_http_501_jsonrpc_error_body_never_fails_over(self) -> None:
        error = jsonrpc_http_error(
            SECRET_ENDPOINT,
            501,
            {"code": -32601, "message": "method not found"},
        )
        with patch("_shared.rpc.urlopen", side_effect=error) as opened, patch(
            "_shared.rpc.time.sleep"
        ), self.assertRaisesRegex(RpcError, "method not found") as raised:
            rpc_failover(
                "eth_call",
                [{"to": "0x00"}],
                endpoints=(SECRET_ENDPOINT, "https://second.local"),
                max_retries=3,
            )
        self.assertEqual(opened.call_count, 1)
        self.assertTrue(error.closed)
        self.assertIn('"code": -32601', str(raised.exception))
        for node in walk_exception_chain(raised.exception):
            self.assertNotIsInstance(node, HTTPError)
            self.assertNotIsInstance(node, TransportError)

    def test_envelopeless_gateway_errors_retry_then_recover(self) -> None:
        cases = (
            (429, "Too Many Requests", "too many requests"),
            (500, "Internal Server Error", "internal"),
            (503, "Service Unavailable", "unavailable"),
        )
        for status, message, reason in cases:
            with self.subTest(status=status):
                first = envelopeless_gateway_http_error(
                    SECRET_ENDPOINT,
                    status,
                    {"code": status, "message": message},
                    reason,
                )
                responses = [first, Response(b'{"result":"0x14a34"}')]
                with patch(
                    "_shared.rpc.urlopen", side_effect=responses
                ) as opened, patch("_shared.rpc.time.sleep") as slept:
                    self.assertEqual(
                        rpc(SECRET_ENDPOINT, "eth_chainId", [], retry_delay=0.25),
                        "0x14a34",
                    )
                self.assertEqual(opened.call_count, 2)
                slept.assert_called_once_with(0.25)
                self.assertTrue(first.closed)
                self.assertTrue(first.fp.closed)

    def test_envelopeless_gateway_errors_fail_over_target_method(self) -> None:
        for status, message, reason in (
            (429, "Too Many Requests", "too many requests"),
            (500, "Internal Server Error", "internal"),
            (503, "Service Unavailable", "unavailable"),
        ):
            with self.subTest(status=status):
                seen: list[tuple[str, str]] = []

                def open_response(request: object, **_kwargs: object) -> Response:
                    url = str(getattr(request, "full_url"))
                    method = request_method(request)
                    seen.append((url, method))
                    if method == "eth_chainId":
                        return Response(b'{"result":"0x2105"}')
                    if url == SECRET_ENDPOINT:
                        raise envelopeless_gateway_http_error(
                            SECRET_ENDPOINT,
                            status,
                            {"code": status, "message": message},
                            reason,
                        )
                    return Response(b'{"result":"0xabc"}')

                with patch(
                    "_shared.rpc.urlopen", side_effect=open_response
                ), patch("_shared.rpc.time.sleep"):
                    result = rpc_failover(
                        "eth_blockNumber",
                        [],
                        endpoints=(SECRET_ENDPOINT, "https://second.local"),
                        max_retries=1,
                    )
                self.assertEqual(result, "0xabc")
                self.assertEqual(
                    seen,
                    [
                        (SECRET_ENDPOINT, "eth_chainId"),
                        (SECRET_ENDPOINT, "eth_blockNumber"),
                        ("https://second.local", "eth_chainId"),
                        ("https://second.local", "eth_blockNumber"),
                    ],
                )

    def test_http_500_jsonrpc_matching_custom_request_id_never_retries(self) -> None:
        error = jsonrpc_http_error(
            SECRET_ENDPOINT,
            500,
            {"code": -32000, "message": "execution reverted"},
            response_id=7,
        )
        with patch("_shared.rpc.urlopen", side_effect=error) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept, self.assertRaisesRegex(RpcError, "execution reverted") as raised:
            rpc(SECRET_ENDPOINT, "eth_call", [], 7, attempts=3, retry_delay=0)
        self.assertEqual(opened.call_count, 1)
        slept.assert_not_called()
        self.assertTrue(error.closed)
        self.assertIn('"code": -32000', str(raised.exception))
        self.assertIn('"message": "execution reverted"', str(raised.exception))
        for node in walk_exception_chain(raised.exception):
            self.assertNotIsInstance(node, HTTPError)
            self.assertNotIsInstance(node, TransportError)

    def test_http_500_jsonrpc_null_id_parse_error_never_retries(self) -> None:
        error = jsonrpc_http_error(
            SECRET_ENDPOINT,
            500,
            {"code": -32700, "message": "Parse error"},
            response_id=None,
        )
        with patch("_shared.rpc.urlopen", side_effect=error) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept, self.assertRaisesRegex(RpcError, "Parse error") as raised:
            rpc(SECRET_ENDPOINT, "eth_call", [], attempts=3, retry_delay=0)
        self.assertEqual(opened.call_count, 1)
        slept.assert_not_called()
        self.assertTrue(error.closed)
        self.assertIn('"code": -32700', str(raised.exception))
        self.assertIn('"message": "Parse error"', str(raised.exception))
        for node in walk_exception_chain(raised.exception):
            self.assertNotIsInstance(node, HTTPError)
            self.assertNotIsInstance(node, TransportError)

    def test_http_500_jsonrpc_null_id_invalid_request_never_fails_over(self) -> None:
        error = jsonrpc_http_error(
            SECRET_ENDPOINT,
            500,
            {"code": -32600, "message": "Invalid Request"},
            response_id=None,
        )
        with patch("_shared.rpc.urlopen", side_effect=error) as opened, patch(
            "_shared.rpc.time.sleep"
        ), self.assertRaisesRegex(RpcError, "Invalid Request") as raised:
            rpc_failover(
                "eth_call",
                [{"to": "0x00"}],
                endpoints=(SECRET_ENDPOINT, "https://second.local"),
                max_retries=3,
            )
        self.assertEqual(opened.call_count, 1)
        self.assertTrue(error.closed)
        self.assertIn('"code": -32600', str(raised.exception))
        for node in walk_exception_chain(raised.exception):
            self.assertNotIsInstance(node, HTTPError)
            self.assertNotIsInstance(node, TransportError)

    def test_http_429_jsonrpc_null_id_application_error_retries_then_recovers(
        self,
    ) -> None:
        first = jsonrpc_http_error(
            SECRET_ENDPOINT,
            429,
            {"code": 429, "message": "Too Many Requests"},
            response_id=None,
        )
        responses = [first, Response(b'{"result":"0x14a34"}')]
        with patch("_shared.rpc.urlopen", side_effect=responses) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept:
            self.assertEqual(
                rpc(SECRET_ENDPOINT, "eth_chainId", [], retry_delay=0.25),
                "0x14a34",
            )
        self.assertEqual(opened.call_count, 2)
        slept.assert_called_once_with(0.25)
        self.assertTrue(first.closed)

    def test_http_429_jsonrpc_id_mismatch_retries_then_recovers(self) -> None:
        first = jsonrpc_http_error(
            SECRET_ENDPOINT,
            429,
            {"code": -32000, "message": "execution reverted"},
            response_id=999,
        )
        responses = [first, Response(b'{"result":"0x14a34"}')]
        with patch("_shared.rpc.urlopen", side_effect=responses) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept:
            self.assertEqual(
                rpc(SECRET_ENDPOINT, "eth_chainId", [], retry_delay=0.25),
                "0x14a34",
            )
        self.assertEqual(opened.call_count, 2)
        slept.assert_called_once_with(0.25)
        self.assertTrue(first.closed)

    def test_http_500_jsonrpc_id_mismatch_retries_then_recovers(self) -> None:
        first = jsonrpc_http_error(
            SECRET_ENDPOINT,
            500,
            {"code": -32000, "message": "execution reverted"},
            response_id="other",
        )
        responses = [first, Response(b'{"result":"0x14a34"}')]
        with patch("_shared.rpc.urlopen", side_effect=responses) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept:
            self.assertEqual(
                rpc(SECRET_ENDPOINT, "eth_chainId", [], retry_delay=0.25),
                "0x14a34",
            )
        self.assertEqual(opened.call_count, 2)
        slept.assert_called_once_with(0.25)
        self.assertTrue(first.closed)

    def test_http_500_jsonrpc_missing_id_retries_then_recovers(self) -> None:
        first = http_error_with_json(
            SECRET_ENDPOINT,
            500,
            {
                "jsonrpc": "2.0",
                "error": {"code": -32000, "message": "execution reverted"},
            },
            "internal",
        )
        responses = [first, Response(b'{"result":"0x14a34"}')]
        with patch("_shared.rpc.urlopen", side_effect=responses) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept:
            self.assertEqual(
                rpc(SECRET_ENDPOINT, "eth_chainId", [], retry_delay=0.25),
                "0x14a34",
            )
        self.assertEqual(opened.call_count, 2)
        slept.assert_called_once_with(0.25)
        self.assertTrue(first.closed)

    def test_http_429_jsonrpc_version_mismatch_retries_then_recovers(self) -> None:
        first = http_error_with_json(
            SECRET_ENDPOINT,
            429,
            {
                "jsonrpc": "1.0",
                "id": 1,
                "error": {"code": -32000, "message": "execution reverted"},
            },
            "too many requests",
        )
        responses = [first, Response(b'{"result":"0x14a34"}')]
        with patch("_shared.rpc.urlopen", side_effect=responses) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept:
            self.assertEqual(
                rpc(SECRET_ENDPOINT, "eth_chainId", [], retry_delay=0.25),
                "0x14a34",
            )
        self.assertEqual(opened.call_count, 2)
        slept.assert_called_once_with(0.25)
        self.assertTrue(first.closed)

    def test_malformed_http_jsonrpc_envelopes_use_http_classifier(self) -> None:
        valid_error = {"code": -32000, "message": "execution reverted"}
        cases = (
            {"jsonrpc": "2.0", "id": True, "error": valid_error},
            {"jsonrpc": "2.0", "id": "1", "error": valid_error},
            {"jsonrpc": "2.0", "id": 1.0, "error": valid_error},
            {"jsonrpc": "2.0", "id": 1, "result": None, "error": valid_error},
            {"jsonrpc": "2.0", "id": 1, "error": []},
            {"jsonrpc": "2.0", "id": 1, "error": {"code": True, "message": "bad"}},
            {"jsonrpc": "2.0", "id": 1, "error": {"code": -32000.0, "message": "bad"}},
            {"jsonrpc": "2.0", "id": 1, "error": {"code": -32000, "message": 7}},
            {"jsonrpc": "2.0", "id": None, "error": {"code": True, "message": "bad"}},
        )
        for envelope in cases:
            with self.subTest(envelope=envelope):
                first = http_error_with_json(SECRET_ENDPOINT, 500, envelope)
                with patch(
                    "_shared.rpc.urlopen",
                    side_effect=[first, Response(b'{"result":"0x14a34"}')],
                ) as opened, patch("_shared.rpc.time.sleep") as slept:
                    result = rpc(SECRET_ENDPOINT, "eth_chainId", [], retry_delay=0)
                self.assertEqual(result, "0x14a34")
                self.assertEqual(opened.call_count, 2)
                slept.assert_called_once_with(0)
                self.assertTrue(first.closed)

    def test_string_request_id_binds_only_string_response_id(self) -> None:
        error = jsonrpc_http_error(
            SECRET_ENDPOINT,
            500,
            {"code": -32000, "message": "string id error"},
            response_id="request-1",
        )
        with patch("_shared.rpc.urlopen", side_effect=error) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept, self.assertRaisesRegex(RpcError, "string id error"):
            rpc(SECRET_ENDPOINT, "eth_call", [], "request-1")  # type: ignore[arg-type]
        self.assertEqual(opened.call_count, 1)
        slept.assert_not_called()

    def test_http_jsonrpc_long_valid_message_is_confirmed_truncated_rpc_error(
        self,
    ) -> None:
        data_secret = "rpc-error-data-secret-marker"
        tail_secret = "rpc-error-message-tail-secret"
        indicator = RPC_ERROR_MESSAGE_TRUNCATION_INDICATOR
        keep = max(MAX_RPC_ERROR_MESSAGE_CHARS - len(indicator), 0)
        long_message = ("x" * keep) + tail_secret
        self.assertGreater(len(long_message), MAX_RPC_ERROR_MESSAGE_CHARS)
        expected_message = (long_message[:keep] + indicator)[:MAX_RPC_ERROR_MESSAGE_CHARS]
        self.assertEqual(len(expected_message), MAX_RPC_ERROR_MESSAGE_CHARS)
        self.assertTrue(expected_message.endswith(indicator))
        simple_body = json.dumps(
            {
                "jsonrpc": "2.0",
                "id": 1,
                "error": {"code": -32000, "message": "x" * 4097},
            }
        ).encode("utf-8")
        self.assertEqual(len(simple_body), 4166)
        self.assertLessEqual(len(simple_body), MAX_RPC_ERROR_BODY_BYTES)
        error_fields: dict[str, object] = {
            "code": -32000,
            "message": long_message,
            "data": {"credential": data_secret, "endpoint": SECRET_ENDPOINT},
            "extra": data_secret,
        }
        expected_exc = (
            "RPC eth_call failed: "
            + json.dumps(
                {"code": -32000, "message": expected_message},
                sort_keys=True,
            )
        )

        def assert_safe_rpc_error(exc: BaseException) -> None:
            self.assertEqual(str(exc), expected_exc)
            payload = json.loads(str(exc).split("failed: ", 1)[1])
            self.assertEqual(
                payload, {"code": -32000, "message": expected_message}
            )
            self.assertLessEqual(len(payload["message"]), MAX_RPC_ERROR_MESSAGE_CHARS)
            self.assertLessEqual(len(str(exc)), len(expected_exc))
            surface = exception_chain_text(exc)
            for fragment in (
                data_secret,
                tail_secret,
                long_message,
                *SECRET_FRAGMENTS,
            ):
                self.assertNotIn(fragment, surface)
            self.assertNotIn('"data"', surface)
            self.assertNotIn('"extra"', surface)
            for node in walk_exception_chain(exc):
                self.assertNotIsInstance(node, HTTPError)
                self.assertNotIsInstance(node, TransportError)

        first = jsonrpc_http_error(SECRET_ENDPOINT, 500, error_fields)
        with patch("_shared.rpc.urlopen", side_effect=first) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept, self.assertRaises(RpcError) as raised:
            rpc(SECRET_ENDPOINT, "eth_call", [], attempts=3, retry_delay=0.25)
        self.assertEqual(opened.call_count, 1)
        slept.assert_not_called()
        self.assertTrue(first.closed)
        self.assertTrue(first.fp.closed)
        assert_safe_rpc_error(raised.exception)

        seen: list[tuple[str, str]] = []

        def open_response(request: object, **_kwargs: object) -> Response:
            url = str(getattr(request, "full_url"))
            method = request_method(request)
            seen.append((url, method))
            if method == "eth_chainId":
                return Response(b'{"result":"0x2105"}')
            raise jsonrpc_http_error(url, 500, error_fields)

        with patch("_shared.rpc.urlopen", side_effect=open_response) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept, self.assertRaises(RpcError) as raised:
            rpc_failover(
                "eth_call",
                [],
                endpoints=(SECRET_ENDPOINT, "https://second.local"),
                max_retries=3,
            )
        self.assertEqual(
            seen,
            [(SECRET_ENDPOINT, "eth_chainId"), (SECRET_ENDPOINT, "eth_call")],
        )
        self.assertEqual(opened.call_count, 2)
        slept.assert_not_called()
        assert_safe_rpc_error(raised.exception)

    def test_http_jsonrpc_message_at_char_bound_is_not_truncated(self) -> None:
        message = "z" * MAX_RPC_ERROR_MESSAGE_CHARS
        error = jsonrpc_http_error(
            SECRET_ENDPOINT,
            500,
            {"code": -32000, "message": message},
        )
        with patch("_shared.rpc.urlopen", side_effect=error) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept, self.assertRaises(RpcError) as raised:
            rpc(SECRET_ENDPOINT, "eth_call", [], attempts=3, retry_delay=0)
        self.assertEqual(opened.call_count, 1)
        slept.assert_not_called()
        self.assertTrue(error.closed)
        self.assertIn(message, str(raised.exception))
        self.assertNotIn(RPC_ERROR_MESSAGE_TRUNCATION_INDICATOR, str(raised.exception))

    def test_http_jsonrpc_body_over_byte_limit_uses_http_classifier(self) -> None:
        envelope = json.dumps(
            {
                "jsonrpc": "2.0",
                "id": 1,
                "error": {"code": -32000, "message": "execution reverted"},
            }
        ).encode("utf-8")
        cases = (
            (MAX_RPC_ERROR_BODY_BYTES, True),
            (MAX_RPC_ERROR_BODY_BYTES + 1, False),
        )
        for size, confirmed in cases:
            with self.subTest(size=size, confirmed=confirmed):
                raw = envelope + (b" " * (size - len(envelope)))
                self.assertEqual(len(raw), size)
                first = HTTPError(
                    SECRET_ENDPOINT,
                    500,
                    "internal",
                    {},
                    io.BytesIO(raw),
                )
                if confirmed:
                    with patch(
                        "_shared.rpc.urlopen", side_effect=first
                    ) as opened, patch("_shared.rpc.time.sleep") as slept, self.assertRaises(
                        RpcError
                    ) as raised:
                        rpc(
                            SECRET_ENDPOINT,
                            "eth_call",
                            [],
                            attempts=3,
                            retry_delay=0.25,
                        )
                    self.assertEqual(opened.call_count, 1)
                    slept.assert_not_called()
                    self.assertIn('"code": -32000', str(raised.exception))
                    self.assertIn('"message": "execution reverted"', str(raised.exception))
                    self.assertNotIn('"data"', str(raised.exception))
                    for node in walk_exception_chain(raised.exception):
                        self.assertNotIsInstance(node, HTTPError)
                        self.assertNotIsInstance(node, TransportError)
                else:
                    with patch(
                        "_shared.rpc.urlopen",
                        side_effect=[first, Response(b'{"result":"0x14a34"}')],
                    ) as opened, patch("_shared.rpc.time.sleep") as slept:
                        result = rpc(
                            SECRET_ENDPOINT,
                            "eth_chainId",
                            [],
                            retry_delay=0.25,
                        )
                    self.assertEqual(result, "0x14a34")
                    self.assertEqual(opened.call_count, 2)
                    slept.assert_called_once_with(0.25)
                self.assertTrue(first.closed)
                self.assertTrue(first.fp.closed)

    def test_http_jsonrpc_short_message_echoing_secret_endpoint_is_redacted(
        self,
    ) -> None:
        raw_message = f"execution reverted at {SECRET_ENDPOINT}"
        expected_message = _without_absolute_urls(raw_message)
        self.assertIn("<redacted-url>", expected_message)
        self.assertNotIn(SECRET_ENDPOINT, expected_message)
        self.assertLessEqual(len(expected_message), MAX_RPC_ERROR_MESSAGE_CHARS)
        error_fields: dict[str, object] = {
            "code": -32000,
            "message": raw_message,
            "data": {"endpoint": SECRET_ENDPOINT, "credential": "leaked-query"},
        }
        expected_exc = (
            "RPC eth_call failed: "
            + json.dumps({"code": -32000, "message": expected_message}, sort_keys=True)
        )

        def assert_redacted_rpc_error(exc: BaseException) -> None:
            self.assertEqual(str(exc), expected_exc)
            payload = json.loads(str(exc).split("failed: ", 1)[1])
            self.assertEqual(payload, {"code": -32000, "message": expected_message})
            self.assertNotIn(RPC_ERROR_MESSAGE_TRUNCATION_INDICATOR, payload["message"])
            surface = exception_chain_text(exc)
            for fragment in SECRET_FRAGMENTS:
                self.assertNotIn(fragment, surface)
            self.assertNotIn("https://", str(exc))
            self.assertNotIn("http://", str(exc))
            self.assertNotIn('"data"', surface)
            for node in walk_exception_chain(exc):
                self.assertNotIsInstance(node, HTTPError)
                self.assertNotIsInstance(node, TransportError)

        first = jsonrpc_http_error(SECRET_ENDPOINT, 500, error_fields)
        with patch("_shared.rpc.urlopen", side_effect=first) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept, self.assertRaises(RpcError) as raised:
            rpc(SECRET_ENDPOINT, "eth_call", [], attempts=3, retry_delay=0.25)
        self.assertEqual(opened.call_count, 1)
        slept.assert_not_called()
        self.assertTrue(first.closed)
        self.assertTrue(first.fp.closed)
        assert_redacted_rpc_error(raised.exception)

        seen: list[tuple[str, str]] = []

        def open_response(request: object, **_kwargs: object) -> Response:
            url = str(getattr(request, "full_url"))
            method = request_method(request)
            seen.append((url, method))
            if method == "eth_chainId":
                return Response(b'{"result":"0x2105"}')
            raise jsonrpc_http_error(url, 500, error_fields)

        with patch("_shared.rpc.urlopen", side_effect=open_response) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept, self.assertRaises(RpcError) as raised:
            rpc_failover(
                "eth_call",
                [],
                endpoints=(SECRET_ENDPOINT, "https://second.local"),
                max_retries=3,
            )
        self.assertEqual(
            seen,
            [(SECRET_ENDPOINT, "eth_chainId"), (SECRET_ENDPOINT, "eth_call")],
        )
        self.assertEqual(opened.call_count, 2)
        slept.assert_not_called()
        assert_redacted_rpc_error(raised.exception)

    def test_http_jsonrpc_long_message_echoing_secret_endpoint_is_redacted_and_truncated(
        self,
    ) -> None:
        indicator = RPC_ERROR_MESSAGE_TRUNCATION_INDICATOR
        keep = max(MAX_RPC_ERROR_MESSAGE_CHARS - len(indicator), 0)
        placeholder = _without_absolute_urls(SECRET_ENDPOINT)
        self.assertEqual(placeholder, "<redacted-url>")
        # Spaces stop the absolute-URL matcher from consuming the filler.
        filler_len = MAX_RPC_ERROR_MESSAGE_CHARS - (2 * len(placeholder) + 2) + 1
        filler = "x" * filler_len
        long_message = f"{SECRET_ENDPOINT} {filler} {SECRET_ENDPOINT}"
        self.assertGreater(len(long_message), MAX_RPC_ERROR_MESSAGE_CHARS)
        redacted = _without_absolute_urls(long_message)
        self.assertEqual(redacted, f"{placeholder} {filler} {placeholder}")
        self.assertGreater(len(redacted), MAX_RPC_ERROR_MESSAGE_CHARS)
        expected_message = (redacted[:keep] + indicator)[:MAX_RPC_ERROR_MESSAGE_CHARS]
        self.assertEqual(len(expected_message), MAX_RPC_ERROR_MESSAGE_CHARS)
        self.assertTrue(expected_message.endswith(indicator))
        self.assertTrue(expected_message.startswith(placeholder))
        expected_exc = (
            "RPC eth_call failed: "
            + json.dumps({"code": -32000, "message": expected_message}, sort_keys=True)
        )

        def assert_redacted_truncated(exc: BaseException) -> None:
            self.assertEqual(str(exc), expected_exc)
            payload = json.loads(str(exc).split("failed: ", 1)[1])
            self.assertEqual(payload, {"code": -32000, "message": expected_message})
            self.assertLessEqual(len(payload["message"]), MAX_RPC_ERROR_MESSAGE_CHARS)
            surface = exception_chain_text(exc)
            for fragment in SECRET_FRAGMENTS:
                self.assertNotIn(fragment, surface)
            self.assertNotIn("https://", str(exc))
            self.assertNotIn("http://", str(exc))
            for node in walk_exception_chain(exc):
                self.assertNotIsInstance(node, HTTPError)
                self.assertNotIsInstance(node, TransportError)

        first = jsonrpc_http_error(
            SECRET_ENDPOINT,
            500,
            {"code": -32000, "message": long_message, "data": SECRET_ENDPOINT},
        )
        with patch("_shared.rpc.urlopen", side_effect=first) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept, self.assertRaises(RpcError) as raised:
            rpc(SECRET_ENDPOINT, "eth_call", [], attempts=3, retry_delay=0.25)
        self.assertEqual(opened.call_count, 1)
        slept.assert_not_called()
        self.assertTrue(first.closed)
        self.assertTrue(first.fp.closed)
        assert_redacted_truncated(raised.exception)

        expanding = ("http://a " * (MAX_RPC_ERROR_MESSAGE_CHARS // 9 + 10)).rstrip()
        expanded = _without_absolute_urls(expanding)
        self.assertGreater(len(expanded), len(expanding))
        self.assertGreater(len(expanded), MAX_RPC_ERROR_MESSAGE_CHARS)
        expansion_error = jsonrpc_http_error(
            SECRET_ENDPOINT,
            500,
            {"code": -32000, "message": expanding},
        )
        with patch("_shared.rpc.urlopen", side_effect=expansion_error) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept, self.assertRaises(RpcError) as raised:
            rpc(SECRET_ENDPOINT, "eth_call", [], attempts=3, retry_delay=0)
        self.assertEqual(opened.call_count, 1)
        slept.assert_not_called()
        payload = json.loads(str(raised.exception).split("failed: ", 1)[1])
        self.assertEqual(len(payload["message"]), MAX_RPC_ERROR_MESSAGE_CHARS)
        self.assertTrue(payload["message"].endswith(indicator))
        self.assertNotIn("http://", payload["message"])
        for fragment in SECRET_FRAGMENTS:
            self.assertNotIn(fragment, exception_chain_text(raised.exception))

    def test_http_jsonrpc_unicode_long_message_rendered_output_is_bounded(
        self,
    ) -> None:
        indicator = RPC_ERROR_MESSAGE_TRUNCATION_INDICATOR
        keep = max(MAX_RPC_ERROR_MESSAGE_CHARS - len(indicator), 0)
        char = "\u03b1"
        long_message = char * (MAX_RPC_ERROR_MESSAGE_CHARS + 8)
        expected_message = (char * keep) + indicator
        self.assertEqual(len(expected_message), MAX_RPC_ERROR_MESSAGE_CHARS)
        self.assertGreater(len(long_message), MAX_RPC_ERROR_MESSAGE_CHARS)
        expected_exc = (
            "RPC eth_call failed: "
            + json.dumps({"code": -32000, "message": expected_message}, sort_keys=True)
        )
        unbounded = json.dumps(
            {"code": -32000, "message": long_message}, sort_keys=True
        )
        bounded_json = json.dumps(
            {"code": -32000, "message": expected_message}, sort_keys=True
        )
        self.assertLess(len(bounded_json), len(unbounded))
        error = jsonrpc_http_error(
            SECRET_ENDPOINT,
            500,
            {"code": -32000, "message": long_message},
        )
        with patch("_shared.rpc.urlopen", side_effect=error) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept, self.assertRaises(RpcError) as raised:
            rpc(SECRET_ENDPOINT, "eth_call", [], attempts=3, retry_delay=0)
        self.assertEqual(opened.call_count, 1)
        slept.assert_not_called()
        self.assertTrue(error.closed)
        self.assertEqual(str(raised.exception), expected_exc)
        payload = json.loads(str(raised.exception).split("failed: ", 1)[1])
        self.assertEqual(payload, {"code": -32000, "message": expected_message})
        self.assertLessEqual(len(payload["message"]), MAX_RPC_ERROR_MESSAGE_CHARS)
        self.assertLessEqual(len(str(raised.exception)), len(expected_exc))
        self.assertLess(
            len(str(raised.exception)),
            len("RPC eth_call failed: ") + len(unbounded),
        )
        self.assertLessEqual(
            len(str(raised.exception)),
            256 + (MAX_RPC_ERROR_MESSAGE_CHARS * 12),
        )
        surface = exception_chain_text(raised.exception)
        for fragment in SECRET_FRAGMENTS:
            self.assertNotIn(fragment, surface)
        for node in walk_exception_chain(raised.exception):
            self.assertNotIsInstance(node, HTTPError)
            self.assertNotIsInstance(node, TransportError)

    def test_http_jsonrpc_oversized_body_with_secret_message_uses_http_classifier(
        self,
    ) -> None:
        envelope = json.dumps(
            {
                "jsonrpc": "2.0",
                "id": 1,
                "error": {
                    "code": -32000,
                    "message": f"execution reverted at {SECRET_ENDPOINT}",
                },
            }
        ).encode("utf-8")
        size = MAX_RPC_ERROR_BODY_BYTES + 1
        raw = envelope + (b" " * (size - len(envelope)))
        self.assertEqual(len(raw), size)
        self.assertGreater(len(raw), MAX_RPC_ERROR_BODY_BYTES)
        first = HTTPError(
            SECRET_ENDPOINT,
            500,
            "internal",
            {},
            io.BytesIO(raw),
        )
        with patch(
            "_shared.rpc.urlopen",
            side_effect=[first, Response(b'{"result":"0x14a34"}')],
        ) as opened, patch("_shared.rpc.time.sleep") as slept:
            result = rpc(SECRET_ENDPOINT, "eth_chainId", [], retry_delay=0.25)
        self.assertEqual(result, "0x14a34")
        self.assertEqual(opened.call_count, 2)
        slept.assert_called_once_with(0.25)
        self.assertTrue(first.closed)
        self.assertTrue(first.fp.closed)

    def test_caught_http_rpc_error_exposes_only_code_and_message(self) -> None:
        secret_marker = "rpc-error-data-secret-marker"
        error = jsonrpc_http_error(
            SECRET_ENDPOINT,
            500,
            {
                "code": -32000,
                "message": "execution reverted",
                "data": {"credential": secret_marker},
                "extra": secret_marker,
            },
        )
        with patch("_shared.rpc.urlopen", side_effect=error), self.assertRaises(
            RpcError
        ) as raised:
            rpc(SECRET_ENDPOINT, "eth_call", [], attempts=1)
        surface = exception_chain_text(raised.exception)
        self.assertIn('"code": -32000', surface)
        self.assertIn('"message": "execution reverted"', surface)
        self.assertNotIn(secret_marker, surface)
        self.assertNotIn('"data"', surface)
        self.assertNotIn('"extra"', surface)

    def test_matching_http_rpc_error_stops_on_target_method(self) -> None:
        seen: list[tuple[str, str]] = []

        def open_response(request: object, **_kwargs: object) -> Response:
            url = str(getattr(request, "full_url"))
            method = request_method(request)
            seen.append((url, method))
            if method == "eth_chainId":
                return Response(b'{"result":"0x2105"}')
            raise jsonrpc_http_error(
                url,
                500,
                {"code": -32000, "message": "execution reverted"},
            )

        with patch("_shared.rpc.urlopen", side_effect=open_response), patch(
            "_shared.rpc.time.sleep"
        ) as slept, self.assertRaisesRegex(RpcError, "execution reverted"):
            rpc_failover(
                "eth_call",
                [],
                endpoints=(SECRET_ENDPOINT, "https://second.local"),
                max_retries=3,
            )
        self.assertEqual(
            seen,
            [(SECRET_ENDPOINT, "eth_chainId"), (SECRET_ENDPOINT, "eth_call")],
        )
        slept.assert_not_called()

    def test_http_error_response_is_closed(self) -> None:
        error = HTTPError(
            SECRET_ENDPOINT,
            503,
            "unavailable",
            {},
            io.BytesIO(b"<html>unavailable</html>"),
        )
        with patch("_shared.rpc.urlopen", side_effect=error) as opened, patch(
            "_shared.rpc.time.sleep"
        ), self.assertRaises(RuntimeError):
            rpc(SECRET_ENDPOINT, "eth_call", [], attempts=1, retry_delay=0)
        self.assertEqual(opened.call_count, 1)
        self.assertTrue(error.closed)
        self.assertTrue(error.fp.closed)

    def test_http_500_without_jsonrpc_error_body_still_retries(self) -> None:
        first = HTTPError(
            SECRET_ENDPOINT,
            500,
            "internal",
            {},
            io.BytesIO(b"<html>fail</html>"),
        )
        responses = [first, Response(b'{"result":"0x14a34"}')]
        with patch("_shared.rpc.urlopen", side_effect=responses) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept:
            self.assertEqual(
                rpc(SECRET_ENDPOINT, "eth_chainId", [], retry_delay=0.25),
                "0x14a34",
            )
        self.assertEqual(opened.call_count, 2)
        slept.assert_called_once_with(0.25)
        self.assertTrue(first.closed)

    def test_http_501_retries_then_recovers_on_same_endpoint(self) -> None:
        responses = [
            HTTPError(SECRET_ENDPOINT, 501, "not implemented", {}, None),
            Response(b'{"result":"0x14a34"}'),
        ]
        with patch("_shared.rpc.urlopen", side_effect=responses) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept:
            self.assertEqual(
                rpc(SECRET_ENDPOINT, "eth_chainId", [], retry_delay=0.25),
                "0x14a34",
            )
        self.assertEqual(opened.call_count, 2)
        slept.assert_called_once_with(0.25)

    def test_http_599_retries_then_uses_same_endpoint(self) -> None:
        responses = [
            Response(b"{}", 599),
            Response(b'{"result":"0x2105"}'),
        ]
        with patch("_shared.rpc.urlopen", side_effect=responses) as opened, patch(
            "_shared.rpc.time.sleep"
        ) as slept:
            selected = select_working_base_rpc(
                endpoints=(SECRET_ENDPOINT,),
                max_retries=3,
            )
        self.assertEqual(selected, SECRET_ENDPOINT)
        self.assertEqual(opened.call_count, 2)
        slept.assert_called_once()

    def test_confirmed_json_rpc_error_never_retries_after_http_501(self) -> None:
        seen: list[tuple[str, str]] = []

        def open_response(request: object, **_kwargs: object) -> Response:
            url = str(getattr(request, "full_url"))
            method = request_method(request)
            seen.append((url, method))
            if url == SECRET_ENDPOINT and method == "eth_chainId" and seen == [
                (SECRET_ENDPOINT, "eth_chainId")
            ]:
                raise HTTPError(SECRET_ENDPOINT, 501, "not implemented", {}, None)
            if method == "eth_chainId":
                return Response(b'{"result":"0x2105"}')
            return Response(
                b'{"error":{"code":-32000,"message":"execution reverted"}}'
            )

        with patch("_shared.rpc.urlopen", side_effect=open_response), patch(
            "_shared.rpc.time.sleep"
        ), self.assertRaisesRegex(RpcError, "execution reverted"):
            rpc_failover(
                "eth_call",
                [{"to": "0x00"}],
                endpoints=(SECRET_ENDPOINT, "https://second.local"),
                max_retries=3,
            )
        self.assertEqual(
            seen,
            [
                (SECRET_ENDPOINT, "eth_chainId"),
                (SECRET_ENDPOINT, "eth_chainId"),
                (SECRET_ENDPOINT, "eth_call"),
            ],
        )
        self.assertNotIn("https://second.local", [url for url, _method in seen])

    def test_exception_chain_cannot_retain_secret_bearing_urls(self) -> None:
        error = HTTPError(SECRET_ENDPOINT, 501, "not implemented", {}, None)
        with patch("_shared.rpc.urlopen", side_effect=error), patch(
            "_shared.rpc.time.sleep"
        ), self.assertRaises(RuntimeError) as raised:
            rpc(SECRET_ENDPOINT, "eth_call", [], attempts=1, retry_delay=0)
        surface = exception_chain_text(raised.exception)
        for fragment in SECRET_FRAGMENTS:
            self.assertNotIn(fragment, surface)
        self.assertIn("HTTP 501", str(raised.exception))
        self.assertIn("https://rpc.example", str(raised.exception))
        for node in walk_exception_chain(raised.exception):
            self.assertNotIsInstance(node, HTTPError)
            filename = getattr(node, "filename", None)
            if isinstance(filename, str):
                for fragment in SECRET_FRAGMENTS:
                    self.assertNotIn(fragment, filename)

    def test_urlerror_context_cannot_retain_secret_bearing_urls(self) -> None:
        nested = URLError("timed out")
        nested.filename = SECRET_ENDPOINT
        with patch("_shared.rpc.urlopen", side_effect=nested), patch(
            "_shared.rpc.time.sleep"
        ), self.assertRaises(RuntimeError) as raised:
            rpc_failover(
                "eth_chainId",
                [],
                endpoints=(SECRET_ENDPOINT,),
                max_retries=1,
            )
        surface = exception_chain_text(raised.exception)
        for fragment in SECRET_FRAGMENTS:
            self.assertNotIn(fragment, surface)
        for node in walk_exception_chain(raised.exception):
            self.assertNotIsInstance(node, URLError)
            filename = getattr(node, "filename", None)
            if isinstance(filename, str):
                for fragment in SECRET_FRAGMENTS:
                    self.assertNotIn(fragment, filename)


if __name__ == "__main__":
    unittest.main()
