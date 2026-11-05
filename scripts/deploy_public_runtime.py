#!/usr/bin/env python3
"""Deploy a publicly reachable, green main revision without changing payment flags or secrets.

Only --migrate-source changes configuration, and only repository/branch/autodeploy.
All service identities are checked before any mutation. No provisioning, wallet,
environment, database, secret, or payment endpoint is used by this controller.
"""
from __future__ import annotations
import argparse
import datetime as dt
import json
import os
from pathlib import Path
import re
import subprocess
import time
import urllib.error
import urllib.request

ROOT=Path(__file__).resolve().parents[1]
PUBLIC='https://github.com/NSPG13/agent-bounties'
TERMINAL_FAILURE={'build_failed','update_failed','pre_deploy_failed','canceled','deactivated'}

class ReleaseError(RuntimeError):pass

def validate_release_approval(revision, expected, approved, event):
    if not re.fullmatch(r'[0-9a-f]{40}',revision):raise ReleaseError('exact Git revision required')
    if approved:
        if event!='workflow_dispatch' or approved!=revision:
            raise ReleaseError('manual approval must name this exact public revision')
    elif expected and expected!=revision:
        raise ReleaseError('production expectation differs; supply an exact manual approval without changing live monitors')

def validate_accepted_reader(previous, expected, manifest, deploys, deploy_id, revision):
    # A historical matching deployment is not the current running artifact.
    # Its original binding must already have been the reviewed public source.
    validate_service(previous,expected,manifest,False)
    newest=deploys[0].get('deploy',deploys[0]) if deploys else {}
    if newest.get('id')!=deploy_id or not validate_deploy(newest,revision):
        raise ReleaseError('accepted autonomous deployment is no longer the exact current live revision')

class Render:
    def __init__(self, token):
        if not token:raise ReleaseError('RENDER_API_KEY is required')
        self.token=token
    def request(self, method, path, body=None):
        request=urllib.request.Request('https://api.render.com/v1/'+path,
            data=json.dumps(body).encode() if body is not None else None,
            method=method,headers={'Authorization':'Bearer '+self.token,'Content-Type':'application/json','Accept':'application/json'})
        try:
            with urllib.request.urlopen(request,timeout=30) as response:return json.load(response)
        except urllib.error.HTTPError as error:
            # Provider response bodies may contain secrets; never include them.
            raise ReleaseError(f'Render {method} {path} returned HTTP {error.code}') from None

def validate_service(service, expected, manifest, migrate):
    if any(service.get(field)!=expected[field] for field in ('id','name','type')) or service.get('ownerId')!=manifest['workspace_id']:
        raise ReleaseError('service identity or workspace does not match the reviewed inventory')
    if expected.get('dockerfile'):
        build=service.get('serviceDetails',{}).get('envSpecificDetails',{})
        if (service.get('rootDir','')!='' or build.get('dockerfilePath')!=expected['dockerfile']
                or build.get('dockerContext')!='.' or build.get('dockerCommand','')!=''):
            raise ReleaseError('production build recipe differs from reviewed source')
    repos={PUBLIC}
    if migrate:repos.add(manifest['migration_from_repository'])
    if service.get('repo','').removesuffix('.git') not in repos:
        raise ReleaseError('unexpected service source repository')
    if service.get('repo','').removesuffix('.git')==PUBLIC:
        if service.get('branch')!='main' or service.get('autoDeploy')!='no':
            raise ReleaseError('public production must use main with native auto-deploy disabled')
    elif not migrate:raise ReleaseError('private source requires the explicit migration operation')

def public_ci_revision(revision):
    if not re.fullmatch(r'[0-9a-f]{40}',revision):raise ReleaseError('exact Git revision required')
    head=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip()
    if revision!=head:raise ReleaseError('controller checkout must equal deployment revision')
    subprocess.run(['python3','scripts/check_public_runtime_parity.py','--published'],cwd=ROOT,check=True)
    for workflow in ('ci.yml', 'containers.yml'):
        query=f'https://api.github.com/repos/NSPG13/agent-bounties/actions/workflows/{workflow}/runs?head_sha={revision}&event=push&per_page=100'
        with urllib.request.urlopen(urllib.request.Request(query,headers={'Accept':'application/vnd.github+json','User-Agent':'agentizable-public-release'}),timeout=30) as response:
            runs=json.load(response)['workflow_runs']
        eligible=[run for run in runs if run.get('head_sha')==revision and run.get('head_branch')=='main' and run.get('event')=='push']
        if not eligible or max(eligible,key=lambda run:run['id']).get('conclusion')!='success':
            raise ReleaseError(f'latest public main {workflow} attempt must have succeeded for this exact revision')

