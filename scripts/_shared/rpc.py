"""JSON-RPC transport shared by local fork rehearsal scripts, with retry-safe
Base RPC failover and chain-id validation.

The transport accepts an ordered HTTPS Base endpoint list, validates chain ID
8453 before using an endpoint, and retries only bounded transport failures,
HTTP 429, and HTTP 5xx responses with deterministic exponential backoff.
Confirmed JSON-RPC execution errors are never retried, and endpoint
credentials are never surfaced in errors.
"""

from __future__ import annotations

import json
import time
from typing import Any, Sequence
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

# Ordered Base mainnet HTTPS endpoints. Credentials must never appear here.
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
    """Transport-level failure (network, HTTP 429, or HTTP 5xx).

    ``retryable`` is True for bounded transport failures that may succeed on
    retry, and False for confirmed non-retryable HTTP errors.
    """

    def __init__(self, message: str, *, retryable: bool = True) -> None:
        super().__init__(message)
        self.retryable = retryable


class RpcError(RuntimeError):
    """Confirmed JSON-RPC execution error. Never retried."""


def _backoff(attempt: int) -> float:
    """Deterministic exponential backoff: 1s, 2s, 4s, ..."""
    return INITIAL_BACKOFF_SECONDS * (2 ** max(attempt - 1, 0))


def _retryable_status(code: int) -> bool:
    """HTTP 429 and 5xx responses are retryable; everything else is not."""
    return code == 429 or 500 <= code < 600


def _redact_endpoint(endpoint: str) -> str:
    """Strip any credentials an endpoint URL might carry before logging."""
    if "@" not in endpoint:
        return endpoint
    try:
        scheme, rest = endpoint.split("://", 1)
        host = rest.split("@", 1)[-1]
        return f"{scheme}://{host}"
    except Exception:  # noqa: BLE001 - never crash on malformed endpoints
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
    """Low-level JSON-RPC call separating transport from execution errors."""
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
                except Exception:  # noqa: BLE001 - fall back to 200
                    status = 200
            body = json.load(response)
    except HTTPError as error:
        code = int(getattr(error, "code", 0) or 0)
        raise TransportError(
            f"HTTP {code} from {_redact_endpoint(endpoint)}",
            retryable=_retryable_status(code),
        ) from error
    except URLError as error:
        raise TransportError(
            f"RPC transport failed for {method}: {error}", retryable=True
        ) from error

    if _retryable_status(int(status or 0)):
        raise TransportError(
            f"HTTP {status} from {_redact_endpoint(endpoint)}", retryable=True
        )
    if body.get("error"):
        raise RpcError(
            f"RPC {method} failed: {json.dumps(body['error'], sort_keys=True)}"
        )
    return body.get("result")


def _chain_id_of(result: Any) -> int | None:
    """Coerce a chain-id RPC result (hex string or int) to an int."""
    if isinstance(result, str):
        try:
            return int(result, 16) if result.startswith("0x") else int(result)
        except ValueError:
            return None
    if isinstance(result, int):
        return result
    return None


def _validate_chain(endpoint: str) -> int | None:
    """Return the chain id when ``endpoint`` reports Base (8453), else None.

    Endpoints that fail the probe or report another chain are never used.
    """
    try:
        result = _rpc_call(endpoint, "eth_chainId", [], 1)
    except Exception:  # noqa: BLE001 - probe failures skip the endpoint
        return None
    chain_id = _chain_id_of(result)
    return chain_id if chain_id == BASE_CHAIN_ID else None


def rpc_failover(
    method: str,
    params: list[Any],
    request_id: int = 1,
    endpoints: Sequence[str] | None = None,
    max_retries: int = MAX_RETRIES,
) -> Any:
    """JSON-RPC with ordered HTTPS failover, chain-id gate, and bounded retries.

    Each endpoint must be HTTPS and must report chain ID 8453 before use.
    Bounded transport failures, HTTP 429, and HTTP 5xx are retried with
    deterministic backoff; confirmed JSON-RPC execution errors propagate
    immediately and are never retried. Endpoint credentials are never logged.
    """
    if endpoints is None:
        endpoints = BASE_RPC_ENDPOINTS
    if not endpoints:
        raise RuntimeError("RPC failover exhausted: no endpoints configured")

    last_error: Exception | None = None
    for endpoint in endpoints:
        endpoint = str(endpoint)
        if not endpoint.lower().startswith("https://"):
            last_error = RuntimeError(
                f"refusing non-HTTPS endpoint {_redact_endpoint(endpoint)}"
            )
            continue
        if _validate_chain(endpoint) != BASE_CHAIN_ID:
            last_error = RuntimeError(
                f"wrong chain on {_redact_endpoint(endpoint)}"
            )
            continue

        for attempt in range(1, max_retries + 1):
            try:
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
            except Exception as error:  # noqa: BLE001 - terminal for this endpoint
                last_error = error
                break

    raise RuntimeError(
        f"RPC failover exhausted for {method}: {last_error}"
    ) from last_error
