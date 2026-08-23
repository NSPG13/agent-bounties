import importlib.util
from pathlib import Path
import unittest


PATH = Path(__file__).with_name("rebalance_open_competition_v2_beta3_broker.py")
SPEC = importlib.util.spec_from_file_location("rebalance_open_competition_v2_beta3_broker", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


class BrokerRebalanceTests(unittest.TestCase):
    def test_exact_shortfall_is_returned(self):
        self.assertEqual(
            MODULE.rebalance_amount(
                deployer_usdc=300_000,
                broker_usdc=1_000_000,
                minimum_deployer_usdc=600_000,
                minimum_broker_usdc=100_000,
                maximum_transfer=400_000,
            ),
            300_000,
        )

    def test_already_ready_is_idempotent(self):
        self.assertEqual(
            MODULE.rebalance_amount(
                deployer_usdc=635_000,
                broker_usdc=110_000,
                minimum_deployer_usdc=635_000,
                minimum_broker_usdc=110_000,
                maximum_transfer=525_000,
            ),
            0,
        )

    def test_broker_reserve_fails_closed(self):
        with self.assertRaisesRegex(MODULE.BrokerRebalanceError, "cannot cover"):
            MODULE.rebalance_amount(
                deployer_usdc=0,
                broker_usdc=700_000,
                minimum_deployer_usdc=635_000,
                minimum_broker_usdc=110_000,
                maximum_transfer=635_000,
            )

    def test_transfer_cap_fails_closed(self):
        with self.assertRaisesRegex(MODULE.BrokerRebalanceError, "exceeds transfer cap"):
            MODULE.rebalance_amount(
                deployer_usdc=0,
                broker_usdc=1_000_000,
                minimum_deployer_usdc=635_000,
                minimum_broker_usdc=110_000,
                maximum_transfer=525_000,
            )

    def test_invalid_values_fail_closed(self):
        with self.assertRaisesRegex(MODULE.BrokerRebalanceError, "cannot be negative"):
            MODULE.rebalance_amount(
                deployer_usdc=-1,
                broker_usdc=1,
                minimum_deployer_usdc=1,
                minimum_broker_usdc=1,
                maximum_transfer=1,
            )


if __name__ == "__main__":
    unittest.main()
