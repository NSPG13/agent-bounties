#!/usr/bin/env python3
"""Exercise three real images built sequentially with the same Docker cache.

Requires standby(A/revision1), reader(A/revision1), reader(B/revision2).
No provider calls, signing keys, production database, or exposed ports.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import tempfile
import time

A = 'srv-aaaaaaaaaaaaaaaaaaaa'
B = 'srv-bbbbbbbbbbbbbbbbbbbb'
R1 = '1' * 40
R2 = '2' * 40
BINARY = '/usr/local/bin/recovery-reader'


def docker(*args, timeout=15):
    return subprocess.run(['docker', *args], text=True, capture_output=True, timeout=timeout)


def container(image, environment, arguments, *, alive=False):
    with tempfile.TemporaryDirectory(prefix='recovery-image-control-') as folder:
        cidfile = Path(folder) / 'container.id'
        options = ['run', '--detach', '--cidfile', str(cidfile), '--network', 'none',
                   '--read-only', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges',
                   '--cpus', '0.5', '--memory', '256m', '--memory-swap', '256m', '--pids-limit', '64']
        for key, value in environment.items():options += ['--env', key + '=' + value]
        cid = None
        try:
            # Independent in-container lifetime also bounds an interrupted
            # launcher; this is fixture containment, not a production command.
            command = arguments or ['/usr/local/bin/run-recovery-indexers']
            result = docker(*options, image, '/usr/bin/timeout', '--signal=TERM',
                            '--kill-after=3s', '12s', *command)
            if cidfile.exists():cid = cidfile.read_text().strip()
            assert result.returncode == 0 and cid, result.stderr
            if alive:
                # Invalid dependencies would fail immediately if constructed.
                time.sleep(1)
                state = docker('inspect', '--format', '{{json .State}}', cid)
                assert state.returncode == 0 and json.loads(state.stdout)['Running'], state.stdout
                stopped = docker('stop', '--time', '3', cid)
                assert stopped.returncode == 0, stopped.stderr
            waited = docker('wait', cid)
            assert waited.returncode == 0, waited.stderr
            logs = docker('logs', cid)
            return int(waited.stdout.strip()), logs.stdout, logs.stderr
        finally:
            if cid is None and cidfile.exists():cid = cidfile.read_text().strip()
            if cid:
                assert re.fullmatch('[0-9a-f]{64}', cid), 'invalid owned fixture CID'
                removed = docker('rm', '--force', cid)
                assert removed.returncode == 0, 'owned image-control container cleanup failed'


def verify(standby, reader_a, reader_b):
    images = [(standby, A, R1, 'standby'), (reader_a, A, R1, 'reader'), (reader_b, B, R2, 'reader')]
    for image, identity, revision, capability in images:
        config = docker('image', 'inspect', '--format', '{{json .Config}}', image)
        assert config.returncode == 0, config.stderr
        config = json.loads(config.stdout)
        assert config.get('Entrypoint') is None
        assert config['Cmd'] == ['/usr/local/bin/run-recovery-indexers']
        assert config['User'] == 'app'
        defaults = dict(entry.split('=', 1) for entry in config['Env'])
        assert defaults['RECOVERY_READER_MODE'] == 'standby'
        assert all(defaults[key] == 'false' for key in [
            'ENABLE_HOSTED_FINANCIAL_SIGNING', 'ENABLE_BASE_TX_BROADCAST',
            'ENABLE_X402_HOSTED_RELAY', 'ENABLE_STRIPE_LIVE_EXECUTION'])
        code, out, err = container(image, {}, [BINARY, '--build-identity'])
        assert code == 0, err
        assert json.loads(out) == {'schema': 'agentizable/recovery-reader-build-v1',
            'binding': {'service_id': identity, 'source_revision': revision, 'capability': capability},
            'unbound_inert_only': False}
        matching = {'RENDER_SERVICE_ID': identity, 'RENDER_GIT_COMMIT': revision}
        code, _, err = container(image, {**matching, 'DATABASE_URL': 'invalid-fixture',
            'BASE_MAINNET_RPC_URL': 'invalid-fixture', 'ENABLE_HOSTED_FINANCIAL_SIGNING': 'true',
            'ENABLE_BASE_TX_BROADCAST': 'true', 'X402_RELAYER_PRIVATE_KEY': 'invalid-fixture'}, [], alive=True)
        assert code == 0, err
        for service, source in [(identity, revision), ('', revision), (identity, ''),
                                (B if identity == A else A, revision), (identity, R2 if revision == R1 else R1),
                                ('srv-d982ftu7r5hc73ar4st0', revision)]:
            environment = {'RENDER_SERVICE_ID': service, 'RENDER_GIT_COMMIT': source,
                'RECOVERY_READER_MODE': 'active',
                # These runtime overrides must not change baked identity/capability.
                'RECOVERY_READER_BOUND_SERVICE_ID': service,
                'RECOVERY_READER_BOUND_REVISION': source, 'RECOVERY_READER_BOUND_CAPABILITY': 'reader',
                'RECOVERY_READER_ARTIFACT_CAPABILITY': 'reader',
                'DATABASE_URL': 'invalid-fixture', 'BASE_MAINNET_RPC_URL': 'invalid-fixture',
                'ENABLE_HOSTED_FINANCIAL_SIGNING': 'true', 'X402_RELAYER_PRIVATE_KEY': 'invalid-fixture'}
            code, _, err = container(image, environment, [BINARY, '--check-binding'])
            expected = capability == 'reader' and service == identity and source == revision
            assert (code == 0) == expected, err
            if not expected:
                assert 'Recovery runtime identity' in err or 'Standby artifact' in err
        # The predeploy command must execute directly, independently of reader mode.
        code, _, err = container(image, matching, ['/usr/local/bin/check-recovery-revision', revision])
        assert code == 0, err
        code, _, _ = container(image, matching, ['/bin/sh', '-c', 'exit 23'])
        assert code == 23
    print('recovery_reader_images: immutable capability, cache A-to-B binding, runtime admission and predeploy controls passed')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--standby-image', required=True)
    parser.add_argument('--reader-a-image', required=True)
    parser.add_argument('--reader-b-image', required=True)
    args = parser.parse_args()
    verify(args.standby_image, args.reader_a_image, args.reader_b_image)
