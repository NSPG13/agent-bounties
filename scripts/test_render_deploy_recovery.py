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

    def get_build_log_summary(self, service, deploy):
        return {
            "available": True,
            "classifications": ["compile"],
            "excerpts": ["error: could not compile api"],
            "log_count": 1,
            "content_sha256": "a" * 64,
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


class EnvironmentClient:
    def __init__(self, changed=True) -> None:
        self.changed = changed
        self.calls = []

    def ensure_env_var(self, service, key, value):
        self.calls.append((service["name"], key, value))
        return {"key": key, "value": value, "changed": self.changed}


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

    def test_public_base_urls_require_bare_https_origins(self) -> None:
        self.assertEqual(
            recovery.normalize_public_base_url(
                "PUBLIC_BASE_URL", " https://api.bountyboard.global/ "
            ),
            "https://api.bountyboard.global",
        )
        for value in (
            "http://api.bountyboard.global",
            "https://user@api.bountyboard.global",
            "https://api.bountyboard.global:8443",
            "https://api.bountyboard.global/path",
            "https://api.bountyboard.global?query=1",
        ):
            with self.subTest(value=value), self.assertRaises(recovery.RecoveryError):
                recovery.normalize_public_base_url("PUBLIC_BASE_URL", value)

    def test_leaderboard_environment_requires_exact_addresses(self) -> None:
        values = recovery.leaderboard_environment_values(
            "0x" + "AA" * 20,
            "0x" + "bb" * 20,
        )
        self.assertEqual(
            values,
            {
                "BASE_MAINNET_LEADERBOARD_REWARD_CONTRACT": "0x" + "aa" * 20,
                "BASE_SEPOLIA_LEADERBOARD_REWARD_CONTRACT": "0x" + "bb" * 20,
            },
        )
        for value in ("", "0x1234", "0x" + "zz" * 20):
            with self.subTest(value=value), self.assertRaises(recovery.RecoveryError):
                recovery.leaderboard_environment_values(value, None)

    def test_omitted_leaderboard_environment_stays_omitted(self) -> None:
        self.assertEqual(recovery.leaderboard_environment_values(None, None), {})

    def test_matching_public_env_var_is_verified_without_mutation(self) -> None:
        expected = {
            "key": "PUBLIC_BASE_URL",
            "value": "https://api.bountyboard.global",
        }
        client = RecordingClient()
        client._read_with_retry = lambda _path: expected
        result = client.ensure_env_var(
            {"id": "srv-api", "name": "agent-bounties-api"},
            expected["key"],
            expected["value"],
        )
        self.assertFalse(result["changed"])
        self.assertEqual(client.requests, [])

    def test_stale_public_env_var_is_updated_and_read_back(self) -> None:
        expected = {
            "key": "MCP_BASE_URL",
            "value": "https://mcp.bountyboard.global",
        }
        reads = iter(
            [
                {"key": "MCP_BASE_URL", "value": "https://old.example"},
                expected,
            ]
        )
        client = RecordingClient(response=expected)
        client._read_with_retry = lambda _path: next(reads)
        result = client.ensure_env_var(
            {"id": "srv-mcp", "name": "agent-bounties-mcp"},
            expected["key"],
            expected["value"],
        )
        self.assertTrue(result["changed"])
        self.assertEqual(
            client.requests,
            [
                (
                    "PUT",
                    "/services/srv-mcp/env-vars/MCP_BASE_URL",
                    {"value": expected["value"]},
                )
            ],
        )

    def test_changed_environment_forces_same_revision_redeploy(self) -> None:
        revision = "a" * 40
        client = RecordingClient(
            deploys=[
                {
                    "deploy": {
                        "id": "dep-live",
                        "status": "live",
                        "commit": {"id": revision},
                    }
                }
            ],
            response={
                "id": "dep-new",
                "status": "created",
                "commit": {"id": revision},
            },
        )
        result = client.ensure_deploy(
            {"id": "srv-api"}, revision, force=True
        )
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

    def test_cloud_environment_reconciliation_never_returns_secret_value(self) -> None:
        client = EnvironmentClient()
        service = {"id": "srv-api", "name": "agent-bounties-api"}
        runtime, secrets, changed = recovery.reconcile_cloud_agent_environment(
            client,
            service,
            "  model-secret-value  ",
        )

        self.assertTrue(changed)
        self.assertEqual(len(runtime), len(recovery.CLOUD_AGENT_RUNTIME_ENVIRONMENT))
        self.assertEqual(
            secrets,
            [
                {
                    "service": "agent-bounties-api",
                    "key": "CLOUD_AGENT_API_KEY",
                    "configured": True,
                    "changed": True,
                }
            ],
        )
        self.assertEqual(client.calls[-1], (
            "agent-bounties-api",
            "CLOUD_AGENT_API_KEY",
            "model-secret-value",
        ))
        self.assertNotIn("model-secret-value", recovery.json.dumps([runtime, secrets]))

    def test_missing_cloud_secret_is_never_written_or_invented(self) -> None:
        client = EnvironmentClient(changed=False)
        service = {"id": "srv-api", "name": "agent-bounties-api"}
        runtime, secrets, changed = recovery.reconcile_cloud_agent_environment(
            client,
            service,
            None,
        )

        self.assertFalse(changed)
        self.assertEqual(len(runtime), len(recovery.CLOUD_AGENT_RUNTIME_ENVIRONMENT))
        self.assertEqual(secrets, [])
        self.assertNotIn("CLOUD_AGENT_API_KEY", [key for _, key, _ in client.calls])

    def test_public_env_var_update_fails_when_readback_drifts(self) -> None:
        client = RecordingClient(
            response={
                "key": "PUBLIC_BASE_URL",
                "value": "https://api.bountyboard.global",
            }
        )
        client._read_with_retry = lambda _path: {
            "key": "PUBLIC_BASE_URL",
            "value": "https://old.example",
        }
        with self.assertRaisesRegex(recovery.RecoveryError, "did not retain"):
            client.ensure_env_var(
                {"id": "srv-api", "name": "agent-bounties-api"},
                "PUBLIC_BASE_URL",
                "https://api.bountyboard.global",
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

    def test_build_log_summary_classifies_and_redacts(self) -> None:
        result = recovery.summarize_build_logs(
            {
                "logs": [
                    {"message": "==> Root directory ./service is missing"},
                    {"message": "error: package requires rustc 1.95 or newer"},
                    {"message": "DATABASE_URL=postgres://user:do-not-print@example/db failed"},
                ]
            }
        )
        self.assertIn("configuration", result["classifications"])
        self.assertIn("missing_file", result["classifications"])
        self.assertIn("rust_toolchain", result["classifications"])
        self.assertIn("==> Root directory ./service is missing", result["excerpts"])
        self.assertIn("[sensitive build diagnostic redacted]", result["excerpts"])
        self.assertNotIn("do-not-print", recovery.json.dumps(result))
        self.assertRegex(result["content_sha256"], r"^[0-9a-f]{64}$")

    def test_build_log_query_is_scoped_to_service_and_workspace(self) -> None:
        client = RecordingClient()
        paths = []
        client._read_with_retry = lambda path: (
            paths.append(path) or {"logs": [{"message": "failed to solve build"}]}
        )
        result = client.get_build_log_summary(
            {"id": "srv-api", "ownerId": "tea-owner123"},
            {"createdAt": "2026-07-18T07:31:00Z"},
        )
        query = recovery.urllib.parse.parse_qs(
            recovery.urllib.parse.urlsplit(paths[0]).query
        )
        self.assertEqual(query["ownerId"], ["tea-owner123"])
        self.assertEqual(query["resource"], ["srv-api"])
        self.assertEqual(query["type"], ["build"])
        self.assertEqual(query["startTime"], ["2026-07-18T07:31:00Z"])
        self.assertIn("docker", result["classifications"])

    def test_build_pipeline_quota_is_classified(self) -> None:
        result = recovery.summarize_build_logs(
            {
                "logs": [
                    {
                        "message": "Build canceled: your workspace has run out of build pipeline minutes."
                    }
                ]
            }
        )
        self.assertEqual(result["classifications"], ["pipeline_quota"])

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
            {"agent-bounties-api": ({"id": "srv-api"}, "dep-api")},
            "a" * 40,
            timeout_seconds=20,
            poll_seconds=2,
            clock=clock,
            sleeper=clock.sleep,
        )
        self.assertEqual(result["agent-bounties-api"]["status"], "live")

    def test_poll_fails_closed_on_build_failure(self) -> None:
        client = FakeClient(["build_failed"])
        with self.assertRaisesRegex(recovery.RenderDeployFailure, "dep-api.*build_failed") as caught:
            recovery.poll_deploys(
                client,
                {"agent-bounties-api": ({"id": "srv-api"}, "dep-api")},
                "a" * 40,
                timeout_seconds=20,
                poll_seconds=2,
            )
        self.assertEqual(caught.exception.evidence["deploy_id"], "dep-api")
        self.assertEqual(
            caught.exception.evidence["build_logs"]["classifications"],
            ["compile"],
        )

    def test_poll_timeout_is_bounded(self) -> None:
        client = FakeClient(["queued"])
        clock = FakeClock()
        with self.assertRaisesRegex(recovery.RecoveryError, "timed out"):
            recovery.poll_deploys(
                client,
                {"agent-bounties-api": ({"id": "srv-api"}, "dep-api")},
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

    def test_cloud_readiness_requires_supplied_credential_to_be_live(self) -> None:
        unavailable = {
            "schema_version": "agent-bounties/cloud-agent-readiness-v1",
            "available": False,
            "execution": "hosted_cloud_api",
            "provider": "openai-compatible",
            "model": "gpt-4.1-mini",
            "public_drafts": True,
            "local_fallback": False,
            "authority": "draft_only",
            "missing_configuration": ["CLOUD_AGENT_API_KEY"],
        }
        observed = recovery.validate_cloud_agent_readiness(
            unavailable,
            credential_supplied=False,
        )
        self.assertFalse(observed["available"])
        with self.assertRaisesRegex(recovery.RecoveryError, "did not become ready"):
            recovery.validate_cloud_agent_readiness(
                unavailable,
                credential_supplied=True,
            )

        ready = dict(unavailable, available=True, missing_configuration=[])
        observed = recovery.validate_cloud_agent_readiness(
            ready,
            credential_supplied=True,
        )
        self.assertTrue(observed["available"])
        self.assertNotIn("CLOUD_AGENT_API_KEY", recovery.json.dumps(observed))

    def test_cloud_readiness_cannot_claim_a_local_fallback(self) -> None:
        payload = {
            "schema_version": "agent-bounties/cloud-agent-readiness-v1",
            "available": True,
            "execution": "hosted_cloud_api",
            "local_fallback": True,
            "authority": "draft_only",
            "missing_configuration": [],
        }
        with self.assertRaisesRegex(recovery.RecoveryError, "local fallback"):
            recovery.validate_cloud_agent_readiness(
                payload,
                credential_supplied=False,
            )

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