def validate_blueprints(client, manifest):
    policies=manifest.get('blueprints')
    if not isinstance(policies,list):
        raise ReleaseError('reviewed Blueprint inventory is required')
    seen=set()
    for expected in policies:
        identity=expected.get('id','')
        if not re.fullmatch(r'exs-[a-z0-9]+',identity) or identity in seen:
            raise ReleaseError('invalid or duplicate Blueprint identity')
        seen.add(identity)
        value=client.request('GET','blueprints/'+identity)
        resources=value.get('resources')
        expected_ids=expected.get('resource_ids')
        if (any(value.get(key)!=expected.get(key) for key in ('id','name','path'))
                or value.get('ownerId')!=manifest['workspace_id']
                or value.get('repo','').removesuffix('.git')!=PUBLIC
                or value.get('branch')!='main' or value.get('autoSync') is not False
                or value.get('status')!='paused'
                or not isinstance(resources,list) or not isinstance(expected_ids,list)
                or not expected_ids or len(set(expected_ids))!=len(expected_ids)
                or len(resources)!=len(expected_ids)
                or {row.get('id') for row in resources}!=set(expected_ids)):
            raise ReleaseError('Blueprint identity, ownership or automatic-sync policy changed')

def prepare(client, manifest, migrate, bootstrap=False):
    if manifest.get('repository')!=PUBLIC or manifest.get('branch')!='main':raise ReleaseError('invalid public source policy')
    bootstrap_id=manifest.get('autonomous_bootstrap_service_id')
    if bootstrap and (not migrate or sum(row.get('id')==bootstrap_id and row.get('type')=='background_worker'
            and row.get('dockerfile')=='./Dockerfile.posting-indexers' for row in manifest['services'])!=1):
        raise ReleaseError('bootstrap requires explicit migration and the reviewed autonomous reader service')
    services=[]
    for expected in manifest['services']:
        service=client.request('GET','services/'+expected['id'])
        validate_service(service,expected,manifest,migrate)
        services.append((expected,service))
    validate_blueprints(client,manifest)
    # Validate the entire inventory before narrowing the mutation boundary.
    # Bootstrap is not a full release: its checkpoint acceptance is independent.
    if bootstrap:services=[pair for pair in services if pair[0]['id']==bootstrap_id]
    for expected,service in services:
        if service['repo'].removesuffix('.git')!=PUBLIC:
            client.request('PATCH','services/'+expected['id'],{'repo':PUBLIC,'branch':'main','autoDeployTrigger':'off'})
    # Re-read every binding after all source migrations and before any deploy.
    for expected,_ in services:
        validate_service(client.request('GET','services/'+expected['id']),expected,manifest,False)
    return services

def reader_readiness(manifest, revision=None):
    policy = manifest['marketplace_readiness']
    request = urllib.request.Request(policy['url'], headers={'Cache-Control': 'no-cache'})
    try:
        with urllib.request.urlopen(request, timeout=15) as response:
            observed_revision = response.headers.get('x-agent-bounties-revision', '')
            if response.status != 200 or not re.fullmatch(r'[0-9a-f]{40}', observed_revision):
                raise ReleaseError('marketplace readiness or revision is unavailable')
            if revision is not None and observed_revision != revision:
                raise ReleaseError('marketplace readiness has the wrong source revision')
            value = json.load(response)
        readers = value['readers']
        checked = dt.datetime.fromisoformat(value['checked_at'].replace('Z', '+00:00'))
        age = (dt.datetime.now(dt.timezone.utc) - checked).total_seconds()
        expected = policy['protocols']
        if (value.get('schema') != 'agent-bounties/marketplace-readiness-v1'
                or value.get('ready') is not True or not -5 <= age <= 30
                or not expected or len(readers) != len(expected)
                or {row['protocol'] for row in readers} != set(expected)
                or any(row.get('ready') is not True or row.get('blocker') is not None for row in readers)):
            raise ReleaseError('marketplace readers are not all current and ready')
        return {'revision': observed_revision, 'checked_at': value['checked_at'],
                'protocols': [row['protocol'] for row in readers]}
    except (urllib.error.URLError, TimeoutError, KeyError, TypeError, ValueError):
        raise ReleaseError('marketplace readiness is unavailable; investigate before restarting readers') from None

def prepare_release(client, manifest, migrate, bootstrap=False):
    # Deploying a worker also restarts it. Never use a routine source release to
    # bypass a reader's integrity halt or an unresolved lag/health incident.
    reader_readiness(manifest)
    return prepare(client, manifest, migrate, bootstrap)

def verify_live_readers(manifest, revision):
    deadline = time.monotonic() + 300
    successes = 0
    while successes < 8:
        try:
            observation = reader_readiness(manifest, revision)
            successes += 1
        except ReleaseError:
            successes = 0
        if time.monotonic() >= deadline:
            raise ReleaseError('marketplace readers did not recover after the release')
        if successes < 8:
            time.sleep(3 if successes else 15)
    return observation

