"""Finite CI cleanup of exactly this nonce's fixture containers; no production I/O."""
import json
import re
import subprocess
import sys
import time
from pathlib import Path

out, nonce, deadline, config = Path(sys.argv[1]), sys.argv[2], float(sys.argv[3]), sys.argv[4]
assert re.fullmatch('[0-9a-f]{48}', nonce)
env = {'PATH': '/usr/bin:/bin', 'DOCKER_CONFIG': config, 'GOMAXPROCS': '2'}


def run(args):
    return subprocess.run(args, env=env, capture_output=True, text=True, timeout=10, check=True)


while time.monotonic() < deadline:
    if (out / 'watchdog-cancel.json').exists():
        raise SystemExit(0)
    time.sleep(.2)
errors, removed = [], []
try:
    ids = run(['docker', 'ps', '-aq', '--no-trunc', '--filter', 'label=ab.recovery.owner=' + nonce]).stdout.splitlines()
    assert len(ids) <= 4
    for cid in ids:
        try:
            assert re.fullmatch('[0-9a-f]{64}', cid)
            obj = json.loads(run(['docker', 'inspect', cid]).stdout)[0]
            assert obj['Id'] == cid and obj['Config']['Labels'].get('ab.recovery.owner') == nonce
            assert obj['Config']['Labels'].get('ab.recovery.role') in ('funding-image-fixture', 'funding-native-fixture')
            run(['docker', 'rm', '--force', '--volumes', cid])
            removed.append(cid)
        except Exception as error:
            errors.append(str(error)[:300])
except Exception as error:
    errors.append(str(error)[:300])
with (out / 'watchdog-fired.json').open('x') as target:
    json.dump({'removed': removed, 'errors': errors}, target)
raise SystemExit(1 if errors else 0)
