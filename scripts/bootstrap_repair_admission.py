#!/usr/bin/env python3
"""One expiring, manual admission for the reviewed 2026-11-05 bootstrap incident.

This is not a general restart policy. Raw authenticated logs remain in memory;
only the fixed case identity, counts and evidence hashes may enter release artifacts.
"""
from __future__ import annotations
import datetime as dt
import hashlib
import json
import os
import re
import time
import urllib.error
import urllib.parse
import urllib.request

CASE = 'legacy-bootstrap-429-20261105'
EXPIRES = '2026-11-06T03:00:00Z'
SERVICE = 'srv-d982ftu7r5hc73ar4st0'
WORKSPACE = 'tea-d4tk87f5r7bs73au55lg'
DEPLOY = 'dep-dbmgdvmgekts73er1irg'
PREVIOUS = 'db53a13bc48839c6cfc2a6f722ff96be5bfeacea'
CREATED = '2026-11-05T22:45:18.212709Z'
INSTANCE = SERVICE + '-rswfh'
FACTORY = '0x082c52131aaf0c56e76b075f895eab6fcab6d2f9'
CURSOR = 53643259
PROTOCOL = 'agent-bounties/autonomous-v1'
PROTOCOLS = {PROTOCOL, 'agent-bounties/autonomous-v2', 'agent-bounties/open-competition-v1',
             'agent-bounties/open-competition-v2-beta3'}
STARTUP_COUNT = 10
STARTUP_HASH = '75ffe178265772590d4188ba68a48d55053d008ce81d869f4554ec22fdb0c2d2'
STARTUP_CUTOFF = '2026-11-05T22:51:00Z'
BOUNDARY = ('Retry resumes from the persisted monotonic cursor. It cannot create funding, '
            'verification, payout, or settlement evidence.')
READINESS = 'https://api.agentizable.ai/readiness/marketplace'
HEALTH = 'https://api.agentizable.ai/health'
DEPLOY_STATUSES = {'created', 'queued', 'build_in_progress', 'pre_deploy_in_progress',
                  'update_in_progress', 'live', 'deactivated', 'build_failed',
                  'pre_deploy_failed', 'update_failed', 'canceled'}

class AdmissionError(RuntimeError):
    """Only stable, non-sensitive reason codes are supplied to this exception."""

def require(ok, code):
    if not ok:
        raise AdmissionError('bootstrap_repair_' + code)

def timestamp(value):
    require(isinstance(value, str) and re.fullmatch(r'\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,9})?Z', value), 'invalid_timestamp')
    try:
        return dt.datetime.fromisoformat(value.replace('Z', '+00:00'))
    except ValueError:
        raise AdmissionError('bootstrap_repair_invalid_timestamp') from None

def now():
    return dt.datetime.now(dt.timezone.utc)

def integer(value, minimum=0, maximum=2**63-1):
    return type(value) is int and minimum <= value <= maximum

def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()

def strict_json(raw):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, 'duplicate_json_key')
            result[key] = value
        return result
    try:
        return json.loads(raw, object_pairs_hook=pairs,
                          parse_constant=lambda _: require(False, 'invalid_json'))
    except (ValueError, TypeError, UnicodeError, RecursionError):
        raise AdmissionError('bootstrap_repair_invalid_json') from None

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None

def read(url, token=None, expected=200, limit=524288):
    headers = {'Accept': 'application/json', 'Cache-Control': 'no-cache'}
    if token:
        require(url.startswith('https://api.render.com/v1/logs?'), 'invalid_log_origin')
        headers['Authorization'] = 'Bearer ' + token
    request = urllib.request.Request(url, headers=headers, method='GET')
    return transport(request, {expected}, limit)

def transport(request, statuses, limit):
    try:
        try:
            response = urllib.request.build_opener(NoRedirect()).open(request, timeout=15)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            require(response.status in statuses and response.geturl() == request.full_url, 'unexpected_response')
            raw = response.read(limit + 1)
            require(len(raw) <= limit, 'response_too_large')
            return raw, response.headers.get('x-agent-bounties-revision', '')
    except AdmissionError:
        raise
    except Exception:
        # Network exceptions can include URLs/headers/provider text. Never expose them.
        raise AdmissionError('bootstrap_repair_observation_unavailable') from None

