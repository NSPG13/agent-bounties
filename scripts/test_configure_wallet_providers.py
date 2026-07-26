#!/usr/bin/env python3
"""Deterministic tests for Coinbase public project configuration and CORS gates."""

from __future__ import annotations

import importlib.util
import os
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
MODULE_PATH = ROOT / "scripts" / "configure-wallet-providers.py"
SPEC = importlib.util.spec_from_file_location("configure_wallet_providers", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load configure-wallet-providers.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)

ORIGIN = "https://agentbounties.app"
PROJECT = "9dfed88a-0b37-47e8-b867-96f1dfd0d4ee"


class FakeResponse:
    def __init__(self, status: int, headers: dict[str, str], body: bytes = b"") -> None:
        self.status = status
        self.headers = headers
        self._body = body

    def __enter__(self) -> "FakeResponse":
        return self

    def __exit__(self, *_args: object) -> None:
        return None

    def read(self) -> bytes:
        return self._body


def requested_header(request: object, name: str) -> str:
    headers = getattr(request, "headers", {})
    for key, value in headers.items():
        if str(key).lower() == name.lower():
            return str(value)
    return ""


def good_opener(calls: list[tuple[str, str, str]]):
    def open_request(request: object, *, timeout: float) -> FakeResponse:
        del timeout
        url = str(getattr(request, "full_url"))
        method = str(request.get_method())
        requested = requested_header(request, "Access-Control-Request-Headers")
        calls.append((method, url, requested))
        common = {
            "Access-Control-Allow-Origin": ORIGIN,
            "Access-Control-Allow-Credentials": "true",
        }
        if method == "GET" and url.endswith(f"/projects/{PROJECT}/config"):
            return FakeResponse(
                200,
                {**common, "Content-Type": "application/json"},
                b'{"projectId":"redacted","walletsEnabled":true}',
            )
        if method == "OPTIONS" and url.endswith(f"/projects/{PROJECT}/auth/init"):
            return FakeResponse(
                204,
                {
                    **common,
                    "Access-Control-Allow-Methods": "POST",
                    "Access-Control-Allow-Headers": requested,
                },
            )
        raise AssertionError(f"unexpected Coinbase request: {method} {url}")

    return open_request


def expect_exit(callback, fragment: str) -> None:
    try:
        callback()
    except SystemExit as error:
        if fragment not in str(error):
            raise AssertionError(f"expected {fragment!r}, received {error!s}") from error
    else:
        raise AssertionError(f"expected SystemExit containing {fragment!r}")


def test_verified_routes() -> None:
    calls: list[tuple[str, str, str]] = []
    result = MODULE.verify_production_origin(PROJECT, ORIGIN, opener=good_opener(calls))
    assert result["project_config_status"] == 200
    assert len(result["preflights"]) == 2
    assert calls[0] == (
        "GET",
        f"https://api.cdp.coinbase.com/platform/v2/embedded-wallet-api/projects/{PROJECT}/config",
        "",
    )
    assert calls[1][0] == "OPTIONS" and calls[1][1].endswith("/auth/init")
    assert set(calls[1][2].split(",")) == {"content-type", "x-idempotency-key"}
    assert calls[2][0] == "OPTIONS" and calls[2][1].endswith("/auth/init")
    assert set(calls[2][2].split(",")) == {"content-type", "x-wallet-auth"}
    assert all("/embedded-wallet-api/projects/" not in url.replace("/platform/v2/embedded-wallet-api/projects/", "") for _, url, _ in calls)


def test_missing_origin_fails_closed() -> None:
    def opener(request: object, *, timeout: float) -> FakeResponse:
        del request, timeout
        return FakeResponse(200, {"Access-Control-Allow-Credentials": "true"}, b"{}")

    expect_exit(
        lambda: MODULE.verify_production_origin(PROJECT, ORIGIN, opener=opener),
        "project configuration browser-origin check",
    )


def test_missing_link_header_fails_closed() -> None:
    calls = 0

    def opener(request: object, *, timeout: float) -> FakeResponse:
        nonlocal calls
        del timeout
        calls += 1
        common = {
            "Access-Control-Allow-Origin": ORIGIN,
            "Access-Control-Allow-Credentials": "true",
        }
        if request.get_method() == "GET":
            return FakeResponse(200, {**common, "Content-Type": "application/json"}, b"{}")
        requested = requested_header(request, "Access-Control-Request-Headers")
        allowed = "content-type,x-idempotency-key" if calls == 2 else "content-type"
        return FakeResponse(
            204,
            {
                **common,
                "Access-Control-Allow-Methods": "POST",
                "Access-Control-Allow-Headers": allowed,
            },
        )

    expect_exit(
        lambda: MODULE.verify_production_origin(PROJECT, ORIGIN, opener=opener),
        "authenticated method linking preflight",
    )


def test_origin_and_default_configuration() -> None:
    assert MODULE.normalized_https_origin("https://agentbounties.app/") == ORIGIN
    for invalid in (
        "http://agentbounties.app",
        "https://agentbounties.app/path",
        "https://user@agentbounties.app",
        "https://agentbounties.app?query=1",
        "https://agentbounties.app#fragment",
        "https://agentbounties.app:bad",
    ):
        expect_exit(lambda value=invalid: MODULE.normalized_https_origin(value), "--verify-origin")

    clean = {
        "GITHUB_ACTIONS": "true",
        "GITHUB_REPOSITORY": MODULE.CANONICAL_REPOSITORY,
    }
    with patch.dict(os.environ, clean, clear=True):
        assert MODULE.configured_default() == MODULE.CANONICAL_PROJECT_ID
    with patch.dict(os.environ, {"GITHUB_ACTIONS": "true", "GITHUB_REPOSITORY": "other/repo"}, clear=True):
        assert MODULE.configured_default() == ""


def main() -> int:
    test_verified_routes()
    test_missing_origin_fails_closed()
    test_missing_link_header_fails_closed()
    test_origin_and_default_configuration()
    print("Coinbase project configuration and production-origin tests passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
