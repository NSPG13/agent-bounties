#!/usr/bin/env python3
"""Offline tests for diagnose_hosted_api classification helpers."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from diagnose_hosted_api import (  # noqa: E402
    Diagnosis,
    PathResult,
    diagnose,
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

    def test_all_404_classification_shape(self) -> None:
        # Structural: PathResult serializable
        p = PathResult(path="/health", ok=False, status=404, error="HTTPError 404")
        self.assertEqual(p.status, 404)
        payload = {
            "base_url": "https://agent-bounties-api.onrender.com",
            "hostname": "agent-bounties-api.onrender.com",
            "dns_ok": True,
            "dns_error": None,
            "paths": [
                {
                    "path": "/health",
                    "ok": False,
                    "status": 404,
                    "error": "HTTPError 404",
                    "body_preview": "Not Found",
                }
            ],
            "likely_causes": ["Missing Render deployment"],
            "repair_steps": ["Apply Blueprint"],
            "overall": "not_found",
            "disclaimer": "test",
        }
        d = Diagnosis(
            **{
                **payload,
                "paths": [PathResult(**payload["paths"][0])],
            }
        )
        self.assertEqual(d.overall, "not_found")
        self.assertIn("Blueprint", d.repair_steps[0])


if __name__ == "__main__":
    unittest.main()
