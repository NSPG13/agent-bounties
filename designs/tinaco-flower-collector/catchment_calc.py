#!/usr/bin/env python3
"""Deterministic rainwater-catchment calculator for the flower collector.

A petaled collector bowl is modelled as a flat circular disk with radius
equal to the petal-tip radius (`flower_radius`). That is the projected
catchment area perpendicular to rainfall, which is what a flat roof/cone
sweeps. Collection in litres for a given rainfall depth is:

    volume_litres = pi * r_m^2 * rainfall_mm

since 1 mm of rain over 1 m^2 collects exactly 1 litre.

Includes a Mexico City monthly-rainfall example using climatological normals.
"""

from __future__ import annotations

import argparse
import math
import sys

# Average monthly rainfall (mm) for Mexico City (Observatorio de Tacubaya,
# climatological normals), used only as an illustrative planning example.
CDMX_MONTHLY_RAIN_MM = {
    1: 7.6, 2: 5.6, 3: 13.0, 4: 26.8, 5: 56.2,
    6: 134.1, 7: 161.4, 8: 169.4, 9: 136.4, 10: 59.2, 11: 12.9, 12: 6.6,
}


def catchment_area_m2(flower_radius_mm: float) -> float:
    """Projected catchment area of the flower bowl in square metres."""
    if flower_radius_mm <= 0:
        raise ValueError("flower_radius_mm must be positive")
    r_m = flower_radius_mm / 1000.0
    return math.pi * r_m * r_m


def litres_for_rainfall(flower_radius_mm: float, rainfall_mm: float) -> float:
    """Litres collected from one rainfall event of `rainfall_mm` depth."""
    if rainfall_mm < 0:
        raise ValueError("rainfall_mm must be non-negative")
    return catchment_area_m2(flower_radius_mm) * rainfall_mm


def monthly_litres(flower_radius_mm: float, rain_mm: float) -> float:
    """Litres collected over a month with `rain_mm` of total rainfall."""
    return litres_for_rainfall(flower_radius_mm, rain_mm)


def annual_litres(flower_radius_mm: float) -> float:
    """Total litres over a full CDMX year using the monthly normals."""
    return sum(monthly_litres(flower_radius_mm, m) for m in CDMX_MONTHLY_RAIN_MM.values())


def _fmt_litres(value: float) -> str:
    return f"{value:,.1f}"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--flower-radius-mm", type=float, default=300.0,
        help="Outer collector radius from centre to petal tip (mm).",
    )
    parser.add_argument(
        "--rainfall-mm", type=float, default=None,
        help="Single-event rainfall depth (mm). Defaults to the wettest CDMX month.",
    )
    args = parser.parse_args(argv)

    area = catchment_area_m2(args.flower_radius_mm)
    event_mm = args.rainfall_mm if args.rainfall_mm is not None else CDMX_MONTHLY_RAIN_MM[8]
    print(f"Flower radius:            {args.flower_radius_mm:.0f} mm "
          f"({(args.flower_radius_mm / 1000):.2f} m)")
    print(f"Projected catchment area: {area:.3f} m^2")
    print(f"Rainfall event:           {event_mm:.1f} mm")
    print(f"Collected per event:      {_fmt_litres(litres_for_rainfall(args.flower_radius_mm, event_mm))} L")
    print(f"CDMX average annual:      {_fmt_litres(annual_litres(args.flower_radius_mm))} L")
    return 0


if __name__ == "__main__":
    sys.exit(main())
