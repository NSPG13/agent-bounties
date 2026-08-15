import importlib.util
from pathlib import Path
import unittest


PATH = Path(__file__).with_name("fund_open_competition_v2_beta2_broker.py")
SPEC = importlib.util.spec_from_file_location("fund_open_competition_v2_beta2_broker", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


class BrokerFundingTests(unittest.TestCase):
    def test_deficits_are_idempotent(self):
        self.assertEqual(
            MODULE.deficits(current_usdc=0, current_eth=0, target_usdc=110_000, target_eth=100),
            (110_000, 100),
        )
        self.assertEqual(
            MODULE.deficits(
                current_usdc=200_000,
                current_eth=200,
                target_usdc=110_000,
                target_eth=100,
            ),
            (0, 0),
        )

    def test_deficits_reject_invalid_targets(self):
        self.assertEqual(
            MODULE.deficits(current_usdc=0, current_eth=0, target_usdc=0, target_eth=100),
            (0, 100),
        )
        with self.assertRaisesRegex(MODULE.BrokerFundingError, "are invalid"):
            MODULE.deficits(current_usdc=0, current_eth=0, target_usdc=0, target_eth=0)


if __name__ == "__main__":
    unittest.main()
