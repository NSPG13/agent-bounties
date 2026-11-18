"""Synthetic std-only process checks; never start the financial worker.

The fixture copy changes only the fixed worker path and include_str location.
Actual shipping-image identity and file hashes require separate image checks.
No Cargo, network, database, real key or container is used here.
"""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'crates/worker/src/bin/managed-funding-launcher.rs'
SERVICE = 'srv-aaaaaaaaaaaaaaaaaaaa'
REVISION = '1' * 40
PRIVATE_SLOT = 'MANAGED_BOUNTY_FUNDING_DELEGATE_PRIVATE_KEY'
KEY = '0' * 63 + '1'  # Synthetic scalar only; never a funded wallet.
PINS = {
    'BASE_MAINNET_BOUNTY_V2_FACTORY': '0xc33e2ae33bb9580837ea59df18e57fa1039ae58a',
    'BASE_MAINNET_BOUNTY_V2_IMPLEMENTATION': '0x8420c9bd1ff8a6b1abc4230b349538612c58f3a6',
    'BASE_MAINNET_BOUNTY_V2_PLATFORM_FEE_BPS': '750',
    'BASE_MAINNET_BOUNTY_V2_PLATFORM_FEE_RECIPIENT': '0x884834e884d6e93462655a2820140ad03e6747bc',
}


class FundingLauncherProcessTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix='funding-launcher-fixture-')
        cls.addClassCleanup(cls.temporary.cleanup)
        cls.folder = Path(cls.temporary.name)
        cls.report = cls.folder / 'stub-report'
        cls.stub = cls.folder / 'worker-stub'
        cls.unbound = cls.folder / 'unbound'
        cls.bound = cls.folder / 'bound'
        cls.unit_tests = cls.folder / 'unit-tests'
        # Fixed source assertions prevent a fixture rewrite hiding a production
        # path override. This copy is never installed or published as an image.
        text = SOURCE.read_text()
        fixed = 'const WORKER: &str = "/usr/local/bin/managed-funding-worker";'
        included = 'include_str!("../../../../ops/recovery-reader-denied-services.txt")'
        assert text.count(fixed) == text.count(included) == 1
        text = text.replace(fixed, 'const WORKER: &str = ' + json.dumps(str(cls.stub)) + ';')
        text = text.replace(included, 'include_str!(' + json.dumps(str(ROOT / 'ops/recovery-reader-denied-services.txt')) + ')')
        fixture = cls.folder / 'launcher.rs'
        fixture.write_text(text)
        stub = cls.folder / 'stub.rs'
        stub.write_text('''use std::{env,fs,process,thread,time::Duration};
fn main() {
    let mut names: Vec<_> = env::vars_os().map(|(name,_)| name.into_string().unwrap()).collect();
    names.sort();
    let report = format!("pid={}\\nargs={}\\nkeys={}\\n", process::id(), env::args_os().count(), names.join(","));
    fs::write(REPORT, report).unwrap();
    loop { thread::park_timeout(Duration::from_secs(60)); }
}
const REPORT: &str = ''' + json.dumps(str(cls.report)) + ';\n')
        compiler = os.environ.get('AGENT_BOUNTIES_TEST_RUSTC', 'rustc')
        build_env = {'PATH': '/usr/bin:/bin'}
        bound_env = dict(build_env, RECOVERY_FUNDING_BOUND_SERVICE_ID=SERVICE,
                         RECOVERY_FUNDING_BOUND_REVISION=REVISION)
        for source, output, extra, env in [
            (stub, cls.stub, [], build_env),
            (fixture, cls.unbound, [], build_env),
            (fixture, cls.bound, [], bound_env),
            (SOURCE, cls.unit_tests, ['--test'], build_env),
        ]:
            result = subprocess.run([compiler, '--edition=2021', '-C', 'debuginfo=0',
                                     '-C', 'codegen-units=1', '-C', 'link-arg=-Wl,--threads=1',
                                     *extra, str(source), '-o', str(output)],
                                    env=env, capture_output=True, text=True, timeout=25)
            if result.returncode:
                raise AssertionError('std-only fixture compilation failed: ' + result.stderr)

    def setUp(self):
        self.report.unlink(missing_ok=True)

    def settings(self):
        return dict(PINS, RENDER_SERVICE_ID=SERVICE, RENDER_GIT_COMMIT=REVISION,
                    RECOVERY_FUNDING_MODE='active', ENABLE_HOSTED_FINANCIAL_SIGNING='true',
                    BASE_INDEXER_PROTOCOL='managed-bounty-funding',
                    DATABASE_URL='synthetic-database-slot', BASE_MAINNET_RPC_URL='synthetic-rpc-slot',
                    MANAGED_BOUNTY_FUNDING='synthetic-configuration-slot',
                    MANAGED_BOUNTY_FUNDING_DELEGATE_PRIVATE_KEY=KEY)

    def invoke(self, binary, env, args=()):
        result = subprocess.run([str(binary), *args], env=env, capture_output=True,
                                text=True, timeout=2)
        self.assertNotIn(KEY, result.stdout + result.stderr)
        return result

    def parked(self, binary, env):
        process = subprocess.Popen([str(binary)], env=env, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE)
        def cleanup():
            if process.poll() is None:
                process.terminate()
            try:
                out, err = process.communicate(timeout=2)
            except subprocess.TimeoutExpired:
                process.kill()
                out, err = process.communicate(timeout=2)
                self.fail('fixture ignored termination')
            self.assertNotIn(KEY.encode(), out + err)
        self.addCleanup(cleanup)
        return process

    def test_original_std_unit_tests_pass(self):
        result = self.invoke(self.unit_tests, {}, ['--test-threads=1'])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('7 passed; 0 failed', result.stdout)

    def test_build_identity_is_public_and_never_uses_runtime_settings(self):
        env = {PRIVATE_SLOT: KEY, 'DATABASE_URL': 'must-not-be-read',
               'RECOVERY_FUNDING_MODE': 'invalid'}
        for binary, expected in [(self.bound, {'service_id': SERVICE, 'source_revision': REVISION,
                                             'capability': 'funding'}), (self.unbound, None)]:
            result = self.invoke(binary, env, ['--build-identity'])
            self.assertEqual((result.returncode, result.stderr), (0, ''))
            self.assertEqual(json.loads(result.stdout), {
                'schema': 'agentizable/recovery-funding-build-v1', 'binding': expected,
                'unbound_inert_only': expected is None})
            self.assertFalse(self.report.exists())

    def test_held_bound_and_unbound_processes_start_no_child_or_dependency(self):
        for binary in [self.bound, self.unbound]:
            with self.subTest(binary=binary.name):
                env = self.settings()
                env.update(RECOVERY_FUNDING_MODE='held', ENABLE_HOSTED_FINANCIAL_SIGNING='false')
                env[PRIVATE_SLOT] = 'deliberately-invalid-sensitive-slot'
                process = self.parked(binary, env)
                time.sleep(.1)
                self.assertIsNone(process.poll())
                self.assertEqual(Path(f'/proc/{process.pid}/task/{process.pid}/children').read_text().strip(), '')
                self.assertFalse(self.report.exists())
                process.terminate()
                process.wait(timeout=2)

    def test_active_exec_preserves_pid_and_exposes_only_exact_environment(self):
        env = self.settings()
        env.update(STRIPE_SECRET_KEY='must-not-inherit', AWS_ACCESS_KEY_ID='must-not-inherit',
                   UNRELATED_SECRET='must-not-inherit', RUST_LOG='trace', PATH='/must-not-inherit')
        process = self.parked(self.bound, env)
        deadline = time.monotonic() + 2
        while True:
            self.assertIsNone(process.poll())
            report_text = self.report.read_text() if self.report.exists() else ''
            if report_text.endswith('\n') and '\nkeys=' in report_text:
                break
            self.assertLess(time.monotonic(), deadline)
            time.sleep(.01)
        report = dict(line.split('=', 1) for line in report_text.splitlines())
        self.assertEqual(int(report['pid']), process.pid, 'launcher must exec, not spawn a child')
        self.assertEqual(report['args'], '1')
        expected = self.settings()
        expected['RUST_LOG'] = 'info'
        self.assertEqual(len(expected), 14)
        self.assertEqual(set(report['keys'].split(',')), set(expected))
        raw = Path(f'/proc/{process.pid}/environ').read_bytes()
        actual = dict(entry.decode().split('=', 1) for entry in raw.split(b'\0') if entry)
        self.assertEqual(set(actual), set(expected))
        # Do not render values if a comparison fails, including the synthetic key.
        self.assertTrue(all(actual[name] == value for name, value in expected.items()))
        argv = Path(f'/proc/{process.pid}/cmdline').read_bytes().split(b'\0')
        self.assertEqual(argv, [str(self.stub).encode(), b''])
        self.assertNotIn(KEY.encode(), b' '.join(argv))
        self.assertEqual(Path(f'/proc/{process.pid}/task/{process.pid}/children').read_text().strip(), '')

    def test_unadmitted_modes_identities_and_global_flags_never_start_worker(self):
        cases = [('RECOVERY_FUNDING_MODE', 'invalid'), ('RENDER_SERVICE_ID', 'srv-bbbbbbbbbbbbbbbbbbbb'),
                 ('RENDER_GIT_COMMIT', '2' * 40), ('ENABLE_HOSTED_FINANCIAL_SIGNING', 'false'),
                 ('ENABLE_HOSTED_FINANCIAL_SIGNING', 'TRUE'), ('BASE_INDEXER_PROTOCOL', 'autonomous-v1'),
                 ('BASE_MAINNET_BOUNTY_V2_FACTORY', '0x' + '9' * 40), (PRIVATE_SLOT, 'not-a-private-key')]
        for name, value in cases:
            with self.subTest(name=name):
                env = self.settings(); env[name] = value
                result = self.invoke(self.bound, env)
                self.assertEqual(result.returncode, 1)
                self.assertEqual(result.stdout, '')
                self.assertFalse(self.report.exists())
        result = self.invoke(self.unbound, self.settings())
        self.assertEqual(result.returncode, 1)
        self.assertFalse(self.report.exists())

    def test_arguments_are_fixed_and_binding_probe_never_executes_worker(self):
        for args in [['--once'], ['--check-binding', 'extra'], ['--build-identity', 'extra']]:
            result = self.invoke(self.bound, self.settings(), args)
            self.assertEqual((result.returncode, result.stdout), (1, ''))
            self.assertFalse(self.report.exists())
        result = self.invoke(self.bound, self.settings(), ['--check-binding'])
        self.assertEqual((result.returncode, result.stdout, result.stderr), (0, '', ''))
        self.assertFalse(self.report.exists())


