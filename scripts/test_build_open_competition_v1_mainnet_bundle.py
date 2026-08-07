import argparse
import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).with_name("build_open_competition_v1_mainnet_bundle.py")
SPEC = importlib.util.spec_from_file_location("open_competition_mainnet_bundle", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class OpenCompetitionMainnetBundleTests(unittest.TestCase):
    def args(self, **overrides: object) -> argparse.Namespace:
        values = {
            "deployer": MODULE.ADMIN,
            "deployer_nonce": 45,
            "source_commit": "a" * 40,
            "preflight_block_number": 1,
            "preflight_block_hash": "0x" + "b" * 64,
            "offline": True,
            "preflight_deployer_eth_wei": MODULE.MIN_DEPLOYER_ETH_WEI,
            "preflight_deployer_usdc_base_units": MODULE.MIN_CANARY_USDC,
            "rpc_url": "https://unused.example",
        }
        values.update(overrides)
        return argparse.Namespace(**values)

    def test_bundle_refuses_any_wallet_other_than_frozen_admin(self) -> None:
        with self.assertRaisesRegex(ValueError, "frozen admin wallet"):
            MODULE.build_bundle(self.args(deployer="0x1111111111111111111111111111111111111111"))

    def test_bundle_refuses_insufficient_canary_usdc(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "bounded canary"):
            MODULE.build_bundle(self.args(preflight_deployer_usdc_base_units=MODULE.MIN_CANARY_USDC - 1))

    def test_canary_budget_includes_escrow_and_solver_bond(self) -> None:
        self.assertEqual(MODULE.CANARY_INITIAL_FUNDING, 1_100_000)
        self.assertEqual(MODULE.MIN_CANARY_USDC, 1_200_000)


if __name__ == "__main__":
    unittest.main()
