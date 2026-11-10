import contextlib
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('required_postgres', ROOT / 'scripts/run-required-postgres-test.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def result(stdout='', code=0):
    return subprocess.CompletedProcess([], code, stdout=stdout, stderr='')


class RequiredPostgresTests(unittest.TestCase):
    def run_case(self, responses, **kwargs):
        with patch.object(runner.subprocess, 'run', side_effect=responses) as calls, contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            code = runner.run_required('api', 'tests::selected', **kwargs)
        return code, calls

    def test_absent_selector_fails_before_execution_even_with_cargo_success(self):
        code, calls = self.run_case([result('0 tests, 0 benchmarks\n')], exact=True)
        self.assertEqual(code, 1)
        self.assertEqual(calls.call_count, 1)

    def test_real_selection_and_nonzero_result_pass(self):
        code, calls = self.run_case([result('tests::selected: test\n'), result('test result: ok. 1 passed; 0 failed; 0 ignored;\n')], exact=True, test_threads=1)
        self.assertEqual(code, 0)
        self.assertIn('--exact', calls.call_args_list[0].args[0])
        self.assertIn('--ignored', calls.call_args_list[0].args[0])
        self.assertIn('--list', calls.call_args_list[0].args[0])
        self.assertNotIn('--list', calls.call_args_list[1].args[0])
        self.assertEqual(calls.call_args_list[1].args[0][-2:], ['--test-threads', '1'])

    def test_zero_executed_tests_cannot_pass_after_a_valid_list(self):
        code, _ = self.run_case([result('tests::selected: test\n'), result('test result: ok. 0 passed; 0 failed; 1 ignored;\n')])
        self.assertEqual(code, 1)

    def test_each_selected_test_must_run(self):
        code, _ = self.run_case([result('first: test\nsecond: test\n'), result('test result: ok. 1 passed; 0 failed; 1 ignored;\n')])
        self.assertEqual(code, 1)

    def test_compile_failure_and_test_failure_propagate(self):
        for responses, expected in [([result(code=101)], 101), ([result('tests::selected: test\n'), result(code=101)], 101)]:
            code, _ = self.run_case(responses)
            self.assertEqual(code, expected)

    def test_unrecognized_success_output_is_not_evidence(self):
        code, _ = self.run_case([result('tests::selected: test\n'), result('all good\n')])
        self.assertEqual(code, 1)

    def test_cli_selects_native_then_exe_and_preserves_explicit_override(self):
        cases = [
            ({"cargo": "/native/cargo", "cargo.exe": "/windows/cargo.exe"}, {}, "/native/cargo"),
            ({"cargo.exe": "/windows/cargo.exe"}, {}, "/windows/cargo.exe"),
            ({"cargo": "/native/cargo"}, {"CARGO": "/custom tools/cargo.exe"}, "/custom tools/cargo.exe"),
        ]
        for available, environment, expected in cases:
            with self.subTest(expected=expected), patch.dict(runner.os.environ, environment, clear=True), patch.object(runner.shutil, 'which', side_effect=available.get), patch.object(sys, 'argv', ['runner', '-p', 'api', 'tests::selected', '--exact']), patch.object(runner.subprocess, 'run', side_effect=[result('tests::selected: test\n'), result('test result: ok. 1 passed; 0 failed; 0 ignored;\n')]) as calls, contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(runner.main(), 0)
                self.assertEqual([call.args[0][0] for call in calls.call_args_list], [expected, expected])
                self.assertIn('--exact', calls.call_args_list[0].args[0])
                self.assertIn('--ignored', calls.call_args_list[1].args[0])

    def test_missing_or_invalid_overridden_tool_does_not_fall_back_or_pass(self):
        for environment, expected in [({}, 'cargo'), ({'CARGO': '/missing/override'}, '/missing/override')]:
            with patch.dict(runner.os.environ, environment, clear=True), patch.object(runner.shutil, 'which', return_value=None), patch.object(sys, 'argv', ['runner', '-p', 'api', 'tests::selected']), patch.object(runner.subprocess, 'run', side_effect=FileNotFoundError) as calls:
                with self.assertRaises(FileNotFoundError): runner.main()
                self.assertEqual(calls.call_count, 1)
                self.assertEqual(calls.call_args.args[0][0], expected)

    @unittest.skipUnless(os.name == 'posix', 'Executable shell fixture requires POSIX')
    def test_real_subprocess_works_with_only_cargo_exe_on_path(self):
        # Model the WSL executable-name boundary without Rust or a database.
        with tempfile.TemporaryDirectory(prefix='cargo path ') as directory:
            executable = Path(directory) / 'cargo.exe'
            executable.write_text('#!/bin/sh\ncase " $* " in\n*" --list "*) printf "tests::selected: test\\n" ;;\n*) printf "test result: ok. 1 passed; 0 failed; 0 ignored;\\n" ;;\nesac\n')
            executable.chmod(0o700)
            environment = dict(os.environ, PATH=directory)
            environment.pop('CARGO', None)
            process = subprocess.run([sys.executable, str(ROOT / 'scripts/run-required-postgres-test.py'), '-p', 'api', 'tests::selected', '--exact'], env=environment, text=True, capture_output=True, timeout=10)
            self.assertEqual(process.returncode, 0, process.stderr)
            self.assertIn('required_postgres_test package=api selected=1 passed=1', process.stdout)

    def test_postgres_entrypoints_use_guard_and_current_selectors(self):
        for name in ['.github/workflows/ci.yml', 'scripts/check-postgres.sh', 'scripts/check-postgres.ps1']:
            text = (ROOT / name).read_text()
            self.assertNotRegex(text, r'cargo test[^\n]*--ignored')
            self.assertNotIn('bounty_status_reads_base_events_from_postgres_after_cross_process_indexing', text)
            self.assertIn('tests::bounty_status_reads_scoped_postgres_after_cross_process_funding --exact', text)
            self.assertIn('tests::mcp_bounty_status_reads_scoped_postgres_after_cross_process_funding --exact', text)
            self.assertIn('scripts/run-required-postgres-test.py', text)


class RecoverySecurityRunnerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location(
            'security_postgres', ROOT / 'scripts/check-security-postgres.py')
        cls.runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.runner)

    def suites(self):
        names = {'recovery_read_tests::',
                 'real_process_reads_preserve_data_privacy_quarantine_and_restart_boundaries'}
        selected = tuple(suite for suite in self.runner.SUITES if suite[1] in names)
        self.assertEqual({suite[1] for suite in selected}, names)
        self.assertTrue(all(suite[2] >= 1 for suite in selected))
        return selected

    def run_recovery(self, response):
        environment = {'AGENT_BOUNTIES_LOCAL_TEST_DATABASE': '1',
                       'DATABASE_URL': 'postgres://agent_bounties_test:fixture@127.0.0.1:12345/agent_bounties_test'}
        with patch.object(self.runner, 'SUITES', self.suites()), \
                patch.dict(os.environ, environment, clear=True), \
                patch.object(self.runner.shutil, 'which', return_value='cargo'), \
                patch.object(self.runner.subprocess, 'run', return_value=response) as calls, \
                contextlib.redirect_stdout(io.StringIO()):
            code = self.runner.main()
        return code, calls

    def test_recovery_process_gets_fresh_fixture_and_both_suites_execute_ignored_tests(self):
        code, calls = self.run_recovery(result('test result: ok. 1 passed; 0 failed;'))
        self.assertEqual(code, 0)
        self.assertEqual(calls.call_count, 2)
        database, process = [call.args[0] for call in calls.call_args_list]
        self.assertEqual(database[:5], ['cargo', 'test', '-p', 'db', 'recovery_read_tests::'])
        self.assertEqual(process[:3], [sys.executable, 'scripts/run-with-test-postgres.py', '--'])
        self.assertEqual(process[3:7], ['cargo', 'test', '-p', 'api'])
        for command in [database, process]:
            self.assertEqual(command[-3:], ['--', '--ignored', '--test-threads=1'])

    def test_missing_recovery_tests_cannot_be_reported_as_success(self):
        with self.assertRaisesRegex(RuntimeError, 'Missing security tests'):
            self.run_recovery(result('test result: ok. 0 passed; 0 failed;'))

    def test_recovery_test_failure_stops_runner(self):
        code, calls = self.run_recovery(result(code=101))
        self.assertEqual(code, 101)
        self.assertEqual(calls.call_count, 1)


if __name__ == '__main__':
    unittest.main()
