import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


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
