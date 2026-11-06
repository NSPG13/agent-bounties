#!/usr/bin/env python3
"""Offline controls. Synthetic logs only; no private operational log fixtures."""
import copy
import datetime as dt
import email.message
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
import urllib.error
import urllib.parse
import urllib.request
import urllib.response
from unittest.mock import Mock, patch
import bootstrap_repair_admission as r
import deploy_public_runtime as controller

AT = r.timestamp('2026-11-05T23:08:30Z')
REVISION = 'a' * 40
ENV = {'GITHUB_REPOSITORY': 'NSPG13/agent-bounties', 'GITHUB_REF': 'refs/heads/main',
       'GITHUB_REF_PROTECTED': 'true', 'GITHUB_SHA': REVISION, 'GITHUB_EVENT_NAME': 'workflow_dispatch',
       'GITHUB_WORKFLOW_REF': 'NSPG13/agent-bounties/.github/workflows/public-runtime-release.yml@refs/heads/main',
       'APPROVED_REVISION': REVISION, 'RELEASE_EVENT': 'workflow_dispatch', 'PRODUCTION_EXPECTED_REVISION': 'b' * 40}

def message(error='Base RPC returned HTTP status 429'):
    return {'schema': 'agent-bounties/indexer-recovery-v1', 'network': 'base-mainnet',
            'factory_contract': r.FACTORY, 'evidence_boundary': r.BOUNDARY, 'error': error,
            'decision': {'action': 'retry_from_persisted_cursor', 'backoff_seconds': 120, 'consecutive_failures': 12}}

def row(identity, at, value):
    return {'id': str(identity), 'timestamp': at, 'message': json.dumps(value),
            'labels': [{'name': k, 'value': v} for k, v in
                       {'resource': r.SERVICE, 'instance': r.INSTANCE, 'type': 'app', 'level': 'error'}.items()]}

def logs():
    startup = [row(n, f'2026-11-05T22:50:{n:02d}Z', message('legacy cursor is not yet safe')) for n in range(10)]
    normalized = [{'timestamp': v['timestamp'], 'message': json.loads(v['message']),
                   'labels': {p['name']: p['value'] for p in v['labels']}} for v in startup]
    return {'hasMore': False, 'logs': startup + [row(10, '2026-11-05T23:08:27Z', message())]}, r.digest(normalized)

def readiness():
    readers = [{'protocol': p, 'ready': True, 'blocker': None, 'cursor_block': 53648297,
                'heartbeat_age_seconds': 5, 'lag_blocks': 0} for p in sorted(r.PROTOCOLS)]
    next(v for v in readers if v['protocol'] == r.PROTOCOL).update(
        ready=False, blocker='reader_failed', cursor_block=r.CURSOR, heartbeat_age_seconds=101, lag_blocks=5038)
    return {'schema': 'agent-bounties/marketplace-readiness-v1', 'ready': False,
            'checked_at': '2026-11-05T23:08:29Z', 'latest_block': 53648525, 'safe_block': 53648297,
            'limits': {'heartbeat_age_seconds': 90, 'lag_blocks': 20}, 'readers': readers}

