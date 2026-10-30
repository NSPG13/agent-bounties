from __future__ import annotations

import argparse
from email.message import Message
import importlib.util
import json
from pathlib import Path
import unittest


SPEC = importlib.util.spec_from_file_location(
    "agent_discovery_benchmark", Path(__file__).with_name("agent-discovery-benchmark.py")
)
assert SPEC and SPEC.loader
benchmark = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(benchmark)


def inventory(items, coverage="complete", degraded=False, **extra):
    return {"status": 200, "body": {"result": {
        "structuredContent": {"items": items, "coverage": coverage,
                              "degraded": degraded, **extra}
    }}}


class DiscoveryEvidenceTests(unittest.TestCase):
    def test_complete_empty_feed_is_successful_empty_observation(self):
        observation = benchmark.inventory_observation(inventory([]))
        self.assertEqual(observation["state"], "observed_empty")
        self.assertEqual(observation["matching_items"], 0)

    def test_failed_or_unavailable_feed_is_not_zero_inventory(self):
        for record in [
            {"status": 503},
            {"status": 200, "body": {"result": {"isError": True}}},
            {"status": 200, "body": {"result": {"structuredContent": {}}}},
        ]:
            with self.subTest(record=record):
                observation = benchmark.inventory_observation(record)
                self.assertEqual(observation["state"], "unavailable")
                self.assertIsNone(observation["matching_items"])

    def test_partial_or_degraded_empty_feed_does_not_establish_absence(self):
        self.assertEqual(benchmark.inventory_observation(inventory([], "partial"))["state"], "partial")
        self.assertEqual(benchmark.inventory_observation(inventory([], degraded=True))["state"], "unavailable")
        self.assertEqual(benchmark.inventory_observation(inventory([], coverage=None))["state"], "unavailable")

    def test_observed_items_preserve_scope_and_freshness(self):
        observation = benchmark.inventory_observation(inventory(
            [{"id": "sample"}], coverage_scope="inventory_page", has_more=True,
            generated_at="2026-10-29T00:00:00Z",
        ))
        self.assertEqual(observation["state"], "observed_items")
        self.assertEqual(observation["coverage_scope"], "inventory_page")
        self.assertTrue(observation["has_more"])
        self.assertEqual(observation["generated_at"], "2026-10-29T00:00:00Z")

    def test_benchmark_rejects_mutating_tool_calls_before_network(self):
        probe = benchmark.Probe("https://api.example", "https://mcp.example")
        probe.mcp_endpoint = "https://mcp.example/mcp"
        with self.assertRaisesRegex(ValueError, "only its two read-only"):
            probe.modern("must not run", "tools/call", {"name": "prepare_bounty_action", "arguments": {}})
        self.assertEqual(probe.records, [])

    def test_origin_inputs_reject_credentials_and_nonlocal_http(self):
        for value in ["https://secret@example.com", "http://example.com", "https://example.com?token=secret"]:
            with self.subTest(value=value), self.assertRaises(argparse.ArgumentTypeError):
                benchmark.public_base(value)
        self.assertEqual(benchmark.public_base("http://127.0.0.1:8080/"), "http://127.0.0.1:8080")

    def test_exclusion_credential_is_host_scoped_and_never_in_receipt(self):
        requests = []

        class Response:
            status = 200
            headers = Message()
            headers["Content-Type"] = "application/json"
            headers["X-Agent-Bounties-Analytics-Excluded"] = "true"

            def __enter__(self):
                return self

            def __exit__(self, *_):
                return None

            def read(self, _):
                return b'{"echo": "secret-exclusion-token"}'

        class Opener:
            def open(self, request, timeout):
                requests.append(request)
                return Response()

        probe = benchmark.Probe("https://api.example", "https://mcp.example", "secret-exclusion-token")
        probe.opener = Opener()
        probe.request("API", "https://api.example/health")
        probe.request("website", "https://example.com/agent/index.md")
        self.assertEqual(requests[0].get_header("X-agent-bounties-analytics-exclusion"), "secret-exclusion-token")
        self.assertIsNone(requests[1].get_header("X-agent-bounties-analytics-exclusion"))
        self.assertNotIn("secret-exclusion-token", json.dumps(probe.records))
        self.assertTrue(probe.records[0]["analytics_exclusion_confirmed"])
        self.assertTrue(probe.records[0]["analytics_exclusion_requested"])
        self.assertFalse(probe.records[1]["analytics_exclusion_requested"])

    def test_redirects_do_not_forward_credentials(self):
        redirect = benchmark.NoRedirect()
        self.assertIsNone(redirect.redirect_request(None, None, 302, "", {}, "https://other.example"))


if __name__ == "__main__":
    unittest.main()
