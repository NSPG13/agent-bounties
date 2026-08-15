"""JSON-RPC transport with retry-safe Base failover and chain validation."""

from __future__ import annotations

import json
import time
from typing import Any, Sequence
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit
from urllib.request import Request, urlopen


BASE_CHAIN_ID = 8453
BASE_RPC_ENDPOINTS: Sequence[str] = (
    "https://base.drpc.org",
    "https://base-rpc.publicnode.com",
    "https://mainnet.base.org",
)
MAX_RETRIES = 3
INITIAL_BACKOFF_SECONDS = 0.5
RETRYABLE_HTTP_STATUS = frozenset({408, 429, 500, 502, 503, 504})
RETRYABLE_TRANSPORT_MARKERS = (
    "http error 408",
    "http error 429",
    "http error 500",
    "http error 502",
    "http error 503",
    "http error 504",
    "http 408",
    "http 429",
    "http 500",
    "http 502",
    "http 503",
    "http 504",
    "over rate limit",
    "connection refused",
    "connection reset",
    "connection aborted",
    "name or service not known",
    "temporary failure in name resolution",
    "timed out",
    "timeout",
)


class TransportError(RuntimeError):
    """HTTP or network failure, classified for bounded retry."""

    def __init__(self, message: str, *, retryable: bool) -> None:
        super().__init__(message)
        self.retryable = retryable


class RpcError(RuntimeError):
    """Confirmed JSON-RPC response error; never retry or fail over."""


def _backoff(attempt: int) -> float:
    return INITIAL_BACKOFF_SECONDS * (2 ** max(attempt - 1, 0))


def _redact_endpoint(endpoint: str) -> str:
    """Return only scheme and host so paths, queries, and credentials stay secret."""
    try:
        parsed = urlsplit(endpoint)
        if not parsed.scheme or not parsed.hostname:
            return "<redacted-endpoint>"
        port = f":{parsed.port}" if parsed.port is not None else ""
        return f"{parsed.scheme}://{parsed.hostname}{port}"
    except (ValueError, TypeError):
        return "<redacted-endpoint>"


def redact_rpc_endpoint(endpoint: str) -> str:
    """Public credential-safe endpoint label for reports and logs."""
    return _redact_endpoint(endpoint)


def _is_https(endpoint: str) -> bool:
    try:
        parsed = urlsplit(endpoint)
        return parsed.scheme.lower() == "https" and bool(parsed.hostname)
    except ValueError:
        return False


def is_retryable_transport_output(value: object) -> bool:
    """Classify subprocess output without treating JSON-RPC errors as transport."""
    text = str(value).lower()
    if "json-rpc" in text and "error" in text and "http" not in text:
        return False
    return any(marker in text for marker in RETRYABLE_TRANSPORT_MARKERS)


