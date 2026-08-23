"""5 USDC must never become 5,000,000: exact Decimal<->base-unit conversion."""
import unittest
from decimal import Decimal

from taskmarket_adapter.errors import TaskmarketError
from taskmarket_adapter.units import (
    base_units_to_usdc,
    format_usdc,
    parse_usdc,
    usdc_flag_value,
    usdc_to_base_units,
)


class UnitsTests(unittest.TestCase):
    def test_five_usdc_is_five_million_base_units(self):
        self.assertEqual(usdc_to_base_units("5"), 5_000_000)
        self.assertEqual(usdc_to_base_units(5), 5_000_000)
        self.assertEqual(usdc_to_base_units(Decimal("5")), 5_000_000)

    def test_round_trip(self):
        for units in (1, 1_000_001, 5_500_000, 123_456_789):
            self.assertEqual(usdc_to_base_units(format_usdc(base_units_to_usdc(units))), units)

    def test_fractional_usdc(self):
        self.assertEqual(usdc_to_base_units("0.000001"), 1)
        self.assertEqual(usdc_to_base_units("5.5"), 5_500_000)
        self.assertEqual(usdc_to_base_units("0.1"), 100_000)
        self.assertEqual(base_units_to_usdc(100_000), Decimal("0.1"))

    def test_more_than_six_decimals_refused(self):
        with self.assertRaises(TaskmarketError):
            parse_usdc("0.0000001")

    def test_nonpositive_and_garbage_refused(self):
        for bad in ("0", "-5", "abc", "", None, float("nan"), float("inf"), True, "1e30"):
            with self.assertRaises(TaskmarketError, msg=repr(bad)):
                parse_usdc(bad)

    def test_negative_base_units_refused(self):
        with self.assertRaises(TaskmarketError):
            base_units_to_usdc(-1)

    def test_format_has_no_trailing_zeros(self):
        self.assertEqual(usdc_flag_value("5"), "5")
        self.assertEqual(usdc_flag_value("5.500000"), "5.5")
        self.assertEqual(usdc_flag_value("0.000001"), "0.000001")
        self.assertEqual(usdc_flag_value(Decimal("12.25")), "12.25")

    def test_six_orders_of_magnitude_guard(self):
        # The historical bug: sending base units where USDC is expected.
        self.assertNotEqual(usdc_flag_value("5"), usdc_flag_value("5000000"))
        self.assertEqual(usdc_to_base_units(usdc_flag_value("5000000")), 5_000_000_000_000)


if __name__ == "__main__":
    unittest.main()
