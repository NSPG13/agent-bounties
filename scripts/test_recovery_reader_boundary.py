"""Local shell/process controls; Rust/database/image tests are separate gates."""
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
A = 'srv-aaaaaaaaaaaaaaaaaaaa'
REVISION = '1' * 40


class RecoveryBuildInputs(unittest.TestCase):
    def guard(self, *args):
        return subprocess.run(['bash', str(ROOT / 'scripts/check-recovery-reader-build.sh'), *args],
                              capture_output=True, timeout=3).returncode

    def test_only_new_well_formed_identity_revision_and_capability_are_accepted(self):
        for capability in ['standby', 'reader']:
            self.assertEqual(self.guard(A, REVISION, capability), 0)
        for identity in (ROOT / 'ops/recovery-reader-denied-services.txt').read_text().splitlines():
            self.assertEqual(self.guard(identity, REVISION, 'reader'), 1)
        for args in [[], [A, REVISION], [A, REVISION, 'reader', 'extra'],
                     ['', REVISION, 'reader'], ['srv-;exit 0', REVISION, 'reader'],
                     [A, '', 'reader'], [A, 'A' * 40, 'reader'], [A, REVISION, 'true']]:
            self.assertEqual(self.guard(*args), 2)


class RecoverySupervisor(unittest.TestCase):
    def run_fixture(self, *, mode='active', overrides=None, stop_parent=False, binding_ok=True):
        with tempfile.TemporaryDirectory(prefix='recovery-supervisor-') as folder:
            root = Path(folder)
            executable = root / 'reader'
            executable.write_text('''#!/usr/bin/python3
import json, os, signal, sys, time
from pathlib import Path
root = Path(FIXTURE_ROOT)
if sys.argv[1:] == ['--check-binding']:
    (root / 'binding.json').write_text(json.dumps(dict(os.environ)))
    sys.exit(BINDING_STATUS)
protocol = os.environ.get('BASE_INDEXER_PROTOCOL', 'standby')
(root / (protocol + '.json')).write_text(json.dumps({'pid': os.getpid(), 'env': dict(os.environ)}))
def stop(signum, frame):
    (root / (protocol + '.stopped')).write_text('terminated')
    sys.exit(0)
signal.signal(signal.SIGTERM, stop)
if EXIT_CHILD and protocol == 'autonomous-v1':
    time.sleep(0.3)
    sys.exit(17)
while True: time.sleep(0.05)
'''.replace('FIXTURE_ROOT', repr(folder)).replace('BINDING_STATUS', '0' if binding_ok else '19')
                .replace('EXIT_CHILD', repr(not stop_parent)))
            executable.chmod(0o700)
            supervisor = root / 'run.sh'
            supervisor.write_text((ROOT / 'scripts/run-recovery-indexers.sh').read_text()
                                  .replace('/usr/local/bin/recovery-reader', str(executable)))
            environment = {'PATH': '/usr/bin:/bin', 'RENDER_SERVICE_ID': A, 'RENDER_GIT_COMMIT': REVISION,
                'RECOVERY_READER_MODE': mode, 'DATABASE_URL': 'postgres://fixture-only',
                'BASE_MAINNET_RPC_URL': 'https://rpc.invalid',
                'BASE_INDEXER_BOOTSTRAP_ARCHIVE_RPC_URL': 'https://archive.invalid',
                'BASE_INDEXER_START_BLOCK': '42', 'BASE_INDEXER_MAX_BLOCKS_PER_QUERY': '500',
                'BASE_INDEXER_PROTOCOL': 'open-competition-v2-keeper',
                'BASE_INDEXER_FACTORY_CONTRACT': 'must-not-inherit',
                'POSTING_CREATOR_INDEXER_ENABLED': 'true', 'POSTING_V2_INDEXER_ENABLED': 'true',
                'CREATOR_OPEN_RELEASE_JSON': '{"network":"base-mainnet"}',
                'DISCOVERY_WEBHOOK_SIGNING_KEY': 'fixture-never-inherit',
                'GITHUB_APP_PRIVATE_KEY': 'fixture-never-inherit',
                'X402_RELAYER_PRIVATE_KEY': 'fixture-never-inherit',
                'ENABLE_HOSTED_FINANCIAL_SIGNING': 'true',
                'ENABLE_BASE_TX_BROADCAST': 'true', 'RESEND_API_KEY': 'fixture-never-inherit',
                'VERIFIER_EMAIL_ENABLED': 'true', 'CREATOR_PUSH_ENABLED': 'true'}
            for key, value in (overrides or {}).items():
                if value is None:environment.pop(key, None)
                else:environment[key] = value
            process = subprocess.Popen(['bash', str(supervisor)], env=environment,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
            try:
                if stop_parent:
                    expected = 1 if mode == 'standby' else 3
                    deadline = time.monotonic() + 3
                    while len([p for p in root.glob('*.json') if p.stem != 'binding']) < expected:
                        self.assertIsNone(process.poll())
                        self.assertLess(time.monotonic(), deadline)
                        time.sleep(0.01)
                    process.send_signal(signal.SIGTERM)
                process.communicate(timeout=5)
                records = {p.stem: json.loads(p.read_text()) for p in root.glob('*.json')}
                for name, record in records.items():
                    if name == 'binding':continue
                    with self.assertRaises(ProcessLookupError):os.kill(record['pid'], 0)
                return process.returncode, records, {p.stem for p in root.glob('*.stopped')}
            finally:
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.communicate(timeout=3)

    def test_three_readers_get_only_their_required_environment_and_siblings_stop(self):
        code, records, stopped = self.run_fixture()
        self.assertEqual(code, 17)
        self.assertEqual(set(records), {'binding', 'autonomous-v1', 'autonomous-v2', 'creator-open-v1'})
        self.assertEqual(stopped, {'autonomous-v2', 'creator-open-v1'})
        blocked = {'DISCOVERY_WEBHOOK_SIGNING_KEY', 'GITHUB_APP_PRIVATE_KEY', 'X402_RELAYER_PRIVATE_KEY',
                   'RESEND_API_KEY', 'VERIFIER_EMAIL_ENABLED', 'CREATOR_PUSH_ENABLED',
                   'ENABLE_HOSTED_FINANCIAL_SIGNING', 'ENABLE_BASE_TX_BROADCAST'}
        for name, record in records.items():
            environment = record if name == 'binding' else record['env']
            self.assertFalse(blocked.intersection(environment))
        v1, v2, creator = [records[key]['env'] for key in ['autonomous-v1', 'autonomous-v2', 'creator-open-v1']]
        self.assertEqual(v1['BASE_INDEXER_START_BLOCK'], '42')
        self.assertEqual(v1['BASE_INDEXER_FACTORY_CONTRACT'], '0x082c52131aaf0c56e76b075f895eab6fcab6d2f9')
        self.assertEqual(v2['BASE_INDEXER_FACTORY_CONTRACT'], '0xc33e2ae33bb9580837ea59df18e57fa1039ae58a')
        self.assertEqual(v2['BASE_INDEXER_START_BLOCK'], '52275492')
        for environment in [v2, creator]:
            self.assertNotIn('BASE_INDEXER_BOOTSTRAP_ARCHIVE_RPC_URL', environment)
            self.assertNotIn('BASE_INDEXER_MAX_BLOCKS_PER_QUERY', environment)

    def test_parent_term_stops_and_reaps_every_reader(self):
        code, records, stopped = self.run_fixture(stop_parent=True)
        self.assertEqual(code, 143)
        self.assertEqual(stopped, set(records) - {'binding'})

    def test_standby_inherits_no_database_rpc_or_signer_configuration(self):
        code, records, stopped = self.run_fixture(mode='standby', stop_parent=True)
        self.assertEqual(code, 0)
        self.assertEqual(set(records), {'standby'})
        self.assertEqual(stopped, {'standby'})
        self.assertFalse({'DATABASE_URL', 'BASE_MAINNET_RPC_URL', 'X402_RELAYER_PRIVATE_KEY'}
                         .intersection(records['standby']['env']))

    def test_failed_binding_or_configuration_starts_no_children(self):
        for options in [{'binding_ok': False}, {'mode': 'invalid'},
                        {'overrides': {'POSTING_V2_INDEXER_ENABLED': 'yes'}},
                        {'overrides': {'CREATOR_OPEN_RELEASE_JSON': None}},
                        {'overrides': {'BASE_INDEXER_NETWORK': 'base-sepolia'}},
                        {'overrides': {'DATABASE_URL': None}}]:
            with self.subTest(options=options):
                code, records, _ = self.run_fixture(**options)
                self.assertNotEqual(code, 0)
                self.assertFalse(set(records) - {'binding'})


if __name__ == '__main__':
    unittest.main()
