#!/usr/bin/env python3
"""Offline tests for diagnose_hosted_api classification helpers."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
from diagnose_hosted_api import (  # noqa: E402
    Diagnosis,
    PathResult,
    diagnose,
    normalize_base_url,
    to_markdown,
)


class DiagnoseHostedApiTests(unittest.TestCase):
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
            normalize_base_url("agent-bounties-api.onrender.com"),
            "https://agent-bounties-api.onrender.com",
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
                d = diagnose("https://agent-bounties-api.onrender.com")
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
                d = diagnose("https://agent-bounties-api.onrender.com")
        self.assertEqual(d.overall, "connection_failure")
        self.assertTrue(all(p.status is None for p in d.paths))
        self.assertNotEqual(d.overall, "not_found")


if __name__ == "__main__":
    unittest.main()
