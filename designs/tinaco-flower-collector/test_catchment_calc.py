#!/usr/bin/env python3
"""Minimal self-contained tests for catchment_calc.py.

Run with:  python3 test_catchment_calc.py
Uses only the standard library (`unittest`) so it runs anywhere Python 3 does.
"""

import math
import sys
import unittest

sys.path.insert(0, __file__.rsplit("/", 1)[0] if "/" in __file__ else ".")

import catchment_calc as cc  # noqa: E402


class CatchmentAreaTests(unittest.TestCase):
    def test_one_metre_radius(self):
        # pi * 1.0^2
        self.assertAlmostEqual(cc.catchment_area_m2(1000.0), math.pi, places=6)

    def test_conversion_scales(self):
        # Half the radius -> quarter the area.
        self.assertAlmostEqual(
            cc.catchment_area_m2(500.0),
            cc.catchment_area_m2(1000.0) / 4.0,
            places=6,
        )

    def test_litres_from_mm(self):
        # 1 m radius disk * 10 mm = 10 L/m2 -> 31.4159 L
        self.assertAlmostEqual(cc.litres_for_rainfall(1000.0, 10.0), 10.0 * math.pi, places=4)

    def test_monotonic_in_radius(self):
        for rainfall in (1.0, 25.0, 150.0):
            small = cc.litres_for_rainfall(100.0, rainfall)
            big = cc.litres_for_rainfall(400.0, rainfall)
            self.assertGreater(big, small)

    def test_zero_rainfall(self):
        self.assertEqual(cc.litres_for_rainfall(300.0, 0.0), 0.0)

    def test_negative_radius_rejected(self):
        with self.assertRaises(ValueError):
            cc.catchment_area_m2(-10.0)

    def test_negative_rainfall_rejected(self):
        with self.assertRaises(ValueError):
            cc.litres_for_rainfall(300.0, -5.0)


class CdmxMonthlyTests(unittest.TestCase):
    def test_month_keys(self):
        self.assertEqual(len(cc.CDMX_MONTHLY_RAIN_MM), 12)
        self.assertIn(8, cc.CDMX_MONTHLY_RAIN_MM)

    def test_annual_is_sum(self):
        self.assertAlmostEqual(
            cc.annual_litres(300.0),
            sum(cc.monthly_litres(300.0, m) for m in cc.CDMX_MONTHLY_RAIN_MM.values()),
            places=3,
        )

    def test_annual_positive(self):
        self.assertGreater(cc.annual_litres(300.0), 0.0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
