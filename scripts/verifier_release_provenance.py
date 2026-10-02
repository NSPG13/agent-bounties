#!/usr/bin/env python3
"""Authorize release data from the protected workflow checkout, before release code.

This module never imports or executes a helper from the selected release. Approval
records are read from Git objects at the workflow's exact protected commit.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

FIELDS = {'schema', 'release_id', 'source_revision', 'network', 'worker_build_digest',
          'worker_binary_digest', 'signing_runtime_digest', 'profile_registry_digest', 'signers'}
REVISION = re.compile(r'^[0-9a-f]{40}$')
DIGEST = re.compile(r'^sha256:[0-9a-f]{64}$')


def object_without_duplicates(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate release field')
        result[key] = value
    return result


def parse_release(text: str) -> dict:
    if len(text.encode('utf-8')) > 16384:
        raise ValueError('release manifest exceeds limit')
    value = json.loads(text, object_pairs_hook=object_without_duplicates)
    if not isinstance(value, dict) or set(value) != FIELDS:
        raise ValueError('invalid release fields')
    if value['schema'] != 'agent-bounties/verifier-release-v1' or value['network'] != 'base-mainnet':
        raise ValueError('unsupported release identity')
    if not isinstance(value['source_revision'], str) or not REVISION.fullmatch(value['source_revision']):
        raise ValueError('release must identify an exact commit')
    for key in ('release_id', 'worker_build_digest', 'worker_binary_digest', 'signing_runtime_digest', 'profile_registry_digest'):
        if not isinstance(value[key], str) or not DIGEST.fullmatch(value[key]):
            raise ValueError('invalid release digest')
    signers = value['signers']
    if not isinstance(signers, list) or len(signers) != 2 or any(not isinstance(s, str) or not re.fullmatch(r'0x[0-9a-f]{40}', s) for s in signers) or len(set(signers)) != 2:
        raise ValueError('mainnet release requires two distinct signers')
    body = {k: v for k, v in value.items() if k != 'release_id'}
    identity = 'sha256:' + hashlib.sha256(json.dumps(body, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()).hexdigest()
    if identity != value['release_id']:
        raise ValueError('release identity mismatch')
    return value


def git(root: Path, *args: str) -> str:
    environment = dict(os.environ, GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')
    return subprocess.check_output(['git', '--no-replace-objects', '-C', str(root), *args],
                                   env=environment, text=True, stderr=subprocess.PIPE).strip()


def protected_blob(root: Path, trusted_revision: str, path: str) -> str:
    entry = git(root, 'ls-tree', trusted_revision, '--', path)
    if not entry.startswith('100644 blob '):
        raise ValueError('release has no regular approval record in protected history')
    return git(root, 'show', trusted_revision + ':' + path)


def authorize(root: Path, trusted_revision: str, manifest: str, expected_revision: str = '') -> dict:
    if not REVISION.fullmatch(trusted_revision):
        raise ValueError('trusted workflow revision is not exact')
    if git(root, 'rev-parse', 'HEAD') != trusted_revision:
        raise ValueError('not the protected workflow checkout')
    # This is a fixed remote ref populated by the protected checkout, not a variable.
    git(root, 'merge-base', '--is-ancestor', trusted_revision, 'refs/remotes/origin/main')
    value = parse_release(manifest)
    revision = value['source_revision']
    if expected_revision and revision != expected_revision:
        raise ValueError('release changed between authorization and execution')
    approval = '.github/verifier-releases/' + value['release_id'].removeprefix('sha256:') + '.json'
    approved = parse_release(protected_blob(root, trusted_revision, approval))
    if approved != value:
        raise ValueError('release differs from protected approval record')
    try:
        git(root, 'merge-base', '--is-ancestor', revision, trusted_revision)
    except subprocess.CalledProcessError:
        # A squash merge preserves the reviewed tree but not the branch commit.
        # Only an explicit record in protected history may bind that exact release
        # to an equivalent main commit; an arbitrary configuration cannot do so.
        source = json.loads(protected_blob(root, trusted_revision, approval.removesuffix('.json') + '.source.json'),
                            object_pairs_hook=object_without_duplicates)
        if not isinstance(source, dict) or set(source) != {'schema', 'release_id', 'protected_source_revision'} or source['schema'] != 'agent-bounties/protected-verifier-source-v1' or source['release_id'] != value['release_id'] or not isinstance(source['protected_source_revision'], str) or not REVISION.fullmatch(source['protected_source_revision']):
            raise ValueError('invalid protected source binding')
        protected = source['protected_source_revision']
        git(root, 'merge-base', '--is-ancestor', protected, trusted_revision)
        if git(root, 'rev-parse', revision + '^{tree}') != git(root, 'rev-parse', protected + '^{tree}'):
            raise ValueError('release tree differs from protected source')
    return value


def main() -> None:
    if os.environ.get('GITHUB_REPOSITORY') != 'NSPG13/agent-bounties' or os.environ.get('GITHUB_REF_PROTECTED') != 'true':
        raise ValueError('release execution requires the protected canonical repository')
    if os.environ.get('GITHUB_REF') != 'refs/heads/main' or not os.environ.get('GITHUB_WORKFLOW_REF', '').endswith('@refs/heads/main'):
        raise ValueError('release execution requires a main-branch workflow')
    value = authorize(Path(__file__).resolve().parents[1], os.environ['GITHUB_WORKFLOW_SHA'],
                      os.environ['REGRESSION_VERIFIER_RELEASE_JSON'], os.environ.get('EXPECTED_RELEASE_REVISION', ''))
    with open(os.environ['GITHUB_OUTPUT'], 'a', encoding='utf-8') as output:
        output.write(f"revision={value['source_revision']}\nrelease_id={value['release_id']}\n")
    print('verifier_release_provenance=ok')


if __name__ == '__main__':
    main()
