"""Run the executable guard with real argv/environment and fixed exit contracts."""
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parent / "check-recovery-revision.sh"
EXPECTED = "1234567890abcdef1234567890abcdef12345678"


class RecoveryRevisionGuardTests(unittest.TestCase):
    def invoke(self, args, deployed=None, shell=False):
        env = {"PATH": "/usr/bin:/bin"}
        if deployed is not None:
            env["RENDER_GIT_COMMIT"] = deployed
        command = [str(SCRIPT), *args]
        if shell:
            command = ["/bin/sh", "-c", shlex.join(command)]
        return subprocess.run(command, env=env, capture_output=True, text=True, timeout=2)

    def test_exact_revision_passes_without_output(self):
        result = self.invoke([EXPECTED], EXPECTED)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout + result.stderr, "")

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

    def test_executable_is_preserved_in_source(self):
        self.assertTrue(os.access(SCRIPT, os.X_OK))


if __name__ == "__main__":
    unittest.main()
