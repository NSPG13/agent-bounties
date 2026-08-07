import argparse
import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).with_name("build_open_competition_v1_bundle.py")
SPEC = importlib.util.spec_from_file_location("open_competition_bundle", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class OpenCompetitionBundleTests(unittest.TestCase):
    def test_patched_runtime_fills_every_immutable_reference(self) -> None:
        artifact = {
            "deployedBytecode": {
                "object": "0x" + "00" * 96,
                "immutableReferences": {
                    "1": [{"start": 0, "length": 32}],
                    "2": [{"start": 64, "length": 32}],
                },
            }
        }
        runtime = MODULE.patched_runtime(artifact, [bytes([1]) * 32, bytes([2]) * 32], "fixture")
        self.assertEqual(runtime[:32], bytes([1]) * 32)
        self.assertEqual(runtime[32:64], bytes(32))
        self.assertEqual(runtime[64:], bytes([2]) * 32)

    def test_deployment_action_contains_exact_nonce_and_runtime_hash(self) -> None:
        action = MODULE.deployment_action(
            name="deploy_fixture",
            nonce=7,
            data=b"\x60\x00",
            expected_contract="0x1111111111111111111111111111111111111111",
            runtime=b"\x60\x01",
        )
        self.assertEqual(action["from"], MODULE.ADMIN)
        self.assertEqual(action["from_nonce"], 7)
        self.assertEqual(action["data"], "0x6000")
        self.assertEqual(action["runtime_code_bytes"], 2)
        self.assertTrue(action["runtime_code_hash"].startswith("0x"))

    def test_bundle_refuses_any_wallet_other_than_frozen_admin(self) -> None:
        args = argparse.Namespace(
            deployer="0x1111111111111111111111111111111111111111",
            deployer_nonce=0,
            source_commit="a" * 40,
            preflight_block_number=1,
            preflight_block_hash="0x" + "b" * 64,
            offline=True,
            preflight_deployer_eth_wei=1,
            preflight_deployer_usdc_base_units=MODULE.MIN_REHEARSAL_USDC,
            rpc_url="https://unused.example",
        )
        with self.assertRaisesRegex(ValueError, "frozen admin wallet"):
            MODULE.build_bundle(args)


if __name__ == "__main__":
    unittest.main()
