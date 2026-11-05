#!/usr/bin/env python3
"""Check retirement and caller authorization using only disposable local services.

Build api and mcp-server first. This script starts its own PostgreSQL fixture via
run-with-test-postgres.py unless it is already inside that isolated harness.
It never accepts a remote API, MCP, database, model endpoint, or real credential.
"""
from __future__ import annotations

import argparse
import hashlib
import hmac
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from urllib.parse import urlsplit
import uuid

ROOT = Path(__file__).resolve().parents[1]


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_args, **_kwargs):
        return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    suffix = '.exe' if os.name == 'nt' else ''
    parser.add_argument('--api-binary', type=Path, default=ROOT / 'target/debug' / ('api' + suffix))
    parser.add_argument('--mcp-binary', type=Path, default=ROOT / 'target/debug' / ('mcp-server' + suffix))
    args = parser.parse_args()
    binaries = {'api': args.api_binary.resolve(), 'mcp-server': args.mcp_binary.resolve()}
    for binary in binaries.values():
        if not binary.is_file():
            parser.error('build the local services first: cargo build -p api -p mcp-server')
    if os.environ.get('AGENT_BOUNTIES_LOCAL_TEST_DATABASE') != '1':
        return subprocess.call([sys.executable, str(ROOT / 'scripts/run-with-test-postgres.py'),
                                '--', sys.executable, str(Path(__file__).resolve()), *sys.argv[1:]], cwd=ROOT)
    database = os.environ.get('DATABASE_URL', '')
    parsed = urlsplit(database)
    if (parsed.scheme not in ('postgres', 'postgresql')
            or parsed.hostname not in ('127.0.0.1', 'localhost')
            or parsed.username != 'agent_bounties_test' or parsed.path != '/agent_bounties_test'
            or parsed.query or parsed.fragment):
        raise RuntimeError('Retirement checks require the disposable local PostgreSQL fixture')
    with tempfile.TemporaryDirectory(prefix='agent-bounties-retirement-') as directory:
        exercise(binaries, database, Path(directory))
    return 0


