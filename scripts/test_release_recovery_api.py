"""Offline source/check admission, one-write release and attestation fixtures."""
import copy
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from scripts import release_recovery_api as release

REVISION = "a" * 40
OTHER = "b" * 40
DEPLOY = "dep-" + "a" * 20


def event():
    return {"repository": {"full_name": release.REPOSITORY}, "workflow_run": {
        "name": "CI", "head_sha": REVISION, "head_branch": "main", "event": "push",
        "status": "completed", "conclusion": "success",
        "head_repository": {"full_name": release.REPOSITORY}}}


def service():
    return {"id": release.SERVICE, "ownerId": release.OWNER,
        "name": "agentizable-api-recovery-20261109", "type": "web_service",
        "repo": "https://github.com/NSPG13/agent-bounties", "branch": "main", "rootDir": "",
        "suspended": "not_suspended", "autoDeploy": "no", "autoDeployTrigger": "off",
        "serviceDetails": {"runtime": "docker", "healthCheckPath": "/health",
            "envSpecificDetails": {"dockerfilePath": "./Dockerfile.recovery-api",
                "dockerContext": ".", "dockerCommand": "",
                "preDeployCommand": "/usr/local/bin/check-recovery-revision --built-revision"}}}


def record(status="live", revision=REVISION, deploy_id=DEPLOY):
    return {"id": deploy_id, "status": status, "commit": {"id": revision}}


class GitHub:
    def __init__(self):
        self.calls = []
        self.main = REVISION
        self.protected = True
        self.change_after = None
        self.runs = {}
        for filename, name in (("ci.yml", "CI"), ("containers.yml", "Containers")):
            self.runs[filename] = [{**event()["workflow_run"], "id": 7, "run_attempt": 1, "name": name}]

    def request(self, method, path, payload=None):
        self.calls.append((method, path, payload))
        assert method == "GET"
        if path.endswith("/branches/main"):
            if self.change_after and len(self.calls) >= self.change_after:
                self.main = OTHER
            return {"name": "main", "protected": self.protected, "commit": {"sha": self.main}}
        for filename, runs in self.runs.items():
            if "/" + filename + "/runs?" in path:
                assert "head_sha=" + REVISION in path and "branch=main" in path and "event=push" in path
                return {"workflow_runs": copy.deepcopy(runs), "total_count": len(runs)}
        raise AssertionError("Unexpected GitHub request")


class Render:
    def __init__(self):
        self.calls = []
        self.service = service()
        self.history = [{"deploy": record(revision=OTHER)}]
        self.polls = [record()]
        self.post_error = False

    def request(self, method, path, payload=None):
        self.calls.append((method, path, payload))
        if method == "GET" and path == "/services/" + release.SERVICE:
            return copy.deepcopy(self.service)
        if method == "GET" and path.endswith("/deploys?limit=100"):
            return copy.deepcopy(self.history)
        if method == "POST" and path == "/services/" + release.SERVICE + "/deploys":
            if self.post_error:
                raise release.ReleaseError("api_request_failed_no_mutation_retry")
            return record("build_in_progress")
        if method == "GET" and path == "/services/" + release.SERVICE + "/deploys/" + DEPLOY:
            return copy.deepcopy(self.polls.pop(0))
        raise AssertionError("Unexpected Render request")


