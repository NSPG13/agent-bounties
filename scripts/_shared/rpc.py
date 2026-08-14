"""JSON-RPC transport shared by local fork rehearsal scripts.

Provides a single-shot ``rpc()`` helper plus a :class:`FailoverJsonRpcTransport`
that validates chain ID, retries only bounded transport failures (HTTP 429 / 5xx)
with deterministic backoff, and advances across an ordered HTTPS endpoint list.
Confirmed JSON-RPC execution errors are never retried.
"""

from __future__ import annotations

import json
import time
from typing import Any, Callable, Sequence
from urllib.error import HTTPError, URLError
from urllib.parse import urlparse
from urllib.request import Request, urlopen

BASE_MAINNET_CHAIN_ID = 8453
_RETRIABLE_STATUS = frozenset({429} | set(range(500, 600)))


def _credential_free_https(url: str) -> bool:
    try:
        parsed = urlparse(url)
    except ValueError:
        return False
    return (
        parsed.scheme == "https"
        and bool(parsed.hostname)
        and parsed.username is None
        and parsed.password is None
    )


class RpcExecutionError(RuntimeError):
    """A confirmed JSON-RPC execution error; callers must never retry it."""

    def __init__(self, method: str, error: Any) -> None:
        self.method = method
        self.payload = error
        super().__init__(f"RPC {method} failed: {json.dumps(error, sort_keys=True)}")


class RetriableRpcError(RuntimeError):
    """A bounded transport failure (HTTP 429 or 5xx)."""

    def __init__(self, status: int) -> None:
        self.status = status
        super().__init__(f"RPC HTTP {status}")


class ChainIdMismatch(RuntimeError):
    def __init__(self, expected: int, observed: int) -> None:
        self.expected = expected
        self.observed = observed
        super().__init__(f"chain id mismatch: expected {expected}, observed {observed}")


def rpc(url: str, method: str, params: list[Any], request_id: int = 1) -> Any:
    payload = json.dumps(
        {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
    ).encode("utf-8")
    request = Request(url, data=payload, headers={"content-type": "application/json"})
    try:
        with urlopen(request, timeout=30) as response:
            body = json.load(response)
    except HTTPError as error:
        if error.code in _RETRIABLE_STATUS:
            raise RetriableRpcError(error.code) from error
        raise RuntimeError(f"RPC HTTP {error.code} for {method}") from error
    except URLError as error:
        raise RuntimeError(f"RPC transport failed for {method}: {error}") from error
    if body.get("error"):
        raise RpcExecutionError(method, body["error"])
    return body.get("result")


class FailoverJsonRpcTransport:
    """Ordered HTTPS endpoint failover with chain-ID validation and bounded retry."""

    def __init__(
        self,
        endpoints: Sequence[str],
        expected_chain_id: int = BASE_MAINNET_CHAIN_ID,
        max_retries: int = 3,
        base_backoff_ms: int = 200,
        max_backoff_ms: int = 5_000,
        sleep: Callable[[float], None] = time.sleep,
    ) -> None:
        cleaned: list[str] = []
        for endpoint in endpoints:
            endpoint = endpoint.strip()
            if not endpoint:
                continue
            if not _credential_free_https(endpoint):
                raise ValueError(f"endpoint must be a credential-free HTTPS URL: {endpoint!r}")
            cleaned.append(endpoint)
        if not cleaned:
            raise ValueError("at least one HTTPS endpoint is required")
        self.endpoints = cleaned
        self.expected_chain_id = expected_chain_id
        self.max_retries = max_retries
        self.base_backoff_ms = base_backoff_ms
        self.max_backoff_ms = max_backoff_ms
        self._sleep = sleep

    def _backoff_ms(self, attempt: int) -> int:
        return min(self.base_backoff_ms * (1 << min(attempt, 10)), self.max_backoff_ms)

    def _validate_chain_id(self, endpoint: str) -> None:
        result = rpc(endpoint, "eth_chainId", [], 0)
        try:
            observed = int(str(result), 16)
        except (TypeError, ValueError):
            observed = -1
        if observed != self.expected_chain_id:
            raise ChainIdMismatch(self.expected_chain_id, observed)

    def post(self, method: str, params: list[Any], request_id: int = 1) -> Any:
        """Post one request, failing over across endpoints on bounded failures."""
        last_error: BaseException | None = None
        for endpoint in self.endpoints:
            try:
                self._validate_chain_id(endpoint)
            except RuntimeError as error:
                last_error = error
                continue
            for attempt in range(self.max_retries + 1):
                if attempt:
                    self._sleep(self._backoff_ms(attempt - 1) / 1000.0)
                try:
                    return rpc(endpoint, method, params, request_id)
                except RpcExecutionError:
                    raise
                except RuntimeError as error:
                    last_error = error
        if last_error is not None:
            raise RuntimeError(f"failover exhausted all endpoints: {last_error}") from last_error
        raise RuntimeError("no HTTPS endpoints configured")
