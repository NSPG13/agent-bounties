#!/usr/bin/env python3
"""Fail-closed tests for portable release evidence."""
import contextlib
import copy
import datetime as dt
import io
import os
import subprocess
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import application_release_gate as gate


class ReleaseGateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.now = dt.datetime.now(dt.timezone.utc)
        self.source = {"commit": "a" * 40, "tree": "b" * 40, "gate_sha256": "c" * 64}
        rows = []
        for name, command in gate.STEPS:
            log = self.root / f"{name}.log"
            log.write_text("fixture passed\n")
            rows.append({"name": name, "command": list(command), "exit_code": 0,
                         "log_sha256": gate.digest(log)})
        self.record = {"schema": gate.SCHEMA, "status": "passed", "source": self.source,
                       "finished_at": self.now.isoformat(), "steps": rows}

    def validate(self, record=None, source=None):
        gate.validate_record(record or self.record, self.root, source or self.source, self.now)

    def test_complete_exact_source_is_accepted(self):
        self.validate()

    def test_failed_missing_reordered_or_replaced_check_is_rejected(self):
        for kind in ("failure", "missing", "reordered", "command"):
            value = copy.deepcopy(self.record)
            if kind == "failure":
                value["steps"][0]["exit_code"] = 1
            elif kind == "missing":
                value["steps"].pop()
            elif kind == "reordered":
                value["steps"].reverse()
            else:
                value["steps"][1]["command"] = ["true"]
            with self.subTest(kind=kind), self.assertRaises(gate.GateError):
                self.validate(value)

    def test_changed_source_or_gate_is_rejected(self):
        for key in self.source:
            current = {**self.source, key: "different"}
            with self.subTest(key=key), self.assertRaises(gate.GateError):
                self.validate(source=current)

    def test_missing_changed_or_linked_log_is_rejected(self):
        log = self.root / "repository.log"
        log.write_text("changed\n")
        with self.assertRaises(gate.GateError):
            self.validate()
        log.unlink()
        with self.assertRaises(gate.GateError):
            self.validate()
        log.symlink_to(self.root / "gate-contract.log")
        with self.assertRaises(gate.GateError):
            self.validate()

    def test_old_future_or_unfinished_evidence_is_rejected(self):
        for timestamp in (self.now - dt.timedelta(days=2), self.now + dt.timedelta(seconds=1)):
            value = {**self.record, "finished_at": timestamp.isoformat()}
            with self.assertRaises(gate.GateError):
                self.validate(value)
        with self.assertRaises(gate.GateError):
            self.validate({**self.record, "status": "running"})

    def test_deployment_wallet_and_provider_credentials_are_not_forwarded(self):
        with patch.dict(os.environ, {"PATH": "/bin", "RESEND_API_KEY": "fixture",
                                     "BASE_KEEPER_PRIVATE_KEY": "fixture",
                                     "RENDER_API_KEY": "fixture", "DATABASE_URL": "fixture",
                                     "GITHUB_TOKEN": "fixture", "OPERATOR_API_TOKEN": "fixture"}, clear=True):
            self.assertEqual(gate.environment(), {"PATH": "/bin"})

    def test_low_disk_build_settings_do_not_disable_test_assertions(self):
        settings = {"CARGO_PROFILE_DEV_DEBUG": "0", "CARGO_PROFILE_TEST_DEBUG": "0",
                    "CARGO_INCREMENTAL": "0"}
        with patch.dict(os.environ, {**settings, "RUSTFLAGS": "--cfg skip_checks",
                                     "STRIPE_SECRET_KEY": "fixture"}, clear=True):
            self.assertEqual(gate.environment(), settings)

    def test_dirty_checkout_is_rejected(self):
        with patch.object(gate, "git", return_value=" M source.rs"):
            with self.assertRaises(gate.GateError):
                gate.source()

    def test_local_fixtures_choose_distinct_unused_ports_and_project(self):
        with gate.socket.socket() as busy:
            busy.bind(("127.0.0.1", 0))
            fixture = gate.local_fixture_environment()
            ports = [int(fixture[name]) for name in (
                "PRODUCTION_COMPOSE_API_PORT", "PRODUCTION_COMPOSE_MCP_PORT")]
            self.assertEqual(len(set(ports)), 2)
            self.assertNotIn(busy.getsockname()[1], ports)
            self.assertTrue(all(0 < port < 65536 for port in ports))
            self.assertNotEqual(fixture["PRODUCTION_COMPOSE_PROJECT"],
                                gate.local_fixture_environment()["PRODUCTION_COMPOSE_PROJECT"])

    def test_publication_postgres_selector_requires_three_executed_tests(self):
        selector = ("db", "terms_publication::tests::", 3)
        self.assertEqual(gate.POSTGRES.count(selector), 1)
        self.assertEqual(gate.POSTGRES.count((
            "api", "terms_publication::tests::publication_http_wallet_ownership_and_exact_historical_replay", 1,
        )), 1)
        environment = {
            "AGENT_BOUNTIES_LOCAL_TEST_DATABASE": "1",
            "AGENT_BOUNTIES_TEST_DATABASE_URL":
                "postgres://agent_bounties_test:fixture@127.0.0.1/agent_bounties_test",
        }
        for passed, ignored, exit_code, accepted in (
            (0, 3, 0, False), (2, 1, 0, False), (3, 0, 1, False), (3, 0, 0, True),
        ):
            output = f"test result: ok. {passed} passed; 0 failed; {ignored} ignored\n"
            result = subprocess.CompletedProcess([], exit_code, stdout=output)
            with self.subTest(passed=passed, ignored=ignored, exit_code=exit_code), \
                    patch.dict(os.environ, environment, clear=True), \
                    patch.object(gate, "POSTGRES", (selector,)), \
                    patch.object(gate.subprocess, "run", return_value=result) as run, \
                    contextlib.redirect_stdout(io.StringIO()):
                if accepted:
                    self.assertEqual(gate.postgres(), 0)
                else:
                    with self.assertRaises(gate.GateError):
                        gate.postgres()
                self.assertEqual(run.call_args.args[0], [
                    "cargo", "test", "-p", "db", "terms_publication::tests::", "--",
                    "--ignored", "--test-threads=1",
                ])

    def test_remote_database_is_rejected(self):
        with patch.dict(os.environ, {"AGENT_BOUNTIES_LOCAL_TEST_DATABASE": "1",
                                     "AGENT_BOUNTIES_TEST_DATABASE_URL":
                                     "postgres://agent_bounties_test:fixture@production/agent_bounties_test"}, clear=True):
            with self.assertRaises(gate.GateError):
                gate.postgres()


if __name__ == "__main__":
    unittest.main()
