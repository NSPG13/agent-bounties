"""Attended, allowlisted posting rollout; never signs or sends a chain transaction."""
import json
import os
from pathlib import Path
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

BRANCH = 'codex/posting-production-cutover-20261001'
PRIVATE_BRANCH = 'codex/posting-private-full-coverage-20260929'
PRIVATE_REVISION = 'e6bd24bac27c8fb5e77fad3fe91fdb3283d4288e'
SPENDER_REVISION = '6786110f4ea241c08f8d8b51eb2e039e87aa5d98'
ACTIVE_REVISION = 'dab255f22ae7c2436f44715a4be3dc85fb20872f'
CANARY_MCP = 'srv-dau3p1favr4c73fij9jg'
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
PUBLIC_RESTORATION_REVISION = 'd1ca9a3f30091a0dceb92e438026dc9b52fa0eb7'
HELD_WORKFLOWS = frozenset({
    'regression-verifier-runner.yml', 'regression-verifier-signer.yml',
    'regression-verifier-signing-reusable.yml', 'synthetic-paid-loop-canary.yml',
    'activate-routed-v3-replacements.yml',
})
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


def deploy(role, revision=PRIVATE_REVISION):
    path = '/services/' + SERVICES[role] + '/deploys'
    before = {row['deploy']['id'] for row in api('render', path + '?limit=10')}
    value = api('render', '/services/' + SERVICES[role] + '/deploys',
                'POST', {'commitId': revision, 'clearCache': 'do_not_clear'})
    # Render can accept the request with an empty 202 body. Reconcile against
    # the before-snapshot and the API trigger instead of sending another POST.
    if value is None:
        for _ in range(20):
            rows = api('render', path + '?limit=10')
            candidates = [row['deploy'] for row in rows
                          if row['deploy']['id'] not in before
                          and row['deploy'].get('trigger') == 'api']
            require(len(candidates) <= 1, 'Ambiguous deployment acknowledgement')
            if candidates:
                value = candidates[0]
                break
            time.sleep(1)
    require(isinstance(value, dict) and str(value.get('id', '')).startswith('dep-'),
            'Provider did not acknowledge a deployment ID')
    # A newly queued deployment may have commit:null until the source resolves.
    # Poll only that ID; never submit a replacement deployment on this condition.
    deploy_id = value['id']
    for _ in range(20):
        if value.get('commit') is not None:
            break
        time.sleep(1)
        value = api('render', '/services/' + SERVICES[role] + '/deploys/' + deploy_id)
        require(value['id'] == deploy_id, 'Deployment identity changed')
    require((value.get('commit') or {}).get('id') == revision,
            'Provider did not acknowledge exact deployment revision')
    return {'id': value['id'], 'status': value['status'], 'commit': revision}



def restore_workflows(evidence):
    require(api('github', '/git/ref/heads/main')['object']['sha'] == PUBLIC_RESTORATION_REVISION,
            'Public main changed from the reviewed budget and containment release')
    require(all(item['state'] == 'disabled_manually' and not item['active_runs']
                for item in evidence['workflows'].values()), 'Workflows must start paused')
    for role in SERVICES:
        expected = ACTIVE_REVISION if role in ('api', 'mcp') else SPENDER_REVISION
        live = [d for d in evidence['services'][role]['deploys'] if d['status'] == 'live']
        require(len(live) == 1 and live[0]['commit'] == expected,
                'Exact reviewed runtime must be live: ' + role)
    for role in ('keeper', 'broker'):
        setting = api('render', '/services/' + SERVICES[role] + '/env-vars/POSTING_SENDS_PAUSED')
        require(setting.get('value', setting.get('envVar', {}).get('value')) == 'false',
                'Budgeted worker has not resumed')
    evidence['restored_workflows'] = {}
    for workflow in WORKFLOWS:
        if workflow not in HELD_WORKFLOWS:
            api('github', '/actions/workflows/' + workflow + '/enable', 'PUT')
        state = api('github', '/actions/workflows/' + workflow)['state']
        require(state == ('disabled_manually' if workflow in HELD_WORKFLOWS else 'active'),
                'Workflow state differs: ' + workflow)
        evidence['restored_workflows'][workflow] = state
    # This last mutation closes the attended deployment window. Existing
    # environment/approval gates remain unchanged; no workflow is dispatched.
    api('github', '/actions/workflows/render-deploy-recovery.yml/disable', 'PUT')
    require(api('github', '/actions/workflows/render-deploy-recovery.yml')['state'] == 'disabled_manually',
            'Attended controller did not return to its disabled state')
    evidence['controller_state'] = 'disabled_manually'


