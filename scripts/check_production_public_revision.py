#!/usr/bin/env python3
"""Read-only production observer: both hosts must run one publicly reachable revision."""
from pathlib import Path
import argparse
import json
import re
import subprocess
import urllib.request

ROOT=Path(__file__).resolve().parents[1]
HOSTS=('https://api.agentizable.ai','https://mcp.agentizable.ai')

def validate(revisions, public_ancestor):
    if len(revisions)!=2 or any(not re.fullmatch(r'[0-9a-f]{40}',sha or '') for sha in revisions):
        raise ValueError('production must report full source revisions on both hosts')
    if len(set(revisions))!=1:raise ValueError('API and MCP production revisions disagree')
    if not public_ancestor(revisions[0]):raise ValueError('production revision is not reachable from public main')
    return revisions[0]

def main():
    revisions=[]
    for host in HOSTS:
        request=urllib.request.Request(host+'/health',headers={'Cache-Control':'no-cache','User-Agent':'agentizable-public-source-observer'})
        with urllib.request.urlopen(request,timeout=30) as response:
            if response.status!=200:raise ValueError('unhealthy production host')
            revisions.append(response.headers.get('x-agent-bounties-revision','').lower())
    subprocess.run(['git','fetch','--quiet','https://github.com/NSPG13/agent-bounties.git','refs/heads/main'],cwd=ROOT,check=True,timeout=120)
    sha=validate(revisions,lambda sha:subprocess.run(['git','merge-base','--is-ancestor',sha,'FETCH_HEAD'],cwd=ROOT,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode==0)
    print(json.dumps({'public_repository':'https://github.com/NSPG13/agent-bounties','revision':sha,'hosts':HOSTS}))
if __name__=='__main__':main()
