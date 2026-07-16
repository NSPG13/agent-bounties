from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).with_name("render_deploy_recovery.py")
SPEC = importlib.util.spec_from_file_location("render_deploy_recovery", SCRIPT)
assert SPEC and SPEC.loader
recovery = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = recovery
SPEC.loader.exec_module(recovery)


class FakeClock:
    def __init__(self) -> None:
        self.value = 0.0

    def __call__(self) -> float:
        return self.value

    def sleep(self, seconds: float) -> None:
        self.value += seconds


class FakeClient:
    def __init__(self, statuses: list[str]) -> None:
        self.statuses = statuses
        self.calls = 0

    def get_deploy(self, service_id: str, deploy_id: str):
        status = self.statuses[min(self.calls, len(self.statuses) - 1)]
        self.calls += 1
        return {
            "id": deploy_id,
            "status": status,
            "commit": {"id": "a" * 40},
            "trigger": "api",
        }


class RecordingClient(recovery.RenderClient):
    def __init__(self, *, deploys=None, response=None, error=None) -> None:
        self.deploys = [] if deploys is None else deploys
        self.response = response
        self.error = error
        self.requests = []
        self._sleep = lambda _seconds: None

    def list_deploys(self, service_id: str):
        return self.deploys

    def _request_json(self, method: str, path: str, payload=None):
        self.requests.append((method, path, payload))
        if self.error is not None:
            raise self.error
        return self.response


class ResolutionFailureClient:
    def __init__(self) -> None:
        self.resolved = []
        self.mutations = []

    def resolve_service(self, spec):
        self.resolved.append(spec.name)
        if spec.name == "agent-bounties-base-indexer":
            raise recovery.RecoveryError("unexpected repository")
        return {
            "id": f"srv-{len(self.resolved)}",
            "name": spec.name,
            "autoDeploy": False,
        }

    def disable_native_auto_deploy(self, service):
        self.mutations.append(("disable", service["name"]))

    def ensure_deploy(self, service, revision):
        self.mutations.append(("deploy", service["name"]))

    def ensure_custom_domain(self, service, domain):
        self.mutations.append(("domain", service["name"], domain))


