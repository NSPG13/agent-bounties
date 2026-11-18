#!/usr/bin/env python3
"""Pure, once-bound inventory substitution; never creates or deploys a service.

The binding is a nonsecret input supplied by the separately admitted recovery
procedure after checking its native create receipt. JSON is not an attestation
of service creation or authorization. This tool has no provider client.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / 'scripts/fixtures/recovery-runtime-inventory-base.json'
BASE_SHA256 = '48917b98de94ce7ff838159f5c285e40f95cf4c8588a914d4f9cdb90d75a0c75'
WORKSPACE = 'tea-d4tk87f5r7bs73au55lg'
API_ID = 'srv-dbp7j78m7kps73a35vm0'
READER_NAME = 'agentizable-readers-recovery-20261110'
HOLD = {
    'schema': 'agentizable/public-runtime-transition-hold-v1',
    'case_id': 'reader-api-recovery-20261110',
    'reason': 'Separate recovery admission is required before public runtime deployment.',
}
FIXED_BINDING = {
    'schema': 'agentizable/recovery-reader-native-binding-v1',
    'workspace_id': WORKSPACE,
    'reader_name': READER_NAME,
    'type': 'background_worker',
    'region': 'oregon',
}
ALLOWED_PATHS = {
    '/services/0/id', '/services/0/name', '/services/0/dockerfile',
    '/services/2/id', '/services/2/name', '/services/2/dockerfile',
}


class InventoryError(ValueError):
    pass


def changed_paths(left, right, path=''):
    if type(left) is not type(right):
        return {path}
    if isinstance(left, dict):
        if left.keys() != right.keys():
            return {path}
        return set().union(*(changed_paths(left[k], right[k], path + '/' + k) for k in left))
    if isinstance(left, list):
        if len(left) != len(right):
            return {path}
        return set().union(*(changed_paths(a, b, path + '/' + str(i)) for i, (a, b) in enumerate(zip(left, right))))
    return set() if left == right else {path}


def materialize(base_bytes, binding):
    if hashlib.sha256(base_bytes).hexdigest() != BASE_SHA256:
        raise InventoryError('recovery_inventory_base_changed')
    base = json.loads(base_bytes)
    if base.get('transition_hold') != HOLD or base.get('workspace_id') != WORKSPACE:
        raise InventoryError('recovery_inventory_hold_or_workspace_changed')
    if (not isinstance(binding, dict)
            or binding.keys() != (FIXED_BINDING.keys() | {'reader_service_id'})
            or any(binding.get(key) != value for key, value in FIXED_BINDING.items())):
        raise InventoryError('recovery_native_binding_changed')
    identity = binding['reader_service_id']
    old_ids = {row['id'] for row in base['services']} | {API_ID}
    if (not isinstance(identity, str) or not re.fullmatch(r'srv-[a-z0-9]{20}', identity)
            or identity in old_ids):
        raise InventoryError('recovery_reader_identity_rejected')
    result = copy.deepcopy(base)
    result['services'][0].update(id=API_ID, name='agentizable-api-recovery-20261109',
                                 dockerfile='./Dockerfile.recovery-api')
    result['services'][2].update(id=identity, name=READER_NAME,
                                 dockerfile='./Dockerfile.recovery-indexers')
    if changed_paths(base, result) != ALLOWED_PATHS:
        raise InventoryError('recovery_inventory_substitution_changed')
    return result


def validate_materialized(candidate, base_bytes, binding):
    expected = materialize(base_bytes, binding)
    # Type-sensitive recursive comparison also rejects false/zero substitutions.
    if changed_paths(expected, candidate):
        raise InventoryError('recovery_inventory_candidate_changed')
    return expected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', type=Path, default=BASE)
    parser.add_argument('--binding', type=Path, required=True)
    destination = parser.add_mutually_exclusive_group(required=True)
    destination.add_argument('--output', type=Path)
    destination.add_argument('--validate', type=Path)
    args = parser.parse_args()
    try:
        base = args.base.read_bytes()
        binding = json.loads(args.binding.read_text())
        if args.validate:
            validate_materialized(json.loads(args.validate.read_text()), base, binding)
        else:
            value = materialize(base, binding)
            with args.output.open('x') as output:
                output.write(json.dumps(value, indent=2) + '\n')
    except (OSError, UnicodeError, ValueError):
        raise SystemExit('recovery_inventory_materialization_rejected') from None


if __name__ == '__main__':
    main()