def exercise(binaries: dict[str, Path], DATABASE: str, OUT: Path) -> None:
    # Allocate distinct ephemeral loopback ports, with no inherited endpoint or proxy.
    listeners = [socket.socket() for _ in range(3)]
    try:
        for listener in listeners:
            listener.bind(('127.0.0.1', 0))
        API, MCP, SITE = [f'http://127.0.0.1:{listener.getsockname()[1]}' for listener in listeners]
    finally:
        for listener in listeners:
            listener.close()
    SECRET = 'synthetic-session-secret-' + uuid.uuid4().hex
    RUN_ID = uuid.uuid4().hex
    http = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    checks = []
    provider_calls = []
    processes = []
    logs = []

    class Spy(BaseHTTPRequestHandler):
        def do_POST(self):
            provider_calls.append(self.path)
            self.rfile.read(int(self.headers.get('Content-Length', '0')))
            self.send_response(500)
            self.end_headers()
        do_GET = do_POST
        def log_message(self, *_args):
            pass

    def request(base, path, method='GET', data=None, headers=None):
        body = data if isinstance(data, bytes) else json.dumps(data).encode() if data is not None else None
        req = urllib.request.Request(base + path, data=body, method=method,
            headers={'Content-Type': 'application/json', **(headers or {})})
        try:
            response = http.open(req, timeout=10)
        except urllib.error.HTTPError as error:
            response = error
        raw = response.read()
        try:
            value = json.loads(raw)
        except ValueError:
            value = raw.decode()
        return response.status, value, response.headers

    def check(label, assertion):
        if not assertion:
            raise AssertionError(label)
        checks.append(label)

    def start(service):
        env = {
            **{name: os.environ[name] for name in ('PATH', 'SystemRoot') if name in os.environ},
            'DATABASE_URL': DATABASE,
            'PUBLIC_BASE_URL': API,
            'MCP_BASE_URL': MCP,
            'WEBSITE_BASE_URL': SITE,
            'SITE_AUTH_ALLOWED_ORIGINS': SITE,
            'MCP_ALLOWED_ORIGINS': SITE,
            'API_BIND_ADDR': API.removeprefix('http://'),
            'MCP_BIND_ADDR': MCP.removeprefix('http://'),
            'OPERATOR_API_TOKEN': 'local-fixture-only-operator',
            'AUTH_SESSION_SECRET': SECRET,
            'AUTH_WALLET_LINK_SECRET': SECRET,
            # Even stale flags/keys and an invalid legacy endpoint cannot restore AI.
            'CLOUD_AGENT_ENABLED': 'true',
            'CLOUD_AGENT_PUBLIC_DRAFTS': 'true',
            'CLOUD_AGENT_API_KEY': 'synthetic-local-provider-key',
            'CLOUD_AGENT_MODEL': 'synthetic-local-model',
            'CLOUD_AGENT_PROTOCOL': 'openai_responses',
            'CLOUD_AGENT_ENDPOINT': f'http://127.0.0.1:{spy.server_port}/v1/responses',
            'ENABLE_BASE_TX_BROADCAST': 'false',
            'ENABLE_STRIPE_LIVE_EXECUTION': 'false',
            'ENABLE_X402_HOSTED_RELAY': 'false',
            'RUST_LOG': 'error',
        }
        log = (OUT / f'http-{service}-{len(processes)}.log').open('w')
        logs.append(log)
        process = subprocess.Popen([str(binaries[service])], cwd=ROOT, env=env,
            stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        processes.append(process)
        base = API if service == 'api' else MCP
        for _ in range(120):
            if process.poll() is not None:
                raise RuntimeError(f'{service} exited during local fixture startup')
            try:
                if request(base, '/health')[0] == 200:
                    return process
            except (OSError, urllib.error.URLError):
                pass
            time.sleep(0.25)
        raise RuntimeError(f'{service} startup timed out')

    def stop(process):
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)

    def cookie(identity='owner'):
        now = int(time.time())
        user = dict(provider='github', sub='local-retirement-fixture-' + RUN_ID + identity, name='Local fixture',
            email='fixture@example.invalid', avatar='', iat=now, exp=now + 600)
        encoded = json.dumps(user, separators=(',', ':')).encode().hex()
        signature = hmac.new(SECRET.encode(), encoded.encode(), hashlib.sha256).hexdigest()
        return {'Cookie': f'agent_bounties_session={encoded}.{signature}', 'Origin': SITE}

    def check_retired(base, path, method, data=None):
        status, body, headers = request(base, path, method, data)
        check(f'{base}{path}: HTTP410', status == 410)
        check(f'{path}: permanent machine error', body.get('error_code') == 'hosted_generation_removed' and body.get('retryable') is False)
        if base == MCP or method == 'POST':
            check(f'{path}: private no-store', headers.get('Cache-Control') == 'no-store, private')

    def rpc_params(name, modern):
        params = {'name': name, 'arguments': 'invalid legacy arguments'}
        headers = {}
        if modern:
            params['_meta'] = {
                'io.modelcontextprotocol/protocolVersion': '2026-07-28',
                'io.modelcontextprotocol/clientInfo': {'name': 'local-retirement-fixture', 'version': '1'},
                'io.modelcontextprotocol/clientCapabilities': {},
            }
            headers = {'mcp-protocol-version': '2026-07-28', 'mcp-method': 'tools/call', 'mcp-name': name}
        return {'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call', 'params': params}, headers

    spy = ThreadingHTTPServer(('127.0.0.1', 0), Spy)
    thread = threading.Thread(target=spy.serve_forever, daemon=True)
    thread.start()
    try:
        api = start('api')
        mcp = start('mcp-server')
        status, readiness, _headers = request(API, '/v1/cloud-agent/readiness')
        check('readiness permanently removed under legacy enabled flags', status == 200 and readiness.get('status') == 'removed' and not readiness['available'] and not readiness['public_drafts'])
        for path in ['/v1/cloud-agent/objective-plans', '/v1/cloud-agent/bounty-drafts']:
            check_retired(API, path, 'POST', {})
            check_retired(API, path, 'POST', b'{')
        check_retired(API, '/v1/base/autonomous-bounties/invalid-contract/analysis', 'GET')
        names = ['draft_bounty_with_cloud_agent', 'compile_objective_with_cloud_agent', 'analyze_bounty_fit']
        for name in names:
            check_retired(MCP, '/tools/' + name, 'POST', {})
            check_retired(MCP, '/tools/' + name, 'POST', b'{')
            for modern in (False, True):
                body, headers = rpc_params(name, modern)
                status, reply, response_headers = request(MCP, '/mcp', 'POST', body, headers)
                result = reply.get('result', {})
                check(f'/mcp {name} modern={modern}: permanent tool error', status == 200 and result.get('isError') is True and result.get('structuredContent', {}).get('http_status') == 410 and result['structuredContent']['error_code'] == 'hosted_generation_removed')
                check(f'/mcp modern={modern}: private no-store', response_headers.get('Cache-Control') == 'no-store, private')
        status, catalog, headers = request(MCP, '/tools')
        check('advanced tool catalog omits all retired generators', status == 200 and not set(names).intersection(tool['name'] for tool in catalog))
        status, catalog, headers = request(MCP, '/mcp', 'POST', {'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list', 'params': {}})
        check('public MCP catalog omits all retired generators', status == 200 and not set(names).intersection(tool['name'] for tool in catalog['result']['tools']))
        plan = {'schema_version': 'broad-verification/v1', 'criteria': [{'id': 'work', 'text': 'Deliver the work', 'review': 'Does it meet the brief?'}], 'checks': [{'id': 'files', 'criteria': ['work'], 'purpose': 'required', 'rule': {'checker': 'contents_v1', 'paths': ['work.txt'], 'folders': []}}]}
        status, check_catalog, _ = request(API, '/v1/verification/checkers')
        check('broad catalog exposes exact claims and parameter schemas', status == 200 and len(check_catalog['checks']) == 7 and all(isinstance(c['parameters_schema'], dict) for c in check_catalog['checks']))
        validation = request(API, '/v1/verification/plans/validate', 'POST', plan)
        check('broad plan validates without provider calls or payment authority', validation[0] == 200 and validation[1]['valid'] and not validation[1]['payment_authorized'])
        for name, arguments, expected in [('find_verification_checks', {}, check_catalog), ('validate_verification_plan', {'plan': plan}, validation[1])]:
            direct = request(MCP, '/tools/' + name, 'POST', arguments)
            check('direct MCP shares exact API ' + name, direct[0] == 200 and direct[1].get('content', [{}])[0].get('json') == expected)
            for modern in (False, True):
                body, protocol_headers = rpc_params(name, modern)
                body['params']['arguments'] = arguments
                remote = request(MCP, '/mcp', 'POST', body, protocol_headers)
                check(f'remote MCP shares exact API {name} modern={modern}', remote[0] == 200 and remote[1].get('result', {}).get('structuredContent') == expected)
        upload_args = {'files': [{'path': 'work.txt', 'base64': 'd29yaw=='}], 'idempotency_key': 'broad-local-upload-' + RUN_ID}
        for name, arguments in [('upload_verification_artifact', upload_args), ('run_verification_checks', {'artifact_id': str(uuid.uuid4()), 'plan': plan}), ('list_review_tasks', {})]:
            direct = request(MCP, '/tools/' + name, 'POST', arguments)
            check('broad MCP does not attach operator authority: ' + name, direct[0] == 200 and '401' in direct[1].get('error', ''))
            for modern in (False, True):
                body, protocol_headers = rpc_params(name, modern)
                body['params']['arguments'] = arguments
                remote = request(MCP, '/mcp', 'POST', body, protocol_headers)
                check(f'protected broad MCP {name} fails closed modern={modern}', remote[0] == 200 and remote[1].get('result', {}).get('isError') is True)
        headers = {**cookie(), 'Idempotency-Key': upload_args['idempotency_key']}
        absent_storage = request(API, '/v1/verification/artifacts', 'POST', {'files': upload_args['files']}, headers)
        check('missing durable storage cannot pretend files were saved', absent_storage[0] == 503)
        prepared = {'title': 'Caller prepared local request', 'goal': 'Preserve the exact reviewed content without calling a model.',
            'acceptance_criteria': ['The prepared content is retained.', 'No generated or pending demo exists.'],
            'idempotency_key': 'local-retirement-' + uuid.uuid4().hex, 'source_url': 'https://example.invalid/local-fixture'}
        check('anonymous publication denied', request(API, '/v1/unfunded-bounties', 'POST', prepared)[0] == 401)
        auth = cookie()
        check('website cookie authenticates the account', request(API, '/v1/auth/session', headers=auth)[0] == 200)
        wrong_origin = {**auth, 'Origin': 'https://other.invalid'}
        check('website publication rejects an unrelated Origin', request(API, '/v1/unfunded-bounties', 'POST', prepared, wrong_origin)[0] == 401)
        status, published, headers = request(API, '/v1/unfunded-bounties', 'POST', prepared, auth)
        check('website cookie publishes prepared content', status == 200 and all(published.get(k) == prepared[k] for k in ('title', 'goal', 'acceptance_criteria', 'source_url')))
        check('publication has no generated or pending demo', published['demo_agent_solution'] is None and published['agent_solutions'] == [] and not published['canonical_bounty_created'] and not published['payment_promised'])
        check('authenticated publication response is private no-store', headers.get('Cache-Control') == 'no-store, private')
        same = request(API, '/v1/unfunded-bounties', 'POST', prepared, auth)
        check('same account/content/idempotency key replays one publication', same[0] == 200 and same[1]['bounty_id'] == published['bounty_id'])
        changed = {**prepared, 'goal': 'Different content'}
        check('same idempotency key with changed content conflicts', request(API, '/v1/unfunded-bounties', 'POST', changed, auth)[0] == 409)
        mcp_reply = request(MCP, '/tools/publish_unfunded_bounty', 'POST', prepared, auth)
        mcp_published = mcp_reply[1].get('content', [{}])[0].get('json', {})
        check('MCP forwards only the caller cookie and preserves publication replay', mcp_reply[0] == 200 and mcp_published.get('bounty_id') == published['bounty_id'])
        other = cookie('other')
        operator = {'X-Operator-Token': 'local-fixture-only-operator'}
        for name, method in [('events', 'POST'), ('reviews', 'GET')]:
            for principal_name, principal_headers in [('anonymous', {}), ('unrelated-account', other)]:
                status, result, headers = request(API, '/v1/risk/' + name, headers=principal_headers)
                check(f'{principal_name} API risk {name} denied and no-store', status == 401 and headers.get('Cache-Control') == 'no-store, private')
                status, result, headers = request(MCP, '/tools/list_risk_' + name, method, {} if method == 'POST' else None, principal_headers)
                check(f'{principal_name} MCP risk {name} denied and no-store', status == 200 and 'error' in result and headers.get('Cache-Control') == 'no-store, private')
            status, result, headers = request(API, '/v1/risk/' + name, headers=operator)
            check(f'explicit operator API risk {name} authorized and no-store', status == 200 and isinstance(result, list) and headers.get('Cache-Control') == 'no-store, private')
            status, result, headers = request(MCP, '/tools/list_risk_' + name, method, {} if method == 'POST' else None, operator)
            check(f'explicit operator MCP risk {name} authorized and no-store', status == 200 and isinstance(result.get('content', [{}])[0].get('json'), list) and headers.get('Cache-Control') == 'no-store, private')
        status, result, headers = request(MCP, '/llms.txt', headers=auth)
        check('credentialed MCP public text is no-store', status == 200 and headers.get('Cache-Control') == 'no-store, private')
        status, result, headers = request(MCP, '/r/chatgpt/mcp', 'POST', {'jsonrpc': '2.0', 'id': 3, 'method': 'tools/list', 'params': {}})
        check('rail-specific MCP responses are no-store', headers.get('Cache-Control') == 'no-store, private')
        registration = {'handle': 'local-owner-' + RUN_ID, 'payout_wallet': None}
        check('anonymous API registration denied', request(API, '/v1/agents', 'POST', registration)[0] == 401)
        denied = request(MCP, '/tools/register_agent', 'POST', registration)
        check('anonymous MCP adapter registration denied without operator forwarding', denied[0] == 200 and '401' in denied[1].get('error', ''))
        status, agent, _headers = request(API, '/v1/agents', 'POST', registration, auth)
        check('owner account creates an owned agent', status == 200 and agent.get('id') is not None)
        capability = {'agent_id': agent['id'], 'class': 'Coding', 'template_slugs': ['patch-with-tests'],
            'min_price_minor': 100, 'max_price_minor': 1000, 'currency': 'usdc', 'latency_seconds': 60,
            'supported_verifiers': ['Manual']}
        check('wrong owner API capability write denied', request(API, '/v1/capabilities', 'POST', capability, other)[0] == 404)
        denied = request(MCP, '/tools/register_capability', 'POST', capability, other)
        check('wrong owner MCP adapter capability write denied', denied[0] == 200 and '404' in denied[1].get('error', ''))
        allowed = request(MCP, '/tools/register_capability', 'POST', capability, auth)
        check('owner MCP adapter capability write succeeds', allowed[0] == 200 and allowed[1].get('content', [{}])[0].get('json', {}).get('agent_id') == agent['id'])
        solution = {'agent_id': agent['id'], 'summary': 'LOCAL_SOLUTION_SHOULD_NOT_BE_WRITTEN',
            'deliverable_markdown': 'Unapproved wrong-owner solution', 'evidence': {}}
        solution_path = '/v1/unfunded-bounties/' + published['bounty_id'] + '/solutions'
        check('anonymous API solution write denied', request(API, solution_path, 'POST', solution)[0] == 401)
        check('wrong owner API solution write denied', request(API, solution_path, 'POST', solution, other)[0] == 404)
        solution_args = {**solution, 'bounty_id': published['bounty_id']}
        denied = request(MCP, '/tools/submit_unfunded_bounty_solution', 'POST', solution_args, other)
        check('wrong owner MCP adapter solution write denied', denied[0] == 200 and '404' in denied[1].get('error', ''))
        private_request = {'title': 'PRIVATE_LOCAL_TITLE_' + RUN_ID, 'template_slug': 'patch-with-tests',
            'target_amount_minor': 100, 'currency': 'usdc', 'funding_mode': 'Simulated', 'privacy': 'Private'}
        status, private_bounty, _headers = request(API, '/v1/bounties/pooled', 'POST', private_request, auth)
        check('owner account creates a private bounty fixture', status == 200 and private_bounty.get('privacy') == 'Private')
        private_id = private_bounty['id']
        private_path = '/v1/bounties/' + private_id
        check('owner API private read succeeds', request(API, private_path, headers=auth)[0] == 200)
        for principal_name, principal_headers in [('anonymous', {}), ('wrong-owner', other)]:
            status, body, headers = request(API, private_path, headers=principal_headers)
            check(f'{principal_name} API private read denied', status == 404 and private_request['title'] not in str(body))
            check(f'{principal_name} API private read is no-store', headers.get('Cache-Control') == 'no-store, private')
        stop(mcp)
        mcp = start('mcp-server')
        for name in ('get_bounty_status', 'get_paid_status'):
            for principal_name, principal_headers in [('anonymous', {}), ('wrong-owner', other)]:
                status, body, headers = request(MCP, '/tools/' + name, 'POST', {'bounty_id': private_id}, principal_headers)
                check(f'{principal_name} hydrated MCP {name} private read denied', status == 200 and 'error' in body and private_request['title'] not in str(body))
                check(f'{principal_name} MCP private read is no-store', headers.get('Cache-Control') == 'no-store, private')
        owned = request(MCP, '/tools/get_bounty_status', 'POST', {'bounty_id': private_id}, auth)
        check('owner MCP private read succeeds through caller authorization', owned[0] == 200 and owned[1].get('content', [{}])[0].get('json', {}).get('bounty', {}).get('id') == private_id)
        status, result, _headers = request(MCP, '/tools/get_paid_status', 'POST', {'bounty_id': private_id}, auth)
        value = result.get('content', [{}])[0].get('json', {})
        check('authorized MCP bounty payout keeps its response envelope', status == 200 and value.get('scope') == 'bounty' and value.get('bounty_id') == private_id and value.get('bounty_status') == private_bounty['status'])
        status, result, _headers = request(MCP, '/tools/get_paid_status', 'POST', {'agent_id': agent['id']}, auth)
        value = result.get('content', [{}])[0].get('json', {})
        check('authorized MCP agent payout keeps its response envelope', status == 200 and value.get('scope') == 'agent' and value.get('agent_id') == agent['id'])
        for name, arguments in [('register_agent', registration), ('publish_unfunded_bounty', prepared),
                                ('submit_unfunded_bounty_solution', solution_args), ('get_paid_status', {'bounty_id': private_id})]:
            for modern in (False, True):
                for principal_name, principal_headers in [('anonymous', {}), ('wrong-owner', other)]:
                    body, protocol_headers = rpc_params(name, modern)
                    body['params']['arguments'] = arguments
                    status, reply, headers = request(MCP, '/mcp', 'POST', body, {**protocol_headers, **principal_headers})
                    denied = reply.get('error') or reply.get('result', {}).get('isError')
                    check(f'/mcp {name} modern={modern} {principal_name}: unadvertised protected tool unavailable', status in (200, 400) and bool(denied) and private_request['title'] not in str(reply))
        check('denied solution writes did not change stored solutions', request(API, '/v1/unfunded-bounties/' + published['bounty_id'])[1]['agent_solutions'] == [])
        stop(api)
        api = start('api')
        check_retired(API, '/v1/cloud-agent/objective-plans', 'POST', {})
        check_retired(MCP, '/tools/compile_objective_with_cloud_agent', 'POST', {})
        for modern in (False, True):
            body, headers = rpc_params('compile_objective_with_cloud_agent', modern)
            status, reply, _headers = request(MCP, '/mcp', 'POST', body, headers)
            result = reply.get('result', {})
            check(f'restarted MCP modern={modern}: hosted generation remains removed', status == 200 and result.get('isError') is True and result.get('structuredContent', {}).get('http_status') == 410)
        restarted = request(API, '/v1/unfunded-bounties', 'POST', prepared, auth)
        check('API restart keeps durable publication idempotency and no demo', restarted[0] == 200 and restarted[1]['bounty_id'] == published['bounty_id'] and restarted[1]['demo_agent_solution'] is None)
        check('synthetic provider received zero calls', provider_calls == [])
    finally:
        for process in processes:
            stop(process)
        spy.shutdown()
        spy.server_close()
        for log in logs:
            log.close()
        cleanup = {'service_processes_exited': all(p.poll() is not None for p in processes), 'provider_spy_closed': True}
    print(json.dumps({'checks_passed': len(checks), 'provider_requests': len(provider_calls), 'cleanup': cleanup}, indent=2))


if __name__ == "__main__":
    raise SystemExit(main())
