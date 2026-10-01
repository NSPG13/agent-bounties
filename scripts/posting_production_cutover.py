"""Attended, allowlisted posting rollout; never signs or sends a chain transaction."""
import json
import os
from pathlib import Path
import sys
import urllib.error
import urllib.parse
import urllib.request

BRANCH = 'codex/posting-production-cutover-20261001'
PRIVATE_BRANCH = 'codex/posting-private-full-coverage-20260929'
PRIVATE_REVISION = 'e6bd24bac27c8fb5e77fad3fe91fdb3283d4288e'
WORKSPACE = 'tea-d4tk87f5r7bs73au55lg'
PUBLIC_REPO = 'https://github.com/NSPG13/agent-bounties'
PRIVATE_REPO = 'https://github.com/NSPG13/AB-PRIVATE'
SERVICES = {
    'api': 'srv-d982ftu7r5hc73ar4stg',
    'mcp': 'srv-d982ftu7r5hc73ar4su0',
    'legacy': 'srv-d982ftu7r5hc73ar4st0',
    'v1': 'srv-d9r19q2ju40c73e1nr20',
    'v2': 'srv-da32s4qjnfac73bjctn0',
    'shadow': 'srv-da32s5ht0dsc73ckfvkg',
    'keeper': 'srv-da32s6ek1f9s73duedlg',
    'broker': 'srv-da32s70ae00c73a01ffg',
}
WORKFLOWS = (
    'agent-bounty-create-comments.yml', 'bounded-wallet-gas-relay.yml',
    'leaderboard-reward-signer.yml', 'leaderboard-reward-signing-reusable.yml',
    'paid-bounty-claim-comments.yml', 'paid-bounty-funding-comments.yml',
    'paid-bounty-proofs.yml', 'participant-registration.yml',
    'regression-verifier-signer.yml', 'regression-verifier-signing-reusable.yml',
    'activate-routed-v3-replacements.yml', 'autonomous-gas-relay.yml',
    'leaderboard-reward-runner.yml', 'regression-verifier-runner.yml',
    'synthetic-paid-loop-canary.yml',
)
FLAGS_OFF = {
    'ENABLE_BASE_TX_BROADCAST': 'false', 'ENABLE_X402_HOSTED_RELAY': 'false',
    'ENABLE_SPONSORED_BOUNTY_CREATION': 'false', 'ENABLE_SPONSORED_SETUP': 'false',
    'ENABLE_CREATOR_OPEN_SPONSORSHIP': 'false', 'VERIFIER_EMAIL_ENABLED': 'false',
    'POSTING_CREATOR_INDEXER_ENABLED': 'false',
}


def require(value, message):
    if not value:
        raise RuntimeError(message)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def api(provider, path, method='GET', body=None):
    root, token = {
        'render': ('https://api.render.com/v1', os.environ['RENDER_API_KEY']),
        'github': ('https://api.github.com/repos/NSPG13/agent-bounties', os.environ['GH_TOKEN']),
    }[provider]
    require(path.startswith('/') and '://' not in path and '..' not in path,
            'Invalid API path')
    request = urllib.request.Request(root + path, method=method,
        data=None if body is None else json.dumps(body).encode(),
        headers={'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json',
                 'Accept': 'application/json', 'User-Agent': 'posting-attended-cutover/1'})
    try:
        with urllib.request.build_opener(NoRedirect()).open(request, timeout=30) as response:
            raw = response.read(8_000_001)
            require(len(raw) <= 8_000_000, 'API response exceeded bound')
            return json.loads(raw) if raw else None
    except urllib.error.HTTPError as error:
        # Never include provider response bodies or credentials in logs/artifacts.
        raise RuntimeError(f'{provider} {method} {path}: HTTP {error.code}') from None


def service(role):
    value = api('render', '/services/' + SERVICES[role])
    require(value['id'] == SERVICES[role] and value['ownerId'] == WORKSPACE,
            'Service/workspace identity changed')
    require(value['repo'].removesuffix('.git') in (PUBLIC_REPO, PRIVATE_REPO),
            'Unexpected source repository')
    require(value['serviceDetails']['plan'] == 'starter' and
            value['serviceDetails']['region'] == 'oregon', 'Plan or region changed')
    require(value['autoDeploy'] == 'no', 'Automatic deployment must remain off')
    require(value['type'] == ('web_service' if role in ('api', 'mcp') else 'background_worker'),
            'Unexpected service type')
    return value


def snapshot():
    evidence = {'schema': 'agent-bounties/posting-cutover-v1', 'services': {}, 'workflows': {}}
    for role in SERVICES:
        value = service(role)
        deploys = api('render', '/services/' + value['id'] + '/deploys?limit=10')
        evidence['services'][role] = {
            key: value[key] for key in ('id', 'name', 'repo', 'branch', 'suspended')}
        evidence['services'][role]['deploys'] = [
            {'id': row['deploy']['id'], 'status': row['deploy']['status'],
             'commit': row['deploy'].get('commit', {}).get('id')}
            for row in deploys]
    for workflow in WORKFLOWS:
        value = api('github', '/actions/workflows/' + workflow)
        runs = api('github', '/actions/workflows/' + workflow + '/runs?per_page=30')['workflow_runs']
        active = [row['id'] for row in runs if row['status'] != 'completed']
        evidence['workflows'][workflow] = {'id': value['id'], 'state': value['state'], 'active_runs': active}
    return evidence


