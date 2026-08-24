import importlib.util
import json
import sys
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("discoverability_snapshot.py")
SPEC = importlib.util.spec_from_file_location("discoverability_snapshot", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class DiscoverabilitySnapshotTests(unittest.TestCase):
    def setUp(self):
        self.now = datetime(2026, 8, 24, 12, 0, tzinfo=timezone.utc)

    def test_checksum_is_deterministic_across_key_order(self):
        left = {"totals": {"clicks": 0, "impressions": 116}}
        right = {"totals": {"impressions": 116, "clicks": 0}}
        self.assertEqual(MODULE.payload_checksum(left), MODULE.payload_checksum(right))
        self.assertEqual(len(MODULE.payload_checksum(left)), 64)

    def test_snapshot_request_contains_only_hash_and_operator_payload(self):
        payload = {
            "totals": {"impressions": 116, "clicks": 0, "average_position": 7.8},
            "dimensions": {"query_page_rows": [{"keys": ["private query", "/private"]}]},
        }
        snapshot = MODULE.Snapshot(
            "search_console",
            self.now,
            self.now - timedelta(days=35),
            self.now - timedelta(days=3),
            self.now - timedelta(days=3),
            payload,
        ).as_request()
        self.assertEqual(snapshot["payload"], payload)
        public_safe_metadata = {key: value for key, value in snapshot.items() if key != "payload"}
        self.assertNotIn("private query", json.dumps(public_safe_metadata))

    def test_invalid_window_fails_closed(self):
        snapshot = MODULE.Snapshot(
            "github",
            self.now,
            self.now,
            self.now - timedelta(days=1),
            self.now,
            {"totals": {}},
        )
        with self.assertRaises(ValueError):
            snapshot.as_request()

    def test_chatgpt_normalization_does_not_infer_generic_mcp(self):
        for value in ["chatgpt.com", "chat.openai.com", "links.chatgpt.com", "openai"]:
            self.assertTrue(MODULE.channel_is_chatgpt(value))
        for value in ["mcp", "api", "direct", "example.com"]:
            self.assertFalse(MODULE.channel_is_chatgpt(value))

    def test_validation_helpers_reject_negative_counts_and_rates(self):
        with self.assertRaises(ValueError):
            MODULE.nonnegative_int(-1, "count")
        with self.assertRaises(ValueError):
            MODULE.bounded_rate(1.01, "rate")

    def test_search_console_dimensions_are_paginated_without_dropping_rows(self):
        calls = []

        def query(body):
            calls.append(body)
            pages = {
                0: [{"keys": ["q1", "/one"]}, {"keys": ["q2", "/two"]}],
                2: [{"keys": ["q3", "/three"]}],
            }
            return {"rows": pages[body["startRow"]]}

        rows = MODULE.paginate_search_console_dimensions(
            query=query,
            start_date="2026-07-18",
            end_date="2026-08-21",
            row_limit=2,
        )
        self.assertEqual([body["startRow"] for body in calls], [0, 2])
        self.assertEqual([row["keys"][0] for row in rows], ["q1", "q2", "q3"])

    def test_search_console_uses_28_day_headlines_inside_35_day_recovery_window(self):
        headline_start, recovery_start, end = MODULE.search_console_date_ranges(self.now)

        self.assertEqual((end - headline_start).days + 1, 28)
        self.assertEqual((end - recovery_start).days + 1, 35)
        self.assertEqual((self.now.date() - end).days, 3)

    def test_failure_labels_never_include_exception_text_or_secrets(self):
        secret = "secret-service-account-private-key"
        label = MODULE.failure_label(RuntimeError(secret))
        self.assertEqual(label, "RuntimeError")
        self.assertNotIn(secret, label)

    def test_ingestion_token_is_used_only_for_upload(self):
        source = MODULE_PATH.read_text(encoding="utf-8")
        self.assertNotIn("/v1/operator/discoverability/report", source)
        self.assertIn('"POST",\n        f"{api_base}/v1/operator/discoverability/snapshots"', source)

    def test_collector_uploads_only_external_provider_snapshots(self):
        self.assertEqual(
            MODULE.PROVIDERS,
            ("search_console", "github", "first_party"),
        )


if __name__ == "__main__":
    unittest.main()
