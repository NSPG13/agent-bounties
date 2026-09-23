"""Deterministic JSON-RPC client with ordered endpoint failover."""
from __future__ import annotations

import json
import urllib.error
import urllib.request


class RpcFailoverError(RuntimeError):
    pass


def _post(endpoint: str, payload: dict, timeout: float) -> dict:
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        endpoint,
        data=data,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            body = json.loads(resp.read().decode("utf-8"))
    except (
        urllib.error.URLError,
        TimeoutError,
        ConnectionError,
        OSError,
        json.JSONDecodeError,
        UnicodeDecodeError,
    ) as exc:
        raise RpcFailoverError(f"transport error from {endpoint}: {exc}") from exc
    if not isinstance(body, dict):
        raise RpcFailoverError(f"malformed response from {endpoint}: {body!r}")
    if body.get("error") is not None:
        raise RpcFailoverError(f"rpc error from {endpoint}: {body['error']}")
    if "result" not in body:
        raise RpcFailoverError(f"malformed response from {endpoint}: {body!r}")
    return body


def rpc_call(
    endpoints: list[str],
    method: str,
    params: list | None = None,
    timeout: float = 5.0,
    request_id: int = 1,
) -> tuple[str, dict]:
    """Return ``(endpoint, body)`` from the first successful JSON-RPC call.

    Endpoints are tried in the given order. Transport failures, malformed
    bodies, and JSON-RPC ``error`` objects are not success; control moves to
    the next endpoint. ``RpcFailoverError`` is raised only when the list is
    empty or every endpoint has failed.
    """
    if not endpoints:
        raise RpcFailoverError("no endpoints configured")
    payload = {
        "jsonrpc": "2.0",
        "id": request_id,
        "method": method,
        "params": params or [],
    }
    errors: list[str] = []
    for endpoint in endpoints:
        try:
            return endpoint, _post(endpoint, payload, timeout)
        except RpcFailoverError as exc:
            errors.append(str(exc))
    raise RpcFailoverError("all endpoints failed: " + " | ".join(errors))