def set_env(role, key, value):
    api('render', '/services/' + SERVICES[role] + '/env-vars/' + key,
        'PUT', {'value': value})
    observed = api('render', '/services/' + SERVICES[role] + '/env-vars/' + key)
    actual = observed.get('value', observed.get('envVar', {}).get('value'))
    require(actual == value, 'Environment update did not read back: ' + key)


def rebind(role):
    service(role)
    patch = {'repo': PRIVATE_REPO, 'branch': PRIVATE_BRANCH, 'autoDeployTrigger': 'off'}
    if role == 'legacy':
        patch['serviceDetails'] = {'envSpecificDetails': {'dockerfilePath': './Dockerfile.posting-indexers'}}
    api('render', '/services/' + SERVICES[role], 'PATCH', patch)
    observed = service(role)
    require(observed['repo'].removesuffix('.git') == PRIVATE_REPO and
            observed['branch'] == PRIVATE_BRANCH, 'Source rebind did not read back')


def deploy(role):
    value = api('render', '/services/' + SERVICES[role] + '/deploys',
                'POST', {'commitId': PRIVATE_REVISION, 'clearCache': 'do_not_clear'})
    require(value.get('commit', {}).get('id') == PRIVATE_REVISION,
            'Provider did not acknowledge exact deployment revision')
    return {'id': value['id'], 'status': value['status'], 'commit': PRIVATE_REVISION}


def run(phase):
    require(os.environ.get('GITHUB_REPOSITORY') == 'NSPG13/agent-bounties' and
            os.environ.get('GITHUB_REF') == 'refs/heads/' + BRANCH and
            os.environ.get('GITHUB_EVENT_NAME') == 'workflow_dispatch',
            'Only attended dispatch on the exact control branch is permitted')
    require(phase in ('inventory', 'pause-and-api', 'workers', 'mainnet-preflight'), 'Unknown cutover phase')
    require(len(os.environ.get('GAS_SPONSOR_BUDGET_TOKEN', '')) >= 32,
            'Dedicated reservation credential missing')
    output = Path('posting-cutover-evidence.json')
    evidence = snapshot()
    output.write_text(json.dumps(evidence, indent=2) + '\n')
    print(json.dumps(evidence, sort_keys=True), flush=True)
    if phase == 'inventory':
        return
    require(all(not item['active_runs'] for item in evidence['workflows'].values()),
            'A spending workflow still has an active run')
    if phase == 'mainnet-preflight':
        require(all(item['state'] == 'disabled_manually' for item in evidence['workflows'].values()), 'Spending workflows must remain paused')
        for role in ('keeper', 'broker'):
            require(service(role)['suspended'] == 'suspended', 'Spending worker must remain paused')
        return
    if phase == 'pause-and-api':
        for workflow in WORKFLOWS:
            api('github', '/actions/workflows/' + workflow + '/disable', 'PUT')
        for workflow in WORKFLOWS:
            observed = api('github', '/actions/workflows/' + workflow)
            require(observed['state'] == 'disabled_manually', 'Workflow did not pause')
            runs = api('github', '/actions/workflows/' + workflow + '/runs?per_page=30')['workflow_runs']
            require(all(row['status'] == 'completed' for row in runs), 'Workflow raced the pause')
        for role in ('keeper', 'broker'):
            value = service(role)
            if value['suspended'] == 'not_suspended':
                api('render', '/services/' + SERVICES[role] + '/suspend', 'POST')
            require(service(role)['suspended'] == 'suspended', 'Worker did not pause')
        evidence['started_deploys'] = {}
        for role in ('api', 'mcp'):
            for key, value in FLAGS_OFF.items():
                set_env(role, key, value)
            set_env(role, 'GAS_SPONSOR_BUDGET_TOKEN', os.environ['GAS_SPONSOR_BUDGET_TOKEN'])
            rebind(role)
            evidence['started_deploys'][role] = deploy(role)
            output.write_text(json.dumps(evidence, indent=2) + '\n')
    elif phase == 'workers':
        require(all(item['state'] == 'disabled_manually' for item in evidence['workflows'].values()),
                'Spending workflows are not paused')
        for role in ('keeper', 'broker'):
            require(service(role)['suspended'] == 'suspended', 'Spending worker is not paused')
        for role in ('api', 'mcp'):
            require(any(d['status'] == 'live' and d['commit'] == PRIVATE_REVISION
                        for d in evidence['services'][role]['deploys']), 'Budget API must be live first')
        evidence['started_deploys'] = {}
        for role in ('legacy', 'v1', 'v2', 'shadow', 'keeper', 'broker'):
            set_env(role, 'POSTING_CREATOR_INDEXER_ENABLED', 'false')
            rebind(role)
            evidence['started_deploys'][role] = deploy(role)
            output.write_text(json.dumps(evidence, indent=2) + '\n')


if __name__ == '__main__':
    try:
        run(sys.argv[1])
        print('Attended cutover phase completed; evidence contains no credentials.')
    except Exception as error:
        print(f'Cutover stopped: {type(error).__name__}: {error}', file=sys.stderr)
        sys.exit(1)
