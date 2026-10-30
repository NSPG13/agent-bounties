import copy
from datetime import datetime, timezone, timedelta
import json
import hashlib
from pathlib import Path
import sys
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parent))
from marketplace_readiness import PROTOCOLS, enabled_for_release, inspect, main, observe


class MarketplaceReadinessTests(unittest.TestCase):
    def test_activation_requires_exact_verified_release_and_does_not_probe_early(self):
        self.assertFalse(enabled_for_release('a' * 40, None))
        self.assertTrue(enabled_for_release('a' * 40, 'A' * 40))
        for expected, verified in [('a' * 40, 'b' * 40), ('main', 'a' * 40),
                                   (None, 'a' * 40), ('a' * 40, 'a' * 8)]:
            with self.assertRaises(ValueError):
                enabled_for_release(expected, verified)
        with patch('marketplace_readiness.observe') as probe:
            self.assertEqual(main(['--api-base-url', 'https://example.invalid',
                                   '--expected-revision', 'a' * 40]), 0)
            probe.assert_not_called()

    def setUp(self):
        self.now = datetime.now(timezone.utc)
        self.payload = {'schema': 'agent-bounties/marketplace-readiness-v1', 'ready': True,
                        'checked_at': self.now.isoformat(), 'safe_block': 1000, 'latest_block': 1010,
                        'readers': [{'protocol': p, 'ready': True, 'blocker': None,
                                     'heartbeat_age_seconds': 1, 'cursor_block': 999, 'lag_blocks': 1}
                                    for p in sorted(PROTOCOLS)]}

    def test_all_readers_required_even_when_services_and_aggregate_are_green(self):
        self.assertEqual(inspect(self.payload, self.now)['state'], 'healthy')
        for position in range(3):
            for patch in [{'ready': False}, {'blocker': 'reader_failed'}, {'heartbeat_age_seconds': 91},
                          {'heartbeat_age_seconds': -1}, {'lag_blocks': 21}, {'cursor_block': 900},
                          {'heartbeat_age_seconds': True}, {'cursor_block': None}]:
                data = copy.deepcopy(self.payload); data['readers'][position].update(patch)
                self.assertNotEqual(inspect(data, self.now)['state'], 'healthy')

    def test_configured_v2_is_checked_with_every_existing_reader(self):
        data = copy.deepcopy(self.payload)
        data['readers'].append(dict(data['readers'][0], protocol='agent-bounties/autonomous-v2'))
        result = inspect(data, self.now)
        self.assertEqual(result['state'], 'healthy')
        self.assertEqual(len(result['readers']), 4)
        self.assertFalse(result['restartable'])
        for change in [{'ready': False, 'blocker': 'reader_failed'},
                       {'heartbeat_age_seconds': 91}, {'cursor_block': 1011},
                       {'lag_blocks': 21}, {'heartbeat_age_seconds': True},
                       {'cursor_block': None}]:
            invalid = copy.deepcopy(data)
            invalid['readers'][-1].update(change)
            result = inspect(invalid, self.now)
            self.assertNotEqual(result['state'], 'healthy')
            self.assertFalse(result['restartable'])

    def test_v2_cannot_replace_required_reader_or_admit_duplicates_or_unknowns(self):
        v2 = dict(self.payload['readers'][0], protocol='agent-bounties/autonomous-v2')
        for readers in [self.payload['readers'][1:] + [v2],
                        self.payload['readers'] + [self.payload['readers'][0]],
                        self.payload['readers'] + [v2, v2],
                        self.payload['readers'] + [dict(v2, protocol='unknown')],
                        self.payload['readers'] + [None]]:
            result = inspect(dict(self.payload, readers=readers), self.now)
            self.assertEqual(result['state'], 'unavailable')
            self.assertFalse(result['restartable'])

    def test_missing_duplicate_future_or_stale_data_fail_closed(self):
        for patch in [{'readers': self.payload['readers'][:2]},
                      {'readers': [self.payload['readers'][0]] * 3}, {'safe_block': None},
                      {'checked_at': (self.now - timedelta(seconds=91)).isoformat()},
                      {'checked_at': (self.now + timedelta(seconds=1)).isoformat()},
                      {'checked_at': 'invalid'}, {'schema': 'other'}, {'readers': [None]*3}]:
            data = dict(self.payload, **patch)
            self.assertEqual(inspect(data, self.now)['state'], 'unavailable')

    def test_cursor_upper_bound_respects_each_protocol_clock(self):
        for protocol in PROTOCOLS:
            for cursor, expected in [(1000, True), (1008, True), (2000, False)]:
                data = copy.deepcopy(self.payload)
                reader = next(r for r in data['readers'] if r['protocol'] == protocol)
                reader.update(cursor_block=cursor, lag_blocks=0)
                with self.subTest(protocol=protocol, cursor=cursor):
                    result = inspect(data, self.now)
                    self.assertEqual(result['state'] == 'healthy', expected)
                    self.assertFalse(result['restartable'])
        for latest in [None, True, -1, 999, '1010']:
            self.assertEqual(inspect(dict(self.payload, latest_block=latest), self.now)['state'], 'unavailable')
        missing = dict(self.payload); del missing['latest_block']
        self.assertEqual(inspect(missing, self.now)['state'], 'unavailable')
        # A different provider's recently sampled safe tag may lag even the
        # Beta3 reader's previous safe checkpoint. Latest remains the upper bound.
        data = copy.deepcopy(self.payload)
        data.update(safe_block=52706877, latest_block=52706963)
        for reader in data['readers']:
            reader.update(cursor_block=52706913, lag_blocks=0)
        self.assertEqual(inspect(data, self.now)['state'], 'healthy')

    def test_real_http_503_and_200_both_check_body(self):
        payload = self.payload
        response_status = [200]
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(response_status[0]); self.end_headers()
                self.wfile.write(json.dumps(payload).encode())
            def log_message(self, *args): pass
        with HTTPServer(('127.0.0.1', 0), Handler) as server:
            thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
            try:
                url = 'http://127.0.0.1:' + str(server.server_port)
                self.assertEqual(observe(url)['state'], 'healthy')
                payload['ready'] = False
                payload['readers'][0].update(ready=False, blocker='reader_stale_or_behind',
                                             heartbeat_age_seconds=91)
                response_status[0] = 503
                result = observe(url)
                self.assertEqual(result['state'], 'stale')
                self.assertFalse(result['restartable'])
                payload['ready'] = True
                payload['readers'][0].update(ready=True, blocker=None, heartbeat_age_seconds=1)
                self.assertEqual(observe(url)['state'], 'unavailable')
            finally: server.shutdown(); thread.join()

    def head_unavailable(self):
        payload = copy.deepcopy(self.payload)
        payload.update(ready=False, safe_block=None, latest_block=None)
        for reader in payload['readers']:
            reader.update(ready=False, blocker='safe_head_unavailable', lag_blocks=None)
        return payload

    def test_missing_head_preserves_bounded_readers_without_claiming_staleness(self):
        payload = self.head_unavailable()
        result = inspect(payload, self.now)
        self.assertEqual(result['state'], 'unavailable')
        self.assertEqual(result['blocker'], 'marketplace_head_unavailable')
        self.assertEqual(result['readers'], payload['readers'])
        self.assertFalse(result['restartable'])
        for update in [{'ready': True}, {'safe_block': 1000}, {'latest_block': True}]:
            invalid = inspect(dict(payload, **update), self.now)
            self.assertEqual(invalid['blocker'], 'marketplace_readiness_invalid')
        for update in [{'ready': True}, {'blocker': None}, {'lag_blocks': 0},
                       {'blocker': 'secret-provider-url'}, {'cursor_block': True},
                       {'heartbeat_age_seconds': 'secret-database-error'}]:
            malformed = copy.deepcopy(payload)
            malformed['readers'][0].update(update)
            result = inspect(malformed, self.now)
            self.assertEqual(result['blocker'], 'marketplace_readiness_invalid')
            self.assertNotIn('secret-', json.dumps(result))

    def test_partial_heads_preserve_validated_unavailable_diagnostics(self):
        for safe, latest, cursor, lag, blocker in [
                (1000, None, 999, 1, 'latest_head_unavailable'),
                (1000, None, 979, 21, 'latest_head_unavailable'),
                (None, 1010, 999, None, 'safe_head_unavailable'),
                (1000, None, None, None, 'durable_cursor_unavailable'),
                (None, 1010, None, None, 'durable_cursor_unavailable'),
                (2**64 - 1, None, 2**64 - 1, 0, 'latest_head_unavailable')]:
            with self.subTest(safe=safe, latest=latest, cursor=cursor):
                payload = self.head_unavailable()
                payload.update(safe_block=safe, latest_block=latest)
                for reader in payload['readers']:
                    reader.update(blocker=blocker, cursor_block=cursor, lag_blocks=lag)
                result = inspect(payload, self.now)
                self.assertEqual(result['state'], 'unavailable')
                self.assertEqual(result['blocker'], 'marketplace_head_unavailable')
                self.assertEqual(result['readers'], payload['readers'])
                self.assertFalse(result['restartable'])

    def test_partial_heads_reject_malformed_or_inconsistent_evidence(self):
        for present in ('safe_block', 'latest_block'):
            payload = self.head_unavailable()
            payload[present] = 1000
            if present == 'safe_block':
                for reader in payload['readers']:
                    reader.update(blocker='latest_head_unavailable', lag_blocks=1)
            for value in (True, -1, 2**64, 1000.0, 'secret-head'):
                result = inspect(dict(payload, **{present: value}), self.now)
                self.assertEqual(result['validation_reason'], 'head_order_invalid')
                self.assertFalse(result['restartable'])
                self.assertNotIn('secret-head', json.dumps(result))
            result = inspect(dict(payload, ready=True), self.now)
            self.assertEqual(result['validation_reason'], 'head_readiness_inconsistent')
            updates = [{'ready': True, 'blocker': None}, {'blocker': None},
                       {'blocker': 'secret-blocker'}, {'lag_blocks': 0},
                       {'cursor_block': None, 'lag_blocks': 0}]
            if present == 'safe_block':
                updates.append({'lag_blocks': None})
            for position in range(3):
                for update in updates:
                    malformed = copy.deepcopy(payload)
                    malformed['readers'][position].update(update)
                    result = inspect(malformed, self.now)
                    self.assertEqual(result['blocker'], 'marketplace_readiness_invalid')
                    self.assertFalse(result['restartable'])
                    self.assertNotIn('secret-', json.dumps(result))

    def test_clock_boundaries_have_distinct_codes_without_tolerance(self):
        for age, blocker in [(0, None), (90, None),
                             (90.000001, 'marketplace_readiness_stale'),
                             (-0.000001, 'marketplace_readiness_clock_skew')]:
            result = inspect(self.payload, self.now + timedelta(seconds=age))
            self.assertEqual(result['blocker'], blocker)
            self.assertEqual(result['report_age_seconds'], age)
            self.assertFalse(result['restartable'])
        for timestamp in ['invalid-secret', self.now.replace(tzinfo=None).isoformat(), None, True]:
            result = inspect(dict(self.payload, checked_at=timestamp), self.now)
            self.assertEqual(result['validation_reason'], 'checked_at_invalid')
            self.assertNotIn('invalid-secret', json.dumps(result))

    def test_actual_reader_failure_and_staleness_are_distinct(self):
        for position in range(3):
            for blocker, age, state, aggregate in [
                    ('reader_failed', 1, 'unavailable', 'marketplace_reader_unavailable'),
                    ('reader_stale_or_behind', 91, 'stale', 'marketplace_reader_not_ready'),
                    ('heartbeat_in_future', -1, 'unavailable', 'marketplace_reader_unavailable')]:
                payload = copy.deepcopy(self.payload)
                payload['ready'] = False
                payload['readers'][position].update(ready=False, blocker=blocker,
                                                    heartbeat_age_seconds=age)
                result = inspect(payload, self.now)
                self.assertEqual((result['state'], result['blocker']), (state, aggregate))
                self.assertEqual(result['readers'][position]['blocker'], blocker)
                self.assertFalse(result['restartable'])

    def test_existing_heartbeat_and_lag_limits_remain_exact(self):
        for age, lag, healthy in [(90, 20, True), (91, 20, False), (90, 21, False)]:
            payload = copy.deepcopy(self.payload)
            payload['readers'][0].update(heartbeat_age_seconds=age, cursor_block=1000 - lag,
                                        lag_blocks=lag)
            self.assertEqual(inspect(payload, self.now)['state'] == 'healthy', healthy)

    def test_missing_cursor_cannot_establish_even_zero_lag(self):
        payload = copy.deepcopy(self.payload)
        payload['ready'] = False
        reader = payload['readers'][0]
        reader.update(ready=False, blocker='durable_cursor_unavailable', cursor_block=None,
                      lag_blocks=None)
        self.assertEqual(inspect(payload, self.now)['blocker'], 'marketplace_reader_unavailable')
        for lag in (0, 21):
            reader['lag_blocks'] = lag
            result = inspect(payload, self.now)
            self.assertEqual(result['state'], 'unavailable')
            self.assertEqual(result['blocker'], 'marketplace_readiness_invalid')
            self.assertEqual(result['validation_reason'], 'reader_lag_inconsistent')

    def test_real_http_unavailable_head_and_malformed_responses_keep_safe_receipts(self):
        payload = self.head_unavailable()
        payload['private_marker'] = 'secret-upstream-detail'
        payload['readers'][0]['raw_error'] = 'secret-upstream-detail'
        response = [503, json.dumps(payload).encode()]
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(response[0]); self.end_headers(); self.wfile.write(response[1])
            def log_message(self, *args): pass
        with HTTPServer(('127.0.0.1', 0), Handler) as server:
            thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
            try:
                url = 'http://127.0.0.1:' + str(server.server_port)
                result = observe(url)
                self.assertEqual(result['blocker'], 'marketplace_head_unavailable')
                self.assertEqual(len(result['readers']), 3)
                self.assertNotIn('secret-upstream-detail', json.dumps(result))
                receipt = result['probe_evidence']
                self.assertEqual(receipt['http_status'], 503)
                self.assertEqual(receipt['response_sha256'], hashlib.sha256(response[1]).hexdigest())
                self.assertEqual(receipt['response_sha256_scope'], 'complete_body')
                self.assertLessEqual(datetime.fromisoformat(receipt['started_at']),
                                     datetime.fromisoformat(receipt['finished_at']))
                for safe, latest, lag, blocker in [
                        (1000, None, 1, 'latest_head_unavailable'),
                        (None, 1010, None, 'safe_head_unavailable')]:
                    partial = self.head_unavailable()
                    partial.update(safe_block=safe, latest_block=latest)
                    for reader in partial['readers']:
                        reader.update(lag_blocks=lag, blocker=blocker)
                    response[:] = [503, json.dumps(partial).encode()]
                    result = observe(url)
                    self.assertEqual(result['blocker'], 'marketplace_head_unavailable')
                    self.assertEqual(result['readers'], partial['readers'])
                    self.assertFalse(result['restartable'])
                for status, body, code in [(503, b'{bad-secret-json', 'json_invalid'),
                                           (503, b'x' * 70000, 'response_too_large'),
                                           (404, b'secret-error-page', 'http_status_unavailable')]:
                    response[:] = [status, body]
                    result = observe(url)
                    self.assertEqual(result['state'], 'unavailable')
                    self.assertEqual(result['validation_reason'], code)
                    self.assertNotIn('secret', json.dumps(result))
                    self.assertLessEqual(result['probe_evidence']['response_bytes'], 65537)
                    if code == 'response_too_large':
                        self.assertEqual(result['probe_evidence']['response_sha256_scope'], 'bounded_prefix')
                        self.assertEqual(result['probe_evidence']['response_sha256'],
                                         hashlib.sha256(body[:65537]).hexdigest())
            finally: server.shutdown(); thread.join()
        with patch('marketplace_readiness.urllib.request.urlopen',
                   side_effect=OSError('https://secret-provider.invalid/?key=secret')):
            result = observe('https://example.invalid')
        self.assertEqual(result['blocker'], 'marketplace_probe_unavailable')
        self.assertNotIn('secret', json.dumps(result))


if __name__ == '__main__': unittest.main()
