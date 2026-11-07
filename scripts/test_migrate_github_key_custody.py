#!/usr/bin/env python3
"""Synthetic GitHub API failures and reconciliation tests; no network writes."""
import base64
import copy
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock
import zipfile

SPEC = importlib.util.spec_from_file_location("relay", Path(__file__).with_name("migrate_github_key_custody.py"))
m = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(m)
c = m.c
REVISION = "a" * 40


class Api:
    def __init__(self):
        self.calls = []
        self.data = {}
        self.collections = {}

    def api(self, path, method="GET", data=None, binary=False):
        self.calls.append((path, method, data))
        return copy.deepcopy(self.data[path])

    def pages(self, path, field):
        self.calls.append((path, "GET", None))
        return copy.deepcopy(self.collections[(path, field)])


class RelayTest(unittest.TestCase):
    def setUp(self):
        self.api = Api()
        self.plan = {"schema": 1, "repository": c.REPOSITORY, "nonce": "b" * 64,
            "expires_at": "2099-01-01T00:00:00+00:00", "environments": {
                name: {"id": index + 1, "key_id": "123", "key": base64.b64encode(bytes([index + 1]) * 32).decode()}
                for index, name in enumerate(sorted(m.EXPECTED))}}
        for name, values in self.plan["environments"].items():
            path = f"{m.PREFIX}/environments/{name}"
            self.api.data[path] = {"id": values["id"], "name": name,
                "deployment_branch_policy": {"protected_branches": False, "custom_branch_policies": True}}
            self.api.data[path + "/secrets/public-key"] = {key: values[key] for key in ("key", "key_id")}
            self.api.collections[path + "/deployment-branch-policies", "branch_policies"] = [{"name": "main", "type": "branch"}]
            self.api.collections[path + "/variables", "variables"] = []
            self.api.collections[path + "/secrets", "secrets"] = [
                {"name": name, "created_at": "2026-11-07T16:00:00Z", "updated_at": "2026-11-07T16:00:00Z"}
                for name in m.EXPECTED[name]]

    def run_fixture(self, run_id="1234"):
        self.api.data[f"{m.PREFIX}/actions/runs/{run_id}"] = {
            "id": int(run_id), "head_sha": REVISION, "head_branch": "main", "event": "workflow_dispatch",
            "run_attempt": 1, "status": "completed", "conclusion": "success", "path": c.WORKFLOW,
            "repository": {"full_name": c.REPOSITORY}, "actor": {"login": "NSPG13"},
            "triggering_actor": {"login": "NSPG13"}}
        self.api.collections[f"{m.PREFIX}/actions/runs/{run_id}/jobs", "jobs"] = [
            {"name": slot, "conclusion": "success"} for slot in c.SLOTS]

    def artifact_fixture(self, run_id="1234", verified=False):
        artifacts = []
        for index, slot in enumerate(c.SLOTS):
            value = {**c.binding(self.plan, slot, REVISION, "1234"), "verified_aliases": list(c.SLOTS[slot][1]),
                     "verified": True, "verification_run_id": run_id}
            buffer = io.BytesIO()
            with zipfile.ZipFile(buffer, "w") as archive:
                archive.writestr(slot + ("-verified" if verified else "") + ".json", json.dumps(value))
            artifacts.append({"id": index + 1, "name": f"key-custody-{slot}-{run_id}", "expired": False, "size_in_bytes": len(buffer.getvalue())})
            self.api.data[f"{m.PREFIX}/actions/artifacts/{index+1}/zip"] = buffer.getvalue()
        self.api.collections[f"{m.PREFIX}/actions/runs/{run_id}/artifacts", "artifacts"] = artifacts

    def test_correct_main_only_environments_and_exact_aliases(self):
        result = m.environment_controls(self.api, self.plan, filled=True)
        self.assertEqual(sum(len(v["secrets"]) for v in result.values()), 8)
        self.assertTrue(all(method == "GET" for _, method, _ in self.api.calls))

    def test_missing_alias_widened_branch_wrong_id_key_or_override_refused(self):
        path = f"{m.PREFIX}/environments/legacy-keeper"
        mutations = [
            lambda: self.api.collections.__setitem__((path + "/secrets", "secrets"), []),
            lambda: self.api.collections[path + "/deployment-branch-policies", "branch_policies"].append({"name": "*", "type": "branch"}),
            lambda: self.api.collections.__setitem__((path + "/deployment-branch-policies", "branch_policies"), [{"name": "main", "type": "tag"}]),
            lambda: self.api.data[path].__setitem__("id", 999),
            lambda: self.api.data[path + "/secrets/public-key"].__setitem__("key_id", "999"),
            lambda: self.api.collections[path + "/variables", "variables"].append({"name": "KEY_CUSTODY_MIGRATION_REVISION"}),
        ]
        for index, mutate in enumerate(mutations):
            self.setUp()
            mutate()
            with self.subTest(index=index), self.assertRaises(c.Refused):
                m.environment_controls(self.api, self.plan, filled=True)

    def test_unknown_existing_secret_is_never_overwritten(self):
        path = f"{m.PREFIX}/environments/legacy-verifier-one/secrets"
        self.api.collections[path, "secrets"].append({"name": "UNEXPECTED", "created_at": "x", "updated_at": "x"})
        with self.assertRaises(c.Refused):
            m.environment_controls(self.api, self.plan, filled=False)

    def test_only_exact_successful_owner_workflow_run_is_accepted(self):
        self.run_fixture()
        m.read_run(self.api, "1234", REVISION)
        for field, bad in {"head_sha": "b" * 40, "head_branch": "other", "event": "pull_request_target",
                "run_attempt": 2, "status": "in_progress", "conclusion": "failure", "path": "another.yml",
                "repository": {"full_name": "elsewhere/repo"}, "actor": {"login": "other"},
                "triggering_actor": {"login": "other"}}.items():
            self.run_fixture()
            self.api.data[f"{m.PREFIX}/actions/runs/1234"][field] = bad
            with self.subTest(field=field), self.assertRaises(c.Refused):
                m.read_run(self.api, "1234", REVISION)
        self.run_fixture()
        self.api.collections[f"{m.PREFIX}/actions/runs/1234/jobs", "jobs"][0]["conclusion"] = "skipped"
        with self.assertRaises(c.Refused):
            m.read_run(self.api, "1234", REVISION)

    def test_verification_receipts_bind_all_aliases_and_source_run(self):
        self.run_fixture("1235")
        self.artifact_fixture("1235", verified=True)
        result, _, _ = m.verify_receipts(self.api, self.plan, REVISION, "1234", "1235")
        self.assertEqual(len(result), 4)
        with self.assertRaises(c.Refused):
            m.verify_receipts(self.api, self.plan, REVISION, "1233", "1235")
        with self.assertRaises(c.Refused):
            m.verify_receipts(self.api, self.plan, REVISION, "1235", "1235")

    def test_artifact_traversal_duplicate_extra_or_oversize_refused(self):
        for names in (["../keeper.json"], ["keeper.json", "extra.json"], ["keeper.json", "keeper.json"]):
            self.artifact_fixture()
            buffer = io.BytesIO()
            with zipfile.ZipFile(buffer, "w") as archive:
                for name in names:
                    archive.writestr(name, "{}")
            self.api.data[f"{m.PREFIX}/actions/artifacts/1/zip"] = buffer.getvalue()
            with self.subTest(names=names), self.assertRaises(c.Refused):
                m.receipts(self.api, "1234")
        self.artifact_fixture()
        self.api.collections[f"{m.PREFIX}/actions/runs/1234/artifacts", "artifacts"][0]["size_in_bytes"] = 999999
        with self.assertRaises(c.Refused):
            m.receipts(self.api, "1234")

    def test_workflow_hold_inventory_and_active_job_refusal(self):
        paths = []
        for path in (m.ROOT / ".github/workflows").iterdir():
            if path.suffix in {".yml", ".yaml"} and path.name != "github-key-custody.yml":
                text = path.read_text().lower()
                if any(key.lower() in text for key in m.ORIGINALS | {"secrets.verifier_private_key", "secrets.signer_private_key"}):
                    paths.append(path.relative_to(m.ROOT).as_posix())
        self.api.data[f"{m.PREFIX}/git/ref/heads/main"] = {"object": {"sha": REVISION}}
        self.api.collections[f"{m.PREFIX}/actions/workflows", "workflows"] = [
            {"id": i + 1, "path": path, "state": "disabled_manually"} for i, path in enumerate(paths)]
        for status in ("queued", "in_progress", "waiting", "pending", "requested"):
            self.api.collections[f"{m.PREFIX}/actions/runs?status={status}", "workflow_runs"] = []
        with mock.patch.object(m.subprocess, "run", return_value=SimpleNamespace(stdout=REVISION)):
            self.assertGreaterEqual(len(m.held_consumers(self.api, REVISION)), 22)
            self.api.collections[f"{m.PREFIX}/actions/workflows", "workflows"][0]["state"] = "active"
            with self.assertRaises(c.Refused):
                m.held_consumers(self.api, REVISION)
            self.api.collections[f"{m.PREFIX}/actions/workflows", "workflows"][0]["state"] = "disabled_manually"
            self.api.collections[f"{m.PREFIX}/actions/runs?status=waiting", "workflow_runs"] = [{"workflow_id": 1}]
            with self.assertRaises(c.Refused):
                m.held_consumers(self.api, REVISION)

    def test_cli_never_exports_auth_token_and_sends_only_supplied_ciphertext(self):
        api = m.GitHub("gh")
        with mock.patch.object(m.subprocess, "run", return_value=SimpleNamespace(returncode=0, stdout=b"")) as command:
            api.api("repos/owner/repo/environments/test/secrets/NAME", "PUT", {"key_id": "123", "encrypted_value": "ciphertext"})
        args, kwargs = command.call_args
        self.assertEqual(args[0][0:4], ["gh", "api", "--method", "PUT"])
        self.assertNotIn("token", args[0])
        self.assertEqual(json.loads(kwargs["input"]), {"key_id": "123", "encrypted_value": "ciphertext"})

    def test_staged_import_verification_partial_retirement_and_cleanup(self):
        self.run_fixture("1234")
        source = self.api.data[f"{m.PREFIX}/actions/runs/1234"]
        source.update(workflow_id=999, created_at="2026-11-07T15:00:00Z", updated_at="2026-11-07T15:05:00Z")
        self.artifact_fixture("1234")
        for index, slot in enumerate(c.SLOTS):
            value = f"{index+1:064x}"
            envelope = c.seal(self.plan, slot, REVISION, "1234", value)
            buffer = io.BytesIO()
            with zipfile.ZipFile(buffer, "w") as archive:
                archive.writestr(slot + ".json", json.dumps(envelope))
            self.api.data[f"{m.PREFIX}/actions/artifacts/{index+1}/zip"] = buffer.getvalue()
        for name in m.EXPECTED:
            self.api.collections[f"{m.PREFIX}/environments/{name}/secrets", "secrets"] = []
        originals = [{"name": name, "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"}
                     for name in sorted(m.ORIGINALS)]
        self.api.collections[f"{m.PREFIX}/actions/secrets", "secrets"] = copy.deepcopy(originals)
        variables = [{"name": "KEY_CUSTODY_MIGRATION_REVISION", "value": REVISION},
                     {"name": "KEY_CUSTODY_MIGRATION_PHASE", "value": "seal"}]
        self.api.collections[f"{m.PREFIX}/actions/variables", "variables"] = variables
        self.api.data[f"{m.PREFIX}/actions/workflows/999"] = {"state": "active"}
        original_api = self.api.api
        writes = []
        fail_delete = [False]

        def mutate(path, method="GET", data=None, binary=False):
            if method == "GET":
                return original_api(path, method, data, binary)
            writes.append((path, method, data))
            if "/environments/" in path and "/secrets/" in path:
                self.assertEqual(set(data), {"encrypted_value", "key_id"})
                environment, name = path.split("/environments/")[1].split("/secrets/")
                rows = self.api.collections[f"{m.PREFIX}/environments/{environment}/secrets", "secrets"]
                rows[:] = [row for row in rows if row["name"] != name]
                rows.append({"name": name, "created_at": "2026-11-07T16:00:00Z", "updated_at": "2026-11-07T16:00:00Z"})
            elif "/actions/variables" in path:
                name = path.rsplit("/", 1)[1] if method == "DELETE" else data["name"]
                variables[:] = [row for row in variables if row["name"] != name]
                if method != "DELETE":
                    variables.append(data)
            elif "/actions/secrets/" in path:
                rows = self.api.collections[f"{m.PREFIX}/actions/secrets", "secrets"]
                if fail_delete[0] and len(rows) == 2:
                    fail_delete[0] = False
                    raise c.Refused()
                rows[:] = [row for row in rows if row["name"] != path.rsplit("/", 1)[1]]
            elif path.endswith("/disable"):
                self.api.data[path.removesuffix("/disable")]["state"] = "disabled_manually"
            elif "/actions/artifacts/" in path:
                artifact_id = int(path.rsplit("/", 1)[1])
                for (collection, field), rows in self.api.collections.items():
                    if field == "artifacts":
                        rows[:] = [row for row in rows if row["id"] != artifact_id]
            else:
                self.fail("unexpected mutation")

        def verification_fixture(run_id, offset):
            self.run_fixture(run_id)
            self.api.data[f"{m.PREFIX}/actions/runs/{run_id}"]["created_at"] = "2098-01-01T00:00:00Z"
            artifacts = []
            for index, slot in enumerate(c.SLOTS):
                receipt = {**c.binding(self.plan, slot, REVISION, "1234"), "verified_aliases": list(c.SLOTS[slot][1]),
                           "verified": True, "verification_run_id": run_id}
                buffer = io.BytesIO()
                with zipfile.ZipFile(buffer, "w") as archive:
                    archive.writestr(slot + "-verified.json", json.dumps(receipt))
                artifact_id = offset + index
                artifacts.append({"id": artifact_id, "name": f"key-custody-{slot}-{run_id}", "expired": False, "size_in_bytes": len(buffer.getvalue())})
                self.api.data[f"{m.PREFIX}/actions/artifacts/{artifact_id}/zip"] = buffer.getvalue()
            self.api.collections[f"{m.PREFIX}/actions/runs/{run_id}/artifacts", "artifacts"] = artifacts

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            before = root / "before.json"
            before.write_text(json.dumps({"repository_secrets": {"secrets": originals}}))
            args = ["relay", "inspect", "--gh", "gh", "--revision", REVISION, "--source-run", "1234",
                    "--before", str(before), "--evidence", str(root / "evidence")]
            with mock.patch.object(m, "GitHub", return_value=self.api), \
                 mock.patch.object(self.api, "api", side_effect=mutate), \
                 mock.patch.object(c, "read_json", return_value=self.plan), \
                 mock.patch.object(m, "held_consumers", return_value=[{"state": "disabled_manually"}]), \
                 contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                for phase in ("inspect", "import"):
                    args[1] = phase
                    with mock.patch.object(sys, "argv", args):
                        self.assertEqual(m.main(), 0, phase)
                puts = [row for row in writes if "/environments/" in row[0]]
                self.assertEqual(len(puts), 8)
                self.assertFalse(any(method == "DELETE" for _, method, _ in writes))
                verification_fixture("1235", 10)
                args += ["--verification-run", "1235"]
                args[1] = "verify"
                with mock.patch.object(sys, "argv", args):
                    self.assertEqual(m.main(), 0)
                fail_delete[0] = True
                args[1] = "retire"
                with mock.patch.object(sys, "argv", args):
                    self.assertEqual(m.main(), 1)
                    self.assertEqual(len(self.api.collections[f"{m.PREFIX}/actions/secrets", "secrets"]), 2)
                    self.assertEqual(m.main(), 0)
                verification_fixture("1236", 20)
                args[1] = "complete"
                args[-1] = "1236"
                with mock.patch.object(sys, "argv", args):
                    self.assertEqual(m.main(), 0)
                self.assertEqual(variables, [])
                self.assertEqual(self.api.data[f"{m.PREFIX}/actions/workflows/999"]["state"], "disabled_manually")
                self.assertTrue((root / "evidence/complete-before-cleanup.json").exists())
                self.assertTrue((root / "evidence/complete.json").exists())
                self.assertFalse(any(path.endswith("/enable") for path, _, _ in writes))


if __name__ == "__main__":
    unittest.main()
