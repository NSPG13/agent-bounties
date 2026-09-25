#!/usr/bin/env python3
"""Read-only release checks, except deliberately invalid requests rejected before work.
Never signs, publishes, funds, claims, submits, or modifies a saved account record.
Browser acceptance in production-acceptance.md remains required separately.
"""
import argparse, datetime, hashlib, json, pathlib, urllib.error, urllib.request

parser = argparse.ArgumentParser()
parser.add_argument('--expected-runtime-revision', required=True)
parser.add_argument('--website-source', required=True, type=pathlib.Path)
parser.add_argument('--output', required=True, type=pathlib.Path)
parser.add_argument('--runtime-only', action='store_true', help='Verify the deployed services while the website release is pending')
args = parser.parse_args()
checks = []
def record(name, passed, detail):
    checks.append({'check': name, 'passed': bool(passed), 'detail': detail})
def read(url, body=None):
    request = urllib.request.Request(url, data=body, headers={'Accept': 'application/json', **({'Content-Type': 'application/json'} if body is not None else {})})
    try: response = urllib.request.urlopen(request, timeout=25)
    except urllib.error.HTTPError as error: response = error
    raw = response.read()
    try: value = json.loads(raw)
    except ValueError: value = None
    return response.status, dict(response.headers), raw, value

def run():
    for host in ['api', 'mcp']:
        status, headers, _, _ = read(f'https://{host}.agentbounties.app/health')
        revision = next((v for k, v in headers.items() if k.lower() == 'x-agent-bounties-revision'), None)
        record(f'{host}_exact_revision', status == 200 and revision == args.expected_runtime_revision, {'status': status, 'revision': revision})
    retired = ['autonomous_bounty_analysis', 'cloud_bounty_drafts', 'cloud_objective_plans']
    for host in (['api.agentbounties.app', 'mcp.agentbounties.app'] if args.runtime_only else ['agentbounties.app', 'api.agentbounties.app', 'mcp.agentbounties.app']):
        status, _, _, doc = read(f'https://{host}/.well-known/agent-bounties.json')
        active = [key for key in retired if key in (doc or {}).get('endpoints', {})]
        missing = [key for key in retired if key not in (doc or {}).get('retired_endpoints', {})]
        record(f'{host}_retired_capabilities', status == 200 and not active and not missing, {'still_active': active, 'missing_retirement': missing})
    status, _, _, doc = read('https://api.agentbounties.app/api-docs/openapi.json')
    missing = []
    def walk(value):
        if isinstance(value, dict):
            ref = value.get('$ref', '')
            if ref.startswith('#/'):
                current = doc
                try:
                    for part in ref[2:].split('/'): current = current[part.replace('~1', '/').replace('~0', '~')]
                except (KeyError, TypeError): missing.append(ref)
            for item in value.values(): walk(item)
        elif isinstance(value, list):
            for item in value: walk(item)
    walk(doc)
    paths = (doc or {}).get('paths', {})
    readiness = paths.get('/v1/base/agent-wallet/readiness', {}).get('post', {}).get('requestBody')
    record('readiness_schema', status == 200 and bool(readiness), {'request_body_present': bool(readiness)})
    record('all_schema_references_resolve', status == 200 and not missing, {'unresolved': sorted(set(missing))})
    allowed_bodyless = {'/a2a/v1/tasks/{id}:cancel', '/v1/auth/session/revoke', '/v1/base/autonomous-bounties/claims/{id}/withdraw', '/v1/base/open-competition-v2-beta3/proof-jobs/{job_id}/payment', '/v1/distribution/handoffs/wallet-review', '/v1/help-requests/{id}/quotes', '/v1/objectives/{id}/reconcile', '/v1/stripe/live/funding-intents/{id}/checkout-session'}
    bodyless = {path for path, value in paths.items() if 'post' in value and 'requestBody' not in value['post']}
    record('exact_bodyless_post_allowlist', bodyless == allowed_bodyless, {'unexpected': sorted(bodyless - allowed_bodyless), 'missing_expected': sorted(allowed_bodyless - bodyless)})
    for name, path, body, expected_status, field in [
        ('missing_network_correction', '/v1/base/agent-wallet/readiness', b'{}', 422, 'network'),
        ('invalid_view_correction', '/v1/opportunities?view=not_a_view', None, 400, 'view')]:
        status, _, _, problem = read('https://api.agentbounties.app' + path, body)
        problem = problem or {}
        passed = status == expected_status and problem.get('error_code') == 'invalid_request' and problem.get('field') == field and problem.get('retryable') is False and problem.get('request_schema_url') == '/api-docs/openapi.json' and bool(problem.get('next_action'))
        if field == 'view': passed = passed and 'ready_to_earn' in problem.get('allowed_values', [])
        record(name, passed, {'status': status, 'problem': problem})
    for asset in ([] if args.runtime_only else ['submissions.js', 'solarpunk-home.js', 'marketplace-workflow.js', 'webmcp.js', 'solarpunk.css']):
        status, _, raw, _ = read('https://agentbounties.app/' + asset)
        expected = hashlib.sha256((args.website_source / 'site' / asset).read_bytes()).hexdigest()
        actual = hashlib.sha256(raw).hexdigest()
        record('website_asset:' + asset, status == 200 and expected == actual, {'expected_sha256': expected, 'actual_sha256': actual})
try:
    run()
except Exception as error:
    record('probe_completed', False, {'error': str(error)})
report = {'observed_at': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'scope': 'runtime_only' if args.runtime_only else 'runtime_and_website', 'expected_runtime_revision': args.expected_runtime_revision, 'passed': all(c['passed'] for c in checks), 'checks': checks, 'browser_acceptance_required': True}
args.output.write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps({'passed': report['passed'], 'checks': len(checks), 'failed_checks': [c['check'] for c in checks if not c['passed']], 'output': str(args.output)}))
raise SystemExit(0 if report['passed'] else 1)
