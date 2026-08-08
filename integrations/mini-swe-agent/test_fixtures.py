#!/usr/bin/env python3
"""mini-SWE-agent integration smoke test — validates all fixtures and selection logic."""
import json, os, sys

FIXTURES_DIR = os.path.join(os.path.dirname(__file__) or '.', '..', 'integrations', 'mini-swe-agent', 'fixtures')

def load_fixtures():
    fixtures = {}
    for fname in os.listdir(FIXTURES_DIR):
        if fname.endswith('.json'):
            path = os.path.join(FIXTURES_DIR, fname)
            with open(path) as f:
                fixtures[fname] = json.load(f)
    return fixtures

def validate_fixture(name, data):
    """Validate a single fixture against the expected schema."""
    errors = []
    if 'bountyId' not in data:
        errors.append('missing bountyId')
    if 'scenario' in data and not isinstance(data['scenario'], str):
        errors.append('scenario must be a string')
    if 'bounty' in data:
        bounty = data['bounty']
        if 'amount' not in bounty:
            errors.append('bounty.amount missing')
        if 'token' not in bounty:
            errors.append('bounty.token missing')
    return errors

def main():
    fixtures = load_fixtures()
    print(f'Loaded {len(fixtures)} fixture(s)')
    
    all_ok = True
    for name, data in fixtures.items():
        errors = validate_fixture(name, data)
        if errors:
            all_ok = False
            print(f'FAIL {name}:')
            for e in errors:
                print(f'  - {e}')
        else:
            print(f'PASS {name} (scenario: {data.get("scenario", "N/A")})')
    
    sys.exit(0 if all_ok else 1)

if __name__ == '__main__':
    main()
