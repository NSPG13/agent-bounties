#!/usr/bin/env python3
"""Offline synthetic phase vectors; never a chain, signature or payment verifier."""
import hashlib
import json
from copy import deepcopy
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def canonical_bytes(value):
    """JCS-compatible only for this explicitly restricted ASCII/integer domain."""
    def validate(x):
        if x is None or isinstance(x, bool):
            return
        if isinstance(x, int) and abs(x) <= 9007199254740991:
            return
        if isinstance(x, str) and x.isascii():
            return
        if isinstance(x, list):
            for v in x:
                validate(v)
            return
        if isinstance(x, dict) and all(isinstance(k, str) and k.isascii() for k in x):
            for v in x.values():
                validate(v)
            return
        raise ValueError('outside the restricted ASCII / safe integer domain')
    validate(value)
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()


def evaluate_synthetic_observations(case):
    """Assumes canonicality adapters already supplied the mocked chain observations.

    Real callers must NOT supply these booleans to establish real-world facts.
    They are controlled test inputs, with no RPC, signature or anchoring verifier.
    """
    assert case['synthetic'] is True
    bind = case['binding']
    events = case['observations']['events']
    identities = [(x['chain'], x['contract'], x['tx_hash'], x['log_index']) for x in events]
    if len(identities) != len(set(identities)):
        return dict(funded=False, delivered=False, settled=False, completeness=False)

    def lifecycle_identity(value):
        if not isinstance(value, dict):
            return None
        parts = [value.get(key) for key in ('chain', 'contract', 'tx_hash')]
        index = value.get('log_index')
        if (not all(isinstance(part, str) and part and part == part.strip() for part in parts)
                or type(index) is not int or not 0 <= index <= 9007199254740991):
            return None
        return (*parts, index)

    lifecycle = lifecycle_identity(bind.get('lifecycle'))

    def same_lifecycle(observation):
        # The adapter verifies the relationship to this anchor, not merely that
        # the anchor exists. Reusable content hashes cannot supply this identity.
        return (lifecycle is not None
                and lifecycle[:2] == (bind['chain'], bind['contract'])
                and observation.get('lifecycle_verified') is True
                and lifecycle_identity(observation.get('lifecycle')) == lifecycle)

    def valid(x):
        return (x['canonical'] is True and x['transaction_status'] == 'success'
                and x['chain'] == bind['chain'] and x['contract'] == bind['contract']
                and x['asset'] == bind['asset'] and x['terms_hash'] == bind['terms_hash']
                and same_lifecycle(x))
    funded = any(valid(x) and x['event'] == 'FundingAdded' for x in events)
    delivery = case['observations']['delivery']
    delivered = (delivery is not None and delivery['verified'] is True
                 and same_lifecycle(delivery)
                 and delivery['terms_hash'] == bind['terms_hash']
                 and delivery['evidence_hash'] == bind['evidence_hash']
                 and delivery['verifier_result_hash'] == bind['verifier_result_hash'])
    settled = any(valid(x) and x['event'] == 'BountySettled'
                  and x['solver'] == bind['expected_solver']
                  and x['solver_amount'] == bind['expected_solver_amount'] for x in events)
    # Neither an issuer sequence nor existence anchoring proves no omissions.
    return dict(funded=funded, delivered=delivered, settled=settled, completeness=False)


def check_lifecycle_negative_controls():
    positive = json.loads((ROOT/'settled-expected-solver.json').read_text())
    for key, other in [('chain', 'eip155:84532'), ('contract', 'synthetic-other-contract'),
                       ('tx_hash', 'synthetic-other-occurrence'), ('log_index', 1)]:
        case = deepcopy(positive)
        case['observations']['delivery']['lifecycle'][key] = other
        assert evaluate_synthetic_observations(case) == dict(
            funded=True, delivered=False, settled=True, completeness=False), key

    for field, value in [('lifecycle', None), ('lifecycle_verified', False)]:
        for missing in (True, False):
            case = deepcopy(positive)
            delivery = case['observations']['delivery']
            if missing:
                delivery.pop(field)
            else:
                delivery[field] = value
            assert evaluate_synthetic_observations(case) == dict(
                funded=True, delivered=False, settled=True, completeness=False), field
            case = deepcopy(positive)
            for event in case['observations']['events']:
                if missing:
                    event.pop(field)
                else:
                    event[field] = value
            assert evaluate_synthetic_observations(case) == dict(
                funded=False, delivered=True, settled=False, completeness=False), field

    for invalid in (None, {}, 'claimed-lifecycle-tag',
                    dict(positive['binding']['lifecycle'], log_index=True),
                    dict(positive['binding']['lifecycle'], log_index=-1),
                    dict(positive['binding']['lifecycle'], tx_hash='')):
        case = deepcopy(positive)
        case['binding']['lifecycle'] = invalid
        for observation in case['observations']['events'] + [case['observations']['delivery']]:
            observation['lifecycle'] = invalid
        assert evaluate_synthetic_observations(case) == dict(
            funded=False, delivered=False, settled=False, completeness=False)


def main():
    manifest = json.loads((ROOT/'manifest.json').read_text())
    for entry in manifest['vectors']:
        path = ROOT/entry['file']
        data = path.read_bytes()
        case = json.loads(data)
        assert canonical_bytes(case) == data, f'noncanonical bytes: {path.name}'
        assert hashlib.sha256(data).hexdigest() == entry['sha256'], path.name
        assert evaluate_synthetic_observations(case) == case['expected'], path.name
        tampered = json.loads(data)
        tampered['binding']['expected_solver'] = 'synthetic-other-solver'
        assert hashlib.sha256(canonical_bytes(tampered)).hexdigest() != entry['sha256']
    check_lifecycle_negative_controls()
    print(f"synthetic_phase_vectors=ok count={len(manifest['vectors'])}; no live evidence verified")


if __name__ == '__main__':
    main()
