#!/usr/bin/env python3
"""Offline tests for diagnose_hosted_api classification helpers."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path
from unittest.mock import MagicMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
from diagnose_hosted_api import (  # noqa: E402
    Diagnosis,
    PathResult,
    diagnose,
    fetch,
    normalize_base_url,
    to_markdown,
)


class DiagnoseHostedApiTests(unittest.TestCase):
    @staticmethod
    def response(status: int, body: bytes) -> MagicMock:
        response = MagicMock()
        response.status = status
        response.read.return_value = body
        response.__enter__.return_value = response
        return response

    def test_markdown_contains_disclaimer(self) -> None:
        d = Diagnosis(
            base_url="https://example.invalid",
            hostname="example.invalid",
            dns_ok=False,
            dns_error="name not found",
            overall="dns_failure",
            likely_causes=["x"],
            repair_steps=["y"],
        )
        md = to_markdown(d)
        self.assertIn("does not create", md.lower())
        self.assertIn("dns_failure", md)

    def test_normalize_schemeless_base_url(self) -> None:
        self.assertEqual(
            normalize_base_url("api.bountyboard.global"),
            "https://api.bountyboard.global",
        )
        self.assertEqual(
            normalize_base_url("https://example.com/path/"),
            "https://example.com",
        )

    def test_all_404_via_diagnose(self) -> None:
        """Mock DNS + fetch so diagnose() returns not_found only when HTTP 404s observed."""

        def fake_fetch(url: str, timeout: float = 20.0) -> PathResult:
            from urllib.parse import urlparse

            p = urlparse(url).path or "/"
            return PathResult(path=p, ok=False, status=404, error="HTTPError 404")

        with patch(
            "diagnose_hosted_api.socket.getaddrinfo",
            return_value=[(None, None, None, None, None)],
        ):
            with patch("diagnose_hosted_api.fetch", side_effect=fake_fetch):
                d = diagnose("https://api.bountyboard.global")
        self.assertEqual(d.overall, "not_found")
        self.assertTrue(all(p.status == 404 for p in d.paths))
        self.assertTrue(len(d.paths) >= 1)

    def test_all_no_status_is_connection_failure(self) -> None:
        """When every path returns status=None, classify connection_failure — not not_found."""

        def fake_fetch(url: str, timeout: float = 20.0) -> PathResult:
            from urllib.parse import urlparse

            p = urlparse(url).path or "/"
            return PathResult(
                path=p, ok=False, status=None, error="URLError: Connection refused"
            )

        with patch(
            "diagnose_hosted_api.socket.getaddrinfo",
            return_value=[(None, None, None, None, None)],
        ):
            with patch("diagnose_hosted_api.fetch", side_effect=fake_fetch):
                d = diagnose("https://api.bountyboard.global")
        self.assertEqual(d.overall, "connection_failure")
        self.assertTrue(all(p.status is None for p in d.paths))
        self.assertNotEqual(d.overall, "not_found")

    def test_health_requires_exact_ok_body(self) -> None:
        with patch(
            "diagnose_hosted_api.urllib.request.urlopen",
            return_value=self.response(200, b"<html>not the API</html>"),
        ):
            result = fetch("https://example.com/health")
        self.assertFalse(result.ok)
        self.assertEqual(result.error, "expected body 'ok'")

    def test_json_routes_require_json_body(self) -> None:
        with patch(
            "diagnose_hosted_api.urllib.request.urlopen",
            return_value=self.response(200, b"not-json"),
        ):
            result = fetch("https://example.com/v1/readiness/live-money")
        self.assertFalse(result.ok)
        self.assertEqual(result.error, "expected JSON body")

    def test_expected_contract_bodies_are_healthy(self) -> None:
        responses = {
            "/health": b"ok",
            "/v1/readiness/live-money": b'{"ready": false}',
            "/v1/bounties/funding-feed": b'{"items": []}',
        }

        def fake_fetch(url: str, timeout: float = 20.0) -> PathResult:
            from urllib.parse import urlparse

            path = urlparse(url).path
            with patch(
                "diagnose_hosted_api.urllib.request.urlopen",
                return_value=self.response(200, responses[path]),
            ):
                return fetch(url, timeout)

        with patch(
            "diagnose_hosted_api.socket.getaddrinfo",
            return_value=[(None, None, None, None, None)],
        ), patch("diagnose_hosted_api.fetch", side_effect=fake_fetch):
            diagnosis = diagnose("https://api.bountyboard.global")
        self.assertEqual(diagnosis.overall, "healthy")


if __name__ == "__main__":
    unittest.main()
