"""Real-process boundary tests.

A recording fake `taskmarket` CLI runs as a real subprocess. Tests assert the
exact argv against the official contract and - critically - that NO process
is launched when authorization or path policy refuses the call.
"""
import os
import pathlib
import subprocess
import tempfile
import unittest
from datetime import datetime, timedelta, timezone

from taskmarket_adapter import client as client_mod
from taskmarket_adapter import security
from taskmarket_adapter.authorization import (
    ACTION_TASK_CREATE,
    ACTION_TASK_SUBMIT,
    load_authorization,
)
from taskmarket_adapter.client import TaskmarketClient, build_create_argv, build_submit_argv
from taskmarket_adapter.errors import SecurityError, TaskmarketError

from . import _support

SECRET = "boundary-test-secret"


class ClientBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(self._cleanup)
        self.bin_dir = self.tmp / "bin"
        _support.make_fake_cli(self.bin_dir)
        self.log = self.tmp / "invocations.log"
        # Apply the sandbox to the real process environment so any child
        # process (client -> fake CLI) sees it; restore on teardown.
        saved = {k: os.environ.get(k) for k in _support.ENV_KEYS}
        self.addCleanup(lambda: [os.environ.pop(k, None) or
                                 (os.environ.__setitem__(k, v) if v is not None else None)
                                 for k, v in saved.items()])
        for key in _support.ENV_KEYS:
            os.environ.pop(key, None)
        os.environ["PATH"] = f"{self.bin_dir}{os.pathsep}{os.environ.get('PATH', '')}"
        os.environ["FAKE_CLI_LOG"] = str(self.log)
        os.environ["TASKMARKET_OPERATOR_SECRET"] = SECRET
        self.env = dict(os.environ)

    def _cleanup(self):
        import shutil
        shutil.rmtree(self.tmp, ignore_errors=True)

    def _artifact(self, **kwargs):
        artifact_path = _support.write_create_artifact(self.tmp / "auth.json", SECRET, **kwargs)
        os.environ["TASKMARKET_AUTHORIZATION_FILE"] = str(artifact_path)
        return artifact_path

    def _client(self) -> TaskmarketClient:
        return TaskmarketClient()

    # ---------- argv contract ----------
    def test_create_argv_matches_official_contract_exactly(self):
        self._artifact(
            action=ACTION_TASK_CREATE,
            expires_at=(datetime.now(timezone.utc) + timedelta(hours=1)).isoformat(),
            max_reward_usdc="100",
            max_duration_hours=720,
        )
        spec = load_authorization(ACTION_TASK_CREATE, env=os.environ)
        result = self._client().create_task(
            spec, description="Fix login bug", reward_usdc="5", duration_hours=48,
        )
        invocations = _support.read_invocations(self.log)
        self.assertEqual(len(invocations), 1)
        # Exact official contract: task create --description --reward <usdc> --duration <h>
        expected = ["task", "create",
                    "--description", "Fix login bug",
                    "--reward", "5",
                    "--duration", "48"]
        self.assertEqual(invocations[0], expected)
        self.assertEqual(result["argv"], expected)

    def test_five_usdc_never_becomes_five_million(self):
        self._artifact(
            action=ACTION_TASK_CREATE,
            expires_at=(datetime.now(timezone.utc) + timedelta(hours=1)).isoformat(),
            max_reward_usdc="10",
        )
        spec = load_authorization(ACTION_TASK_CREATE, env=os.environ)
        self._client().create_task(spec, description="d", reward_usdc="5", duration_hours=24)
        reward_index = _support.read_invocations(self.log)[0].index("--reward")
        sent = _support.read_invocations(self.log)[0][reward_index + 1]
        self.assertEqual(sent, "5")  # human-readable USDC, NOT base units 5000000

    def test_submit_argv_matches_official_contract(self):
        artifact_root = self.tmp / "artifacts"
        (artifact_root / "out").mkdir(parents=True)
        deliverable = artifact_root / "out" / "result.zip"
        deliverable.write_bytes(b"payload")
        self._artifact(action=ACTION_TASK_SUBMIT,
                       expires_at=(datetime.now(timezone.utc) + timedelta(hours=1)).isoformat())
        os.environ["TASKMARKET_ARTIFACT_ROOTS"] = str(artifact_root)
        spec = load_authorization(ACTION_TASK_SUBMIT, env=os.environ)
        task_id = "0x" + "11" * 32
        self._client().submit(spec, task_id=task_id, file_path=str(deliverable))
        expected = ["task", "submit", task_id, "--file", str(deliverable.resolve())]
        self.assertEqual(_support.read_invocations(self.log), [expected])

    def test_argv_builders_pure(self):
        self.assertEqual(
            build_create_argv("d", "7.5", 12),
            ["task", "create", "--description", "d", "--reward", "7.5", "--duration", "12"],
        )
        self.assertEqual(build_submit_argv("tid", "/p/f.zip"), ["task", "submit", "tid", "--file", "/p/f.zip"])

    # ---------- no process without valid authority ----------
    def assert_no_process_launched(self):
        self.assertFalse(self.log.exists(),
                         f"a CLI process was launched: {_support.read_invocations(self.log)}")

    def test_no_authorization_artifact_launches_no_process(self):
        spec_env = dict(self.env)
        spec_env.pop("TASKMARKET_AUTHORIZATION_FILE", None)
        with self.assertRaises(TaskmarketError):
            self._client().create_task(
                load_authorization(ACTION_TASK_CREATE, env=spec_env),
                description="d", reward_usdc="5", duration_hours=24,
            )
        self.assert_no_process_launched()

    def test_expired_artifact_launches_no_process(self):
        self._artifact(action=ACTION_TASK_CREATE,
                       expires_at=(datetime.now(timezone.utc) - timedelta(seconds=1)).isoformat())
        with self.assertRaises(TaskmarketError):
            self._client().create_task(
                load_authorization(ACTION_TASK_CREATE, env=os.environ),
                description="d", reward_usdc="5", duration_hours=24,
            )
        self.assert_no_process_launched()

    def test_wrong_action_artifact_launches_no_process(self):
        self._artifact(action=ACTION_TASK_SUBMIT,
                       expires_at=(datetime.now(timezone.utc) + timedelta(hours=1)).isoformat())
        with self.assertRaises(TaskmarketError):
            self._client().create_task(
                load_authorization(ACTION_TASK_CREATE, env=os.environ),
                description="d", reward_usdc="5", duration_hours=24,
            )
        self.assert_no_process_launched()

    def test_reward_over_cap_launches_no_process(self):
        self._artifact(action=ACTION_TASK_CREATE, max_reward_usdc="1",
                       expires_at=(datetime.now(timezone.utc) + timedelta(hours=1)).isoformat())
        with self.assertRaises(TaskmarketError):
            self._client().create_task(
                load_authorization(ACTION_TASK_CREATE, env=os.environ),
                description="d", reward_usdc="2", duration_hours=24,
            )
        self.assert_no_process_launched()

    def test_nonpositive_values_launch_no_process(self):
        self._artifact(action=ACTION_TASK_CREATE,
                       expires_at=(datetime.now(timezone.utc) + timedelta(hours=1)).isoformat())
        spec = load_authorization(ACTION_TASK_CREATE, env=os.environ)
        for bad_reward in ("0", "-3"):
            with self.assertRaises(TaskmarketError):
                self._client().create_task(spec, description="d", reward_usdc=bad_reward, duration_hours=24)
        for bad_duration in (0, -4):
            with self.assertRaises(TaskmarketError):
                self._client().create_task(spec, description="d", reward_usdc="1", duration_hours=bad_duration)
        self.assert_no_process_launched()

    def test_outside_root_submit_launches_no_process(self):
        outside = self.tmp / "host-secret.txt"
        outside.write_text("keys")
        os.environ["TASKMARKET_ARTIFACT_ROOTS"] = str(self.tmp / "artifacts-root")
        self._artifact(action=ACTION_TASK_SUBMIT,
                       expires_at=(datetime.now(timezone.utc) + timedelta(hours=1)).isoformat())
        spec = load_authorization(ACTION_TASK_SUBMIT, env=os.environ)
        with self.assertRaises(SecurityError):
            self._client().submit(spec, task_id="0x1", file_path=str(outside))
        self.assert_no_process_launched()

    def test_symlink_escape_submit_launches_no_process(self):
        root = self.tmp / "root2"
        root.mkdir()
        outside = self.tmp / "real.dat"
        outside.write_bytes(b"x")
        link = root / "link.dat"
        os.symlink(outside, link)
        os.environ["TASKMARKET_ARTIFACT_ROOTS"] = str(root)
        self._artifact(action=ACTION_TASK_SUBMIT,
                       expires_at=(datetime.now(timezone.utc) + timedelta(hours=1)).isoformat())
        spec = load_authorization(ACTION_TASK_SUBMIT, env=os.environ)
        with self.assertRaises(SecurityError):
            self._client().submit(spec, task_id="0x1", file_path=str(link))
        self.assert_no_process_launched()

    def test_bound_task_id_mismatch_launches_no_process(self):
        root = self.tmp / "root3"
        root.mkdir()
        (root / "f.txt").write_text("x")
        os.environ["TASKMARKET_ARTIFACT_ROOTS"] = str(root)
        self._artifact(action=ACTION_TASK_SUBMIT, task_id="0xaaa",
                       expires_at=(datetime.now(timezone.utc) + timedelta(hours=1)).isoformat())
        spec = load_authorization(ACTION_TASK_SUBMIT, env=os.environ)
        with self.assertRaises(TaskmarketError):
            self._client().submit(spec, task_id="0xbbb", file_path=str(root / "f.txt"))
        self.assert_no_process_launched()

    # ---------- sanitized errors ----------
    def test_cli_failure_is_sanitized(self):
        os.environ["FAKE_CLI_MODE"] = "fail"
        c = self._client()
        with self.assertRaises(TaskmarketError) as ctx:
            c.stats()
        message = str(ctx.exception)
        self.assertIn("exit 1", message)
        self.assertNotIn("SECRET-HOST-DETAIL", message)
        self.assertNotIn("leaked-token", message)

    def test_missing_cli_is_sanitized(self):
        c = TaskmarketClient(cli="definitely-not-taskmarket-cli-xyz")
        with self.assertRaises(TaskmarketError) as ctx:
            c.stats()
        self.assertNotIn(str(self.tmp), str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
