"""JSON-RPC transport with retry-safe Base RPC failover and chain-id validation."""

from __future__ import annotations

import json
import time
from typing import Any, Sequence
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

# Ordered Base mainnet HTTPS endpoints (no credentials in URLs).
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


class TransportError(RuntimeError):
    """Retryable transport / HTTP 429 / HTTP 5xx failure."""

    def __init__(self, message: str, *, retryable: bool = True) -> None:
        super().__init__(message)
        self.retryable = retryable


class RpcError(RuntimeError):
    """Non-retryable JSON-RPC execution error."""


# Public aliases used by tests and callers.
_TransportError = TransportError
_RpcError = RpcError


def _backoff(attempt: int) -> float:
    """Deterministic exponential backoff: 1s, 2s, 4s, ..."""
    return INITIAL_BACKOFF_SECONDS * (2 ** max(attempt - 1, 0))


def _retryable_status(code: int) -> bool:
    return code == 429 or 500 <= code < 600


def _redact_endpoint(endpoint: str) -> str:
    """Never surface credentials if an endpoint URL ever carries them."""
    if "@" not in endpoint:
        return endpoint
    try:
        scheme, rest = endpoint.split("://", 1)
        host = rest.split("@", 1)[-1]
        return f"{scheme}://{host}"
    except Exception:  # noqa: BLE001
        return "<redacted-endpoint>"


def rpc(url: str, method: str, params: list[Any], request_id: int = 1) -> Any:
    """Single-endpoint JSON-RPC call (preserved for backward compatibility)."""
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


def _rpc_call(endpoint: str, method: str, params: list[Any], request_id: int) -> Any:
    """Low-level call with transport vs execution error separation."""
    payload = json.dumps(
        {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
    ).encode("utf-8")
    request = Request(
        endpoint, data=payload, headers={"content-type": "application/json"}
    )
    try:
        with urlopen(request, timeout=30) as response:
            status = getattr(response, "status", None)
            if status is None and hasattr(response, "getcode"):
                try:
                    status = response.getcode()
                except Exception:  # noqa: BLE001
                    status = 200
            if status is None:
                status = 200
            body = json.load(response)
    except HTTPError as error:
        code = int(getattr(error, "code", 0) or 0)
        if _retryable_status(code):
            raise TransportError(
                f"HTTP {code} from {_redact_endpoint(endpoint)}",
                retryable=True,
            ) from error
        raise TransportError(
            f"HTTP {code} from {_redact_endpoint(endpoint)}",
            retryable=False,
        ) from error
    except URLError as error:
        raise TransportError(
            f"RPC transport failed for {method}: {error}",
            retryable=True,
        ) from error

    if _retryable_status(int(status or 0)):
        raise TransportError(
            f"HTTP {status} from {_redact_endpoint(endpoint)}",
            retryable=True,
        )

    if body.get("error"):
        raise RpcError(
            f"RPC {method} failed: {json.dumps(body['error'], sort_keys=True)}"
        )
    return body.get("result")


def _validate_chain(endpoint: str) -> int | None:
    """Return chain id when endpoint reports Base (8453); otherwise None."""
    try:
        result = _rpc_call(endpoint, "eth_chainId", [], 1)
        if isinstance(result, str):
            chain_id = int(result, 16) if result.startswith("0x") else int(result)
        else:
            chain_id = int(result)
        if chain_id == BASE_CHAIN_ID:
            return chain_id
    except Exception:  # noqa: BLE001 - skip bad endpoints
        return None
    return None


def rpc_failover(
    method: str,
    params: list[Any],
    request_id: int = 1,
    endpoints: Sequence[str] | None = None,
    max_retries: int = MAX_RETRIES,
) -> Any:
    """JSON-RPC with ordered HTTPS failover, chain-id gate, and bounded retries.

    Retries only transport failures, HTTP 429, and HTTP 5xx. JSON-RPC execution
    errors propagate immediately. Endpoint credentials are never logged.
    """
    if endpoints is None:
        endpoints = BASE_RPC_ENDPOINTS
    if not endpoints:
        raise RuntimeError("RPC failover exhausted: no endpoints configured")

    last_error: Exception | None = None
    for endpoint in endpoints:
        if not str(endpoint).lower().startswith("https://"):
            last_error = RuntimeError(
                f"refusing non-HTTPS endpoint {_redact_endpoint(str(endpoint))}"
            )
            continue
        chain_id = _validate_chain(endpoint)
        if chain_id != BASE_CHAIN_ID:
            last_error = RuntimeError(
                f"wrong chain on {_redact_endpoint(endpoint)}"
            )
            continue

        for attempt in range(1, max_retries + 1):
            try:
                # After chain validation, perform the requested method.
                if method == "eth_chainId":
                    return hex(BASE_CHAIN_ID)
                return _rpc_call(endpoint, method, params, request_id)
            except RpcError:
                raise
            except TransportError as error:
                last_error = error
                if not error.retryable or attempt >= max_retries:
                    break
                time.sleep(_backoff(attempt))
            except Exception as error:  # noqa: BLE001
                last_error = error
                break

    raise RuntimeError(
        f"RPC failover exhausted for {method}: {last_error}"
    ) from last_error