class Render:
    """Bounded, no-redirect repair transport; no configuration writes or retries."""
    def __init__(self, token, revision):
        require(isinstance(token, str) and bool(token), 'credential_missing')
        self.token, self.revision = token, revision
        self.post_attempted = False

    def request(self, method, path, body=None):
        if method == 'GET':
            require(body is None and bool(re.fullmatch(
                r'(?:services/srv-[a-z0-9]+(?:/deploys(?:\?limit=20|/dep-[a-z0-9]+))?|blueprints/exs-[a-z0-9]+)', path)),
                'request_not_allowed')
        else:
            require(method == 'POST' and path == f'services/{SERVICE}/deploys' and not self.post_attempted and
                    body == {'commitId': self.revision, 'clearCache': 'do_not_clear'}, 'request_not_allowed')
            # Consume before sending; an unknown outcome requires reconciliation.
            self.post_attempted = True
        request = urllib.request.Request('https://api.render.com/v1/' + path, method=method,
            data=json.dumps(body).encode() if body is not None else None,
            headers={'Authorization': 'Bearer ' + self.token, 'Content-Type': 'application/json', 'Accept': 'application/json'})
        raw, _ = transport(request, {200} if method == 'GET' else {200, 201, 202}, 1048576)
        value = strict_json(raw)
        if method == 'GET' and path.endswith('/deploys?limit=20'):
            require(isinstance(value, list) and 1 <= len(value) <= 20, 'invalid_deploy_response')
            projected = []
            for item in value:
                require(isinstance(item, dict), 'invalid_deploy_response')
                row = item.get('deploy', item)
                require(isinstance(row, dict) and isinstance(row.get('id'), str) and
                        re.fullmatch(r'dep-[a-z0-9]+', row['id']) and
                        isinstance(row.get('status'), str) and row['status'] in DEPLOY_STATUSES,
                        'invalid_deploy_response')
                timestamp(row.get('createdAt'))
                safe = {'id': row['id'], 'status': row['status'], 'createdAt': row['createdAt']}
                if 'commit' in row or row['status'] == 'live':
                    require(isinstance(row.get('commit'), dict) and isinstance(row['commit'].get('id'), str) and
                            re.fullmatch(r'[0-9a-f]{40}', row['commit']['id']), 'invalid_deploy_response')
                    safe['commit'] = {'id': row['commit']['id']}
                projected.append({'deploy': safe})
            return projected
        if method == 'POST' or re.fullmatch(r'services/srv-[a-z0-9]+/deploys/dep-[a-z0-9]+', path):
            # The general controller saves deployment status/id. Keep untrusted
            # provider text out of that public artifact, including pending polls.
            expected_id = path.rsplit('/', 1)[-1] if method == 'GET' else None
            expected_revision = PREVIOUS if expected_id == DEPLOY else self.revision
            require(isinstance(value, dict) and isinstance(value.get('id'), str) and
                    re.fullmatch(r'dep-[a-z0-9]+', value['id']) and
                    (value['id'] == expected_id if expected_id else value['id'] != DEPLOY), 'invalid_deploy_response')
            require(isinstance(value.get('status'), str) and value['status'] in DEPLOY_STATUSES,
                'invalid_deploy_response')
            result = {'id': value['id'], 'status': value['status']}
            if 'commit' in value or value['status'] == 'live':
                require(isinstance(value.get('commit'), dict) and value['commit'].get('id') == expected_revision,
                        'invalid_deploy_response')
                result['commit'] = {'id': expected_revision}
            if expected_id == DEPLOY:
                require(value.get('createdAt') == CREATED, 'invalid_deploy_response')
                result['createdAt'] = CREATED
            return result
        return value

def validate_request(case, revision, approved, event, migrate, bootstrap, accepted, at):
    require(case == CASE and timestamp(CREATED) <= at < timestamp(EXPIRES), 'case_unavailable')
    require(event == 'workflow_dispatch' and approved == revision and
            bool(re.fullmatch(r'[0-9a-f]{40}', revision)) and revision != PREVIOUS and
            migrate is True and bootstrap is True and not accepted, 'manual_approval_required')
    require(os.environ.get('GITHUB_REPOSITORY') == 'NSPG13/agent-bounties' and
            os.environ.get('GITHUB_REF') == 'refs/heads/main' and
            os.environ.get('GITHUB_REF_PROTECTED') == 'true' and
            os.environ.get('GITHUB_SHA') == revision and
            os.environ.get('GITHUB_EVENT_NAME') == 'workflow_dispatch' and
            os.environ.get('GITHUB_WORKFLOW_REF') ==
            'NSPG13/agent-bounties/.github/workflows/public-runtime-release.yml@refs/heads/main',
            'protected_workflow_required')