def run(phase):
    require(os.environ.get('GITHUB_REPOSITORY') == 'NSPG13/agent-bounties' and
            os.environ.get('GITHUB_REF') == 'refs/heads/' + BRANCH and
            os.environ.get('GITHUB_EVENT_NAME') == 'workflow_dispatch',
            'Only attended dispatch on the exact control branch is permitted')
    require(phase in ('inventory', 'pause-and-api', 'workers', 'mainnet-preflight', 'resume-probe', 'hold-spenders', 'upgrade-held-spenders', 'upgrade-runtime', 'activate-posting', 'resume-budgeted-workers', 'restore-workflows'), 'Unknown cutover phase')
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
    if phase == 'restore-workflows':
        restore_workflows(evidence)
        output.write_text(json.dumps(evidence, indent=2) + '\n')
        print(json.dumps({'restored_workflows': evidence['restored_workflows'],
                          'controller_state': evidence['controller_state']}), flush=True)
        return
    if phase == 'resume-probe':
        value = api('render', '/services/' + CANARY_MCP)
        require(value['ownerId'] == WORKSPACE and value['name'] == 'agent-bounties-posting-mcp-canary'
                and value['serviceDetails']['plan'] == 'free' and value['autoDeploy'] == 'no'
                and value['repo'].removesuffix('.git') == PRIVATE_REPO, 'Unexpected isolated canary')
        for flag in ('ENABLE_X402_HOSTED_RELAY', 'ENABLE_BASE_TX_BROADCAST', 'ENABLE_CREATOR_OPEN_SPONSORSHIP'):
            setting = api('render', '/services/' + CANARY_MCP + '/env-vars/' + flag)
            require(setting.get('value', setting.get('envVar', {}).get('value')) == 'false', 'Canary signing not disabled')
        api('render', '/services/' + CANARY_MCP + '/suspend', 'POST')
        require(api('render', '/services/' + CANARY_MCP)['suspended'] == 'suspended', 'Canary not suspended')
        api('render', '/services/' + CANARY_MCP, 'PATCH', {'branch': PRIVATE_BRANCH, 'autoDeployTrigger': 'off'})
        api('render', '/services/' + CANARY_MCP + '/env-vars/POSTING_RESUME_PROBE', 'PUT', {'value': SPENDER_REVISION})
        api('render', '/services/' + CANARY_MCP + '/resume', 'POST')
        evidence['resume_probe'] = api('render', '/services/' + CANARY_MCP + '/deploys?limit=2')
        output.write_text(json.dumps(evidence, indent=2) + '\n')
        print(json.dumps({'resume_probe': evidence['resume_probe']}), flush=True)
        return
    if phase in ('hold-spenders', 'upgrade-held-spenders'):
        require(all(item['state'] == 'disabled_manually' for item in evidence['workflows'].values()),
                'Spending workflows must remain paused')
        for role in ('keeper', 'broker'):
            require(service(role)['suspended'] == 'suspended', 'Spending worker must remain suspended until staged')
            set_env(role, 'POSTING_SENDS_PAUSED', 'true')
            set_env(role, 'BASE_INDEXER_PROTOCOL', 'posting-cutover-paused')
            api('render', '/services/' + SERVICES[role], 'PATCH', {
                'repo': PRIVATE_REPO, 'branch': PRIVATE_BRANCH, 'autoDeployTrigger': 'off',
                'serviceDetails': {'envSpecificDetails': {'dockerfilePath': './Dockerfile.posting-spenders'}}})
            observed = service(role)
            require(observed['repo'].removesuffix('.git') == PRIVATE_REPO and observed['branch'] == PRIVATE_BRANCH
                    and observed['serviceDetails']['envSpecificDetails']['dockerfilePath'] == './Dockerfile.posting-spenders',
                    'Paused image configuration differs')
        if phase == 'upgrade-held-spenders':
            # Attended handoff only: a separate production SQL session holds
            # migration advisory lock 4270265017 before this phase is dispatched.
            # Both exact legacy worker revisions acquire it before initializing
            # their protocols or signers. Keep that session until old waiters
            # disappear and both replacement images report the paused entrypoint.
            evidence['startup_guard'] = 'production migration advisory lock 4270265017 held by attended SQL session'
            evidence['started_deploys'] = {}
            try:
                for role in ('keeper', 'broker'):
                    api('render', '/services/' + SERVICES[role] + '/resume', 'POST')
                    evidence['started_deploys'][role] = deploy(role, SPENDER_REVISION)
                    output.write_text(json.dumps(evidence, indent=2) + '\n')
                deadline = time.monotonic() + 600
                while time.monotonic() < deadline:
                    for role, deployed in evidence['started_deploys'].items():
                        current = api('render', '/services/' + SERVICES[role] + '/deploys/' + deployed['id'])
                        deployed['status'] = current['status']
                        require(current['status'] not in ('build_failed', 'update_failed', 'canceled'),
                                'Paused spender build failed')
                    if all(d['status'] == 'live' for d in evidence['started_deploys'].values()):
                        output.write_text(json.dumps(evidence, indent=2) + '\n')
                        print(json.dumps({'paused_spender_deploys': evidence['started_deploys']}), flush=True)
                        return
                    time.sleep(5)
                raise RuntimeError('Paused spender deployment timed out')
            except Exception:
                for role in ('keeper', 'broker'):
                    api('render', '/services/' + SERVICES[role] + '/suspend', 'POST')
                raise
        return
    if phase in ('activate-posting', 'resume-budgeted-workers'):
        require(all(item['state'] == 'disabled_manually' for item in evidence['workflows'].values()),
                'Spending workflows must remain paused during service activation')
        for role in SERVICES:
            expected = ACTIVE_REVISION if phase == 'resume-budgeted-workers' and role in ('api', 'mcp') else SPENDER_REVISION
            require(any(d['status'] == 'live' and d['commit'] == expected
                        for d in evidence['services'][role]['deploys']), 'Reviewed runtime must be live: ' + role)
        for role in ('keeper', 'broker'):
            setting = api('render', '/services/' + SERVICES[role] + '/env-vars/POSTING_SENDS_PAUSED')
            require(setting.get('value', setting.get('envVar', {}).get('value')) == 'true',
                    'Worker must remain paused until the reviewed handoff')
        evidence['started_deploys'] = {}
        if phase == 'activate-posting':
            for role in ('api', 'mcp'):
                for key, value in {
                    'ENABLE_BASE_TX_BROADCAST': 'false', 'ENABLE_X402_HOSTED_RELAY': 'true',
                    'ENABLE_SPONSORED_BOUNTY_CREATION': 'true', 'ENABLE_SPONSORED_SETUP': 'true',
                    'ENABLE_CREATOR_OPEN_SPONSORSHIP': 'true', 'VERIFIER_EMAIL_ENABLED': 'false',
                }.items():
                    set_env(role, key, value)
                evidence['started_deploys'][role] = deploy(role, ACTIVE_REVISION)
                output.write_text(json.dumps(evidence, indent=2) + '\n')
        else:
            # Existing paid services only. Both binaries attach the same Postgres
            # budget before their keeper/broker is constructed; no principal top-up.
            for role, protocol in [('keeper', 'open-competition-v2-keeper'),
                                   ('broker', 'open-competition-v2-broker')]:
                set_env(role, 'BASE_INDEXER_PROTOCOL', protocol)
                set_env(role, 'POSTING_SENDS_PAUSED', 'false')
                evidence['started_deploys'][role] = deploy(role, SPENDER_REVISION)
                output.write_text(json.dumps(evidence, indent=2) + '\n')
        print(json.dumps({'started_deploys': evidence['started_deploys']}), flush=True)
        return
    if phase == 'upgrade-runtime':
        require(all(item['state'] == 'disabled_manually' for item in evidence['workflows'].values()),
                'Spending workflows must remain paused')
        for role in ('keeper', 'broker'):
            require(any(d['status'] == 'live' and d['commit'] == SPENDER_REVISION
                        for d in evidence['services'][role]['deploys']), 'Budget-aware paused worker must be live')
            setting = api('render', '/services/' + SERVICES[role] + '/env-vars/POSTING_SENDS_PAUSED')
            require(setting.get('value', setting.get('envVar', {}).get('value')) == 'true',
                    'Worker must remain paused')
        root = Path(__file__).resolve().parents[1] / 'ops'
        creator = (root / 'posting-creator-open-mainnet-release-20261001.json').read_text()
        setup = (root / 'posting-setup-mainnet-release-20261001.json').read_text()
        require(json.loads(creator)['factory'] == '0xf9c684ee0157d311ab20e365323575130c95af0d'
                and json.loads(creator)['deployment_block'] == 52020697, 'Creator release differs')
        require(json.loads(setup)['wallet_factory'] == '0xe3d4f7b203c5e8576e0225d3e64a8532429d3876'
                and json.loads(setup)['deployment_block'] == 49539535, 'Setup release differs')
        evidence['started_deploys'] = {}
        for role in ('api', 'mcp'):
            for key, value in FLAGS_OFF.items():
                set_env(role, key, value)
            set_env(role, 'CREATOR_OPEN_MAINNET_RELEASE_JSON', creator)
            set_env(role, 'SPONSORED_SETUP_MAINNET_RELEASE_JSON', setup)
        set_env('legacy', 'CREATOR_OPEN_RELEASE_JSON', creator)
        set_env('legacy', 'POSTING_CREATOR_INDEXER_ENABLED', 'true')
        for role in ('api', 'mcp', 'legacy', 'v1', 'v2', 'shadow'):
            rebind(role)
            evidence['started_deploys'][role] = deploy(role, SPENDER_REVISION)
            output.write_text(json.dumps(evidence, indent=2) + '\n')
        print(json.dumps({'started_deploys': evidence['started_deploys']}), flush=True)
        return
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
