#!/usr/bin/env python3
"""Reject unprotected signing jobs and keep deploy credentials out of key custody."""
from pathlib import Path
import re
import sys
import yaml

ROOT=Path(__file__).resolve().parents[1]
KEY=re.compile(r'secrets\.(?:[A-Z0-9_]*PRIVATE_KEY|KEEPER_KEY)\b', re.IGNORECASE)
PIN=re.compile(r'[^@]+@[0-9a-f]{40}$')
SIGNING_WORKFLOWS = {
    './.github/workflows/regression-verifier-signing-reusable.yml',
    './.github/workflows/leaderboard-reward-signing-reusable.yml',
}

def violations(document):
    errors=[]
    if KEY.search(str(document.get('env',{}))): errors.append('workflow-level signing key')
    for name,job in document.get('jobs',{}).items():
        if not KEY.search(str(job)): continue
        prefix=f'{name}: '
        if job.get('uses'):
            if job['uses'] not in SIGNING_WORKFLOWS:
                errors.append(prefix+'unreviewed reusable signing workflow')
            if job.get('environment'):
                errors.append(prefix+'environment must protect the called execution job')
        elif not job.get('environment'): errors.append(prefix+'missing protected environment')
        guard=str(job.get('if',''))
        for required in ["github.repository == 'NSPG13/agent-bounties'", "github.ref == 'refs/heads/main'", "github.event_name != 'pull_request'"]:
            if required not in guard: errors.append(prefix+'missing trusted-main boundary: '+required)
        if KEY.search(str(job.get('env',{}))): errors.append(prefix+'job-level signing key')
        for step in job.get('steps',[]):
            action=step.get('uses','')
            if action and not action.startswith('./') and not PIN.fullmatch(action): errors.append(prefix+'action is not pinned to a full commit: '+action)
            if action.startswith('actions/checkout@') and step.get('with',{}).get('persist-credentials') is not False: errors.append(prefix+'checkout retains credentials')
            if KEY.search(str(step)) and (not step.get('run') or step.get('uses')): errors.append(prefix+'signing keys may only enter an explicit run step')
    return errors

def main():
    errors=[]
    for path in sorted((ROOT/'.github/workflows').glob('*.yml')):
        for error in violations(yaml.safe_load(path.read_text())):errors.append(f'{path.name}: {error}')
    for error in errors:print(error,file=sys.stderr)
    if errors:return 1
    print('Signing workflows are restricted to protected main, pinned actions and explicit signing steps.')
    return 0
if __name__=='__main__':raise SystemExit(main())