def validate_deployment(client):
    values = client.request('GET', f'services/{SERVICE}/deploys?limit=20')
    require(isinstance(values, list) and values, 'missing_deployment')
    rows = [row.get('deploy', row) for row in values if isinstance(row, dict)]
    require(len(rows) == len(values) and all(isinstance(row, dict) for row in rows), 'invalid_deployment')
    require(len(rows) <= 20 and all(isinstance(row.get('id'), str) and
            re.fullmatch(r'dep-[a-z0-9]+', row['id']) for row in rows), 'invalid_deployment')
    require(len({row.get('id') for row in rows}) == len(rows), 'invalid_deployment')
    dates = [timestamp(row.get('createdAt')) for row in rows]
    require(all(earlier > later for earlier, later in zip(dates, dates[1:])), 'deployment_order_changed')
    require(all(row.get('status') in {'deactivated', 'build_failed', 'pre_deploy_failed', 'update_failed', 'canceled'}
                for row in rows[1:]), 'competing_deployment')
    value = rows[0]
    require(value.get('id') == DEPLOY and value.get('status') == 'live' and
            value.get('commit', {}).get('id') == PREVIOUS and value.get('createdAt') == CREATED,
            'deployment_changed')
    value = client.request('GET', f'services/{SERVICE}/deploys/{DEPLOY}')
    require(isinstance(value, dict) and value.get('id') == DEPLOY and value.get('status') == 'live' and
            value.get('commit', {}).get('id') == PREVIOUS and value.get('createdAt') == CREATED,
            'deployment_changed')

def validate_logs(value, end, observation=None):
    require(isinstance(value, dict) and value.get('hasMore') is False and
            isinstance(value.get('logs'), list) and 1 <= len(value['logs']) <= 100, 'incomplete_logs')
    startup, retries, holds, ids = [], [], [], set()
    labels_expected = {'resource': SERVICE, 'instance': INSTANCE, 'level': 'error', 'type': 'app'}
    for row in value['logs']:
        require(isinstance(row, dict) and isinstance(row.get('id'), str) and row['id'] and
                row['id'] not in ids and isinstance(row.get('labels'), list), 'invalid_log_row')
        ids.add(row['id'])
        labels = {}
        for label in row['labels']:
            require(isinstance(label, dict) and set(label) == {'name', 'value'} and
                    isinstance(label['name'], str) and label['name'] not in labels, 'invalid_log_labels')
            labels[label['name']] = label['value']
        require(labels == labels_expected, 'unexpected_log_identity')
        at = timestamp(row.get('timestamp'))
        require(timestamp(CREATED) <= at <= end, 'invalid_log_window')
        require(isinstance(row.get('message'), str) and len(row['message']) <= 8192, 'invalid_log_message')
        message = strict_json(row['message'])
        require(isinstance(message, dict), 'unknown_error')
        if message.get('error') == 'legacy cursor is not yet safe':
            require(at < timestamp(STARTUP_CUTOFF), 'unexpected_startup_error')
            startup.append({'timestamp': row['timestamp'], 'message': message, 'labels': labels})
            continue
        require(set(message) == {'schema', 'decision', 'error', 'evidence_boundary', 'factory_contract', 'network'}, 'unknown_error')
        require(message['schema'] == 'agent-bounties/indexer-recovery-v1' and
                message['factory_contract'] == FACTORY and message['network'] == 'base-mainnet' and
                message['evidence_boundary'] == BOUNDARY, 'unknown_error')
        decision = message['decision']
        require(isinstance(decision, dict) and set(decision) == {'action', 'backoff_seconds', 'consecutive_failures'} and
                decision['action'] == 'retry_from_persisted_cursor' and integer(decision['backoff_seconds'], 1, 120) and
                integer(decision['consecutive_failures'], 1, 10000), 'unknown_error')
        error = message['error']
        typed = re.fullmatch(r'Base RPC read retry delay ([1-9][0-9]{0,3})s \(HTTP class 429\)', error) if isinstance(error, str) else None
        require(error == 'Base RPC returned HTTP status 429' or
                (typed is not None and int(typed[1]) <= 7200), 'unknown_error')
        if typed is not None:
            holds.append(at + dt.timedelta(seconds=int(typed[1])))
        retries.append(at)
    startup.sort(key=lambda row: row['timestamp'])
    require(len(startup) == STARTUP_COUNT and digest(startup) == STARTUP_HASH, 'startup_evidence_changed')
    require(retries and 0 <= (end - max(retries)).total_seconds() <= 180, 'fresh_retry_missing')
    require(all(end >= hold for hold in holds), 'provider_hold_active')
    if observation is not None:
        observation['latest_retry'] = max(retries)
    return {'case': CASE, 'previous_deploy': DEPLOY, 'previous_revision': PREVIOUS,
            'startup_count': len(startup), 'startup_sha256': STARTUP_HASH,
            'retry_count': len(retries), 'error_count': len(value['logs']), 'logs_sha256': digest(value)}