class AdmissionTests(unittest.TestCase):
    def test_http_errors_never_expose_credentials_or_provider_bodies(self):
        client = release.Client("https://api.render.com/v1", "fixture-secret-never-print")
        opener = unittest.mock.Mock()
        opener.open.side_effect = release.urllib.error.URLError("fixture-secret-never-print")
        with patch.object(release.urllib.request, "build_opener", return_value=opener), \
             self.assertRaisesRegex(release.ReleaseError, "^api_request_failed_no_mutation_retry$"):
            client.request("POST", "/services/" + release.SERVICE + "/deploys", {"commitId": REVISION})
        self.assertEqual(opener.open.call_count, 1)
        self.assertIsNone(release.NoRedirect().redirect_request(None, None, 302, "", {}, "https://other.invalid"))

    def test_real_health_probe_requires_exact_body_revision_and_status(self):
        for status, body, revision, valid in ((200, b"ok", REVISION, True), (200, b"ok", OTHER, False),
                (200, b"ok", None, False), (200, b"unavailable", REVISION, False), (503, b"ok", REVISION, False)):
            response = unittest.mock.MagicMock()
            response.__enter__.return_value = response
            response.status, response.headers = status, {"x-agent-bounties-revision": revision}
            response.read.return_value = body
            opener = unittest.mock.Mock()
            opener.open.return_value = response
            with self.subTest(status=status, body=body, revision=revision), \
                 patch.object(release.urllib.request, "build_opener", return_value=opener):
                if valid:
                    release.verify_health(REVISION)
                else:
                    with self.assertRaises(release.ReleaseError):
                        release.verify_health(REVISION)
                request = opener.open.call_args.args[0]
                self.assertEqual(request.full_url, release.HEALTH)
                self.assertNotIn("Authorization", request.headers)

    def test_only_successful_same_repository_main_push_events_enter(self):
        self.assertEqual(release.trigger_revision(event()), REVISION)
        for field, value in (("name", "evil"), ("head_sha", "main"), ("head_sha", "$(echo bad)"),
            ("head_branch", "recovery"), ("event", "pull_request"), ("status", "queued"),
            ("conclusion", "failure"), ("head_repository", {"full_name": "fork/agent-bounties"})):
            candidate = event()
            candidate["workflow_run"][field] = value
            with self.subTest(field=field), self.assertRaises(release.ReleaseError):
                release.trigger_revision(candidate)

    def test_requires_both_latest_exact_sha_push_runs(self):
        github = GitHub()
        self.assertEqual(release.qualify(github, REVISION), "qualified")
        for field, value in (("head_sha", OTHER), ("head_branch", "branch"), ("event", "pull_request"),
            ("head_repository", {"full_name": "fork/repo"}), ("status", "in_progress"), ("conclusion", "failure")):
            github = GitHub()
            github.runs["containers.yml"][0][field] = value
            with self.subTest(field=field):
                self.assertEqual(release.qualify(github, REVISION), "awaiting_exact_sha_checks")
        github = GitHub()
        github.runs["ci.yml"].append({**github.runs["ci.yml"][0], "id": 8, "status": "queued", "conclusion": None})
        self.assertEqual(release.qualify(github, REVISION), "awaiting_exact_sha_checks")

    def test_protection_and_main_rechecked_after_checks(self):
        github = GitHub()
        github.protected = False
        with self.assertRaisesRegex(release.ReleaseError, "main_must_be_protected"):
            release.qualify(github, REVISION)
        github = GitHub()
        github.change_after = 4
        self.assertEqual(release.qualify(github, REVISION), "stale_main")

    def test_unqualified_cli_never_constructs_render_client_or_uses_credential(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "event.json").write_text(json.dumps(event()))
            github = GitHub()
            github.runs["containers.yml"] = []
            with patch.dict(os.environ, {"GITHUB_EVENT_PATH": str(root / "event.json"), "GH_TOKEN": "fixture",
                                        "GITHUB_OUTPUT": str(root / "output"), "RENDER_API_KEY": "must-not-use"}), \
                 patch.object(release, "Client", return_value=github) as clients, \
                 patch("sys.argv", ["release", "deploy", "--evidence", str(root / "evidence.json")]):
                self.assertEqual(release.main(), 0)
                clients.assert_called_once_with("https://api.github.com", "fixture")
                self.assertNotIn("must-not-use", (root / "evidence.json").read_text())


