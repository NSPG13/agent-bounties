import importlib.util
import copy
import io
import json
import os
import subprocess
import sys
import textwrap
import unittest
from contextlib import redirect_stdout
from dataclasses import replace
from datetime import datetime, timedelta, timezone
from pathlib import Path
from types import ModuleType
from unittest.mock import MagicMock, patch


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

    def google_clients(self):
        # Keep credential-path tests offline and independent of local packages.
        names = ["google", "google.auth", "google.auth.transport",
                 "google.auth.transport.requests", "google.oauth2",
                 "google.oauth2.credentials", "google.oauth2.service_account"]
        modules = {name: ModuleType(name) for name in names}
        for name in names:
            if "." in name:
                parent, child = name.rsplit(".", 1)
                setattr(modules[parent], child, modules[name])
        token_credentials = MagicMock()
        key_credentials = MagicMock()
        session_factory = MagicMock()
        modules["google.oauth2.credentials"].Credentials = token_credentials
        modules["google.oauth2.service_account"].Credentials = key_credentials
        modules["google.auth.transport.requests"].AuthorizedSession = session_factory
        session_factory.return_value.post.return_value.json.return_value = {"rows": []}
        session_factory.return_value.post.return_value.status_code = 200
        return modules, token_credentials, key_credentials, session_factory

    def test_search_console_ephemeral_token_precedes_json_without_changing_payload(self):
        modules, token, key, session = self.google_clients()
        with patch.dict(sys.modules, modules):
            snapshot = MODULE.collect_search_console(
                access_token="ephemeral-token", service_account_json="not-valid-json",
                property_url=MODULE.DEFAULT_PROPERTY, observed_at=self.now)
        token.assert_called_once_with(token="ephemeral-token")
        key.from_service_account_info.assert_not_called()
        session.assert_called_once_with(token.return_value)
        self.assertEqual(snapshot.payload["totals"], {
            "impressions": 0, "clicks": 0, "average_position": 0.0})
        self.assertEqual(len(session.return_value.post.call_args_list), 2)
        for call in session.return_value.post.call_args_list:
            self.assertEqual(call.args, (
                "https://searchconsole.googleapis.com/webmasters/v3/sites/"
                "https%3A%2F%2Fagentizable.ai%2F/searchAnalytics/query",))
            self.assertEqual(call.kwargs["timeout"], 60)
            self.assertIs(call.kwargs["allow_redirects"], False)
        self.assertEqual(snapshot.as_request()["data_through"], "2026-08-21T23:59:59Z")

    def test_search_console_legacy_json_keeps_readonly_scope(self):
        modules, token, key, session = self.google_clients()
        with patch.dict(sys.modules, modules):
            MODULE.collect_search_console(
                service_account_json='{"fixture": true}',
                property_url=MODULE.DEFAULT_PROPERTY, observed_at=self.now)
        token.assert_not_called()
        key.from_service_account_info.assert_called_once_with(
            {"fixture": True}, scopes=[MODULE.GSC_SCOPE])
        session.assert_called_once_with(key.from_service_account_info.return_value)

    def test_search_console_token_failure_never_falls_back_to_json(self):
        modules, token, key, session = self.google_clients()
        session.return_value.post.return_value.raise_for_status.side_effect = RuntimeError("token rejected")
        with patch.dict(sys.modules, modules), self.assertRaises(RuntimeError):
            MODULE.collect_search_console(
                access_token="rejected-token", service_account_json='{"fixture": true}',
                property_url=MODULE.DEFAULT_PROPERTY, observed_at=self.now)
        token.assert_called_once_with(token="rejected-token")
        key.from_service_account_info.assert_not_called()
        self.assertEqual(session.return_value.post.call_count, 1)

    def test_search_console_redirect_is_rejected_without_following_or_key_fallback(self):
        modules, token, key, session = self.google_clients()
        session.return_value.post.return_value.status_code = 302
        with patch.dict(sys.modules, modules), self.assertRaises(RuntimeError):
            MODULE.collect_search_console(access_token="ephemeral-token",
                property_url=MODULE.DEFAULT_PROPERTY, observed_at=self.now)
        self.assertIs(session.return_value.post.call_args.kwargs["allow_redirects"], False)
        key.from_service_account_info.assert_not_called()
        session.return_value.post.return_value.json.assert_not_called()

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

    def first_party_report(self):
        return {
            "schema_version": "agent-bounties/site-analytics-v2",
            "window_hours": 672,
            "window_started_at": (self.now - timedelta(days=28)).isoformat(),
            "generated_at": (self.now + timedelta(seconds=2)).isoformat(),
            "overview": {"first_event_at": None, "last_event_at": None},
            "event_counts": [
                {"event_name": "market_view", "events": 10},
                {"event_name": "funded_bounty_click", "events": 2},
            ],
            "channels": [{"source": "chatgpt.com", "campaign": None, "visitors": 2}],
            "rates": [{"metric": "market_to_funded_bounty_click", "value": 0.2,
                       "numerator_sessions": 2, "denominator_sessions": 10}],
            "daily": [],
        }

    def collect_report(self, report):
        completed = self.now + timedelta(seconds=3, microseconds=750_000)

        class Clock(datetime):
            @classmethod
            def now(cls, tz=None):
                return completed

        with patch.object(MODULE, "request_json", return_value=report), patch.object(MODULE, "datetime", Clock):
            return MODULE.collect_first_party(api_base="https://example.invalid", observed_at=self.now)

    def test_live_response_after_run_start_uses_completed_observation(self):
        snapshot = self.collect_report(self.first_party_report())
        request = snapshot.as_request()
        self.assertEqual(request["observed_at"], "2026-08-24T12:00:03Z")
        self.assertEqual(request["window_ended_at"], "2026-08-24T12:00:02Z")
        self.assertEqual(request["payload"]["totals"], {
            "captured_chatgpt_referrals": 2, "opportunity_feed_clicks": 0,
            "market_views": 10, "funded_bounty_clicks": 2,
            "market_to_funded_opportunity_ctr": 0.2,
        })

    def test_first_party_requires_supported_schema_and_complete_collections(self):
        for key in ["schema_version", "window_hours", "channels", "event_counts", "rates", "daily", "overview"]:
            report = self.first_party_report()
            del report[key]
            with self.subTest(missing=key), self.assertRaises(ValueError):
                self.collect_report(report)
        for key in ["channels", "event_counts", "rates", "daily"]:
            report = self.first_party_report()
            report[key] = {}
            with self.subTest(malformed=key), self.assertRaises(ValueError):
                self.collect_report(report)
        report = self.first_party_report()
        report["schema_version"] = "other-schema"
        with self.assertRaises(ValueError):
            self.collect_report(report)

    def test_first_party_rejects_duplicate_rows(self):
        for key in ["channels", "event_counts", "rates"]:
            report = self.first_party_report()
            report[key].append(copy.deepcopy(report[key][0]))
            with self.subTest(collection=key), self.assertRaises(ValueError):
                self.collect_report(report)

    def test_first_party_rejects_coercible_or_missing_counts(self):
        for value in [None, True, False, -1, 2.75, "2", float("nan"), float("inf")]:
            for collection, field in [("channels", "visitors"), ("event_counts", "events")]:
                report = self.first_party_report()
                report[collection][0][field] = value
                with self.subTest(value=value, collection=collection), self.assertRaises(ValueError):
                    self.collect_report(report)
        for collection, field in [("channels", "visitors"), ("event_counts", "events")]:
            report = self.first_party_report()
            del report[collection][0][field]
            with self.subTest(missing=field), self.assertRaises(ValueError):
                self.collect_report(report)

    def test_first_party_undefined_or_inconsistent_rate_is_unavailable(self):
        cases = [
            {"value": None}, {"value": True}, {"value": float("nan")},
            {"value": float("inf")}, {"value": 0.9},
            {"denominator_sessions": 0, "numerator_sessions": 0, "value": None},
            {"denominator_sessions": 0, "numerator_sessions": 0, "value": 0.0},
            {"numerator_sessions": 11}, {"numerator_sessions": True},
            {"denominator_sessions": 10.5},
        ]
        for changes in cases:
            report = self.first_party_report()
            report["rates"][0].update(changes)
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                self.collect_report(report)
        report = self.first_party_report()
        report["rates"] = []
        with self.assertRaises(ValueError):
            self.collect_report(report)

    def test_first_party_observed_zero_and_absent_event_rows_remain_valid(self):
        report = self.first_party_report()
        report["rates"][0].update(value=0.0, numerator_sessions=0)
        report["channels"] = []
        report["event_counts"] = [report["event_counts"][0]]
        totals = self.collect_report(report).as_request()["payload"]["totals"]
        self.assertEqual(totals["market_to_funded_opportunity_ctr"], 0.0)
        self.assertEqual(totals["captured_chatgpt_referrals"], 0)
        self.assertEqual(totals["funded_bounty_clicks"], 0)
        self.assertEqual(totals["opportunity_feed_clicks"], 0)

    def test_first_party_rejects_future_naive_and_inverted_windows(self):
        for generated in [self.now + timedelta(seconds=10), self.now.replace(tzinfo=None), self.now - timedelta(days=29)]:
            report = self.first_party_report()
            report["generated_at"] = generated.isoformat()
            with self.subTest(generated=generated), self.assertRaises(ValueError):
                self.collect_report(report).as_request()

    def valid_snapshot(self, provider="github"):
        return MODULE.Snapshot(provider, self.now, self.now - timedelta(days=28),
                               self.now, self.now, {"totals": {}, "private": "private-provider-row"})

    def run_main(self, args, env, snapshots=None):
        snapshots = snapshots or {}
        output = io.StringIO()
        with patch.dict(os.environ, env, clear=True), patch.object(sys, "argv", ["collector", *args]), \
                patch.object(MODULE, "collect_search_console", return_value=snapshots.get("search_console", self.valid_snapshot("search_console"))) as search, \
                patch.object(MODULE, "collect_github", return_value=snapshots.get("github", self.valid_snapshot("github"))) as github, \
                patch.object(MODULE, "collect_first_party", return_value=snapshots.get("first_party", self.valid_snapshot("first_party"))) as first, \
                patch.object(MODULE, "upload_snapshots", return_value={"accepted": 4, "duplicates": 0}) as upload, \
                redirect_stdout(output):
            code = MODULE.main()
        return code, output.getvalue(), upload, (search, github, first)

    def test_dry_run_without_write_credentials_collects_available_sources_only(self):
        code, output, upload, collectors = self.run_main(["--dry-run"], {})
        self.assertEqual(code, 1)
        upload.assert_not_called()
        collectors[0].assert_not_called()
        collectors[1].assert_not_called()
        collectors[2].assert_called_once()
        self.assertIn("available=first_party", output)
        self.assertIn("GSC_SERVICE_ACCOUNT_JSON", output)
        self.assertIn("REPOSITORY_TRAFFIC_TOKEN", output)
        self.assertNotIn("private-provider-row", output)

    def test_live_mode_reports_missing_ingestion_configuration_before_collection(self):
        code, output, upload, collectors = self.run_main([], {})
        self.assertEqual(code, 1)
        self.assertIn("DISCOVERABILITY_INGEST_TOKEN", output)
        upload.assert_not_called()
        for collector in collectors:
            collector.assert_not_called()

    def test_ephemeral_search_token_satisfies_provider_configuration_without_logging(self):
        env = {name: "secret-value-do-not-log" for name in [
            "DISCOVERABILITY_INGEST_TOKEN", "GSC_ACCESS_TOKEN", "REPOSITORY_TRAFFIC_TOKEN"]}
        for args in [["--dry-run"], []]:
            code, output, upload, collectors = self.run_main(args, env)
            with self.subTest(args=args):
                self.assertEqual(code, 0)
                self.assertIn("missing=none", output)
                self.assertNotIn("secret-value-do-not-log", output)
                self.assertNotIn("private-provider-row", output)
                self.assertEqual(collectors[0].call_args.kwargs["access_token"], "secret-value-do-not-log")
                self.assertEqual(collectors[0].call_args.kwargs["service_account_json"], "")
                if args:
                    upload.assert_not_called()

    def test_dry_run_validates_serialization_and_preserves_healthy_providers(self):
        invalid = replace(self.valid_snapshot("first_party"), window_ended_at=self.now + timedelta(days=1))
        env = {name: "secret-value-do-not-log" for name in ["DISCOVERABILITY_INGEST_TOKEN", "GSC_SERVICE_ACCOUNT_JSON", "REPOSITORY_TRAFFIC_TOKEN"]}
        for args in [["--dry-run"], []]:
            code, output, upload, _ = self.run_main(args, env, {"first_party": invalid})
            with self.subTest(args=args):
                self.assertEqual(code, 1)
                self.assertIn("unavailable=first_party", output)
                self.assertNotIn("secret-value-do-not-log", output)
                self.assertNotIn("private-provider-row", output)
                if args:
                    upload.assert_not_called()
                else:
                    self.assertEqual([s.provider for s in upload.call_args.kwargs["snapshots"]], ["search_console", "github"])

    def test_complete_collection_succeeds_without_logging_payloads_or_credentials(self):
        env = {name: "secret-value-do-not-log" for name in ["DISCOVERABILITY_INGEST_TOKEN", "GSC_SERVICE_ACCOUNT_JSON", "REPOSITORY_TRAFFIC_TOKEN"]}
        for args in [["--dry-run"], []]:
            code, output, upload, _ = self.run_main(args, env)
            with self.subTest(args=args):
                self.assertEqual(code, 0)
                self.assertNotIn("secret-value-do-not-log", output)
                self.assertNotIn("private-provider-row", output)
                if args:
                    upload.assert_not_called()
                else:
                    self.assertEqual(upload.call_args.kwargs["ingest_token"], "secret-value-do-not-log")

    def test_collection_exception_diagnostics_do_not_echo_private_values(self):
        output = io.StringIO()
        with patch.dict(os.environ, {}, clear=True), patch.object(sys, "argv", ["collector", "--dry-run"]), \
                patch.object(MODULE, "collect_first_party", side_effect=ValueError("private-query-and-secret")), \
                patch.object(MODULE, "upload_snapshots") as upload, redirect_stdout(output):
            self.assertEqual(MODULE.main(), 1)
        upload.assert_not_called()
        self.assertIn("reason=invalid_evidence", output.getvalue())
        self.assertNotIn("private-query-and-secret", output.getvalue())

    def test_workflow_manual_default_and_shell_flag_boundary(self):
        workflow = (MODULE_PATH.parent.parent / ".github/workflows/discoverability-snapshot.yml").read_text()
        self.assertRegex(workflow, r"dry_run:\s+description:[^\n]+\s+type: boolean\s+default: true")
        self.assertIn("cron: \"17 13 * * 1\"", workflow)
        self.assertIn("github.event_name == 'workflow_dispatch' && inputs.dry_run", workflow)
        step = workflow.split("- name: Snapshot overlapping provider windows\n", 1)[1]
        script = textwrap.dedent(step.split("        run: |\n", 1)[1])
        # Execute the actual workflow shell with a harmless replacement for
        # Python: verify argument forwarding without collection or upload.
        stub = "python() { printf '%s\\n' \"$@\"; };\n"
        for mode, expected in [("true", ["--dry-run"]), ("false", []), ("$(echo unsafe)", [])]:
            with self.subTest(mode=mode):
                result = subprocess.run(["bash", "-c", stub + script], text=True, capture_output=True,
                                        env={**os.environ, "DISCOVERABILITY_DRY_RUN": mode}, check=True)
                self.assertEqual(result.stdout.splitlines(), ["scripts/discoverability_snapshot.py", *expected])

    def test_workflow_keyless_auth_is_bounded_and_immediately_precedes_collection(self):
        workflow = (MODULE_PATH.parent.parent / ".github/workflows/discoverability-snapshot.yml").read_text()
        self.assertIn("if: github.ref == 'refs/heads/main'", workflow)
        self.assertRegex(workflow, r"permissions:\s+contents: read\s+id-token: write")
        self.assertIn("google-github-actions/auth@7c6bc770dae815cd3e89ee6cdf493a5fab2cc093", workflow)
        auth = workflow.split("- name: Authenticate Search Console with keyless identity\n", 1)[1]
        auth_step, collector = auth.split("- name: Snapshot overlapping provider windows\n", 1)
        for expected in [
            "vars.GCP_WORKLOAD_IDENTITY_PROVIDER != '' || vars.GCP_SEARCH_CONSOLE_SERVICE_ACCOUNT != ''",
            "workload_identity_provider: ${{ vars.GCP_WORKLOAD_IDENTITY_PROVIDER }}",
            "service_account: ${{ vars.GCP_SEARCH_CONSOLE_SERVICE_ACCOUNT }}",
            "token_format: access_token", "access_token_lifetime: 600s",
            "access_token_scopes: https://www.googleapis.com/auth/webmasters.readonly",
            "create_credentials_file: false", "export_environment_variables: false",
        ]:
            self.assertIn(expected, auth_step)
        self.assertNotIn("- name:", auth_step)
        self.assertNotIn("- uses:", auth_step)
        self.assertNotIn("continue-on-error:", workflow)
        self.assertNotIn("always()", collector)
        self.assertIn("GSC_ACCESS_TOKEN: ${{ steps.google_auth.outputs.access_token }}", collector)
        before_collector = workflow.split("- name: Snapshot overlapping provider windows\n", 1)[0]
        self.assertNotIn("secrets.", before_collector)
        for name in ["REPOSITORY_TRAFFIC_TOKEN", "GSC_SERVICE_ACCOUNT_JSON", "DISCOVERABILITY_INGEST_TOKEN"]:
            self.assertIn(f"{name}: ${{{{ secrets.{name} }}}}", collector)


if __name__ == "__main__":
    unittest.main()