def observe_logs(client, end):
    query = urllib.parse.urlencode({'ownerId': WORKSPACE, 'resource': SERVICE, 'type': 'app', 'level': 'error',
            'startTime': CREATED, 'endTime': end.isoformat().replace('+00:00', 'Z'), 'direction': 'forward', 'limit': 100})
    raw, _ = read('https://api.render.com/v1/logs?' + query, token=client.token)
    observation = {}
    receipt = validate_logs(strict_json(raw), end, observation)
    return receipt, observation['latest_retry']

def validate_readiness(value, at):
    require(isinstance(value, dict) and value.get('schema') == 'agent-bounties/marketplace-readiness-v1' and
            value.get('ready') is False and value.get('limits') == {'heartbeat_age_seconds': 90, 'lag_blocks': 20}, 'readiness_changed')
    require(-5 <= (at - timestamp(value.get('checked_at'))).total_seconds() <= 30, 'stale_readiness')
    readers = value.get('readers')
    require(isinstance(readers, list) and len(readers) == 4 and all(isinstance(row, dict) for row in readers) and
            {row.get('protocol') for row in readers} == PROTOCOLS, 'readers_changed')
    require(integer(value.get('latest_block')) and integer(value.get('safe_block')) and
            value['latest_block'] >= value['safe_block'] >= CURSOR, 'invalid_chain_head')
    for row in readers:
        require(integer(row.get('cursor_block')) and integer(row.get('heartbeat_age_seconds')) and integer(row.get('lag_blocks')), 'invalid_reader')
        require(row['cursor_block'] <= value['latest_block'] and
                row['lag_blocks'] == max(value['safe_block'] - row['cursor_block'], 0), 'contradictory_reader')
        if row['protocol'] == PROTOCOL:
            require(row.get('ready') is False and row.get('blocker') == 'reader_failed' and row['cursor_block'] == CURSOR, 'legacy_reader_changed')
        else:
            require(row.get('ready') is True and row.get('blocker') is None and
                    row['heartbeat_age_seconds'] <= 90 and row['lag_blocks'] <= 20, 'sibling_unavailable')
    return value['checked_at']

def observe_readiness():
    _, before = read(HEALTH, limit=65536)
    require(bool(re.fullmatch(r'[0-9a-f]{40}', before)), 'health_revision_missing')
    raw, revision = read(READINESS, expected=503, limit=65536)
    value = strict_json(raw)
    checked = validate_readiness(value, now())
    _, after = read(HEALTH, limit=65536)
    require(before == after and (not revision or revision == before), 'health_revision_changed')
    validate_readiness(value, now())
    return {'revision': before, 'revision_source': 'legacy_health_bracket', 'checked_at': checked}

class Ticket:
    def __init__(self, client, inspect_inventory):
        self.client, self.inspect_inventory = client, inspect_inventory
        self.started = time.monotonic()
        self.services = inspect_inventory()
        validate_deployment(client)
        require(now() < timestamp(EXPIRES), 'case_expired')
        self.receipt = {'case': CASE, 'previous_deploy': DEPLOY, 'previous_revision': PREVIOUS}
        self.receipt.update(observe_readiness())
        self.attempted = False

    def before_deploy(self):
        # Recheck the exact source bindings and newest deployment immediately before
        # the only POST. A completed or ambiguous previous attempt is never retried.
        require(not self.attempted, 'already_observed')
        self.attempted = True
        require(time.monotonic() - self.started <= 60 and now() < timestamp(EXPIRES), 'evidence_expired')
        self.inspect_inventory()
        validate_deployment(self.client)
        # Exactly one complete page, as late as possible. Do not reuse an earlier
        # page after slow inventory reads that could hide a new integrity error.
        self.end = now()
        logs, self.latest_retry = observe_logs(self.client, self.end)
        observation = observe_readiness()
        require(observation['revision'] == self.receipt['revision'], 'health_revision_changed')
        require(-5 <= (now() - timestamp(observation['checked_at'])).total_seconds() <= 30, 'stale_readiness')
        require(time.monotonic() - self.started <= 60 and (now() - self.end).total_seconds() <= 60 and
                0 <= (now() - self.latest_retry).total_seconds() <= 180 and
                now() < timestamp(EXPIRES), 'evidence_expired')
        self.receipt.update(logs)
        self.receipt['final_checked_at'] = observation['checked_at']
