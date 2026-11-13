#!/usr/bin/env python3
"""Offline, exact funding process policy. Credentials are markers only."""
import json
from pathlib import Path
import re
import sys

V2 = {
    'BASE_MAINNET_BOUNTY_V2_FACTORY': '0xc33e2ae33bb9580837ea59df18e57fa1039ae58a',
    'BASE_MAINNET_BOUNTY_V2_IMPLEMENTATION': '0x8420c9bd1ff8a6b1abc4230b349538612c58f3a6',
    'BASE_MAINNET_BOUNTY_V2_PLATFORM_FEE_BPS': '750',
    'BASE_MAINNET_BOUNTY_V2_PLATFORM_FEE_RECIPIENT': '0x884834e884d6e93462655a2820140ad03e6747bc',
}
SLOTS = {'DATABASE_URL': '@present', 'BASE_MAINNET_RPC_URL': '@present',
         'MANAGED_BOUNTY_FUNDING_DELEGATE_PRIVATE_KEY': '@private_scalar'}


def check(condition, label):
    if not condition:
        raise ValueError(label)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        check(key not in result, 'duplicate JSON key')
        result[key] = value
    return result


def funding_settings(raw):
    check(isinstance(raw, str) and 0 < len(raw) <= 4096, 'funding setting type/limit')
    config = json.loads(raw, object_pairs_hook=unique_object)
    required = {'network', 'funding_wallet', 'min_paid_age_seconds', 'deadline_margin_seconds',
                'max_gas', 'max_fee_per_gas_wei'}
    check(isinstance(config, dict) and required <= set(config)
          and set(config) <= required | {'lease_seconds', 'poll_seconds'}, 'funding setting keys')
    check(config['network'] == 'base-mainnet', 'funding network')
    address = config['funding_wallet']
    check(isinstance(address, str) and re.fullmatch('0x[0-9a-f]{40}', address)
          and int(address[2:], 16) > 0, 'funding wallet')
    for key, minimum, maximum, default in [
        ('min_paid_age_seconds', 0, 30 * 86400, None),
        ('deadline_margin_seconds', 600, 7 * 86400, None),
        ('max_gas', 100000, 3000000, None),
        ('max_fee_per_gas_wei', 1, 1000000000000, None),
        ('lease_seconds', 90, 600, 180), ('poll_seconds', 5, 300, 30),
    ]:
        value = config.get(key, default)
        check(type(value) is int and minimum <= value <= maximum, 'funding numeric bound')
    return raw  # Preserve the exact selected public JSON bytes, including key order.


def compile_policy(data):
    check(isinstance(data, dict) and set(data) == {
        'schema', 'profile', 'service_id', 'revision', 'branch', 'service_name', 'service_type', 'settings'
    }, 'policy schema keys')
    check(data['schema'] == 'agentizable/funding-process-inspection-policy-v1', 'policy schema')
    check(data['profile'] in ('funding-held', 'funding-active'), 'funding profile')
    check(isinstance(data['service_id'], str) and re.fullmatch('srv-[a-z0-9]{20}', data['service_id']), 'service id')
    check(isinstance(data['revision'], str) and re.fullmatch('[0-9a-f]{40}', data['revision']), 'revision')
    check(data['branch'] in ('main', 'codex/clean-service-recovery-20261110'), 'branch')
    check(isinstance(data['service_name'], str) and re.fullmatch('[a-z0-9][a-z0-9-]{0,99}', data['service_name']), 'service name')
    check(data['service_type'] in ('worker', 'background_worker'), 'service type')
    active = data['profile'] == 'funding-active'
    settings = data['settings']
    check(isinstance(settings, dict), 'settings object')
    complete = set(V2) | set(SLOTS) | {'MANAGED_BOUNTY_FUNDING'}
    check(set(settings) == complete or (not active and not settings), 'funding bundle incomplete or unapproved')
    for key, value in settings.items():
        if key in V2: check(value == V2[key], 'noncanonical V2 setting')
        elif key in SLOTS: check(value == SLOTS[key], 'credential values forbidden')
        else: funding_settings(value)
    values = dict(RENDER_SERVICE_ID=data['service_id'], RENDER_GIT_COMMIT=data['revision'],
                  RECOVERY_FUNDING_MODE='active' if active else 'held',
                  ENABLE_HOSTED_FINANCIAL_SIGNING='true' if active else 'false', RUST_LOG='info')
    if settings:
        values.update(settings, BASE_INDEXER_PROTOCOL='managed-bounty-funding')
    if not active:
        values.update(RENDER_GIT_BRANCH=data['branch'], RENDER_GIT_REPO_SLUG='NSPG13/agent-bounties',
                      RENDER_SERVICE_TYPE=data['service_type'], RENDER='true', IS_PULL_REQUEST='false',
                      RENDER_SERVICE_NAME=data['service_name'])
    role = 'worker' if active else 'parent'
    rows = []
    for key, value in sorted(values.items()):
        check(isinstance(value, str) and not any(c in value for c in '\x00\t\r\n'), 'multiline policy value')
        rows.append('\t'.join([role, key, value]))
    return '\n'.join(rows) + '\n'


if __name__ == '__main__':
    check(len(sys.argv) == 3, 'input and new output paths required')
    check(Path(sys.argv[1]).stat().st_size <= 131072, 'policy size')
    result = compile_policy(json.loads(Path(sys.argv[1]).read_text(), object_pairs_hook=unique_object))
    with Path(sys.argv[2]).open('x') as output:
        output.write(result)
