"""JSON-RPC transport shared by local fork rehearsal scripts."""

from __future__ import annotations

import json
from typing import Any
from urllib.error import URLError
from urllib.request import Request, urlopen


def rpc(url: str, method: str, params: list[Any], request_id: int = 1) -> Any:
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
        raise RuntimeError(f"RPC {method} failed: {json.dumps(body['error'], sort_keys=True)}")
    return body.get("result")
