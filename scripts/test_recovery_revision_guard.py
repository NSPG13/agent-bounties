"""Run the executable guard with real argv/environment and fixed exit contracts."""
import os
import hashlib
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest
from scripts.parse_recovery_artifact_receipt import parse_receipt, verify_file_hashes

SCRIPT = Path(__file__).resolve().parent / "check-recovery-revision.sh"
EXPECTED = "1234567890abcdef1234567890abcdef12345678"


class RecoveryRevisionGuardTests(unittest.TestCase):
    def setUp(self):
        self.fixture = tempfile.TemporaryDirectory(prefix="recovery-guard-files-")
        self.addCleanup(self.fixture.cleanup)
        self.folder = Path(self.fixture.name)
        self.paths = {
            "/usr/local/bin/agent-bounties-service": self.folder / "api",
            "/usr/local/bin/recovery-reader": self.folder / "reader",
            "/usr/local/bin/run-recovery-indexers": self.folder / "supervisor",
            "/bin/bash": self.folder / "bash",
            "/usr/local/bin/check-recovery-revision": self.folder / "guard",
            "/usr/local/bin/managed-funding-launcher": self.folder / "funding-launcher",
            "/usr/local/bin/managed-funding-worker": self.folder / "funding-worker",
        }
        # Fixed production paths are rewritten only in this private test copy.
        # The shipped executable has no runtime path/configuration override.
        self.source = SCRIPT.read_text()
        for original, replacement in self.paths.items():
            self.source = self.source.replace(original, str(replacement))
        self.guard = self.paths["/usr/local/bin/check-recovery-revision"]
        self.guard.write_text(self.source)
        self.guard.chmod(0o755)
        self.make_file("/usr/local/bin/agent-bounties-service")

    def make_file(self, name):
        target = self.paths[name]
        target.write_text("#!/bin/sh\nexit 97 # must only be hashed, never started\n")
        target.chmod(0o755)

    def reader_fixture(self):
        self.paths["/usr/local/bin/agent-bounties-service"].unlink()
        for name in ("/usr/local/bin/recovery-reader", "/usr/local/bin/run-recovery-indexers", "/bin/bash"):
            self.make_file(name)

    def funding_fixture(self):
        for name, path in self.paths.items():
            if name != '/usr/local/bin/check-recovery-revision' and path.exists():
                path.unlink()
        for name in ('/usr/local/bin/managed-funding-launcher', '/usr/local/bin/managed-funding-worker'):
            self.make_file(name)

    def test_funding_receipt_hashes_both_fixed_files_without_executing_them(self):
        self.funding_fixture()
        direct = self.invoke([EXPECTED], EXPECTED)
        shell = self.invoke([EXPECTED], EXPECTED, shell=True)
        self.assertEqual((direct.returncode, direct.stderr), (0, ''))
        self.assertEqual((shell.returncode, shell.stdout), (0, direct.stdout))
        receipt = parse_receipt(direct.stdout, EXPECTED, 'funding')
        for field, name in [('binary_sha256', '/usr/local/bin/managed-funding-worker'),
                            ('launcher_sha256', '/usr/local/bin/managed-funding-launcher'),
                            ('guard_sha256', '/usr/local/bin/check-recovery-revision')]:
            self.assertEqual(receipt[field], hashlib.sha256(self.paths[name].read_bytes()).hexdigest())
        for malformed in [direct.stdout.replace('launcher_sha256=', 'supervisor_sha256='),
                          direct.stdout.replace('artifact_kind=funding', 'artifact_kind=api'),
                          direct.stdout.replace('receipt_complete=true\n', ''),
                          direct.stdout + 'launcher_sha256=' + '0' * 64 + '\n']:
            with self.assertRaises(ValueError):
                parse_receipt(malformed, EXPECTED, 'funding')

    def test_funding_cannot_mix_with_api_or_reader_even_as_dangling_symlink(self):
        self.funding_fixture()
        for name in ('/usr/local/bin/agent-bounties-service', '/usr/local/bin/recovery-reader'):
            for dangling in (False, True):
                with self.subTest(name=name, dangling=dangling):
                    if dangling:
                        self.paths[name].symlink_to(self.folder / 'absent')
                    else:
                        self.make_file(name)
                    result = self.invoke([EXPECTED], EXPECTED)
                    self.assertEqual((result.returncode, result.stdout), (1, ''))
                    self.paths[name].unlink()

    def test_funding_requires_regular_executable_launcher_and_worker(self):
        self.funding_fixture()
        for name in ('/usr/local/bin/managed-funding-launcher', '/usr/local/bin/managed-funding-worker'):
            target = self.paths[name]
            for kind in ('missing', 'directory', 'nonexecutable', 'dangling'):
                with self.subTest(name=name, kind=kind):
                    target.unlink()
                    if kind == 'directory': target.mkdir()
                    elif kind == 'nonexecutable': self.make_file(name); target.chmod(0o644)
                    elif kind == 'dangling': target.symlink_to(self.folder / 'absent')
                    result = self.invoke([EXPECTED], EXPECTED)
                    self.assertEqual((result.returncode, result.stdout), (1, ''))
                    if target.is_dir(): target.rmdir()
                    elif target.exists() or target.is_symlink(): target.unlink()
                    self.make_file(name)

    def invoke(self, args, deployed=None, shell=False):
        env = {"PATH": "/usr/bin:/bin"}
        if deployed is not None:
            env["RENDER_GIT_COMMIT"] = deployed
        command = [str(self.guard), *args]
        if shell:
            command = ["/bin/sh", "-c", shlex.join(command)]
        return subprocess.run(command, env=env, capture_output=True, text=True, timeout=2)

    def test_exact_revision_emits_complete_api_receipt(self):
        result = self.invoke([EXPECTED], EXPECTED)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stderr, "")
        parsed = parse_receipt(result.stdout, EXPECTED, 'api')
        self.assertEqual(parsed['binary_sha256'], hashlib.sha256(self.paths['/usr/local/bin/agent-bounties-service'].read_bytes()).hexdigest())
        self.assertEqual(parsed['guard_sha256'], hashlib.sha256(self.guard.read_bytes()).hexdigest())

    def test_plain_command_also_survives_one_provider_shell_dispatch(self):
        self.assertEqual(self.invoke([EXPECTED], EXPECTED, shell=True).returncode, 0)
        self.assertEqual(self.invoke([EXPECTED], "0" * 40, shell=True).returncode, 1)

    def test_deployed_revision_is_required_valid_and_exact(self):
        for value in (None, "", "0" * 40, "a" * 39, "a" * 41, "g" * 40,
                      EXPECTED.upper(), EXPECTED + "\n", " " + EXPECTED):
            with self.subTest(deployed=value):
                result = self.invoke([EXPECTED], value)
                self.assertEqual(result.returncode, 1)
                self.assertEqual(result.stdout, "")
                self.assertTrue(result.stderr.startswith("recovery revision guard:"))

    def test_expected_revision_and_argument_count_are_strict(self):
        for args in ([], [EXPECTED, EXPECTED], [""], ["--help"], ["a" * 39],
                     ["a" * 41], ["g" * 40], [EXPECTED.upper()], [EXPECTED + "\n"]):
            with self.subTest(args=args):
                result = self.invoke(args, EXPECTED)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(result.stdout, "")
                self.assertNotIn(EXPECTED, result.stderr)

    def test_inputs_are_never_evaluated_or_echoed(self):
        with tempfile.TemporaryDirectory(prefix="recovery-guard-injection-") as directory:
            marker = Path(directory) / "executed"
            command = "touch " + shlex.quote(str(marker))
            for value in (EXPECTED + "; " + command, "$(" + command + ")",
                          "`" + command + "`", EXPECTED + "\n" + command):
                with self.subTest(value=value):
                    for args, deployed, expected_exit in (([value], EXPECTED, 2),
                                                          ([EXPECTED], value, 1)):
                        result = self.invoke(args, deployed)
                        self.assertEqual(result.returncode, expected_exit)
                        self.assertNotIn(value, result.stdout + result.stderr)
                        self.assertFalse(marker.exists())

    def test_reader_receipt_hashes_all_four_fixed_files_without_starting_them(self):
        self.reader_fixture()
        result = self.invoke([EXPECTED], EXPECTED)
        self.assertEqual(result.returncode, 0, result.stderr)
        parsed = parse_receipt(result.stdout, EXPECTED, 'reader')
        for field, name in [('binary_sha256','/usr/local/bin/recovery-reader'),
                            ('supervisor_sha256','/usr/local/bin/run-recovery-indexers'),
                            ('bash_sha256','/bin/bash'),('guard_sha256','/usr/local/bin/check-recovery-revision')]:
            self.assertEqual(parsed[field],hashlib.sha256(self.paths[name].read_bytes()).hexdigest())

    def test_both_neither_and_dangling_other_binary_fail_without_receipt(self):
        self.make_file('/usr/local/bin/recovery-reader')
        for stage in range(3):
            result=self.invoke([EXPECTED],EXPECTED)
            self.assertEqual(result.returncode,1)
            self.assertEqual(result.stdout,'')
            if stage==0:
                self.paths['/usr/local/bin/agent-bounties-service'].unlink()
                self.paths['/usr/local/bin/recovery-reader'].unlink()
            elif stage==1:
                self.make_file('/usr/local/bin/agent-bounties-service')
                self.paths['/usr/local/bin/recovery-reader'].symlink_to(self.folder/'absent')

    def test_missing_nonfile_or_nonexecutable_dependency_fails(self):
        self.reader_fixture()
        for name in ('/usr/local/bin/recovery-reader','/usr/local/bin/run-recovery-indexers','/bin/bash'):
            target=self.paths[name]
            target.unlink()
            result=self.invoke([EXPECTED],EXPECTED)
            self.assertEqual((result.returncode,result.stdout),(1,''))
            target.mkdir()
            result=self.invoke([EXPECTED],EXPECTED)
            self.assertEqual((result.returncode,result.stdout),(1,''))
            target.rmdir();self.make_file(name);target.chmod(0o644)
            result=self.invoke([EXPECTED],EXPECTED)
            self.assertEqual((result.returncode,result.stdout),(1,''))
            target.chmod(0o755)

    def test_hash_failure_and_bad_digest_never_emit_values_or_receipt(self):
        fake=self.folder/'hash-tool'
        for body in ('echo DO-NOT-PRINT-THIS >&2; exit 7', 'echo DO-NOT-PRINT-THIS; exit 0'):
            fake.write_text('#!/bin/sh\n'+body+'\n');fake.chmod(0o755)
            self.guard.write_text(self.source.replace('/usr/bin/sha256sum',str(fake)))
            result=self.invoke([EXPECTED],EXPECTED)
            self.assertEqual((result.returncode,result.stdout),(1,''))
            self.assertNotIn('DO-NOT-PRINT-THIS',result.stderr)

    def test_parser_rejects_truncation_duplicates_extras_wrong_pins_and_noncanonical_values(self):
        valid=self.invoke([EXPECTED],EXPECTED).stdout
        cases=[valid[:-1],valid.replace('receipt_complete=true\n',''),valid+valid,
               valid+'unexpected=DO-NOT-PRINT-THIS\n',valid.replace('artifact_kind=api','artifact_kind=reader'),
               valid.replace('receipt_complete=true','receipt_complete=false'),valid.replace('\n','\r\n'),
               valid.replace('source_revision='+EXPECTED,'source_revision='+'2'*40),
               valid.replace('guard_sha256=','guard_sha256=G'),valid.replace('binary_sha256=','guard_sha256='),
               valid.replace('schema=','schema=DO-NOT-PRINT-THIS'), 'x'*2049]
        for text in cases:
            with self.assertRaises(ValueError):parse_receipt(text,EXPECTED,'api')
        for revision,kind in [('bad','api'),(EXPECTED,'other'),(EXPECTED,'reader')]:
            with self.assertRaises(ValueError):parse_receipt(valid,revision,kind)

    def test_missing_fixed_guard_file_fails_even_when_launched_from_another_path(self):
        launch=self.folder/'alternate-launch'
        launch.write_text(self.source);launch.chmod(0o755)
        self.guard.unlink();self.guard=launch
        result=self.invoke([EXPECTED],EXPECTED)
        self.assertEqual((result.returncode,result.stdout),(1,''))

    def test_independent_hash_comparison_rejects_missing_extra_duplicate_and_wrong_files(self):
        for kind in ('api','reader','funding'):
            if kind=='reader':self.reader_fixture()
            elif kind=='funding':self.funding_fixture()
            receipt=parse_receipt(self.invoke([EXPECTED],EXPECTED).stdout,EXPECTED,kind)
            names=['/usr/local/bin/check-recovery-revision']
            names += {'api':['/usr/local/bin/agent-bounties-service'],
                      'reader':['/usr/local/bin/recovery-reader','/usr/local/bin/run-recovery-indexers','/bin/bash'],
                      'funding':['/usr/local/bin/managed-funding-worker','/usr/local/bin/managed-funding-launcher']}[kind]
            raw=''.join(hashlib.sha256(self.paths[name].read_bytes()).hexdigest()+'  '+name+'\n' for name in names)
            verify_file_hashes(receipt,raw)
            for bad in ('',raw+raw,raw.replace(names[0],'/unexpected'),raw.replace(raw[:64],'0'*64),raw.replace('  ',' ')):
                with self.assertRaises(ValueError):verify_file_hashes(receipt,bad)

    def test_shipped_paths_are_fixed_and_not_environment_overrides(self):
        source=SCRIPT.read_text()
        for name in self.paths:self.assertIn(name,source)
        self.assertIn('/usr/bin/sha256sum -- "$1"',source)
        self.assertNotIn('eval ',source)

    def test_executable_is_preserved_in_source(self):
        self.assertTrue(os.access(SCRIPT, os.X_OK))


if __name__ == "__main__":
    unittest.main()
