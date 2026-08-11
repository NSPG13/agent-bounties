"""JSON-RPC transport with retry-safe Base RPC failover and chain-id validation."""

from __future__ import annotations

import json
import time
from typing import Any, Sequence
from urllib.error import URLError
from urllib.request import Request, urlopen

# Ordered Base mainnet HTTPS endpoints
BASE_RPC_ENDPOINTS: Sequence[str] = (
    "https://mainnet.base.org",
    "https://base.llamarpc.com",
    "https://base-rpc.publicnode.com",
    "https://1rpc.io/base",
    "https://base.drpc.org",
)

BASE_CHAIN_ID = 8453
MAX_RETRIES = 3
INITIAL_BACKOFF_SECONDS = 1.0


def _backoff(attempt: int) -> float:
    """Deterministic backoff: 1s, 2s, 4s."""
    return INITIAL_BACKOFF_SECONDS * (2 ** (attempt - 1))


def _retryable_status(code: int) -> bool:
    """HTTP 429 (rate-limit) and 5xx are retryable."""
    return code == 429 or 500 <= code < 600


def rpc(
    url: str,
    method: str,
    params: list[Any],
    request_id: int = 1,
) -> Any:
    """Single-endpoint JSON-RPC call (preserved for backward compat)."""
    payload = json.dumps(
        {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
    ).encode("utf-8")
    request = Request(url, data=payload, headers={"content-type": "application/json"})
    try:
        with urlopen(request, timeout=30) as response:
            body = json.load(response)
    except URLError as error:
        raise RuntimeError(f"RPC transport failed for {method}: {error}") from error
    if body.get("error"):
        raise RuntimeError(
            f"RPC {method} failed: {json.dumps(body['error'], sort_keys=True)}"
        )
    return body.get("result")


def rpc_failover(
    method: str,
    params: list[Any],
    request_id: int = 1,
    endpoints: Sequence[str] | None = None,
    max_retries: int = MAX_RETRIES,
) -> Any:
    """JSON-RPC call with ordered HTTPS endpoint failover and retry.

    Validates Base chain ID 8453 before reads. Retries only bounded
    transport errors, HTTP 429, and HTTP 5xx with deterministic backoff.
    Preserves JSON-RPC execution errors and never exposes credentials.
    """
    if endpoints is None:
        endpoints = BASE_RPC_ENDPOINTS

    last_error: Exception | None = None

    for endpoint in endpoints:
        chain_id = _validate_chain(endpoint)
        if chain_id is None:
            continue  # skip endpoints that don't validate

        for attempt in range(1, max_retries + 1):
            try:
                return _rpc_call(endpoint, method, params, request_id)
            except _TransportError as e:
                last_error = e
                if e.retryable:
                    if attempt < max_retries:
                        time.sleep(_backoff(attempt))
                        continue
                break  # non-retryable or exhausted retries for this endpoint
            except _RpcError:
                raise  # JSON-RPC execution errors: do NOT retry

    raise RuntimeError(
        f"RPC failover exhausted for {method} across {len(endpoints)} endpoints"
    ) from last_error


class _TransportError(Exception):
    def __init__(self, message: str, retryable: bool = True):
        super().__init__(message)
        self.retryable = retryable


class _RpcError(Exception):
    pass


def _validate_chain(endpoint: str) -> int | None:
    """Call eth_chainId and verify Base chain ID 8453. Returns chain_id or None."""
    try:
        result = _rpc_call(endpoint, "eth_chainId", [], 1)
        chain_id = int(result, 16) if isinstance(result, str) else result
        if chain_id == BASE_CHAIN_ID:
            return chain_id
    except Exception:
        pass
    return None


def _rpc_call(endpoint: str, method: str, params: list[Any], request_id: int) -> Any:
    """Low-level call: transport errors become _TransportError, RPC errors become _RpcError."""
    payload = json.dumps(
        {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
    ).encode("utf-8")
    request = Request(
        endpoint, data=payload, headers={"content-type": "application/json"}
    )
    try:
        with urlopen(request, timeout=30) as response:
            status = response.getcode()
            body = json.load(response)
    except URLError as error:
        raise _TransportError(
            f"RPC transport failed for {method}: {error}", retryable=True
        ) from error

    # HTTP status check before parsing RPC result
    if _retryable_status(status):
        raise _TransportError(
            f"HTTP {status} from {endpoint}", retryable=True
        )

    if body.get("error"):
        raise _RpcError(
            f"RPC {method} failed: {json.dumps(body['error'], sort_keys=True)}"
        )

    return body.get("result")