class Client:
    token = 'synthetic-token-never-transmitted'
    def __init__(self):
        self.manifest = json.loads((Path(__file__).resolve().parents[1] / 'ops/public-runtime-services.json').read_text())
        self.values, self.calls = {}, []
        for expected in self.manifest['services']:
            self.values['services/' + expected['id']] = {**expected, 'ownerId': r.WORKSPACE,
                'repo': controller.PUBLIC, 'branch': 'main', 'autoDeploy': 'no', 'serviceDetails': {
                    'envSpecificDetails': {'dockerfilePath': expected['dockerfile'], 'dockerContext': '.', 'dockerCommand': ''}}}
        for expected in self.manifest['blueprints']:
            self.values['blueprints/' + expected['id']] = {**expected, 'ownerId': r.WORKSPACE,
                'repo': controller.PUBLIC, 'branch': 'main', 'autoSync': False, 'status': 'paused',
                'resources': [{'id': value} for value in expected['resource_ids']]}
        self.deploy = {'id': r.DEPLOY, 'status': 'live', 'commit': {'id': r.PREVIOUS}, 'createdAt': r.CREATED}
        self.values[f'services/{r.SERVICE}/deploys?limit=20'] = [{'deploy': self.deploy}]
        self.values[f'services/{r.SERVICE}/deploys/{r.DEPLOY}'] = self.deploy
    def request(self, method, path, body=None):
        self.calls.append((method, path, body))
        if method == 'POST':
            return {'id': 'dep-synthetic-new', 'status': 'live', 'commit': {'id': REVISION}}
        if method != 'GET':
            raise AssertionError('configuration mutation forbidden')
        return copy.deepcopy(self.values[path])

