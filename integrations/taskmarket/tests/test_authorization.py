"""Operator authorization artifacts: fail closed on every bad input."""
import json
import pathlib
import tempfile
import unittest
from datetime import datetime, timedelta, timezone

from taskmarket_adapter import authorization as auth
from taskmarket_adapter.errors import AuthorizationError

SECRET = "operator-secret-0123456789"


class AuthorizationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.artifact = pathlib.Path(self.tmp.name) / "auth.json"
        self.env = {"TASKMARKET_OPERATOR_SECRET": SECRET}

    def _write(self, **spec):
        auth.write_artifact(self.artifact, spec, SECRET)
        self.env["TASKMARKET_AUTHORIZATION_FILE"] = str(self.artifact)

    def test_valid_artifact_loads(self):
        self._write(
            version=1,
            action=auth.ACTION_TASK_CREATE,
            expires_at=(datetime.now(timezone.utc) + timedelta(minutes=5)).isoformat(),
            max_reward_usdc="10",
        )
        spec = auth.load_authorization(auth.ACTION_TASK_CREATE, env=self.env)
        self.assertEqual(spec.action, auth.ACTION_TASK_CREATE)
        self.assertEqual(spec.max_reward_usdc, __import__("decimal").Decimal("10"))

    def test_missing_secret_refused(self):
        self._write(version=1, action="task_create",
                    expires_at=(datetime.now(timezone.utc) + timedelta(hours=1)).isoformat())
        with self.assertRaises(AuthorizationError):
            auth.load_authorization("task_create", env={})

    def test_missing_file_refused(self):
        with self.assertRaises(AuthorizationError):
            auth.load_authorization("task_create", env=self.env)

    def test_tampered_payload_refused(self):
        self._write(version=1, action=auth.ACTION_TASK_CREATE,
                    expires_at=(datetime.now(timezone.utc) + timedelta(hours=1)).isoformat(),
                    max_reward_usdc="1")
        document = json.loads(self.artifact.read_text())
        document["max_reward_usdc"] = "999999"
        self.artifact.write_text(json.dumps(document))
        with self.assertRaises(AuthorizationError):
            auth.load_authorization(auth.ACTION_TASK_CREATE, env=self.env)

    def test_wrong_action_refused(self):
        self._write(version=1, action=auth.ACTION_TASK_SUBMIT,
                    expires_at=(datetime.now(timezone.utc) + timedelta(hours=1)).isoformat())
        with self.assertRaises(AuthorizationError):
            auth.load_authorization(auth.ACTION_TASK_CREATE, env=self.env)

    def test_expired_refused(self):
        self._write(version=1, action=auth.ACTION_TASK_CREATE,
                    expires_at=(datetime.now(timezone.utc) - timedelta(seconds=1)).isoformat())
        with self.assertRaises(AuthorizationError):
            auth.load_authorization(auth.ACTION_TASK_CREATE, env=self.env)

    def test_unknown_version_refused(self):
        self._write(version=99, action=auth.ACTION_TASK_CREATE,
                    expires_at=(datetime.now(timezone.utc) + timedelta(hours=1)).isoformat())
        with self.assertRaises(AuthorizationError):
            auth.load_authorization(auth.ACTION_TASK_CREATE, env=self.env)

    def test_malformed_json_refused(self):
        self.artifact.write_text("{not json")
        self.env["TASKMARKET_AUTHORIZATION_FILE"] = str(self.artifact)
        with self.assertRaises(AuthorizationError):
            auth.load_authorization(auth.ACTION_TASK_CREATE, env=self.env)

    def test_naive_expiry_refused(self):
        # No timezone: fail closed instead of guessing.
        self._write(version=1, action=auth.ACTION_TASK_CREATE, expires_at="2030-01-01T00:00:00")
        with self.assertRaises(AuthorizationError):
            auth.load_authorization(auth.ACTION_TASK_CREATE, env=self.env)

    def test_z_suffix_expiry_accepted(self):
        self._write(version=1, action=auth.ACTION_TASK_CREATE, expires_at="2030-01-01T00:00:00Z")
        spec = auth.load_authorization(auth.ACTION_TASK_CREATE, env=self.env)
        self.assertIsNotNone(spec.expires_at)

    def test_spend_cap_enforced(self):
        from decimal import Decimal
        spec = auth.AuthorizationSpec(
            action="task_create", expires_at=datetime.now(timezone.utc),
            max_reward_usdc=Decimal("5"), max_duration_hours=24,
        )
        with self.assertRaises(AuthorizationError):
            auth.enforce_spend(spec, Decimal("5.000001"), 24)
        with self.assertRaises(AuthorizationError):
            auth.enforce_spend(spec, Decimal("5"), 25)
        auth.enforce_spend(spec, Decimal("5"), 24)  # exactly at cap is allowed

    def test_task_binding_enforced(self):
        spec = auth.AuthorizationSpec(
            action="task_submit", expires_at=datetime.now(timezone.utc), task_id="0xabc",
        )
        with self.assertRaises(AuthorizationError):
            auth.enforce_task_binding(spec, "0xdef")
        auth.enforce_task_binding(spec, "0xabc")


if __name__ == "__main__":
    unittest.main()
