import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).with_name("run_open_competition_v2_mainnet_fork_replay.py")
SPEC = importlib.util.spec_from_file_location("open_competition_v2_fork", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class OpenCompetitionV2ForkReplayTests(unittest.TestCase):
    def bundle(self) -> dict:
        return {
            "schema_version": MODULE.EXPECTED_SCHEMA,
            "network": "base-mainnet",
            "chain_id": 8453,
            "deployment_state": "blocked",
            "activation": {"mainnet_signing_allowed": False},
        }

    def test_only_fail_closed_mainnet_bundle_is_accepted(self) -> None:
        MODULE.validate_bundle(self.bundle())
        value = self.bundle()
        value["network"] = "base-sepolia"
        with self.assertRaisesRegex(ValueError, "Base mainnet"):
            MODULE.validate_bundle(value)
        value = self.bundle()
        value["activation"]["mainnet_signing_allowed"] = True
        with self.assertRaisesRegex(ValueError, "must not authorize"):
            MODULE.validate_bundle(value)

    def test_abi_result_decoders_fail_closed(self) -> None:
        address = "0x" + "12" * 20
        self.assertEqual(MODULE.address_result("0x" + (bytes(12) + bytes.fromhex(address[2:])).hex()), address)
        self.assertTrue(MODULE.bool_result("0x" + (1).to_bytes(32, "big").hex()))
        with self.assertRaisesRegex(ValueError, "invalid ABI"):
            MODULE.bool_result("0x" + (2).to_bytes(32, "big").hex())


if __name__ == "__main__":
    unittest.main()