class FundingRecipeTests(unittest.TestCase):
    def test_build_guard_requires_new_service_and_full_revision(self):
        script = ROOT / 'scripts/check-recovery-funding-build.sh'
        denied = (ROOT / 'ops/recovery-reader-denied-services.txt').read_text().splitlines()
        for args, code in [([SERVICE, REVISION], 0), ([], 2), ([SERVICE, REVISION, 'extra'], 2),
                           (['invalid', REVISION], 2), ([SERVICE, 'not-a-revision'], 2),
                           *[([service, REVISION], 1) for service in denied]]:
            result = subprocess.run(['/bin/bash', str(script), *args], env={'PATH': '/usr/bin:/bin'},
                                    capture_output=True, text=True, timeout=2)
            self.assertEqual(result.returncode, code)
            self.assertEqual(result.stdout, '')

    def test_recipe_binds_only_public_identity_and_defaults_to_held(self):
        recipe = (ROOT / 'Dockerfile.recovery-funding').read_text()
        lines = [line.strip() for line in recipe.splitlines() if not line.lstrip().startswith('#')]
        self.assertEqual([line for line in lines if line.startswith('ARG ')],
                         ['ARG RENDER_SERVICE_ID', 'ARG RENDER_GIT_COMMIT'])
        self.assertFalse(any(line.startswith('ENTRYPOINT') for line in lines))
        self.assertIn('ENV RECOVERY_FUNDING_MODE=held \\', recipe)
        self.assertIn('ENABLE_HOSTED_FINANCIAL_SIGNING=false', recipe)
        self.assertIn('USER app', lines)
        self.assertIn('CMD ["/usr/local/bin/managed-funding-launcher"]', lines)
        self.assertIn('--bin worker --bin managed-funding-launcher', recipe)
        for name in ['managed-funding-worker', 'managed-funding-launcher', 'check-recovery-revision']:
            self.assertIn('/usr/local/bin/' + name, recipe)
        self.assertIn('command.env_clear();', SOURCE.read_text())
        self.assertIn('const WORKER: &str = "/usr/local/bin/managed-funding-worker";', SOURCE.read_text())


if __name__ == '__main__':
    unittest.main()
