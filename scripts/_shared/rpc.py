"""JSON-RPC transport shared by local fork rehearsal scripts."""

from __future__ import annotations

import json
import time
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


RETRYABLE_HTTP_STATUS = frozenset({408, 429, 500, 502, 503, 504})


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
    if attempts < 1:
        raise ValueError("RPC attempts must be positive")
    if timeout <= 0 or retry_delay < 0:
        raise ValueError("RPC timeout must be positive and retry delay nonnegative")
    payload = json.dumps(
        {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
    ).encode("utf-8")
    request = Request(
        url,
        data=payload,
        headers={
            "content-type": "application/json",
            "user-agent": "agent-bounties-release-tooling/1",
        },
    )
    last_error: BaseException | None = None
    for attempt in range(attempts):
        try:
            with urlopen(request, timeout=timeout) as response:
                body = json.load(response)
            break
        except HTTPError as error:
            if error.code not in RETRYABLE_HTTP_STATUS:
                raise RuntimeError(f"RPC transport failed for {method}: {error}") from error
            last_error = error
        except (TimeoutError, URLError, OSError) as error:
            last_error = error
        if attempt + 1 < attempts:
            time.sleep(retry_delay * (2**attempt))
    else:
        assert last_error is not None
        raise RuntimeError(f"RPC transport failed for {method}: {last_error}") from last_error
    if body.get("error"):
        raise RuntimeError(f"RPC {method} failed: {json.dumps(body['error'], sort_keys=True)}")
    return body.get("result")