def _rpc_call(
    endpoint: str,
    method: str,
    params: list[Any],
    request_id: int,
    *,
    timeout: float = 30,
) -> Any:
    payload = json.dumps(
        {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
    ).encode("utf-8")
    request = Request(
        endpoint,
        data=payload,
        headers={
            "content-type": "application/json",
            "user-agent": "agent-bounties-rpc/1",
        },
    )
    try:
        with urlopen(request, timeout=timeout) as response:
            status = response.getcode() if hasattr(response, "getcode") else 200
            status = 200 if status is None else int(status)
            if status in RETRYABLE_HTTP_STATUS:
                raise TransportError(
                    f"HTTP {status} from {_redact_endpoint(endpoint)}",
                    retryable=True,
                )
            if status >= 400:
                raise TransportError(
                    f"HTTP {status} from {_redact_endpoint(endpoint)}",
                    retryable=False,
                )
            body = json.load(response)
    except HTTPError as error:
        code = int(error.code)
        raise TransportError(
            f"HTTP {code} from {_redact_endpoint(endpoint)}",
            retryable=code in RETRYABLE_HTTP_STATUS,
        ) from error
    except (TimeoutError, URLError, OSError) as error:
        raise TransportError(
            f"RPC transport failed for {method} at {_redact_endpoint(endpoint)}: {error}",
            retryable=True,
        ) from error
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise TransportError(
            f"RPC response was invalid for {method} at {_redact_endpoint(endpoint)}",
            retryable=True,
        ) from error

    if not isinstance(body, dict):
        raise TransportError(
            f"RPC response was not an object for {method} at {_redact_endpoint(endpoint)}",
            retryable=True,
        )
    if body.get("error"):
        raise RpcError(
            f"RPC {method} failed: {json.dumps(body['error'], sort_keys=True)}"
        )
    return body.get("result")


def rpc(
    url: str,
    method: str,
    params: list[Any],
    request_id: int = 1,
    *,
    attempts: int = 3,
    timeout: float = 30,
    retry_delay: float = 0.5,
) -> Any:
    """Call one endpoint with bounded transport retry and no endpoint failover."""
    if attempts < 1:
        raise ValueError("RPC attempts must be positive")
    if timeout <= 0 or retry_delay < 0:
        raise ValueError("RPC timeout must be positive and retry delay nonnegative")

    last_error: TransportError | None = None
    for attempt in range(1, attempts + 1):
        try:
            return _rpc_call(url, method, params, request_id, timeout=timeout)
        except RpcError:
            raise
        except TransportError as error:
            last_error = error
            if not error.retryable or attempt >= attempts:
                break
            time.sleep(retry_delay * (2 ** (attempt - 1)))
    assert last_error is not None
    raise RuntimeError(f"RPC transport failed for {method}: {last_error}") from last_error


def _parse_chain_id(result: object, endpoint: str) -> int:
    try:
        if isinstance(result, str):
            return int(result, 16) if result.startswith("0x") else int(result)
        return int(result)
    except (TypeError, ValueError) as error:
        raise RpcError(
            f"RPC eth_chainId returned malformed data from {_redact_endpoint(endpoint)}"
        ) from error


def _validate_chain(endpoint: str) -> int:
    """Return the reported chain ID while preserving transport and RPC errors."""
    return _parse_chain_id(_rpc_call(endpoint, "eth_chainId", [], 1), endpoint)


def ordered_base_rpc_endpoints(
    preferred: str | None = None,
    endpoints: Sequence[str] | None = None,
) -> tuple[str, ...]:
    ordered: list[str] = []
    preferred = (preferred or "").strip()
    if preferred:
        ordered.append(preferred)
    for endpoint in endpoints if endpoints is not None else BASE_RPC_ENDPOINTS:
        value = str(endpoint).strip()
        if value and value not in ordered:
            ordered.append(value)
    return tuple(ordered)


def _validate_retry_count(max_retries: int) -> None:
    if max_retries < 1:
        raise ValueError("RPC max_retries must be positive")


def select_working_base_rpc(
    preferred: str | None = None,
    endpoints: Sequence[str] | None = None,
    max_retries: int = MAX_RETRIES,
) -> str:
    """Return only the exact HTTPS endpoint that reports Base chain 8453."""
    _validate_retry_count(max_retries)
    ordered = ordered_base_rpc_endpoints(preferred, endpoints)
    if not ordered:
        raise RuntimeError("RPC failover exhausted: no endpoints configured")

    last_error: BaseException | None = None
    for endpoint in ordered:
        if not _is_https(endpoint):
            last_error = RuntimeError(
                f"refusing non-HTTPS endpoint {_redact_endpoint(endpoint)}"
            )
            continue
        for attempt in range(1, max_retries + 1):
            try:
                chain_id = _validate_chain(endpoint)
            except RpcError:
                raise
            except TransportError as error:
                last_error = error
                if not error.retryable or attempt >= max_retries:
                    break
                time.sleep(_backoff(attempt))
                continue
            if chain_id == BASE_CHAIN_ID:
                return endpoint
            last_error = RuntimeError(
                f"wrong chain {chain_id} on {_redact_endpoint(endpoint)}"
            )
            break
    raise RuntimeError(f"RPC failover exhausted for eth_chainId: {last_error}") from last_error


def rpc_failover(
    method: str,
    params: list[Any],
    request_id: int = 1,
    endpoints: Sequence[str] | None = None,
    max_retries: int = MAX_RETRIES,
    *,
    preferred: str | None = None,
) -> Any:
    """Execute one read with Base validation, bounded retry, and endpoint failover."""
    _validate_retry_count(max_retries)
    ordered = ordered_base_rpc_endpoints(preferred, endpoints)
    if not ordered:
        raise RuntimeError("RPC failover exhausted: no endpoints configured")

    last_error: BaseException | None = None
    for endpoint in ordered:
        try:
            selected = select_working_base_rpc(
                endpoints=(endpoint,), max_retries=max_retries
            )
        except RpcError:
            raise
        except (TransportError, RuntimeError) as error:
            last_error = error
            continue
        if method == "eth_chainId":
            return hex(BASE_CHAIN_ID)
        for attempt in range(1, max_retries + 1):
            try:
                return _rpc_call(selected, method, params, request_id)
            except RpcError:
                raise
            except TransportError as error:
                last_error = error
                if not error.retryable or attempt >= max_retries:
                    break
                time.sleep(_backoff(attempt))
    raise RuntimeError(f"RPC failover exhausted for {method}: {last_error}") from last_error