def validate_deploy(value, revision, expected_id=None):
    if expected_id is not None and value.get('id')!=expected_id:
        raise ReleaseError('deployment response does not match the requested artifact ID')
    if value.get('status') in TERMINAL_FAILURE:raise ReleaseError('deploy failed: '+value['status'])
    if value.get('status')=='live':
        if value.get('commit',{}).get('id')!=revision:raise ReleaseError('live artifact has the wrong source revision')
        return True
    return False

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--revision',required=True)
    parser.add_argument('--migrate-source',action='store_true')
    parser.add_argument('--bootstrap-autonomous-readers',action='store_true',
        help='Deploy only the reviewed autonomous reader service; independent checkpoint acceptance is required before the full release')
    parser.add_argument('--accepted-autonomous-deploy-id',
        help='Reuse the exact independently accepted autonomous deployment during manual full promotion')
    parser.add_argument('--evidence-dir',type=Path,required=True)
    args=parser.parse_args()
    approved=os.environ.get('APPROVED_REVISION','')
    event=os.environ.get('RELEASE_EVENT','')
    validate_release_approval(args.revision,os.environ.get('PRODUCTION_EXPECTED_REVISION',''),approved,event)
    if args.accepted_autonomous_deploy_id and (args.bootstrap_autonomous_readers
            or event!='workflow_dispatch' or approved!=args.revision
            or not re.fullmatch(r'dep-[a-z0-9]+',args.accepted_autonomous_deploy_id)):
        raise ReleaseError('accepted reader reuse requires an exact approved manual full release')
    public_ci_revision(args.revision)
    manifest=json.loads((ROOT/'ops/public-runtime-services.json').read_text())
    if args.accepted_autonomous_deploy_id and sum(row.get('id')==manifest.get('autonomous_bootstrap_service_id')
            and row.get('type')=='background_worker' and row.get('dockerfile')=='./Dockerfile.posting-indexers'
            for row in manifest['services'])!=1:
        raise ReleaseError('accepted reader is absent from the reviewed inventory')
    client=Render(os.environ.get('RENDER_API_KEY'))
    directory=args.evidence_dir;directory.mkdir(parents=True,exist_ok=False)
    record={'revision':args.revision,'repository':PUBLIC,'workspace_id':manifest['workspace_id'],
            'phase':'autonomous-bootstrap' if args.bootstrap_autonomous_readers else 'full-release',
            'status':'running','services':[]}
    def save():
        temporary=directory/'release.tmp';temporary.write_text(json.dumps(record,indent=2)+'\n');temporary.replace(directory/'release.json')
    save()
    try:
        services=prepare_release(client,manifest,args.migrate_source,args.bootstrap_autonomous_readers)
        for expected,previous in services:
            deploys=client.request('GET',f"services/{expected['id']}/deploys?limit=20")
            live=[v.get('deploy',v) for v in deploys if v.get('deploy',v).get('status')=='live']
            row={**expected,'previous_live':[{'id':v['id'],'commit':v.get('commit',{}).get('id')} for v in live]}
            if args.accepted_autonomous_deploy_id and expected['id']==manifest['autonomous_bootstrap_service_id']:
                validate_accepted_reader(previous,expected,manifest,deploys,args.accepted_autonomous_deploy_id,args.revision)
                row.update(deploy_id=args.accepted_autonomous_deploy_id,reused_accepted_deployment=True)
            record['services'].append(row);save()
        # Validate any accepted reader before starting another service deploy.
        for row in record['services']:
            if row.get('reused_accepted_deployment'):continue
            deployed=client.request('POST',f"services/{row['id']}/deploys",{'commitId':args.revision,'clearCache':'do_not_clear'})
            validate_deploy(deployed,args.revision)
            row.update(deploy_id=deployed['id'],status=deployed.get('status'));save()
        deadline=time.monotonic()+40*60
        while True:
            all_live=True
            for row in record['services']:
                if row.get('status')=='live':continue
                deploy=client.request('GET',f"services/{row['id']}/deploys/{row['deploy_id']}")
                done=validate_deploy(deploy,args.revision,row['deploy_id']);row['status']=deploy['status'];all_live &= done
            save()
            if all_live:break
            if time.monotonic()>deadline:raise ReleaseError('deployment deadline exceeded')
            time.sleep(15)
        if args.bootstrap_autonomous_readers:
            record['status']='bootstrap_deployed_pending_acceptance';save()
            print('Autonomous readers deployed; historical checkpoint acceptance and full public release are still required: '+args.revision)
            return
        for _ in range(8):
            for row in record['services']:
                if not row.get('health_url'):continue
                request=urllib.request.Request(row['health_url'],headers={'Cache-Control':'no-cache'})
                with urllib.request.urlopen(request,timeout=15) as response:
                    if response.status!=200 or response.headers.get('x-agent-bounties-revision')!=args.revision:
                        raise ReleaseError('production health or public source revision mismatch')
            time.sleep(3)
        record['marketplace_readiness']=verify_live_readers(manifest,args.revision)
        record['status']='passed';save()
        print('Public runtime release verified: '+args.revision)
    except Exception:
        record['status']='failed';save();raise
if __name__=='__main__':main()
