#!/usr/bin/env python3
"""Deploy a publicly reachable, green main revision without changing payment flags or secrets.

Only --migrate-source changes configuration, and only repository/branch/autodeploy.
All service identities are checked before any mutation. No provisioning, wallet,
environment, database, secret, or payment endpoint is used by this controller.
"""
from __future__ import annotations
import argparse
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
    query=f'https://api.github.com/repos/NSPG13/agent-bounties/actions/workflows/ci.yml/runs?head_sha={revision}&event=push&per_page=100'
    with urllib.request.urlopen(urllib.request.Request(query,headers={'Accept':'application/vnd.github+json','User-Agent':'agentizable-public-release'}),timeout=30) as response:
        runs=json.load(response)['workflow_runs']
    eligible=[run for run in runs if run.get('head_sha')==revision and run.get('head_branch')=='main' and run.get('event')=='push']
    if not eligible or max(eligible,key=lambda run:run['id']).get('conclusion')!='success':
        raise ReleaseError('latest public main CI attempt must have succeeded for this exact revision')

def prepare(client, manifest, migrate):
    if manifest.get('repository')!=PUBLIC or manifest.get('branch')!='main':raise ReleaseError('invalid public source policy')
    services=[]
    for expected in manifest['services']:
        service=client.request('GET','services/'+expected['id'])
        validate_service(service,expected,manifest,migrate)
        services.append((expected,service))
    for expected,service in services:
        if service['repo'].removesuffix('.git')!=PUBLIC:
            client.request('PATCH','services/'+expected['id'],{'repo':PUBLIC,'branch':'main','autoDeployTrigger':'off'})
    # Re-read every binding after all source migrations and before any deploy.
    for expected,_ in services:
        validate_service(client.request('GET','services/'+expected['id']),expected,manifest,False)
    return services

def validate_deploy(value, revision):
    if value.get('status') in TERMINAL_FAILURE:raise ReleaseError('deploy failed: '+value['status'])
    if value.get('status')=='live':
        if value.get('commit',{}).get('id')!=revision:raise ReleaseError('live artifact has the wrong source revision')
        return True
    return False

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--revision',required=True)
    parser.add_argument('--migrate-source',action='store_true')
    parser.add_argument('--evidence-dir',type=Path,required=True)
    args=parser.parse_args()
    public_ci_revision(args.revision)
    manifest=json.loads((ROOT/'ops/public-runtime-services.json').read_text())
    client=Render(os.environ.get('RENDER_API_KEY'))
    directory=args.evidence_dir;directory.mkdir(parents=True,exist_ok=False)
    record={'revision':args.revision,'repository':PUBLIC,'workspace_id':manifest['workspace_id'],'status':'running','services':[]}
    def save():
        temporary=directory/'release.tmp';temporary.write_text(json.dumps(record,indent=2)+'\n');temporary.replace(directory/'release.json')
    save()
    try:
        services=prepare(client,manifest,args.migrate_source)
        for expected,previous in services:
            deploys=client.request('GET',f"services/{expected['id']}/deploys?limit=20")
            live=[v.get('deploy',v) for v in deploys if v.get('deploy',v).get('status')=='live']
            row={**expected,'previous_live':[{'id':v['id'],'commit':v.get('commit',{}).get('id')} for v in live]}
            record['services'].append(row);save()
            deployed=client.request('POST',f"services/{expected['id']}/deploys",{'commitId':args.revision,'clearCache':'do_not_clear'})
            row.update(deploy_id=deployed['id'],status=deployed.get('status'));save()
        deadline=time.monotonic()+40*60
        while True:
            all_live=True
            for row in record['services']:
                if row.get('status')=='live':continue
                deploy=client.request('GET',f"services/{row['id']}/deploys/{row['deploy_id']}")
                done=validate_deploy(deploy,args.revision);row['status']=deploy['status'];all_live &= done
            save()
            if all_live:break
            if time.monotonic()>deadline:raise ReleaseError('deployment deadline exceeded')
            time.sleep(15)
        for _ in range(8):
            for row in record['services']:
                if not row.get('health_url'):continue
                request=urllib.request.Request(row['health_url'],headers={'Cache-Control':'no-cache'})
                with urllib.request.urlopen(request,timeout=15) as response:
                    if response.status!=200 or response.headers.get('x-agent-bounties-revision')!=args.revision:
                        raise ReleaseError('production health or public source revision mismatch')
            time.sleep(3)
        record['status']='passed';save()
        print('Public runtime release verified: '+args.revision)
    except Exception:
        record['status']='failed';save();raise
if __name__=='__main__':main()
