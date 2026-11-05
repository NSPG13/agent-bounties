#!/usr/bin/env python3

from __future__ import annotations

import copy
from contextlib import contextmanager, redirect_stdout
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, HTTPServer
import importlib.util
import io
import json
import sys
import threading
import tempfile
import unittest
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))
REPO_ROOT = SCRIPT_DIR.parent
SPEC = importlib.util.spec_from_file_location("self_heal", SCRIPT_DIR / "self_heal.py")
assert SPEC and SPEC.loader
self_heal = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = self_heal
SPEC.loader.exec_module(self_heal)
from marketplace_readiness import PROTOCOLS


@contextmanager
def hosted_monitor_fixture(reader_ready=True, readiness_status=200, revision='a' * 40,
                           ahead_cursor=None, missing_heads=False, partial_head=None,
                           malformed_readiness=False):
    requests = []

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            requests.append(self.path)
            if self.path == '/health':
                self.send_response(200)
                self.send_header('x-agent-bounties-revision', revision)
                self.send_header('x-agent-bounties-protocol', 'agent-bounties/v1')
                self.end_headers()
                self.wfile.write(b'ok')
            elif self.path == '/readiness/marketplace':
                self.send_response(readiness_status)
                self.end_headers()
                if malformed_readiness:
                    self.wfile.write(b'{"readers":')
                    return
                body = {
                    'schema': 'agent-bounties/marketplace-readiness-v1',
                    'checked_at': datetime.now(timezone.utc).isoformat(),
                    'ready': reader_ready, 'safe_block': 1000, 'latest_block': 1010,
                    'readers': [
                        {'protocol': protocol, 'ready': reader_ready,
                         'blocker': None if reader_ready else 'reader_stale_or_behind',
                         'heartbeat_age_seconds': 1 if reader_ready else 91,
                         'cursor_block': 999, 'lag_blocks': 1}
                        for protocol in sorted(PROTOCOLS)
                    ],
                }
                if ahead_cursor is not None:
                    body['readers'][0].update(cursor_block=ahead_cursor, lag_blocks=0)
                if missing_heads:
                    body.update(ready=False, safe_block=None, latest_block=None)
                    for reader in body['readers']:
                        reader.update(ready=False, blocker='safe_head_unavailable', lag_blocks=None)
                if partial_head is not None:
                    body.update(ready=False)
                    body['latest_block' if partial_head == 'safe' else 'safe_block'] = None
                    for reader in body['readers']:
                        reader.update(ready=False,
                                      blocker='latest_head_unavailable' if partial_head == 'safe'
                                      else 'safe_head_unavailable',
                                      lag_blocks=1 if partial_head == 'safe' else None)
                self.wfile.write(json.dumps(body).encode())
            else:
                self.send_response(404)
                self.end_headers()

        def log_message(self, *args):
            pass

        def do_POST(self):
            requests.append('POST ' + self.path)
            self.send_response(405)
            self.end_headers()

    with HTTPServer(('127.0.0.1', 0), Handler) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield 'http://127.0.0.1:' + str(server.server_port), requests
        finally:
            server.shutdown()
            thread.join()


class SelfHealingContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.policy = self_heal.load_json(REPO_ROOT / "ops/self-healing-policy.json")
        cls.fixtures = REPO_ROOT / "ops/fixtures/recovery-cases.json"

    def test_policy_and_recovery_bench_pass(self) -> None:
        self_heal.validate_policy(self.policy)
        report = self_heal.run_bench(self.policy, self.fixtures)

        self.assertTrue(report["gate"])
        self.assertEqual(report["failed"], 0)
        self.assertGreaterEqual(report["cases"], 12)
        self.assertEqual(report["score"], 1.0)

    def test_prohibited_action_cannot_be_allowlisted(self) -> None:
        policy = copy.deepcopy(self.policy)
        policy["prohibited_automatic_actions"].append("retry_probe")

        with self.assertRaisesRegex(self_heal.ContractError, "both automatic and prohibited"):
            self_heal.validate_policy(policy)

    def test_revision_comparison_ignores_hex_case_but_detects_real_skew(self) -> None:
        fixture = self_heal.load_json(self.fixtures)
        for api_revision, mcp_revision, expected, mismatch in (
            ("a" * 40, "a" * 40, "A" * 40, False),
            ("A" * 40, "a" * 40, "a" * 40, False),
            ("a" * 40, "b" * 40, "A" * 40, True),
            ("b" * 40, "b" * 40, "A" * 40, True),
        ):
            with self.subTest(api=api_revision, mcp=mcp_revision, expected=expected):
                snapshot = self_heal.deep_merge(fixture["base_snapshot"], {
                    "expected_revision": expected,
                    "components": {
                        "api": {"revision": api_revision},
                        "mcp": {"revision": mcp_revision},
                    },
                })
                plan = self_heal.evaluate(self.policy, snapshot)
                self.assertEqual("API/MCP deployed revision skew" in plan["reasons"], mismatch)

    def test_automatic_action_risk_cannot_exceed_r2(self) -> None:
        policy = copy.deepcopy(self.policy)
        policy["automatic_actions"]["retry_probe"]["risk_class"] = "R3"

        with self.assertRaisesRegex(self_heal.ContractError, "must be R0, R1, or R2"):
            self_heal.validate_policy(policy)

    def test_integrity_failure_emits_no_automatic_action(self) -> None:
        fixture = self_heal.load_json(self.fixtures)
        snapshot = self_heal.deep_merge(
            fixture["base_snapshot"],
            {"invariants": {"contract_code_hashes_match": False}},
        )

        plan = self_heal.evaluate(self.policy, snapshot)

        self.assertEqual(plan["decision"], "contained")
        self.assertEqual(plan["automatic_actions"], [])
        self.assertIn(
            "platform:freeze_value_movement",
            {action["action_id"] for action in plan["manual_actions"]},
        )

    def test_remote_plain_http_probe_is_rejected(self) -> None:
        with self.assertRaisesRegex(self_heal.ContractError, "require HTTPS"):
            self_heal.normalize_base_url("http://agent-bounties.example.com")

        self.assertEqual(
            self_heal.normalize_base_url("http://127.0.0.1:8080"),
            "http://127.0.0.1:8080",
        )

    def test_snapshot_rejects_negative_failure_count(self) -> None:
        fixture = self_heal.load_json(self.fixtures)
        snapshot = self_heal.deep_merge(
            fixture["base_snapshot"],
            {"components": {"api": {"consecutive_failures": -1}}},
        )

        with self.assertRaisesRegex(self_heal.ContractError, "non-negative integer"):
            self_heal.validate_snapshot(snapshot)

    def test_green_health_and_stale_or_unavailable_reader_require_manual_investigation(self):
        for status in (200, 503, 404):
            with self.subTest(status=status), hosted_monitor_fixture(False, status) as (url, requests):
                snapshot = self_heal.build_public_snapshot(
                    self.policy, url, url, 'a' * 40, 'a' * 40,
                )
            self.assertEqual(requests.count('/health'), 2)
            self.assertEqual(requests.count('/readiness/marketplace'), 1)
            self.assertEqual(snapshot['components']['api']['state'], 'healthy')
            self.assertEqual(snapshot['components']['mcp']['state'], 'healthy')
            self.assertFalse(snapshot['components']['indexer']['restartable'])
            self.assertTrue(all(value is None for value in snapshot['invariants'].values()))
            plan = self_heal.evaluate(self.policy, snapshot)
            self.assertEqual(plan['decision'], 'escalation_required')
            self.assertEqual(plan['automatic_actions'], [])
            self.assertEqual(
                {action['action_id'] for action in plan['manual_actions']},
                {'indexer:investigate_marketplace_readiness'} if status == 404
                else {'indexer:investigate_indexer_integrity'},
            )
            # Staleness must not weaken an independently reported integrity failure.
            snapshot['invariants']['canonical_event_integrity'] = False
            contained = self_heal.evaluate(self.policy, snapshot)
            self.assertEqual(contained['decision'], 'contained')
            self.assertEqual(contained['automatic_actions'], [])

    def test_ready_readers_do_not_hide_actual_release_skew(self):
        for revision in ('a' * 40, 'b' * 40):
            with hosted_monitor_fixture(revision=revision) as (url, _):
                snapshot = self_heal.build_public_snapshot(self.policy, url, url, 'a' * 40, 'a' * 40)
            plan = self_heal.evaluate(self.policy, snapshot)
            self.assertEqual(plan['automatic_actions'], [])
            self.assertEqual(plan['decision'], 'healthy' if revision == 'a' * 40 else 'contained')
            if revision != 'a' * 40:
                self.assertIn('release:block_rollout', {a['action_id'] for a in plan['manual_actions']})

    def test_impossible_cursor_requires_investigation_without_restart(self):
        with hosted_monitor_fixture(ahead_cursor=2000) as (url, _):
            snapshot = self_heal.build_public_snapshot(self.policy, url, url, 'a' * 40, 'a' * 40)
        plan = self_heal.evaluate(self.policy, snapshot)
        self.assertEqual(plan['decision'], 'escalation_required')
        self.assertEqual(plan['automatic_actions'], [])
        self.assertEqual({a['action_id'] for a in plan['manual_actions']}, {'indexer:investigate_marketplace_readiness'})
        self.assertNotIn('indexer heartbeat or confirmed cursor is stale', plan['reasons'])

    def test_head_unavailable_is_manual_readiness_investigation_not_stale_reader(self):
        with hosted_monitor_fixture(readiness_status=503, missing_heads=True) as (url, requests):
            snapshot = self_heal.build_public_snapshot(self.policy, url, url, 'a' * 40, 'a' * 40)
        indexer = snapshot['components']['indexer']
        self.assertEqual(requests.count('/readiness/marketplace'), 1)
        self.assertEqual(indexer['blocker'], 'marketplace_head_unavailable')
        self.assertEqual(indexer['probe_evidence']['http_status'], 503)
        self.assertEqual(len(indexer['readers']), 3)
        for trusted in (False, True):
            if trusted:
                snapshot['invariants'] = {name: True for name in self_heal.CRITICAL_INVARIANTS}
                indexer['cursor_monotonic'] = True
            plan = self_heal.evaluate(self.policy, snapshot)
            self.assertEqual(plan['decision'], 'escalation_required')
            self.assertEqual(plan['automatic_actions'], [])
            self.assertEqual({a['action_id'] for a in plan['manual_actions']},
                             {'indexer:investigate_marketplace_readiness'})
            self.assertEqual(plan['reasons'], [
                'A required chain head is unavailable; reader readiness cannot be established.'])
        snapshot['invariants']['canonical_event_integrity'] = False
        self.assertEqual(self_heal.evaluate(self.policy, snapshot)['decision'], 'contained')

    def test_partial_heads_remain_manual_even_with_trusted_invariants(self):
        for head in ('safe', 'latest'):
            with hosted_monitor_fixture(readiness_status=503, partial_head=head) as (url, requests):
                snapshot = self_heal.build_public_snapshot(self.policy, url, url, 'a' * 40, 'a' * 40)
            indexer = snapshot['components']['indexer']
            self.assertEqual(requests.count('/readiness/marketplace'), 1)
            self.assertEqual(indexer['state'], 'unavailable')
            self.assertEqual(indexer['blocker'], 'marketplace_head_unavailable')
            self.assertFalse(indexer['restartable'])
            self.assertEqual(len(indexer['readers']), 3)
            for reader in indexer['readers']:
                self.assertFalse(reader['ready'])
                self.assertEqual(reader['lag_blocks'], 1 if head == 'safe' else None)
            snapshot['invariants'] = {name: True for name in self_heal.CRITICAL_INVARIANTS}
            indexer['cursor_monotonic'] = True
            plan = self_heal.evaluate(self.policy, snapshot)
            self.assertEqual(plan['decision'], 'escalation_required')
            self.assertEqual(plan['automatic_actions'], [])
            self.assertEqual({a['action_id'] for a in plan['manual_actions']},
                             {'indexer:investigate_marketplace_readiness'})

    def test_public_stale_report_never_grants_replay_even_with_trusted_invariants(self):
        with hosted_monitor_fixture(reader_ready=False, readiness_status=503) as (url, _):
            snapshot = self_heal.build_public_snapshot(self.policy, url, url, 'a' * 40, 'a' * 40)
        snapshot['components']['indexer']['cursor_monotonic'] = True
        snapshot['invariants'] = {name: True for name in self_heal.CRITICAL_INVARIANTS}
        plan = self_heal.evaluate(self.policy, snapshot)
        self.assertEqual(plan['automatic_actions'], [])
        self.assertEqual({a['action_id'] for a in plan['manual_actions']},
                         {'indexer:investigate_indexer_integrity'})

    def test_bad_pins_preserve_cli_snapshot_and_block_all_repairs(self):
        for expected, verified, blocker in [
                (None, None, 'missing'), ('a' * 40, None, 'missing'),
                (None, 'a' * 40, 'missing'), ('a' * 40, 'b' * 40, 'mismatch'),
                ('main', 'a' * 40, 'invalid'), ('a' * 40, 'a' * 8, 'invalid')]:
            with self.subTest(expected=expected, verified=verified), tempfile.TemporaryDirectory() as directory:
                snapshot_path, plan_path = (Path(directory) / name for name in ('snapshot.json', 'plan.json'))
                with hosted_monitor_fixture(reader_ready=False, readiness_status=503) as (url, requests):
                    args = ['observe', '--policy', str(REPO_ROOT / 'ops/self-healing-policy.json'),
                            '--api-url', url, '--mcp-url', url,
                            '--snapshot-out', str(snapshot_path), '--plan-out', str(plan_path)]
                    if expected is not None:
                        args += ['--expected-revision', expected]
                    if verified is not None:
                        args += ['--marketplace-readiness-verified-revision', verified]
                    with redirect_stdout(io.StringIO()):
                        self.assertEqual(self_heal.main(args), 2)
                self.assertEqual(requests, ['/health', '/health', '/readiness/marketplace'])
                snapshot = json.loads(snapshot_path.read_text())
                plan = json.loads(plan_path.read_text())
                self.assertEqual(snapshot['components']['indexer']['state'], 'stale')
                self.assertEqual(len(snapshot['components']['indexer']['readers']), 3)
                self.assertTrue(snapshot['probe_evidence']['marketplace_readiness_enabled'])
                self.assertFalse(snapshot['probe_evidence']['marketplace_release_qualified'])
                self.assertEqual(plan['automatic_actions'], [])
                self.assertEqual(plan['release_admission']['blocker'], 'marketplace_release_pin_' + blocker)
                self.assertTrue({'release:verify_release_pins', 'indexer:investigate_indexer_integrity'} <=
                                {action['action_id'] for action in plan['manual_actions']})

    def test_bad_pins_deny_every_automatic_action_even_with_trusted_runtime_evidence(self):
        fixture = self_heal.load_json(self.fixtures)
        observed_candidates = set()
        for case in fixture['cases']:
            candidate = self_heal.deep_merge(fixture['base_snapshot'], case.get('patch', {}))
            observed_candidates.update(a['action'] for a in self_heal.evaluate(self.policy, candidate)['automatic_actions'])
            for pins in [{}, {'expected_revision': 'a' * 40},
                         {'expected_revision': 'a' * 40, 'marketplace_verified_revision': 'b' * 40},
                         {'expected_revision': 'main', 'marketplace_verified_revision': 'a' * 40}]:
                snapshot = copy.deepcopy(candidate)
                snapshot.pop('expected_revision', None)
                snapshot.pop('marketplace_verified_revision', None)
                snapshot.update(pins)
                # A cached or supplied success flag cannot substitute for exact pins.
                snapshot['release_admission'] = {'qualified': True, 'blocker': None}
                snapshot['probe_evidence'] = {'marketplace_release_qualified': True}
                plan = self_heal.evaluate(self.policy, snapshot)
                self.assertEqual(plan['automatic_actions'], [], (case['name'], pins))
                self.assertFalse(plan['release_admission']['qualified'])
                self.assertIn('release:verify_release_pins', {a['action_id'] for a in plan['manual_actions']})
        self.assertTrue({'restart_service', 'resume_indexer_from_persisted_cursor',
                         'switch_to_attested_rpc', 'replay_verified_webhook',
                         'suppress_unready_inventory', 'retry_probe'} <= observed_candidates)

    def test_malformed_reader_data_never_authorizes_restart_even_after_pins_match(self):
        for verified in (None, 'b' * 40, 'a' * 40):
            with hosted_monitor_fixture(malformed_readiness=True) as (url, requests):
                snapshot = self_heal.build_public_snapshot(self.policy, url, url, 'a' * 40, verified)
            snapshot['invariants'] = {name: True for name in self_heal.CRITICAL_INVARIANTS}
            snapshot['components']['indexer']['cursor_monotonic'] = True
            self.assertEqual(requests, ['/health', '/health', '/readiness/marketplace'])
            self.assertEqual(snapshot['components']['indexer']['blocker'], 'marketplace_readiness_invalid')
            self.assertFalse(snapshot['components']['indexer']['restartable'])
            plan = self_heal.evaluate(self.policy, snapshot)
            self.assertEqual(plan['automatic_actions'], [])
            self.assertIn('indexer:investigate_marketplace_readiness',
                          {action['action_id'] for action in plan['manual_actions']})


if __name__ == "__main__":
    unittest.main()
