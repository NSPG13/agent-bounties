import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
from urllib.parse import urlsplit

spec = importlib.util.spec_from_file_location(
    "local_postgres_fixture", Path(__file__).with_name("run-with-test-postgres.py")
)
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)
seed_spec = importlib.util.spec_from_file_location(
    "local_wallet_fixture", Path(__file__).with_name("seed-smoke-wallet.py")
)
wallet_fixture = importlib.util.module_from_spec(seed_spec)
seed_spec.loader.exec_module(wallet_fixture)


class LocalPostgresFixtureTests(unittest.TestCase):
    def test_wallet_seeding_rejects_unlabelled_database_containers(self):
        with patch("sys.argv", ["seed", "--container", "unrelated", "--output", "unused"]), patch.object(
            wallet_fixture.shutil, "which", return_value="docker"
        ), patch.object(wallet_fixture.subprocess, "check_output", return_value='[{"Config":{"Labels":{}}}]'), patch.object(
            wallet_fixture.subprocess, "run"
        ) as execute, self.assertRaises(SystemExit):
            wallet_fixture.main()
        execute.assert_not_called()

    def test_wallet_seeding_stores_only_hash_and_bounded_session(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "session"
            with patch("sys.argv", ["seed", "--container", "local-fixture", "--output", str(output)]), patch.object(
                wallet_fixture.shutil, "which", return_value="docker"
            ), patch.object(wallet_fixture.subprocess, "check_output", return_value='[{"Config":{"Labels":{"app.agentbounties.local-smoke":"true"},"Env":["POSTGRES_USER=test","POSTGRES_DB=test"]}}]'), patch.object(
                wallet_fixture.subprocess, "run"
            ) as execute:
                wallet_fixture.main()
            token = output.read_text()
            sql = execute.call_args.kwargs["input"]
            self.assertTrue(token.startswith("abws_"))
            self.assertNotIn(token, sql)
            self.assertIn(wallet_fixture.hashlib.sha256(token.encode()).hexdigest(), sql)
            self.assertIn("interval '15 minutes'", sql)
            self.assertEqual(execute.call_args.args[0][:3], ["docker", "exec", "--interactive"])

    def test_local_fixture_overrides_inherited_database_and_always_cleans_up(self):
        calls = []

        def run(command, **kwargs):
            calls.append((command, kwargs))
            return subprocess.CompletedProcess(command, 7 if command == ["fixture-test"] else 0)

        with patch("sys.argv", ["fixture", "--", "fixture-test"]), patch.object(
            fixture.shutil, "which", return_value="docker"
        ), patch.object(fixture.subprocess, "run", side_effect=run), patch.object(
            fixture.subprocess, "check_output", return_value='[{"NetworkSettings":{"Ports":{"5432/tcp":[{"HostIp":"127.0.0.1","HostPort":"15432"}]}}}]'
        ), patch.dict("os.environ", {"DATABASE_URL": "postgres://remote.example/production", "ENABLE_BASE_TX_BROADCAST": "true", "API_UPSTREAM_BASE_URL": "https://remote.example"}):
            self.assertEqual(fixture.main(), 7)
        child_env = next(kwargs["env"] for command, kwargs in calls if command == ["fixture-test"])
        self.assertEqual(urlsplit(child_env["DATABASE_URL"]).hostname, "127.0.0.1")
        self.assertEqual(urlsplit(child_env["DATABASE_URL"]).port, 15432)
        self.assertEqual(child_env["AGENT_BOUNTIES_TEST_DATABASE_URL"], child_env["DATABASE_URL"])
        self.assertEqual(child_env["ENABLE_BASE_TX_BROADCAST"], "false")
        self.assertEqual(child_env["API_UPSTREAM_BASE_URL"], "")
        self.assertEqual(calls[-1][0][:3], ["docker", "rm", "--force"])

    def test_nonloopback_port_binding_aborts_before_running_test(self):
        with patch("sys.argv", ["fixture", "--", "fixture-test"]), patch.object(
            fixture.shutil, "which", return_value="docker"
        ), patch.object(fixture.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)) as run, patch.object(
            fixture.subprocess, "check_output", return_value='[{"NetworkSettings":{"Ports":{"5432/tcp":[{"HostIp":"0.0.0.0","HostPort":"15432"}]}}}]'
        ), self.assertRaisesRegex(RuntimeError, "exclusively to loopback"):
            fixture.main()
        self.assertFalse(any(call.args[0] == ["fixture-test"] for call in run.call_args_list))
        self.assertEqual(run.call_args.args[0][:3], ["docker", "rm", "--force"])

    def test_inherited_fixture_cannot_point_at_remote_database(self):
        for database_url in (
            "postgres://remote.example/production",
            "postgres://localhost/test?host=remote.example",
        ):
            with patch.dict("os.environ", {"DATABASE_URL": database_url, "AGENT_BOUNTIES_LOCAL_TEST_DATABASE": "1"}), self.assertRaises(RuntimeError):
                fixture.validate_existing_fixture()


if __name__ == "__main__":
    unittest.main()
