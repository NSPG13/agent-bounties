#!/usr/bin/env python3
import importlib.util
import json
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("competitor_intelligence", ROOT / "scripts" / "competitor_intelligence.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class CompetitorIntelligenceTests(unittest.TestCase):
    def test_registry_is_direct_and_evidence_bound(self):
        registry = json.loads((ROOT / "ops" / "competitors" / "direct-competitors.json").read_text(encoding="utf-8"))
        MODULE.validate_registry(registry)
        self.assertGreaterEqual(len(registry["competitors"]), 5)
        self.assertTrue(all(item["direct_competitor_reason"] for item in registry["competitors"]))
        self.assertTrue(all(item["sources"] for item in registry["competitors"]))

    def test_extracts_marketplace_and_repository_metrics_without_guessing(self):
        page = b"Paid 42 Available 295 Paid out $4,909.87 Open value $1,338,080.22"
        self.assertEqual(MODULE.extract_metric("bounties_paid", page, "text/html"), 42)
        self.assertEqual(MODULE.extract_metric("bounties_available", page, "text/html"), 295)
        self.assertEqual(MODULE.extract_metric("paid_out_usd", page, "text/html"), 4909.87)
        self.assertEqual(MODULE.extract_metric("bounties_posted_usd", b"$ of bounties posted: $1.5 million", "text/html"), 1_500_000)
        repository = json.dumps({"stargazers_count": 11, "forks_count": 2, "open_issues_count": 3, "pushed_at": "2026-07-21T00:00:00Z"}).encode()
        self.assertEqual(MODULE.extract_metric("github_stars", repository, "application/json"), 11)
        self.assertEqual(MODULE.extract_metric("github_pushed_at", repository, "application/json"), "2026-07-21T00:00:00Z")
        self.assertIsNone(MODULE.extract_metric("bounties_paid", b"no count", "text/html"))

    def test_change_detection_does_not_invent_a_baseline(self):
        observation = MODULE.Observation("opire", "marketplace", "https://example.test", "2026-07-21T00:00:00+00:00", 200, "b" * 64, {"bounties_paid": 43})
        self.assertEqual(MODULE.changes_for(observation, None, {}), [])
        changes = MODULE.changes_for(observation, {"content_sha256": "a" * 64, "error_kind": None}, {"bounties_paid": 42})
        self.assertEqual({change["kind"] for change in changes}, {"source_changed", "metric_changed"})
        failed = MODULE.Observation("opire", "marketplace", "https://example.test", "2026-07-21T00:00:00+00:00", None, None, {}, "timeout", "request timed out")
        self.assertEqual(MODULE.changes_for(failed, {"content_sha256": "a" * 64, "error_kind": None}, {})[0]["kind"], "source_failed")


if __name__ == "__main__":
    unittest.main()
