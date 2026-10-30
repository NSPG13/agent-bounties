import importlib.util
from datetime import datetime, timedelta, timezone
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("growth_baseline", Path(__file__).with_name("discovery-growth-baseline.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class GrowthBaselineTests(unittest.TestCase):
    def history(self):
        end = datetime(2026, 10, 30, 5, tzinfo=timezone.utc)
        return {"generated_at": end.isoformat(), "window_started_at": (end - timedelta(days=57)).isoformat(), "daily": [dict(day=str((end - timedelta(days=i)).date()), visitors=100, sessions=100, **{key: 1 for key in module.ADDITIVE_EVENTS}) for i in range(58)]}

    def test_complete_windows_exclude_partial_days_and_nonadditive_counts(self):
        result = module.calendar_comparison(self.history())["windows"]
        self.assertEqual(result["current"]["start"], "2026-10-02")
        self.assertEqual(result["current"]["end"], "2026-10-29")
        self.assertEqual(result["previous"]["end"], "2026-10-01")
        self.assertEqual(result["current"]["metrics"]["page_views"], 28)
        self.assertNotIn("visitors", result["current"]["metrics"])

    def test_missing_day_is_not_zero_filled(self):
        data = self.history()
        data["daily"] = [r for r in data["daily"] if r["day"] != "2026-10-15"]
        result = module.calendar_comparison(data)["windows"]
        self.assertEqual(result["current"]["status"], "unavailable")
        self.assertIsNone(result["current"]["metrics"])
        self.assertEqual(result["previous"]["status"], "available")

    def test_partial_first_day_disallows_comparison(self):
        data = self.history()
        data["window_started_at"] = "2026-09-04T01:00:00+00:00"
        self.assertEqual(module.calendar_comparison(data)["windows"]["previous"]["status"], "unavailable")

    def test_duplicate_days_are_rejected(self):
        data = self.history()
        data["daily"].append(data["daily"][1])
        with self.assertRaises(ValueError):
            module.calendar_comparison(data)

    def test_invalid_counts_and_schema_are_rejected(self):
        for value in (-1, True, None, "1"):
            with self.assertRaises(ValueError):
                module.count(value)
        with self.assertRaises(ValueError):
            module.validate({}, "agent-bounties/site-analytics-v2")

    def test_null_denominator_stays_null_and_inconsistent_rate_fails(self):
        data = self.history()
        data.update(schema_version="agent-bounties/site-analytics-v2", overview={"unique_visitors": 1, "sessions": 1, "page_views": 1}, channels=[], interfaces=[], rates=[{"numerator_sessions": 0, "denominator_sessions": 0, "value": None}])
        self.assertIsNone(module.validate(data, data["schema_version"])["rates"][0]["value"])
        data["rates"][0]["value"] = 0
        with self.assertRaises(ValueError):
            module.validate(data, data["schema_version"])

    def test_nonfinite_conversion_rates_are_rejected(self):
        data = self.history()
        data.update(schema_version="agent-bounties/site-analytics-v2", overview={"unique_visitors": 1, "sessions": 1, "page_views": 1}, channels=[], interfaces=[])
        for value in (float("nan"), float("inf"), float("-inf")):
            with self.subTest(value=value), self.assertRaises(ValueError):
                data["rates"] = [{"numerator_sessions": 1, "denominator_sessions": 2, "value": value}]
                module.validate(data, data["schema_version"])

    def test_partial_http_responses_remain_unavailable(self):
        class PartialResponse:
            status = 206

            def __enter__(self):
                return self

            def __exit__(self, *_):
                return None

            def read(self, _):
                raise AssertionError("partial aggregate must not be interpreted")

        class Opener:
            def open(self, request, timeout):
                return PartialResponse()

        with patch.object(module, "build_opener", return_value=Opener()):
            result = module.collect()
        self.assertEqual(set(result["retrieval_failures"]), set(module.SOURCES))
        for source in result["sources"].values():
            self.assertEqual(source["status"], "unavailable")
            self.assertEqual(source["http_status"], 206)
            self.assertIsNone(source["response"])


if __name__ == "__main__":
    unittest.main()
