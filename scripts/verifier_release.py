#!/usr/bin/env python3
"""Use an exact reviewed worker release; never compile moving application sources.

A health receipt reports authenticated service checks, not a solver verdict.
Only the isolated signer diagnostic forwards its own signing key to the
verified worker. It signs a fresh non-payment challenge and exposes no signature.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import urllib.request
import urllib.parse
from datetime import datetime, timezone

SCHEMA = 'agent-bounties/verifier-release-v1'
FIELDS = {'schema','release_id','source_revision','network','worker_build_digest','worker_binary_digest','signing_runtime_digest','profile_registry_digest','signers'}
HEX = re.compile(r'^sha256:[0-9a-f]{64}$')
ROOT = Path(__file__).resolve().parents[1]
MAX_BINARY_BYTES = 256 * 1024 * 1024

def sha256(path: Path) -> str:
    result = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024*1024), b''):
            result.update(chunk)
    return 'sha256:' + result.hexdigest()

def validate_release(value: dict) -> dict:
    if set(value) != FIELDS or value['schema'] != SCHEMA or value['network'] not in ('base-mainnet','base-sepolia'):
        raise ValueError('invalid reviewed release schema')
    if not re.fullmatch(r'[0-9a-f]{40}', value['source_revision']):
        raise ValueError('release source must be an exact commit')
    for key in ('worker_build_digest','worker_binary_digest','signing_runtime_digest','profile_registry_digest','release_id'):
        if not isinstance(value[key],str) or not HEX.fullmatch(value[key]): raise ValueError('invalid release digest')
    signers=value['signers']
    if not isinstance(signers,list) or not 1<=len(signers)<=2 or any(not isinstance(s,str) or not re.fullmatch(r'0x[0-9a-f]{40}',s) for s in signers) or len(set(signers))!=len(signers):
        raise ValueError('release signer set is invalid')
    body={k:v for k,v in value.items() if k!='release_id'}
    calculated='sha256:'+hashlib.sha256(json.dumps(body,sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()).hexdigest()
    if calculated!=value['release_id']: raise ValueError('release identity mismatch')
    return value

def verify_sources(release: dict, root: Path) -> None:
    head=subprocess.check_output(['git','-C',str(root),'rev-parse','HEAD'],text=True).strip()
    if head!=release['source_revision']: raise ValueError('checkout is not the reviewed source revision')
    # The helper itself comes from the reviewed checkout. The source guard binds
    # exact tracked bytes, including its own code and all compile-time includes.
    for scope,key in [('worker-build','worker_build_digest'),('signing-runtime','signing_runtime_digest')]:
        subprocess.run(['python3',str(root/'scripts/regression_verifier_source_guard.py'),'--root',str(root),'--scope',scope,'--expected-sha256',release[key].removeprefix('sha256:')],check=True)
    if sha256(root/'crates/verifier-sdk/regression-profiles-v1.json')!=release['profile_registry_digest']:
        raise ValueError('profile registry differs from the reviewed release')

def verify_worker(path: Path, release: dict) -> None:
    if path.is_symlink() or not path.is_file() or not 0<path.stat().st_size<=MAX_BINARY_BYTES or sha256(path)!=release['worker_binary_digest']:
        raise ValueError('worker binary differs from the reviewed release')

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self,*args,**kwargs):
        return None

def validate_download_url(url: str) -> None:
    parsed=urllib.parse.urlsplit(url)
    if parsed.scheme!='https' or parsed.port not in (None,443) or parsed.username or parsed.password or parsed.fragment or parsed.hostname not in ('raw.githubusercontent.com','objects.githubusercontent.com','release-assets.githubusercontent.com','github.com'):
        raise ValueError('unsupported public release download URL')

class ReleaseRedirect(urllib.request.HTTPRedirectHandler):
    max_redirections = 3
    max_repeats = 1
    def redirect_request(self,req,fp,code,msg,headers,newurl):
        validate_download_url(newurl)
        return super().redirect_request(req,fp,code,msg,headers,newurl)

def ensure_worker(path: Path, release: dict, url: str | None = None) -> None:
    if path.exists() or path.is_symlink():
        verify_worker(path,release)
        return
    if not url:
        raise ValueError('an immutable HTTPS binary URL is required for an uncached release')
    validate_download_url(url)
    path.parent.mkdir(parents=True,exist_ok=True)
    temporary=None
    try:
        with tempfile.NamedTemporaryFile(dir=path.parent,delete=False) as output:
            temporary=Path(output.name)
            opener=urllib.request.build_opener(urllib.request.ProxyHandler({}),ReleaseRedirect())
            with opener.open(urllib.request.Request(url,headers={'User-Agent':'AgentBounties-reviewed-worker/1'}),timeout=60) as response:
                received=0
                while chunk:=response.read(1024*1024):
                    received+=len(chunk)
                    if received>MAX_BINARY_BYTES: raise ValueError('release binary exceeds download limit')
                    output.write(chunk)
        verify_worker(temporary,release)
        temporary.chmod(0o700)
        os.replace(temporary,path)
    finally:
        if temporary is not None: temporary.unlink(missing_ok=True)

def post_checkpoint(api_base: str, release: dict, component: str, healthy: bool, token: str) -> None:
    if component not in ['runner',*[f'signer:{address}' for address in release['signers']]]:
        raise ValueError('checkpoint component is outside the approved release')
    if not 32<=len(token)<=256: raise ValueError('checkpoint credential unavailable')
    parsed=urllib.parse.urlsplit(api_base)
    if parsed.scheme!='https' or parsed.port not in (None,443) or parsed.hostname not in ('api.agentbounties.app','mcp.agentbounties.app') or parsed.username or parsed.password or parsed.query or parsed.fragment or parsed.path not in ('','/'):
        raise ValueError('checkpoint destination must be a first-party service')
    payload={'release':release,'component':component,'issued_at':datetime.now(timezone.utc).isoformat(),'healthy':healthy}
    request=urllib.request.Request(api_base.rstrip('/')+'/internal/verifier-checkpoints',data=json.dumps(payload).encode(),headers={'Content-Type':'application/json','Authorization':'Bearer '+token})
    opener=urllib.request.build_opener(urllib.request.ProxyHandler({}),NoRedirect())
    with opener.open(request,timeout=15) as response:
        result=json.load(response)
    if result.get('accepted') is not True or result.get('release_id')!=release['release_id'] or result.get('component')!=component:
        raise ValueError('checkpoint was not accepted for the reviewed release')

def component_environment(component: str) -> dict[str, str]:
    allowed = ('PATH', 'SYSTEMROOT', 'WINDIR', 'TEMP', 'TMP', 'TMPDIR')
    result = {key: os.environ[key] for key in allowed if key in os.environ}
    result['LANG'] = 'C.UTF-8'
    if component.startswith('signer:'):
        key = os.environ.get('REGRESSION_VERIFIER_PRIVATE_KEY', '')
        if not key: raise ValueError('signer credential unavailable')
        result['REGRESSION_VERIFIER_PRIVATE_KEY'] = key
    return result

def diagnose_component(worker: Path, release: dict, component: str, api_base: str) -> dict:
    if component not in ['runner', *[f'signer:{address}' for address in release['signers']]]:
        raise ValueError('diagnostic component is outside the approved release')
    # Verify the worker again immediately before any signer key reaches it.
    verify_worker(worker, release)
    environment = component_environment(component)
    if component == 'runner':
        registry = json.loads((ROOT/'crates/verifier-sdk/regression-profiles-v1.json').read_text())
        images = sorted({p['runner_manifest']['image'] for p in registry['profiles'] if p['status']=='approved'})
        for image in images:
            subprocess.run(['docker','pull',image],env=environment,check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=120)
        command=[str(worker.resolve()),'--check-regression-runner-health']
    else:
        command=[str(worker.resolve()),'--check-regression-signer-health',component.removeprefix('signer:'),release['release_id']]
    result=subprocess.run(command,env=environment,check=True,capture_output=True,text=True,timeout=180)
    if json.loads(result.stdout) != {'healthy':True}: raise ValueError('component diagnostic failed')
    # Keep pending-job discovery outside the public ready-to-earn filter. This
    # read carries no health or signing credential and cannot spend funds.
    parsed=urllib.parse.urlsplit(api_base)
    if parsed.scheme!='https' or parsed.netloc not in ('api.agentbounties.app','mcp.agentbounties.app') or parsed.path not in ('','/') or parsed.query or parsed.fragment:
        raise ValueError('diagnostic destination must be a first-party service')
    verifier = release['signers'][0] if component=='runner' else component.removeprefix('signer:')
    query=urllib.parse.urlencode({'network':release['network'],'verifier':verifier})
    request=urllib.request.Request(api_base.rstrip('/')+'/v1/base/autonomous-bounties/verification-jobs?'+query,headers={'User-Agent':'AgentBounties-reviewed-component/1','Accept':'application/json'})
    opener=urllib.request.build_opener(urllib.request.ProxyHandler({}),NoRedirect())
    with opener.open(request,timeout=15) as response:
        data=response.read(2*1024*1024+1)
    if len(data)>2*1024*1024 or not isinstance(json.loads(data),list): raise ValueError('pending-job discovery unavailable')
    return {'schema':'agent-bounties/verifier-component-health-v1','release_id':release['release_id'],'component':component,'checked_at':datetime.now(timezone.utc).isoformat(),'healthy':True}

def authorize_candidate_release(directory: Path, release: dict) -> None:
    path=directory/'release.json'
    if path.is_symlink() or not path.is_file() or path.stat().st_size>16384:
        raise ValueError('candidate release identity unavailable')
    if validate_release(json.loads(path.read_text())) != release:
        raise ValueError('candidate belongs to a different reviewed release')

def main() -> None:
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation',choices=['verify','fetch','diagnose','checkpoint','authorize'])
    parser.add_argument('--manifest',type=Path,required=True)
    parser.add_argument('--worker',type=Path)
    parser.add_argument('--candidates',type=Path)
    parser.add_argument('--run-id',type=int)
    parser.add_argument('--download-url')
    parser.add_argument('--api-base',default='https://api.agentbounties.app')
    parser.add_argument('--component',default='runner')
    parser.add_argument('--health-result',type=Path,help='Trusted component health result from the reviewed runner or isolated signer')
    args=parser.parse_args()
    release=validate_release(json.loads(args.manifest.read_text()))
    verify_sources(release,ROOT)
    if args.operation=='authorize':
        if not args.candidates or not args.run_id or args.run_id<1: raise ValueError('candidate directory and run identity required')
        authorize_candidate_release(args.candidates,release)
        output=os.environ.get('GITHUB_OUTPUT')
        if not output: raise ValueError('workflow output unavailable')
        with open(output,'a') as stream:
            stream.write(f"authorized=true\nrevision={release['source_revision']}\nrun_id={args.run_id}\n")
        return
    if not args.worker: raise ValueError('a reviewed worker path is required')
    if args.operation=='fetch': ensure_worker(args.worker,release,args.download_url)
    verify_worker(args.worker,release)
    if args.operation=='diagnose':
        if not args.health_result: raise ValueError('a diagnostic output path is required')
        try:
            diagnostic=diagnose_component(args.worker,release,args.component,args.api_base)
        except (ValueError,OSError,subprocess.SubprocessError):
            diagnostic={'schema':'agent-bounties/verifier-component-health-v1','release_id':release['release_id'],'component':args.component,'checked_at':datetime.now(timezone.utc).isoformat(),'healthy':False}
        args.health_result.parent.mkdir(parents=True,exist_ok=True)
        args.health_result.write_text(json.dumps(diagnostic,sort_keys=True))
        args.health_result.chmod(0o600)
        post_checkpoint(args.api_base,release,args.component,diagnostic['healthy'],os.environ.get('REGRESSION_CHECKPOINT_TOKEN',''))
        if not diagnostic['healthy']: raise ValueError('component diagnostic failed; unavailable checkpoint recorded')
    if args.operation=='checkpoint':
        if not args.health_result: raise ValueError('a completed component diagnostic is required')
        diagnostic=json.loads(args.health_result.read_text())
        if set(diagnostic)!={'schema','release_id','component','checked_at','healthy'} or diagnostic['schema']!='agent-bounties/verifier-component-health-v1' or diagnostic['release_id']!=release['release_id'] or diagnostic['component']!=args.component or type(diagnostic['healthy']) is not bool:
            raise ValueError('diagnostic is not bound to this component and release')
        age=(datetime.now(timezone.utc)-datetime.fromisoformat(diagnostic['checked_at'])).total_seconds()
        if not 0<=age<=120: raise ValueError('component diagnostic is stale')
        post_checkpoint(args.api_base,release,args.component,diagnostic['healthy'],os.environ.get('REGRESSION_CHECKPOINT_TOKEN',''))
    print(json.dumps({'release_id':release['release_id'],'worker_verified':True}))

if __name__=='__main__': main()
