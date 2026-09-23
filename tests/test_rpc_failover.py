"""Pinned rpc-failover fixture: skip transport and JSON-RPC errors in order."""
from unittest.mock import patch
import urllib.error

from src.rpc_failover import RpcFailoverError, rpc_call


class _FakeResponse:
    def __init__(self, payload: bytes):
        self._payload = payload

    def read(self):
        return self._payload

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return False


def _fake_urlopen(req, timeout=None):
    url = req.full_url
    if url == "http://primary.local":
        raise urllib.error.URLError("connection refused")
    if url == "http://busy.local":
        return _FakeResponse(
            b'{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"busy"}}'
        )
    if url == "http://secondary.local":
        return _FakeResponse(b'{"jsonrpc":"2.0","id":1,"result":"0x2105"}')
    raise AssertionError(f"unexpected endpoint {url}")


def test_failover_skips_transport_and_rpc_errors_deterministically():
    with patch("src.rpc_failover.urllib.request.urlopen", side_effect=_fake_urlopen):
        endpoint, body = rpc_call(
            [
                "http://primary.local",
                "http://busy.local",
                "http://secondary.local",
            ],
            "eth_chainId",
        )
    assert endpoint == "http://secondary.local"
    assert body["result"] == "0x2105"


def test_all_endpoints_failed_raises():
    with patch("src.rpc_failover.urllib.request.urlopen", side_effect=_fake_urlopen):
        try:
            rpc_call(["http://primary.local", "http://busy.local"], "eth_chainId")
        except RpcFailoverError as exc:
            message = str(exc)
            assert "all endpoints failed" in message
            assert "primary.local" in message
            assert "busy.local" in message
            return
    raise AssertionError("expected RpcFailoverError")
