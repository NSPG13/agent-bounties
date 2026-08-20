import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock


PATH = Path(__file__).with_name("configure_open_competition_v2_beta3_render.py")
SPEC = importlib.util.spec_from_file_location("configure_open_competition_v2_beta3_render", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


def runtime() -> dict:
    return {
        "protocol_version": "agent-bounties/open-competition-v2-beta3",
        "network": "base-mainnet",
        "factory_contract": "0x" + "11" * 20,
        "settlement_token": "0x" + "22" * 20,
        "release_hash": "0x" + "33" * 32,
        "beta_risk_hash": "0x" + "44" * 32,
        "deployment_block": 123,
        "public_creation_enabled": False,
        "proof_broker_enabled": False,
    }


class Beta3RenderTests(unittest.TestCase):
    def test_rpc_preflight_requires_archive_logs_and_common_safe_block(self):
        calls = []

        def rpc(url, method, params, request_id):
            calls.append((url, method, params, request_id))
            if method == "eth_chainId":
                return "0x2105"
            if method == "eth_getLogs":
                return []
            if params[0] == "safe":
                number = 200 if "primary" in url else 198
                return {"number": hex(number), "hash": "0x" + "aa" * 32}
            return {"number": params[0], "hash": "0x" + "bb" * 32}

        value = runtime()
        value["deployment_block"] = 123
        with mock.patch.object(MODULE, "rpc_call", side_effect=rpc):
            result = MODULE.preflight_rpc_pair(
                value, "https://primary.example", "https://shadow.example"
            )

        self.assertTrue(result["passed"])
        self.assertEqual(result["archive_query_from_block"], 123)
        self.assertEqual(result["archive_query_to_block"], 2122)
        self.assertEqual(result["common_safe_block"], 198)
        log_calls = [call for call in calls if call[1] == "eth_getLogs"]
        self.assertEqual(len(log_calls), 2)
        self.assertEqual(log_calls[0][2][0]["fromBlock"], hex(123))
        self.assertEqual(log_calls[0][2][0]["toBlock"], hex(2122))

    def test_rpc_preflight_rejects_provider_log_errors(self):
        value = runtime()
        with mock.patch.object(
            MODULE,
            "rpc_call",
            side_effect=["0x2105", MODULE.Beta3RenderError("eth_getLogs RPC preflight returned an error")],
        ):
            with self.assertRaisesRegex(MODULE.Beta3RenderError, "eth_getLogs"):
                MODULE.preflight_rpc_pair(
                    value, "https://primary.example", "https://shadow.example"
                )

    def test_missing_group_link_is_attached_and_verified(self):
        spec = MODULE.V2_SERVICES[0]
        service = {"id": "srv-api", "name": spec.name}
        group = {"id": "evg-beta3", "serviceLinks": []}
        linked = {
            **group,
            "serviceLinks": [
                {"service": {"id": service["id"], "name": spec.name, "type": "web"}}
            ],
        }
        client = mock.Mock()
        client.get_env_group.side_effect = [group, linked]

        result = MODULE.ensure_group_link(
            client, MODULE.V2_GROUP, group, spec, service
        )

        self.assertEqual(result, linked)
        client._write_with_retry.assert_called_once_with(
            "POST", "/env-groups/evg-beta3/services/srv-api", None
        )

    def test_existing_group_link_is_not_rewritten(self):
        spec = MODULE.V2_SERVICES[0]
        service = {"id": "srv-api", "name": spec.name}
        group = {
            "id": "evg-beta3",
            "serviceLinks": [
                {"service": {"id": service["id"], "name": spec.name, "type": "web"}}
            ],
        }
        client = mock.Mock()
        client.get_env_group.return_value = group

        result = MODULE.ensure_group_link(
            client, MODULE.V2_GROUP, group, spec, service
        )

        self.assertEqual(result, group)
        client._write_with_retry.assert_not_called()

    def test_missing_v2_environment_group_is_created_in_exact_project(self):
        client = mock.Mock()
        client._read_with_retry.return_value = []
        client._write_with_retry.return_value = {"id": "evg-beta3"}
        group = {
            "id": "evg-beta3",
            "name": MODULE.V2_GROUP,
            "ownerId": "tea-owner",
            "environmentId": "evm-production",
            "serviceLinks": [],
        }
        with mock.patch.object(MODULE, "named_group", return_value=group):
            result = MODULE.ensure_v2_group(
                client, "tea-owner", "evm-production"
            )

        self.assertEqual(result, group)
        client._write_with_retry.assert_called_once_with(
            "POST",
            "/env-groups",
            {
                "name": MODULE.V2_GROUP,
                "ownerId": "tea-owner",
                "envVars": [],
                "secretFiles": [],
                "serviceIds": [],
                "environmentId": "evm-production",
            },
        )

    def test_missing_beta3_service_is_materialized_before_revalidation(self):
        services = {
            spec.name: {
                "id": f"srv-{index:020d}",
                "name": spec.name,
                "ownerId": "tea-owner",
            }
            for index, spec in enumerate(MODULE.V2_SERVICES, start=1)
        }
        missing_name = "agent-bounties-open-competition-v2-beta3-indexer"
        recovered = False
        client = mock.Mock()

        def resolve(spec):
            if spec.name == missing_name and not recovered:
                raise MODULE.render.RenderServiceMissing(spec.name)
            return services[spec.name]

        def materialize(_client, spec, reference, groups):
            nonlocal recovered
            self.assertEqual(spec.name, missing_name)
            self.assertEqual(reference["name"], "agent-bounties-api")
            self.assertEqual(set(groups), {MODULE.BASE_GROUP, MODULE.V2_GROUP})
            recovered = True
            return services[spec.name]

        client.resolve_service.side_effect = resolve
        group = {
            "id": "evg-beta3",
            "ownerId": "tea-owner",
            "serviceLinks": [],
        }
        with mock.patch.object(MODULE, "named_group", return_value=group), mock.patch.object(
            MODULE, "ensure_v2_group", return_value=group
        ), mock.patch.object(MODULE, "provision_worker", side_effect=materialize) as provision:
            resolved = MODULE.resolve_services(client)

        self.assertEqual(resolved, services)
        provision.assert_called_once()
        client.ensure_blueprint_service.assert_not_called()
        self.assertEqual(client.resolve_service.call_count, len(MODULE.V2_SERVICES) * 2)

    def test_direct_worker_provisioning_is_exact_and_attaches_required_groups(self):
        spec = next(spec for spec in MODULE.V2_SERVICES if spec.name.endswith("-broker"))
        reference = {
            "id": "srv-api",
            "name": "agent-bounties-api",
            "type": "web_service",
            "branch": "main",
            "repo": "https://github.com/NSPG13/agent-bounties",
            "ownerId": "tea-owner",
            "environmentId": "evm-production",
        }
        created = {
            "id": "srv-beta3broker",
            "name": spec.name,
            "type": "background_worker",
            "branch": "main",
            "repo": "https://github.com/NSPG13/agent-bounties",
            "ownerId": "tea-owner",
            "environmentId": "evm-production",
        }
        groups = {
            name: {
                "id": f"evg-{name.rsplit('-', 1)[-1]}",
                "ownerId": "tea-owner",
                "environmentId": "evm-production",
                "serviceLinks": [],
            }
            for name in (MODULE.BASE_GROUP, MODULE.V2_GROUP, MODULE.RELAYER_GROUP)
        }
        attached: set[str] = set()
        writes = []
        client = mock.Mock()

        def get_env_var(service, key):
            if service is reference:
                self.assertEqual(key, "DATABASE_URL")
                return {"key": key, "value": "postgres://worker:secret@db/app"}
            expected = dict(MODULE.WORKER_ENVIRONMENT[spec.name])
            expected["DATABASE_URL"] = "postgres://worker:secret@db/app"
            return {"key": key, "value": expected[key]}

        def write(method, path, payload):
            writes.append((method, path, payload))
            if path == "/services":
                return {"service": created}
            attached.add(path.split("/")[2])
            return {}

        def get_group(group_id):
            source = next(group for group in groups.values() if group["id"] == group_id)
            links = []
            if group_id in attached:
                links.append(
                    {"service": {"id": created["id"], "name": spec.name, "type": "worker"}}
                )
            return {**source, "serviceLinks": links}

        client.get_env_var.side_effect = get_env_var
        client._write_with_retry.side_effect = write
        client.get_env_group.side_effect = get_group
        client.resolve_service.return_value = created

        result = MODULE.provision_worker(client, spec, reference, groups)

        self.assertEqual(result, created)
        create_payload = writes[0][2]
        self.assertEqual(create_payload["type"], "background_worker")
        self.assertEqual(create_payload["name"], spec.name)
        self.assertEqual(create_payload["environmentId"], "evm-production")
        self.assertEqual(create_payload["serviceDetails"]["runtime"], "docker")
        self.assertEqual(len(attached), 3)
        self.assertNotIn("X402_RELAYER_PRIVATE_KEY", json.dumps(create_payload))

    def test_runtime_validation_rejects_non_mainnet_or_pending_deployment(self):
        for network, block in (("base-sepolia", 123), ("base-mainnet", 0)):
            value = runtime()
            value["network"] = network
            value["deployment_block"] = block
            with tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "runtime.json"
                path.write_text(json.dumps(value), encoding="utf-8")
                with self.assertRaises(MODULE.Beta3RenderError):
                    MODULE.validated_runtime(path)

    def test_environment_is_exact_and_keeps_public_creation_fail_closed(self):
        value = runtime()
        environment = MODULE.runtime_environment(
            value,
            primary_rpc_url="https://primary.example",
            shadow_rpc_url="https://shadow.example",
            prover_url="https://prover.example/v1/prove",
            prover_api_key="a" * 32,
            broker_address="0x" + "55" * 20,
            keeper_address="0x" + "66" * 20,
            deployer_address="0x" + "77" * 20,
            refund_reserve_min_base_units=110_000,
        )
        manifest = json.loads(environment["BASE_MAINNET_OPEN_COMPETITION_V2_BETA3_RELEASE_MANIFEST_JSON"])
        self.assertFalse(manifest["public_creation_enabled"])
        self.assertFalse(manifest["proof_broker_enabled"])
        self.assertEqual(environment["OPEN_COMPETITION_V2_DEPLOYMENT_BLOCK"], "123")
        self.assertNotIn("X402_RELAYER_PRIVATE_KEY", environment)
        self.assertEqual(environment["OPEN_COMPETITION_V2_REFUND_RESERVE_MIN_BASE_UNITS"], "110000")
        self.assertEqual(environment["OPEN_COMPETITION_V2_INDEXER_AGREEMENT_MAX_AGE_SECONDS"], "120")
        self.assertEqual(environment["OPEN_COMPETITION_V2_RELAYER_MAX_GAS"], "8000000")

    def test_environment_rejects_shared_rpc_and_insecure_prover(self):
        common = dict(
            runtime=runtime(),
            primary_rpc_url="https://rpc.example",
            shadow_rpc_url="https://rpc.example/",
            prover_url="http://prover.example/v1/prove",
            prover_api_key="a" * 32,
            broker_address="0x" + "55" * 20,
            keeper_address="0x" + "66" * 20,
            deployer_address="0x" + "77" * 20,
            refund_reserve_min_base_units=110_000,
        )
        with self.assertRaises(MODULE.Beta3RenderError):
            MODULE.runtime_environment(**common)

    def test_environment_rejects_reused_signing_role(self):
        with self.assertRaisesRegex(MODULE.Beta3RenderError, "must be distinct"):
            MODULE.runtime_environment(
                runtime(),
                primary_rpc_url="https://primary.example",
                shadow_rpc_url="https://shadow.example",
                prover_url="https://prover.example/v1/prove",
                prover_api_key="a" * 32,
                broker_address="0x" + "55" * 20,
                keeper_address="0x" + "55" * 20,
                deployer_address="0x" + "77" * 20,
                refund_reserve_min_base_units=110_000,
            )


if __name__ == "__main__":
    unittest.main()
