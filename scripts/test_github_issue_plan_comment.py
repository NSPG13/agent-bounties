#!/usr/bin/env python3
"""Tests for paid-bounty issue validation routing."""

from __future__ import annotations

import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "paid-bounty-issues.yml"


class PaidBountyIssueWorkflowTests(unittest.TestCase):
    def test_canonical_funded_inventory_skips_intake_validation(self) -> None:
        workflow = WORKFLOW.read_text(encoding="utf-8")

        self.assertIn(
            "!contains(toJson(github.event.issue.labels), '\"funded-live\"')",
            workflow,
        )
        self.assertLess(
            workflow.index("!contains(toJson(github.event.issue.labels)"),
            workflow.index("startsWith(github.event.issue.title"),
        )


if __name__ == "__main__":
    unittest.main()
