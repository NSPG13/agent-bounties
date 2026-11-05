#!/usr/bin/env python3
"""Read-only source audit of every reviewed production service, including workers."""
import json
import os
import re
import subprocess

from deploy_public_runtime import ROOT, PUBLIC, Render, ReleaseError, validate_service


def audit(client, manifest, public_ancestor):
    rows = []
    for expected in manifest['services']:
        service = client.request('GET', 'services/' + expected['id'])
        validate_service(service, expected, manifest, False)
        deploys = client.request('GET', f"services/{expected['id']}/deploys?limit=100")
        live = [value.get('deploy', value) for value in deploys
                if value.get('deploy', value).get('status') == 'live']
        if len(live) != 1:
            raise ReleaseError('production service must have exactly one identifiable live deployment')
        revision = live[0].get('commit', {}).get('id', '')
        if not re.fullmatch(r'[0-9a-f]{40}', revision) or not public_ancestor(revision):
            raise ReleaseError('production service uses a revision outside public main')
        rows.append({'id': expected['id'], 'name': expected['name'], 'revision': revision})
    return rows


def main():
    subprocess.run(['git', 'fetch', '--quiet', PUBLIC + '.git', 'refs/heads/main'],
                   cwd=ROOT, check=True, timeout=120)
    manifest = json.loads((ROOT / 'ops/public-runtime-services.json').read_text())
    rows = audit(Render(os.environ.get('RENDER_API_KEY')), manifest,
                 lambda revision: subprocess.run(
                     ['git', 'merge-base', '--is-ancestor', revision, 'FETCH_HEAD'],
                     cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0)
    print(json.dumps({'repository': PUBLIC, 'services': rows}))


if __name__ == '__main__':
    main()