class RenderDeployRecoveryTests(unittest.TestCase):
    def test_revision_requires_full_sha(self) -> None:
        self.assertEqual(recovery.validate_revision("A" * 40), "a" * 40)
        for value in ("a" * 39, "a" * 41, "main", "g" * 40):
            with self.subTest(value=value), self.assertRaises(recovery.RecoveryError):
                recovery.validate_revision(value)

    def test_service_resolution_is_exact_and_repository_bound(self) -> None:
        spec = recovery.SERVICE_SPECS[0]
        service = {
            "id": "srv-abc123",
            "name": spec.name,
            "type": spec.service_type,
            "branch": "main",
            "repo": "https://github.com/NSPG13/agent-bounties.git",
        }
        self.assertEqual(recovery.select_service(spec, [{"service": service}]), service)
        wrong = dict(service, repo="https://github.com/attacker/fork")
        with self.assertRaisesRegex(recovery.RecoveryError, "unexpected repository"):
            recovery.select_service(spec, [{"service": wrong}])
        with self.assertRaisesRegex(recovery.RecoveryError, "exactly one"):
            recovery.select_service(spec, [{"service": service}, {"service": service}])

    def test_existing_deploy_reuses_only_active_exact_revision(self) -> None:
        revision = "a" * 40
        payload = [
            {"deploy": {"id": "dep-old", "status": "live", "commit": {"id": "b" * 40}}},
            {"deploy": {"id": "dep-failed", "status": "build_failed", "commit": {"id": revision}}},
            {"deploy": {"id": "dep-current", "status": "queued", "commit": {"id": revision}}},
        ]
        self.assertEqual(recovery.existing_deploy(payload, revision)["id"], "dep-current")

    def test_historical_live_deploy_is_not_current_evidence(self) -> None:
        revision = "a" * 40
        payload = [
            {"deploy": {"id": "dep-new", "status": "live", "commit": {"id": "b" * 40}}},
            {"deploy": {"id": "dep-old", "status": "live", "commit": {"id": revision}}},
        ]
        self.assertIsNone(recovery.existing_deploy(payload, revision))

    def test_trigger_binds_exact_commit_and_does_not_clear_cache(self) -> None:
        revision = "a" * 40
        client = RecordingClient(
            response={
                "id": "dep-new",
                "status": "created",
                "commit": {"id": revision},
            }
        )
        result = client.ensure_deploy({"id": "srv-api"}, revision)
        self.assertEqual(result["id"], "dep-new")
        self.assertEqual(
            client.requests,
            [
                (
                    "POST",
                    "/services/srv-api/deploys",
                    {"clearCache": "do_not_clear", "commitId": revision},
                )
            ],
        )

    def test_custom_domain_is_reused_or_attached_exactly_once(self) -> None:
        existing = RecordingClient()
        existing._read_with_retry = lambda _path: [
            {"customDomain": {"name": "api.bountyboard.global", "verificationStatus": "verified"}}
        ]
        reused = existing.ensure_custom_domain(
            {"id": "srv-api", "name": "agent-bounties-api"},
            "api.bountyboard.global",
        )
        self.assertEqual(reused["verificationStatus"], "verified")
        self.assertEqual(existing.requests, [])

        created = RecordingClient(
            response={"customDomain": {"name": "api.bountyboard.global", "verificationStatus": "pending"}}
        )
        created._read_with_retry = lambda _path: []
        attached = created.ensure_custom_domain(
            {"id": "srv-api", "name": "agent-bounties-api"},
            "api.bountyboard.global",
        )
        self.assertEqual(attached["verificationStatus"], "pending")
        self.assertEqual(
            created.requests,
            [("POST", "/services/srv-api/custom-domains", {"name": "api.bountyboard.global"})],
        )

    def test_duplicate_custom_domains_fail_closed(self) -> None:
        client = RecordingClient()
        client._read_with_retry = lambda _path: [
            {"name": "api.bountyboard.global"},
            {"customDomain": {"name": "API.BOUNTYBOARD.GLOBAL"}},
        ]
        with self.assertRaisesRegex(recovery.RecoveryError, "duplicate"):
            client.ensure_custom_domain(
                {"id": "srv-api", "name": "agent-bounties-api"},
                "api.bountyboard.global",
            )

    def test_rejected_trigger_fails_without_unbounded_retry(self) -> None:
        client = RecordingClient(error=recovery.RenderHttpError(401, "unauthorized"))
        with self.assertRaises(recovery.RenderHttpError):
            client.ensure_deploy({"id": "srv-api"}, "a" * 40)
        self.assertEqual(len(client.requests), 1)

    def test_disable_native_auto_deploy_is_explicit(self) -> None:
        client = RecordingClient(
            response={
                "id": "srv-api",
                "name": "agent-bounties-api",
                "autoDeploy": "no",
            }
        )
        client.disable_native_auto_deploy(
            {"id": "srv-api", "name": "agent-bounties-api", "autoDeploy": True}
        )
        self.assertEqual(
            client.requests,
            [("PATCH", "/services/srv-api", {"autoDeploy": "no"})],
        )

    def test_disabled_render_enum_skips_redundant_update(self) -> None:
        client = RecordingClient(response=None)
        client.disable_native_auto_deploy(
            {"id": "srv-api", "name": "agent-bounties-api", "autoDeploy": "no"}
        )
        self.assertEqual(client.requests, [])

    def test_legacy_boolean_disabled_response_remains_compatible(self) -> None:
        client = RecordingClient(
            response={
                "id": "srv-api",
                "name": "agent-bounties-api",
                "autoDeploy": False,
            }
        )
        client.disable_native_auto_deploy(
            {"id": "srv-api", "name": "agent-bounties-api", "autoDeploy": True}
        )
        self.assertEqual(
            client.requests,
            [("PATCH", "/services/srv-api", {"autoDeploy": "no"})],
        )

    def test_missing_render_key_fails_before_network_access(self) -> None:
        with self.assertRaisesRegex(recovery.RecoveryError, "RENDER_API_KEY"):
            recovery.RenderClient("")

    def test_all_service_bindings_validate_before_any_mutation(self) -> None:
        client = ResolutionFailureClient()
        with self.assertRaisesRegex(recovery.RecoveryError, "unexpected repository"):
            recovery.deploy(
                client,
                "a" * 40,
                deploy_timeout_seconds=1,
                health_timeout_seconds=1,
                poll_seconds=0,
            )
        self.assertEqual(len(client.resolved), 3)
        self.assertEqual(client.mutations, [])

    def test_poll_succeeds_only_after_exact_deploy_is_live(self) -> None:
        client = FakeClient(["build_in_progress", "live"])
        clock = FakeClock()
        result = recovery.poll_deploys(
            client,
            {"agent-bounties-api": ("srv-api", "dep-api")},
            "a" * 40,
            timeout_seconds=20,
            poll_seconds=2,
            clock=clock,
            sleeper=clock.sleep,
        )
        self.assertEqual(result["agent-bounties-api"]["status"], "live")

    def test_poll_fails_closed_on_build_failure(self) -> None:
        client = FakeClient(["build_failed"])
        with self.assertRaisesRegex(recovery.RecoveryError, "build_failed"):
            recovery.poll_deploys(
                client,
                {"agent-bounties-api": ("srv-api", "dep-api")},
                "a" * 40,
                timeout_seconds=20,
                poll_seconds=2,
            )

    def test_poll_timeout_is_bounded(self) -> None:
        client = FakeClient(["queued"])
        clock = FakeClock()
        with self.assertRaisesRegex(recovery.RecoveryError, "timed out"):
            recovery.poll_deploys(
                client,
                {"agent-bounties-api": ("srv-api", "dep-api")},
                "a" * 40,
                timeout_seconds=3,
                poll_seconds=2,
                clock=clock,
                sleeper=clock.sleep,
            )

    def test_health_contract_requires_exact_revision_and_protocol(self) -> None:
        revision = "a" * 40
        response = (
            200,
            "ok\n",
            {
                "x-agent-bounties-revision": revision,
                "x-agent-bounties-protocol": recovery.PROTOCOL,
            },
        )
        result = recovery.validate_health("api", revision, response)
        self.assertEqual(result["revision"], revision)
        wrong = (200, "ok", {**response[2], "x-agent-bounties-revision": "b" * 40})
        with self.assertRaisesRegex(recovery.RecoveryError, "different revision"):
            recovery.validate_health("api", revision, wrong)

    def test_health_transport_bypasses_cache_and_closes_each_connection(self) -> None:
        class Response:
            status = 200
            headers = {
                "X-Agent-Bounties-Revision": "a" * 40,
                "X-Agent-Bounties-Protocol": recovery.PROTOCOL,
            }

            def __enter__(self):
                return self

            def __exit__(self, *_args):
                return None

            @staticmethod
            def read():
                return b"ok"

        with mock.patch.object(
            recovery.urllib.request, "urlopen", return_value=Response()
        ) as urlopen:
            status, body, _headers = recovery.fetch_health(
                "https://example.test/health?existing=1", 5
            )

        request = urlopen.call_args.args[0]
        self.assertEqual(status, 200)
        self.assertEqual(body, "ok")
        self.assertIn("existing=1&_agent_bounties_probe=", request.full_url)
        self.assertEqual(request.get_header("Cache-control"), "no-cache, no-store")
        self.assertEqual(request.get_header("Connection"), "close")

    def test_health_wait_requires_a_stable_exact_revision_window(self) -> None:
        revision = "a" * 40
        exact = (
            200,
            "ok",
            {
                "x-agent-bounties-revision": revision,
                "x-agent-bounties-protocol": recovery.PROTOCOL,
            },
        )
        stale = (
            200,
            "ok",
            {
                "x-agent-bounties-revision": "b" * 40,
                "x-agent-bounties-protocol": recovery.PROTOCOL,
            },
        )
        responses = [exact, exact, stale, exact, exact, exact]
        clock = FakeClock()

        def probe(_url, _timeout):
            return responses.pop(0)

        result = recovery.wait_for_health(
            recovery.SERVICE_SPECS[0],
            revision,
            timeout_seconds=20,
            poll_seconds=1,
            probe=probe,
            clock=clock,
            sleeper=clock.sleep,
            required_consecutive=3,
        )

        self.assertEqual(result["consecutive_exact_probes"], 3)
        self.assertEqual(result["stability_window_seconds"], 2)
        self.assertEqual(responses, [])

    def test_health_wait_fails_when_old_and_new_revisions_keep_alternating(self) -> None:
        revision = "a" * 40
        calls = 0
        clock = FakeClock()

        def probe(_url, _timeout):
            nonlocal calls
            calls += 1
            observed = revision if calls % 2 else "b" * 40
            return (
                200,
                "ok",
                {
                    "x-agent-bounties-revision": observed,
                    "x-agent-bounties-protocol": recovery.PROTOCOL,
                },
            )

        with self.assertRaisesRegex(recovery.RecoveryError, "timed out"):
            recovery.wait_for_health(
                recovery.SERVICE_SPECS[0],
                revision,
                timeout_seconds=4,
                poll_seconds=1,
                probe=probe,
                clock=clock,
                sleeper=clock.sleep,
                required_consecutive=3,
            )

        self.assertGreaterEqual(calls, 5)

    def test_redaction_removes_credentials(self) -> None:
        value = recovery.redact(
            "Authorization: Bearer secret-token https://api.render.com/deploy/srv-x?key=secret"
        )
        self.assertNotIn("secret-token", value)
        self.assertNotIn("key=secret", value)
        self.assertIn("[redacted]", value)
        self.assertEqual(
            recovery.redact("RENDER_API_KEY is required"),
            "RENDER_API_KEY is required",
        )


if __name__ == "__main__":
    unittest.main()
