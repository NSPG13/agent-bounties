#!/usr/bin/env python3
"""Offline synthetic phase vectors; never a chain, signature or payment verifier."""
import hashlib
import json
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
    def valid(x):
        return (x['canonical'] is True and x['transaction_status'] == 'success'
                and x['chain'] == bind['chain'] and x['contract'] == bind['contract']
                and x['asset'] == bind['asset'] and x['terms_hash'] == bind['terms_hash'])
    funded = any(valid(x) and x['event'] == 'FundingAdded' for x in events)
    delivery = case['observations']['delivery']
    delivered = (delivery is not None and delivery['verified'] is True
                 and delivery['terms_hash'] == bind['terms_hash']
                 and delivery['evidence_hash'] == bind['evidence_hash']
                 and delivery['verifier_result_hash'] == bind['verifier_result_hash'])
    settled = any(valid(x) and x['event'] == 'BountySettled'
                  and x['solver'] == bind['expected_solver']
                  and x['solver_amount'] == bind['expected_solver_amount'] for x in events)
    # Neither an issuer sequence nor existence anchoring proves no omissions.
    return dict(funded=funded, delivered=delivered, settled=settled, completeness=False)


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
    print(f"synthetic_phase_vectors=ok count={len(manifest['vectors'])}; no live evidence verified")


if __name__ == '__main__':
    main()