class ReleaseTests(unittest.TestCase):
    def run_release(self, github=None, render=None, clock=lambda: 0):
        github, render = github or GitHub(), render or Render()
        health = unittest.mock.Mock()
        result = release.release(github, render, REVISION, sleep=lambda _: None, clock=clock, health=health)
        return result, render.calls, health

    def test_exactly_one_post_only_fixed_api_and_post_live_health_proof(self):
        result, calls, health = self.run_release()
        self.assertEqual(result, {"outcome": "live", "deploy_id": DEPLOY})
        self.assertEqual([call for call in calls if call[0] != "GET"], [
            ("POST", "/services/" + release.SERVICE + "/deploys", {"commitId": REVISION, "clearCache": "do_not_clear"})])
        self.assertTrue(all(path.startswith("/services/" + release.SERVICE) for _, path, _ in calls))
        health.assert_called_once_with(REVISION)

    def test_all_service_identity_and_recipe_drift_denied_before_post(self):
        for key in service():
            render = Render()
            render.service[key] = {} if key == "serviceDetails" else "wrong"
            with self.subTest(key=key), self.assertRaises(release.ReleaseError):
                self.run_release(render=render)
            self.assertFalse(any(call[0] != "GET" for call in render.calls))
        for key in service()["serviceDetails"]["envSpecificDetails"]:
            render = Render()
            render.service["serviceDetails"]["envSpecificDetails"][key] = "wrong"
            with self.subTest(key=key), self.assertRaises(release.ReleaseError):
                self.run_release(render=render)

    def test_live_and_existing_same_sha_deploy_never_repost(self):
        for status, outcome in (("live", "already_live"), ("build_in_progress", "live")):
            render = Render()
            render.history = [{"deploy": record(status)}]
            result, calls, health = self.run_release(render=render)
            self.assertEqual(result["outcome"], outcome)
            self.assertFalse(any(method != "GET" for method, _, _ in calls))
            health.assert_called_once_with(REVISION)

    def test_busy_other_revision_and_new_main_skip_without_post(self):
        render = Render()
        render.history = [{"deploy": record("build_in_progress", OTHER)}]
        self.assertEqual(self.run_release(render=render)[0]["outcome"], "service_busy")
        github, render = GitHub(), Render()
        github.change_after = 5
        self.assertEqual(self.run_release(github, render)[0]["outcome"], "stale_main")
        self.assertFalse(any(call[0] != "GET" for call in render.calls))

    def test_post_transport_failure_is_never_retried(self):
        render = Render()
        render.post_error = True
        with self.assertRaises(release.ReleaseError):
            self.run_release(render=render)
        self.assertEqual(sum(call[0] == "POST" for call in render.calls), 1)

    def test_second_workflow_event_cannot_retry_failed_or_unknown_same_sha_attempt(self):
        for status in ("build_failed", "pre_deploy_failed", "update_failed", "canceled", "deactivated", "unknown"):
            render = Render()
            render.history = [{"deploy": record(status)}]
            for _ in range(2):
                with self.subTest(status=status), self.assertRaises(release.ReleaseError):
                    self.run_release(render=render)
            self.assertFalse(any(call[0] != "GET" for call in render.calls))

    def test_failed_older_revision_does_not_block_a_new_main_commit(self):
        render = Render()
        render.history = [{"deploy": record("build_failed", OTHER)}]
        self.assertEqual(self.run_release(render=render)[0]["outcome"], "live")

    def test_wrong_commit_failure_unknown_status_and_timeout_do_not_attest(self):
        for poll in (record(revision=OTHER), record("build_failed"), record("surprise"), record(deploy_id="dep-" + "b" * 20)):
            render = Render()
            render.polls = [poll]
            with self.subTest(poll=poll), self.assertRaises(release.ReleaseError):
                self.run_release(render=render)
        times = iter([0, 1801])
        with self.assertRaisesRegex(release.ReleaseError, "deploy_monitor_timeout"):
            self.run_release(clock=lambda: next(times))

    def test_health_revision_mismatch_keeps_release_failed(self):
        with self.assertRaisesRegex(release.ReleaseError, "api_health_revision_mismatch"):
            release.release(GitHub(), Render(), REVISION, sleep=lambda _: None,
                health=unittest.mock.Mock(side_effect=release.ReleaseError("api_health_revision_mismatch")))


if __name__ == "__main__":
    unittest.main()
