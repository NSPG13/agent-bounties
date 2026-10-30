from __future__ import annotations

import argparse
from contextlib import redirect_stdout
from datetime import datetime, timedelta, timezone
from email.message import Message
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


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


def readiness_payload(**extra):
    return {
        "schema": "agent-bounties/marketplace-readiness-v1", "ready": True,
        "checked_at": datetime.now(timezone.utc).isoformat(),
        "safe_block": 1000, "latest_block": 1010,
        "readers": [{"protocol": protocol, "ready": True, "blocker": None,
                     "heartbeat_age_seconds": 1, "cursor_block": 1000, "lag_blocks": 0}
                    for protocol in ("agent-bounties/autonomous-v1",
                                     "agent-bounties/open-competition-v1",
                                     "agent-bounties/open-competition-v2-beta3")],
        **extra,
    }


class FixtureOpener:
    """Offline public responses; keeps every actual request for boundary assertions."""
    def __init__(self, feed, diagnostic=None, diagnostic_status=200):
        self.feed = feed
        self.diagnostic = diagnostic if diagnostic is not None else readiness_payload()
        self.diagnostic_status = diagnostic_status
        self.requests = []

    def open(self, request, timeout):
        self.requests.append(request)
        status = 200
        if request.full_url.endswith("/readiness/marketplace"):
            body, status = self.diagnostic, self.diagnostic_status
        elif request.data:
            payload = json.loads(request.data)
            method = payload["method"]
            if method == "server/discover":
                result = {"supportedVersions": [benchmark.MODERN_VERSION], "resultType": "complete"}
            elif method == "tools/list":
                result = {"tools": [{"name": name, "annotations": {"readOnlyHint": True}}
                                    for name in ("get_bounty_feed", "get_posting_options")]}
            elif method == "initialize":
                result = {"protocolVersion": benchmark.LEGACY_VERSION}
            elif payload["params"]["name"] == "get_bounty_feed":
                result = self.feed["body"]["result"]
            else:
                result = {"structuredContent": {"choices": []}}
            body = {"result": result}
        elif request.full_url.endswith("/agent/index.md"):
            body = "Connect at https://mcp.example/mcp"
        elif request.full_url == "https://site.example/.well-known/agent-bounties.json":
            body = {"website": "https://site.example/"}
        elif "/autonomous-bounties/feed?" in request.full_url:
            body = []
        else:
            body = {}
        raw = body if isinstance(body, bytes) else json.dumps(body).encode()

        class Response:
            headers = Message()
            headers["Content-Type"] = "application/json"

            def __enter__(self):
                return self

            def __exit__(self, *_):
                return None

            def read(self, limit):
                return raw[:limit]

        response = Response()
        response.status = status
        return response


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

    def test_compact_partial_preserves_protocol_reason_and_unknown_count(self):
        source = {"source_type": "canonical_base", "available": False, "coverage": "partial",
                  "error": "open_competition_v2_read_model_unavailable", "item_count": 64,
                  "protocol_statuses": [{"protocol": "open-competition-v2-beta3",
                                         "coverage": "unavailable", "item_count": None,
                                         "error": "open_competition_v2_read_model_unavailable"}],
                  "internal_debug": "secret-do-not-copy"}
        observation = benchmark.inventory_observation(inventory(
            [], "partial", True, source_statuses=[source]))
        self.assertEqual(observation["state"], "partial")
        compact = observation["source_statuses"][0]
        self.assertFalse(compact["available"])
        self.assertIsNone(compact["protocol_statuses"][0]["item_count"])
        self.assertEqual(compact["protocol_statuses"][0]["error"],
                         "open_competition_v2_read_model_unavailable")
        self.assertNotIn("secret", json.dumps(observation))

    def test_compact_protocol_fields_are_allowlisted_bounded_and_typed(self):
        source = {"source_type": "canonical_base", "available": "true", "coverage": "secret",
                  "error": "https://provider.invalid/?secret=hidden", "item_count": True,
                  "protocol_statuses": [{"protocol": "private-secret", "coverage": "complete",
                                         "item_count": -1, "error": {"secret": "hidden"}}] * 100}
        result = benchmark.inventory_observation(inventory(
            [], "partial", True, source_statuses=[source] * 100))
        self.assertLessEqual(len(result["source_statuses"]), 8)
        row = result["source_statuses"][0]
        self.assertIsNone(row["available"])
        self.assertIsNone(row["item_count"])
        self.assertEqual(row["coverage"], "unknown")
        self.assertEqual(row["error"], "unrecognized_error")
        self.assertLessEqual(len(row["protocol_statuses"]), 8)
        self.assertIsNone(row["protocol_statuses"][0]["item_count"])
        self.assertNotIn("secret", json.dumps(result))
        for malformed in [None, {}, [None]]:
            result = benchmark.inventory_observation(inventory(
                [], "partial", True, source_statuses=malformed))
            self.assertEqual(result["state"], "partial")

    def fixture_run(self, opener, enabled=False, token=None):
        with patch.object(benchmark.urllib.request, "build_opener", return_value=opener):
            return benchmark.run("https://site.example", "https://api.example", "https://mcp.example",
                                 token, capture_readiness_on_partial=enabled)

    def test_optional_diagnostic_is_one_fixed_get_and_cannot_green_partial(self):
        opener = FixtureOpener(inventory([], "partial", True))
        report = self.fixture_run(opener, enabled=True)
        self.assertFalse(report["passed"])
        self.assertFalse(report["journeys"]["mcp_discovery_and_inventory"]["passed"])
        self.assertEqual(report["readiness_diagnostic"]["observation"]["state"], "healthy")
        reads = [r for r in opener.requests if r.full_url.endswith("/readiness/marketplace")]
        self.assertEqual(len(reads), 1)
        self.assertEqual(reads[0].full_url, "https://api.example/readiness/marketplace")
        self.assertEqual(reads[0].get_method(), "GET")
        calls = [json.loads(r.data) for r in opener.requests if r.data]
        feed_calls = [r for r in calls if r.get("params", {}).get("name") == "get_bounty_feed"]
        self.assertEqual(len(feed_calls), 1)
        self.assertEqual(feed_calls[0]["params"]["arguments"], {
            "network": "base-mainnet", "view": "ready_to_earn", "source_type": "canonical_base",
            "work_state": "claimable", "payment_state": "escrowed", "limit": 3})
        self.assertEqual(report["measurement_limits"]["request_count"], 16)
        self.assertEqual(report["measurement_limits"]["unconfirmed_request_count"], 16)

    def test_default_partial_and_enabled_complete_make_no_diagnostic_request(self):
        for feed, enabled, passed in [(inventory([], "partial", True), False, False),
                                      (inventory([]), True, True)]:
            with self.subTest(enabled=enabled, passed=passed):
                opener = FixtureOpener(feed)
                report = self.fixture_run(opener, enabled)
                self.assertEqual(report["passed"], passed)
                self.assertFalse(report["readiness_diagnostic"]["attempted"])
                self.assertEqual(len(opener.requests), 15)

    def test_invalid_stale_future_and_http_diagnostics_preserve_partial_failure(self):
        now = datetime.now(timezone.utc)
        for payload, status, blocker in [
            (None, 200, "marketplace_readiness_invalid"),
            (readiness_payload(checked_at=None), 200, "marketplace_readiness_invalid"),
            (readiness_payload(checked_at=(now - timedelta(seconds=91)).isoformat()), 200,
             "marketplace_readiness_stale"),
            (readiness_payload(checked_at=(now + timedelta(hours=1)).isoformat()), 200,
             "marketplace_readiness_clock_skew"),
            (readiness_payload(readers=[None]), 200, "marketplace_readiness_invalid"),
            (b"not JSON: secret-provider-error", 200, "marketplace_readiness_invalid"),
            (readiness_payload(), 503, "marketplace_readiness_http_inconsistent"),
            (readiness_payload(), 206, "marketplace_probe_unavailable"),
        ]:
            with self.subTest(status=status, blocker=blocker):
                opener = FixtureOpener(inventory([], "partial", True))
                opener.diagnostic, opener.diagnostic_status = payload, status
                report = self.fixture_run(opener, enabled=True)
                self.assertFalse(report["passed"])
                observed = report["readiness_diagnostic"]["observation"]
                self.assertEqual(observed["state"], "unavailable")
                self.assertEqual(observed["blocker"], blocker)
                self.assertNotIn("secret-provider-error", json.dumps(report))

    def test_503_reader_hold_remains_diagnostic_stale_not_empty_inventory(self):
        payload = readiness_payload(ready=False)
        payload["readers"][2].update(ready=False, blocker="reader_stale_or_behind",
                                     heartbeat_age_seconds=91)
        report = self.fixture_run(FixtureOpener(inventory([], "partial", True), payload, 503), True)
        observed = report["readiness_diagnostic"]["observation"]
        self.assertEqual(observed["state"], "stale")
        self.assertFalse(observed["restartable"])
        self.assertFalse(report["passed"])
        self.assertEqual(report["journeys"]["mcp_discovery_and_inventory"]["inventory"]["state"], "partial")

    def test_diagnostic_receipt_drops_unknown_fields_and_bounds_raw_response(self):
        payload = readiness_payload(provider_url="secret-provider-url", token="secret-exclusion-token")
        opener = FixtureOpener(inventory([], "partial", True), payload)
        report = self.fixture_run(opener, True, "secret-exclusion-token")
        record = next(r for r in report["requests"] if r["label"] == "marketplace_readiness_diagnostic")
        self.assertEqual(record["body_scope"], "validated_readiness_summary")
        self.assertNotIn("secret-", json.dumps(report))
        request = next(r for r in opener.requests if r.full_url.endswith("/readiness/marketplace"))
        self.assertEqual(request.get_header("X-agent-bounties-analytics-exclusion"), "secret-exclusion-token")
        report = self.fixture_run(FixtureOpener(inventory([], "partial", True),
                                               b"x" * (benchmark.MAX_BYTES + 1)), True)
        self.assertFalse(report["passed"])
        self.assertEqual(report["readiness_diagnostic"]["observation"]["state"], "unavailable")

    def test_diagnostic_transport_failure_is_redacted_and_not_retried(self):
        class FailingOpener(FixtureOpener):
            def open(self, request, timeout):
                if request.full_url.endswith("/readiness/marketplace"):
                    self.requests.append(request)
                    raise OSError("secret-provider-credential")
                return super().open(request, timeout)

        opener = FailingOpener(inventory([], "partial", True))
        report = self.fixture_run(opener, True)
        self.assertFalse(report["passed"])
        self.assertEqual(report["readiness_diagnostic"]["observation"]["blocker"],
                         "marketplace_probe_unavailable")
        self.assertEqual(len(opener.requests), 16)
        self.assertNotIn("secret-provider-credential", json.dumps(report))

    def test_diagnostic_redirect_headers_do_not_retain_provider_urls(self):
        class RedirectOpener(FixtureOpener):
            def open(self, request, timeout):
                response = super().open(request, timeout)
                if request.full_url.endswith("/readiness/marketplace"):
                    response.headers["Location"] = "https://provider.invalid/?credential=secret-marker"
                    response.headers["ETag"] = "secret-marker"
                    response.headers["Date"] = "secret-marker"
                    response.headers["X-Agent-Bounties-Analytics-Excluded"] = "TRUE"
                return response

        opener = RedirectOpener(inventory([], "partial", True), diagnostic_status=302)
        report = self.fixture_run(opener, True)
        self.assertFalse(report["passed"])
        self.assertEqual(len(opener.requests), 16)
        self.assertNotIn("secret-marker", json.dumps(report))
        record = next(r for r in report["requests"] if r["label"] == "marketplace_readiness_diagnostic")
        self.assertEqual(record["response_headers"], {"x-agent-bounties-analytics-excluded": "true"})
        self.assertTrue(record["analytics_exclusion_confirmed"])

    def test_cli_keeps_exit_one_when_partial_diagnostic_is_healthy(self):
        opener = FixtureOpener(inventory([], "partial", True))
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "receipt.json"
            args = ["agent-discovery-benchmark.py", "--website", "https://site.example",
                    "--api", "https://api.example", "--mcp", "https://mcp.example",
                    "--capture-readiness-on-partial", "--output", str(output)]
            with patch("sys.argv", args), redirect_stdout(io.StringIO()), \
                    patch.object(benchmark.urllib.request, "build_opener", return_value=opener):
                self.assertEqual(benchmark.main(), 1)
            self.assertFalse(json.loads(output.read_text())["passed"])


if __name__ == "__main__":
    unittest.main()