class AdmissionTests(unittest.TestCase):
    def setUp(self):
        self.network = patch('socket.create_connection', side_effect=AssertionError('network forbidden')).start()
        self.addCleanup(patch.stopall)

    def test_request_requires_exact_unexpired_manual_protected_main(self):
        with patch.dict(os.environ, ENV, clear=True):
            r.validate_request(r.CASE, REVISION, REVISION, 'workflow_dispatch', True, True, None, AT)
            for mutation in ('case', 'revision', 'approval', 'event', 'migrate', 'bootstrap', 'accepted', 'expiry', 'before'):
                args = [r.CASE, REVISION, REVISION, 'workflow_dispatch', True, True, None, AT]
                replacements = {'case': (0, 'other'), 'revision': (1, r.PREVIOUS), 'approval': (2, 'b'*40),
                    'event': (3, 'workflow_run'), 'migrate': (4, False), 'bootstrap': (5, False),
                    'accepted': (6, 'dep-other'), 'expiry': (7, r.timestamp(r.EXPIRES)),
                    'before': (7, r.timestamp(r.CREATED) - dt.timedelta(seconds=1))}
                index, value = replacements[mutation]; args[index] = value
                with self.subTest(mutation=mutation), self.assertRaises(r.AdmissionError):
                    r.validate_request(*args)
            for field in ENV:
                if not field.startswith('GITHUB_'): continue
                with self.subTest(field=field), patch.dict(os.environ, {field: 'wrong'}), self.assertRaises(r.AdmissionError):
                    r.validate_request(r.CASE, REVISION, REVISION, 'workflow_dispatch', True, True, None, AT)

    def test_complete_logs_require_exact_startup_and_fresh_retry(self):
        value, expected = logs()
        with patch.object(r, 'STARTUP_HASH', expected):
            result = r.validate_logs(value, AT)
        self.assertEqual(result['startup_count'], 10)
        self.assertEqual(result['retry_count'], 1)
        self.assertNotIn('message', json.dumps(result))
        self.assertNotIn(r.INSTANCE, json.dumps(result))
        self.assertNotIn(r.FACTORY, json.dumps(result))
        with self.assertRaises(r.AdmissionError): r.validate_logs(value, AT)

    def test_truncated_missing_changed_stale_unknown_or_foreign_logs_reject(self):
        changes = ['truncated', 'missing_has_more', 'empty', 'overflow', 'startup_missing', 'startup_extra',
                   'startup_changed', 'startup_late', 'prefix_only', 'stale', 'future', 'before_deploy',
                   'resource', 'instance', 'level', 'type', 'duplicate_label', 'extra_label', 'duplicate_id',
                   'unknown', 'malformed', 'duplicate_json', 'schema', 'network', 'factory', 'action',
                   'zero_backoff', 'big_backoff', 'boolean_failure', 'extra_message', 'boundary']
        for change in changes:
            value, expected = logs(); last = value['logs'][-1]; body = json.loads(last['message'])
            if change == 'truncated': value['hasMore'] = True
            elif change == 'missing_has_more': del value['hasMore']
            elif change == 'empty': value['logs'] = []
            elif change == 'overflow': value['logs'] *= 10
            elif change == 'startup_missing': value['logs'].pop(0)
            elif change == 'startup_extra': value['logs'].insert(0, {**value['logs'][0], 'id': 'extra'})
            elif change == 'startup_changed': value['logs'][0]['message'] = json.dumps({**body, 'error': 'legacy cursor is not yet safe', 'factory_contract': 'changed'})
            elif change == 'startup_late': value['logs'][0]['timestamp'] = r.STARTUP_CUTOFF
            elif change == 'prefix_only': value['logs'].pop()
            elif change == 'stale': last['timestamp'] = '2026-11-05T23:05:29Z'
            elif change == 'future': last['timestamp'] = '2026-11-05T23:08:31Z'
            elif change == 'before_deploy': last['timestamp'] = '2026-11-05T22:45:18Z'
            elif change in ('resource', 'instance', 'level', 'type'):
                next(p for p in last['labels'] if p['name'] == change)['value'] = 'other'
            elif change == 'duplicate_label': last['labels'].append(last['labels'][0])
            elif change == 'extra_label': last['labels'].append({'name': 'other', 'value': 'x'})
            elif change == 'duplicate_id': last['id'] = value['logs'][0]['id']
            elif change == 'unknown': body['error'] = 'private-unknown-error-never-output'
            elif change == 'malformed': last['message'] = '{'
            elif change == 'duplicate_json': last['message'] = '{"error": "private", "error": "same"}'
            elif change in ('schema', 'network', 'boundary', 'factory'):
                body[{'boundary': 'evidence_boundary', 'factory': 'factory_contract'}.get(change, change)] = 'other'
            elif change == 'action': body['decision']['action'] = 'halt_for_operator'
            elif change == 'zero_backoff': body['decision']['backoff_seconds'] = 0
            elif change == 'big_backoff': body['decision']['backoff_seconds'] = 121
            elif change == 'boolean_failure': body['decision']['consecutive_failures'] = True
            elif change == 'extra_message': body['unreviewed'] = True
            if change in ('unknown', 'schema', 'network', 'boundary', 'factory', 'action', 'zero_backoff', 'big_backoff', 'boolean_failure', 'extra_message'):
                last['message'] = json.dumps(body)
            with self.subTest(change=change), patch.object(r, 'STARTUP_HASH', expected), self.assertRaises(r.AdmissionError) as error:
                r.validate_logs(value, AT)
            self.assertNotIn('private', str(error.exception))

    def test_typed_retry_has_positive_bounded_delay_and_exact_http_class(self):
        for delay, accepted in [('1', True), ('7200', False), ('0', False), ('7201', False),
                                ('9999999999', False), ('01', False), ('-1', False), ('1.5', False)]:
            value, expected = logs()
            value['logs'][-1]['message'] = json.dumps(message(f'Base RPC read retry delay {delay}s (HTTP class 429)'))
            with self.subTest(delay=delay), patch.object(r, 'STARTUP_HASH', expected):
                if accepted: r.validate_logs(value, AT)
                else:
                    with self.assertRaises(r.AdmissionError): r.validate_logs(value, AT)
        value, expected = logs(); value['logs'][-1]['message'] = json.dumps(message('Base RPC read retry delay 10s (HTTP class 503)'))
        with patch.object(r, 'STARTUP_HASH', expected), self.assertRaises(r.AdmissionError): r.validate_logs(value, AT)

    def test_typed_retry_must_preserve_provider_not_before_time(self):
        value, expected = logs()
        value['logs'][-1]['timestamp'] = '2026-11-05T23:06:30Z'
        value['logs'][-1]['message'] = json.dumps(message('Base RPC read retry delay 120s (HTTP class 429)'))
        with patch.object(r, 'STARTUP_HASH', expected):
            with self.assertRaisesRegex(r.AdmissionError, 'provider_hold_active'):
                r.validate_logs(value, AT - dt.timedelta(seconds=1))
            r.validate_logs(value, AT)
        # A newer untyped retry cannot erase an older provider hold.
        value['logs'][-1]['message'] = json.dumps(message('Base RPC read retry delay 7200s (HTTP class 429)'))
        value['logs'].append(row(11, '2026-11-05T23:08:27Z', message()))
        with patch.object(r, 'STARTUP_HASH', expected), self.assertRaisesRegex(r.AdmissionError, 'provider_hold_active'):
            r.validate_logs(value, AT)

    def test_log_query_is_one_unfiltered_full_deploy_window(self):
        value, expected = logs(); client = Client()
        with patch.object(r, 'read', return_value=(json.dumps(value).encode(), '')) as read, patch.object(r, 'STARTUP_HASH', expected):
            r.observe_logs(client, AT)
        self.assertEqual(read.call_count, 1)
        query = urllib.parse.parse_qs(urllib.parse.urlparse(read.call_args.args[0]).query)
        self.assertEqual(query, {'ownerId': [r.WORKSPACE], 'resource': [r.SERVICE], 'type': ['app'],
            'level': ['error'], 'startTime': [r.CREATED], 'endTime': ['2026-11-05T23:08:30Z'],
            'direction': ['forward'], 'limit': ['100']})
        self.assertEqual(read.call_args.kwargs, {'token': client.token})

    def test_readiness_accepts_only_exact_legacy_failure_and_healthy_siblings(self):
        r.validate_readiness(readiness(), AT)
        for change in ('schema', 'ready', 'limits', 'stale', 'future', 'missing', 'duplicate', 'unknown',
                       'legacy_ready', 'legacy_blocker', 'legacy_cursor', 'sibling_failed', 'sibling_stale',
                       'sibling_lag', 'boolean_cursor', 'invalid_head', 'hidden_lag', 'future_cursor'):
            value = readiness(); failed = next(v for v in value['readers'] if v['protocol'] == r.PROTOCOL)
            sibling = next(v for v in value['readers'] if v['protocol'] != r.PROTOCOL)
            if change == 'schema': value['schema'] = 'other'
            elif change == 'ready': value['ready'] = True
            elif change == 'limits': value['limits']['lag_blocks'] = 999
            elif change == 'stale': value['checked_at'] = '2026-11-05T23:07:59Z'
            elif change == 'future': value['checked_at'] = '2026-11-05T23:08:36Z'
            elif change == 'missing': value['readers'].pop()
            elif change == 'duplicate': value['readers'][0] = value['readers'][1]
            elif change == 'unknown': sibling['protocol'] = 'other'
            elif change == 'legacy_ready': failed['ready'] = True
            elif change == 'legacy_blocker': failed['blocker'] = 'reader_lag'
            elif change == 'legacy_cursor': failed['cursor_block'] += 1
            elif change == 'sibling_failed': sibling['blocker'] = 'reader_failed'
            elif change == 'sibling_stale': sibling['heartbeat_age_seconds'] = 91
            elif change == 'sibling_lag': sibling['lag_blocks'] = 21
            elif change == 'boolean_cursor': sibling['cursor_block'] = True
            elif change == 'invalid_head': value['safe_block'] = value['latest_block'] + 1
            elif change == 'hidden_lag': sibling['cursor_block'] = 1
            elif change == 'future_cursor': sibling['cursor_block'] = value['latest_block'] + 1
            with self.subTest(change=change), self.assertRaises(r.AdmissionError): r.validate_readiness(value, AT)

    def test_health_bracket_and_optional_readiness_header_must_agree(self):
        for before, header, after, accepted in [(REVISION, '', REVISION, True),
                (REVISION, REVISION, REVISION, True), ('', '', REVISION, False),
                (REVISION, '', 'b'*40, False), (REVISION, 'b'*40, REVISION, False)]:
            with self.subTest(before=before, header=header, after=after), patch.object(r, 'now', return_value=AT), \
                    patch.object(r, 'read', side_effect=[(b'ok', before), (json.dumps(readiness()).encode(), header), (b'ok', after)]):
                if accepted: r.observe_readiness()
                else:
                    with self.assertRaises(r.AdmissionError): r.observe_readiness()

    def test_trailing_health_read_cannot_extend_original_readiness_age(self):
        value = readiness(); checked = r.timestamp(value['checked_at'])
        with patch.object(r, 'now', side_effect=[checked + dt.timedelta(seconds=29), checked + dt.timedelta(seconds=31)]), \
                patch.object(r, 'read', side_effect=[(b'ok', REVISION), (json.dumps(value).encode(), ''), (b'ok', REVISION)]), \
                self.assertRaisesRegex(r.AdmissionError, 'stale_readiness'):
            r.observe_readiness()

    def test_all_nine_service_identities_and_blueprint_checked_without_patches(self):
        client = Client(); selected = controller.repair_inventory(client, client.manifest)
        self.assertEqual([v[0]['id'] for v in selected], [r.SERVICE])
        self.assertEqual(len(client.calls), 10)
        self.assertTrue(all(v[0] == 'GET' for v in client.calls))
        for expected in client.manifest['services']:
            changed = Client(); changed.values['services/' + expected['id']]['ownerId'] = 'wrong'
            with self.subTest(service=expected['id']), self.assertRaises(controller.ReleaseError):
                controller.repair_inventory(changed, changed.manifest)
        client.values['services/' + r.SERVICE]['repo'] = client.manifest['migration_from_repository']
        with self.assertRaises(controller.ReleaseError): controller.repair_inventory(client, client.manifest)
        client = Client(); client.values['blueprints/' + client.manifest['blueprints'][0]['id']]['autoSync'] = True
        with self.assertRaises(controller.ReleaseError): controller.repair_inventory(client, client.manifest)

    def test_newest_deployment_and_exact_details_are_required(self):
        r.validate_deployment(Client())
        for change in ('newer', 'failed', 'source', 'created', 'empty', 'detail_id', 'detail_source',
                       'older_active', 'unordered_newer', 'older_unknown', 'duplicate_time'):
            client = Client(); path = f'services/{r.SERVICE}/deploys?limit=20'
            if change == 'newer': client.values[path].insert(0, {'deploy': {**client.deploy, 'id': 'dep-newer', 'status': 'build_in_progress'}})
            elif change == 'failed': client.deploy['status'] = 'failed'
            elif change == 'source': client.deploy['commit']['id'] = REVISION
            elif change == 'created': client.deploy['createdAt'] = '2026-11-05T22:45:19Z'
            elif change == 'empty': client.values[path] = []
            elif change in ('older_active', 'unordered_newer', 'older_unknown', 'duplicate_time'):
                client.values[path].append({'deploy': {**client.deploy, 'id': 'dep-other',
                    'createdAt': '2026-11-05T22:46:18Z' if change == 'unordered_newer' else
                                 r.CREATED if change == 'duplicate_time' else '2026-11-05T22:44:18Z',
                    'status': 'build_in_progress' if change == 'older_active' else
                              'unknown' if change == 'older_unknown' else 'deactivated'}})
            else:
                client.values[f'services/{r.SERVICE}/deploys/{r.DEPLOY}'] = {**client.deploy,
                    'id': 'dep-other' if change == 'detail_id' else r.DEPLOY, 'commit': {'id': REVISION}}
            with self.subTest(change=change), self.assertRaises(r.AdmissionError): r.validate_deployment(client)

    def test_pre_mutation_rechecks_binding_age_and_health_without_repeating_log_query(self):
        for failure in ('none', 'new_deploy', 'identity', 'api_revision', 'age', 'expired', 'retry_stale'):
            client = Client(); inspect = Mock(side_effect=lambda: controller.repair_inventory(client, client.manifest))
            receipt = {'case': r.CASE}; health = {'revision': REVISION, 'checked_at': readiness()['checked_at']}
            with patch.object(r, 'now', return_value=AT) as clock, patch.object(r.time, 'monotonic', return_value=0) as monotonic, \
                    patch.object(r, 'observe_logs', return_value=(receipt, AT - dt.timedelta(seconds=170))) as fetch, \
                    patch.object(r, 'observe_readiness', return_value=health) as ready:
                ticket = r.Ticket(client, inspect)
                if failure == 'new_deploy': client.deploy['id'] = 'dep-newer'
                elif failure == 'identity': client.values['services/' + r.SERVICE]['branch'] = 'changed'
                elif failure == 'api_revision': ready.return_value = {**health, 'revision': 'b'*40}
                elif failure == 'age': monotonic.return_value = 61
                elif failure == 'expired': clock.return_value = r.timestamp(r.EXPIRES)
                elif failure == 'retry_stale': clock.return_value = AT + dt.timedelta(seconds=11)
                if failure == 'none':
                    ticket.before_deploy(); self.assertEqual(inspect.call_count, 2)
                else:
                    with self.subTest(failure=failure), self.assertRaises((r.AdmissionError, controller.ReleaseError)): ticket.before_deploy()
                self.assertLessEqual(fetch.call_count, 1)
                if failure == 'none': self.assertEqual(fetch.call_count, 1)
                self.assertTrue(all(call[0] == 'GET' for call in client.calls))

    def test_read_response_is_bounded_no_redirect_and_never_exposes_network_errors(self):
        real_build = urllib.request.build_opener
        for url, expected in [(r.READINESS, 503), ('https://api.render.com/v1/logs?limit=100', 200)]:
            calls = []
            class MemoryHttps(urllib.request.HTTPSHandler):
                def https_open(self, request):
                    calls.append(request.full_url)
                    headers = email.message.Message(); headers['Location'] = 'https://foreign.invalid/'
                    response = urllib.response.addinfourl(io.BytesIO(b'private-body'), headers, request.full_url, 302)
                    response.msg = 'Found'; return response
            with patch.object(r.urllib.request, 'build_opener', side_effect=lambda *handlers: real_build(*handlers, MemoryHttps())), \
                    self.assertRaises(r.AdmissionError) as error:
                r.read(url, token='private-secret' if 'render' in url else None, expected=expected)
            self.assertEqual(calls, [url]); self.assertNotIn('private', str(error.exception))
        opener = Mock(); opener.open.side_effect = OSError('private-secret')
        with patch.object(r.urllib.request, 'build_opener', return_value=opener), self.assertRaises(r.AdmissionError) as error: r.read(r.HEALTH)
        self.assertNotIn('private-secret', str(error.exception))
        headers = email.message.Message()
        response = urllib.response.addinfourl(io.BytesIO(b'x'*65), headers, r.HEALTH, 200)
        opener.open.side_effect = None; opener.open.return_value = response
        with patch.object(r.urllib.request, 'build_opener', return_value=opener), self.assertRaises(r.AdmissionError): r.read(r.HEALTH, limit=64)

    def test_expected_503_body_is_read_before_http_error_is_closed(self):
        headers = email.message.Message(); body = json.dumps(readiness()).encode()
        error = urllib.error.HTTPError(r.READINESS, 503, 'Unavailable', headers, io.BytesIO(body))
        opener = Mock(); opener.open.side_effect = error
        with patch.object(r.urllib.request, 'build_opener', return_value=opener):
            self.assertEqual(r.read(r.READINESS, expected=503)[0], body)

    def test_repair_transport_forbids_settings_and_repeat_uncertain_posts(self):
        client = r.Render('synthetic-token', REVISION)
        for method, path, body in [('PATCH', f'services/{r.SERVICE}', {}),
                ('POST', 'services/srv-other/deploys', {}), ('GET', 'services/../env-vars', None),
                ('POST', f'services/{r.SERVICE}/deploys', {'commitId': 'b'*40, 'clearCache': 'do_not_clear'})]:
            with self.subTest(method=method, path=path), self.assertRaises(r.AdmissionError):
                client.request(method, path, body)
        body = {'commitId': REVISION, 'clearCache': 'do_not_clear'}
        with patch.object(r, 'transport', side_effect=r.AdmissionError('bootstrap_repair_observation_unavailable')) as request:
            with self.assertRaises(r.AdmissionError): client.request('POST', f'services/{r.SERVICE}/deploys', body)
            with self.assertRaises(r.AdmissionError): client.request('POST', f'services/{r.SERVICE}/deploys', body)
            self.assertEqual(request.call_count, 1)

    def test_deploy_metadata_cannot_put_provider_text_in_public_artifacts(self):
        for mutation in ('id', 'status', 'commit', 'live_missing_commit', 'old_id'):
            client = r.Render('synthetic-token', REVISION)
            value = {'id': 'dep-new', 'status': 'build_in_progress', 'commit': {'id': REVISION}}
            if mutation in ('id', 'status'): value[mutation] = 'private-provider-secret'
            elif mutation == 'commit': value['commit'] = {'id': 'private-provider-secret'}
            elif mutation == 'live_missing_commit': value = {'id': 'dep-new', 'status': 'live'}
            else: value['id'] = r.DEPLOY
            with self.subTest(mutation=mutation), patch.object(r, 'transport', return_value=(json.dumps(value).encode(), '')) as request:
                with self.assertRaises(r.AdmissionError) as error:
                    client.request('POST', f'services/{r.SERVICE}/deploys', {'commitId': REVISION, 'clearCache': 'do_not_clear'})
                self.assertNotIn('private-provider-secret', str(error.exception))
                with self.assertRaises(r.AdmissionError):
                    client.request('POST', f'services/{r.SERVICE}/deploys', {'commitId': REVISION, 'clearCache': 'do_not_clear'})
                self.assertEqual(request.call_count, 1)
        client = r.Render('synthetic-token', REVISION)
        value = {'id': 'dep-new', 'status': 'live', 'commit': {'id': REVISION}, 'unexpected': 'private-provider-secret'}
        with patch.object(r, 'transport', return_value=(json.dumps(value).encode(), '')):
            result = client.request('GET', f'services/{r.SERVICE}/deploys/dep-new')
            self.assertNotIn('private-provider-secret', json.dumps(result))
            with self.assertRaises(r.AdmissionError): client.request('GET', f'services/{r.SERVICE}/deploys/dep-other')

    def test_intermediate_deploy_list_cannot_publish_provider_text(self):
        client = r.Render('synthetic-token', REVISION)
        value = {'id': r.DEPLOY, 'status': 'live', 'commit': {'id': r.PREVIOUS}, 'createdAt': r.CREATED}
        for field in ('id', 'status', 'createdAt', 'commit'):
            changed = {**value, field: {'id': 'private-provider-secret'} if field == 'commit' else 'private-provider-secret'}
            with self.subTest(field=field), patch.object(r, 'transport', return_value=(json.dumps([{'deploy': changed}]).encode(), '')), \
                    self.assertRaises(r.AdmissionError) as error:
                client.request('GET', f'services/{r.SERVICE}/deploys?limit=20')
            self.assertNotIn('private-provider-secret', str(error.exception))
        raw = [{'cursor': 'private-provider-secret', 'deploy': {**value, 'extra': 'private-provider-secret',
                'commit': {'id': r.PREVIOUS, 'message': 'private-provider-secret'}}}]
        with patch.object(r, 'transport', return_value=(json.dumps(raw).encode(), '')):
            result = client.request('GET', f'services/{r.SERVICE}/deploys?limit=20')
        self.assertEqual(result, [{'deploy': value}])

    def test_main_only_posts_reader_after_all_checks_and_keeps_acceptance_pending(self):
        client = Client(); value, expected = logs()
        with tempfile.TemporaryDirectory() as directory:
            evidence = Path(directory)/'evidence'
            args = ['release', '--revision', REVISION, '--migrate-source', '--bootstrap-autonomous-readers',
                    '--bootstrap-repair-case', r.CASE, '--evidence-dir', str(evidence)]
            with patch('sys.argv', args), patch.dict(os.environ, ENV, clear=True), patch.object(r, 'Render', return_value=client), \
                    patch.object(controller, 'public_ci_revision') as gates, patch.object(r, 'now', return_value=AT), \
                    patch.object(r, 'STARTUP_HASH', expected), patch.object(r, 'read', side_effect=[
                        (b'ok', REVISION), (json.dumps(readiness()).encode(), ''), (b'ok', REVISION),
                        (json.dumps(value).encode(), ''), (b'ok', REVISION), (json.dumps(readiness()).encode(), ''), (b'ok', REVISION)]), \
                    patch.object(controller, 'verify_live_readers', side_effect=AssertionError('no full promotion')):
                controller.main()
            gates.assert_called_once_with(REVISION)
            writes = [call for call in client.calls if call[0] != 'GET']
            self.assertEqual(writes, [('POST', f'services/{r.SERVICE}/deploys', {'commitId': REVISION, 'clearCache': 'do_not_clear'})])
            result = json.loads((evidence/'release.json').read_text())
            self.assertEqual(result['status'], 'bootstrap_deployed_pending_acceptance')
            raw = json.dumps(result)
            for forbidden in (r.BOUNDARY, r.INSTANCE, r.FACTORY, 'synthetic-token', 'legacy cursor is not yet safe'):
                self.assertNotIn(forbidden, raw)

    def test_main_failures_never_mutate_or_leak_logs(self):
        for failure in ('logs', 'truncated', 'sibling', 'final_sibling', 'source', 'new_deploy', 'final_revision'):
            client = Client(); value, expected = logs(); initial = readiness(); final = readiness()
            if failure == 'logs': value['logs'][-1]['message'] = json.dumps(message('private-provider-error'))
            elif failure == 'truncated': value['hasMore'] = True
            elif failure in ('sibling', 'final_sibling'):
                target = initial if failure == 'sibling' else final
                next(v for v in target['readers'] if v['protocol'] != r.PROTOCOL)['ready'] = False
            elif failure == 'source': client.values['services/' + r.SERVICE]['branch'] = 'other'
            elif failure == 'new_deploy': client.deploy['id'] = 'dep-newer'
            last_revision = 'b'*40 if failure == 'final_revision' else REVISION
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                evidence = Path(directory)/'evidence'
                args = ['release', '--revision', REVISION, '--migrate-source', '--bootstrap-autonomous-readers',
                        '--bootstrap-repair-case', r.CASE, '--evidence-dir', str(evidence)]
                with patch('sys.argv', args), patch.dict(os.environ, ENV, clear=True), patch.object(r, 'Render', return_value=client), \
                        patch.object(controller, 'public_ci_revision'), patch.object(r, 'now', return_value=AT), \
                        patch.object(r, 'STARTUP_HASH', expected), patch.object(r, 'read', side_effect=[
                            (b'ok', REVISION), (json.dumps(initial).encode(), ''), (b'ok', REVISION),
                            (json.dumps(value).encode(), ''), (b'ok', last_revision), (json.dumps(final).encode(), ''), (b'ok', last_revision)]), \
                        self.assertRaises((r.AdmissionError, controller.ReleaseError)) as error:
                    controller.main()
                self.assertTrue(all(call[0] == 'GET' for call in client.calls))
                result = (evidence/'release.json').read_text()
                self.assertEqual(json.loads(result)['status'], 'failed')
                self.assertNotIn('private-provider-error', result + str(error.exception))

if __name__ == '__main__': unittest.main()
