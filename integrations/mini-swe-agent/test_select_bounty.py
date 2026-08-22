#!/usr/bin/env python3
"""Focused tests for the deterministic paid-work selector."""

from __future__ import annotations

import importlib.util
import json
import unittest
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).parent
SPEC = importlib.util.spec_from_file_location("select_bounty", ROOT / "select_bounty.py")
assert SPEC and SPEC.loader
SELECTOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SELECTOR)


class SelectorTests(unittest.TestCase):
    def fixture(self, name: str):
        return json.loads((ROOT / "fixtures" / name).read_text(encoding="utf-8"))

    def test_committed_fixture_actions(self):
        expected = {
            "multiple.json": "claim",
            "empty.json": "wait",
            "stale.json": "refresh",
            "no-margin.json": "skip",
            "exclusive-claimant.json": "skip",
        }
        for name, action in expected.items():
            with self.subTest(name=name):
                result = SELECTOR.select(self.fixture(name))
                self.assertEqual(action, result["action"])
                self.assertTrue(result["next_action"].strip())

    def test_deterministic_highest_margin_choice(self):
        result = SELECTOR.select(self.fixture("multiple.json"))
        self.assertEqual(
            "0x00000000000000000000000000000000000000bb",
            result["selected"]["bounty_contract"],
        )

    def test_matching_exclusive_wallet_can_continue(self):
        payload = self.fixture("exclusive-claimant.json")
        result = SELECTOR.select(
            payload, solver_wallet="0x1111111111111111111111111111111111111111"
        )
        self.assertEqual("claim", result["action"])

    def test_old_timestamp_refreshes(self):
        payload = self.fixture("multiple.json")
        payload["generated_at"] = "2020-01-01T00:00:00Z"
        result = SELECTOR.select(
            payload,
            max_age_seconds=300,
            now=datetime(2026, 8, 7, tzinfo=timezone.utc),
        )
        self.assertEqual("refresh", result["action"])


if __name__ == "__main__":
    unittest.main()
