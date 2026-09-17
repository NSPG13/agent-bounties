"""Real-process MCP protocol tests.

Spawns `python -m taskmarket_adapter.server` as a subprocess and speaks
line-delimited JSON-RPC 2.0 over stdio: initialize, tools/list, tools/call.
Refusal paths must return isError AND launch no CLI process.
"""
import json
import os
import pathlib
import selectors
import subprocess
import sys
import tempfile
import unittest
from datetime import datetime, timedelta, timezone

from . import _support

SECRET = "mcp-protocol-test-secret"
ROOT = pathlib.Path(__file__).resolve().parents[1]
SRC = ROOT / "src"


class McpServerProtocolTests(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(self._cleanup)
        self.bin_dir = self.tmp / "bin"
        _support.make_fake_cli(self.bin_dir)
        self.log = self.tmp / "invocations.log"
        self.artifact_root = self.tmp / "artifacts"
        self.artifact_root.mkdir()

    def _cleanup(self):
        import shutil
        shutil.rmtree(self.tmp, ignore_errors=True)

    def _env(self) -> dict:
        env = _support.clean_env(dict(os.environ))
        env["PATH"] = f"{self.bin_dir}{os.pathsep}{env.get('PATH', '')}"
        env["FAKE_CLI_LOG"] = str(self.log)
        env["TASKMARKET_ARTIFACT_ROOTS"] = str(self.artifact_root)
        env["PYTHONPATH"] = str(SRC)
        return env

    def _start_server(self):
        return subprocess.Popen(
            [sys.executable, "-m", "taskmarket_adapter.server"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, env=self._env(), cwd=str(self.tmp),
        )

    def _rpc(self, proc, payload: dict) -> dict:
        assert proc.stdin and proc.stdout
        proc.stdin.write(json.dumps(payload) + "\n")
        proc.stdin.flush()
        selector = selectors.DefaultSelector()
        selector.register(proc.stdout, selectors.EVENT_READ)
        ready = selector.select(timeout=15)
        selector.close()
        if not ready:
            self.fail("server did not answer in time")
        line = proc.stdout.readline()
        return json.loads(line)

    def _initialize(self, proc):
        resp = self._rpc(proc, {
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {}},
        })
        self.assertEqual(resp["result"]["serverInfo"]["name"], "taskmarket-adapter")
        proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        proc.stdin.flush()

    # ---------- tests ----------
    def test_initialize_and_tools_list(self):
        proc = self._start_server()
        self.addCleanup(self._terminate, proc)
        self._initialize(proc)
        resp = self._rpc(proc, {"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
        names = {t["name"] for t in resp["result"]["tools"]}
        expected = {
            "taskmarket_list_tasks", "taskmarket_get_task", "taskmarket_wallet_stats",
            "taskmarket_inbox", "taskmarket_submit", "taskmarket_create_task",
        }
        self.assertEqual(names, expected)
        create_tool = next(t for t in resp["result"]["tools"] if t["name"] == "taskmarket_create_task")
        self.assertNotIn("authorized",
                         create_tool["inputSchema"].get("properties", {}),
                         "no caller-controlled authority flag may exist")

    def test_unauthorized_create_is_refused_and_launches_no_process(self):
        proc = self._start_server()
        self.addCleanup(self._terminate, proc)
        self._initialize(proc)
        resp = self._rpc(proc, {
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "taskmarket_create_task",
                       "arguments": {"description": "d", "reward": "5", "duration_hours": 24}},
        })
        self.assertTrue(resp["result"]["isError"])
        self.assertFalse(self.log.exists(), "refused call must not launch the CLI")

    def test_caller_supplied_authorized_flag_changes_nothing(self):
        proc = self._start_server()
        self.addCleanup(self._terminate, proc)
        self._initialize(proc)
        resp = self._rpc(proc, {
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": {"name": "taskmarket_create_task",
                       "arguments": {"description": "d", "reward": "5",
                                     "duration_hours": 24, "authorized": True}},
        })
        self.assertTrue(resp["result"]["isError"])
        self.assertFalse(self.log.exists())

    def test_authorized_create_runs_real_cli_process_with_exact_argv(self):
        auth_file = _support.write_create_artifact(
            self.tmp / "auth.json", SECRET,
            action="task_create",
            expires_at=datetime.now(timezone.utc) + timedelta(hours=1),
            max_reward_usdc="10", max_duration_hours=72,
        )
        env = self._env()
        env["TASKMARKET_AUTHORIZATION_FILE"] = str(auth_file)
        env["TASKMARKET_OPERATOR_SECRET"] = SECRET
        proc = subprocess.Popen(
            [sys.executable, "-m", "taskmarket_adapter.server"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, env=env, cwd=str(self.tmp),
        )
        self.addCleanup(self._terminate, proc)
        self._initialize(proc)
        resp = self._rpc(proc, {
            "jsonrpc": "2.0", "id": 5, "method": "tools/call",
            "params": {"name": "taskmarket_create_task",
                       "arguments": {"description": "Ship it", "reward": "5", "duration_hours": 48,
                                     "network": "base-sepolia"}},
        })
        self.assertNotIn("isError", resp.get("result", {}), resp)
        invocations = _support.read_invocations(self.log)
        self.assertEqual(invocations, [
            ["task", "create", "--description", "Ship it",
             "--reward", "5", "--duration", "48"],
        ])

    def test_unknown_network_refused_before_any_process(self):
        auth_file = _support.write_create_artifact(
            self.tmp / "auth.json", SECRET, action="task_create",
            expires_at=datetime.now(timezone.utc) + timedelta(hours=1),
        )
        env = self._env()
        env["TASKMARKET_AUTHORIZATION_FILE"] = str(auth_file)
        env["TASKMARKET_OPERATOR_SECRET"] = SECRET
        proc = subprocess.Popen(
            [sys.executable, "-m", "taskmarket_adapter.server"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, env=env, cwd=str(self.tmp),
        )
        self.addCleanup(self._terminate, proc)
        self._initialize(proc)
        resp = self._rpc(proc, {
            "jsonrpc": "2.0", "id": 6, "method": "tools/call",
            "params": {"name": "taskmarket_create_task",
                       "arguments": {"description": "d", "reward": "5", "duration_hours": 24,
                                     "network": "ethereum"}},
        })
        self.assertTrue(resp["result"]["isError"])
        self.assertFalse(self.log.exists())

    @staticmethod
    def _terminate(proc: subprocess.Popen):
        if proc.poll() is None:
            proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()


if __name__ == "__main__":
    unittest.main()
