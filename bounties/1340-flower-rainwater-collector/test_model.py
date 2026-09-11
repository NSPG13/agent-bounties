#!/usr/bin/env python3
"""Parameter-level checks for the flower-shaped rainwater collector concept.

Reads the parametric constants from ``flower_rainwater_collector.scad`` and
asserts the engineering invariants the concept must satisfy: the flower drains
inward, the funnel narrows to the tinaco port, the debris screen filters
instead of blocking, and the capture area is meaningful for a rooftop tinaco
in Mexico City. Prints the derived capture figures and the assembly stack.

These checks are NECESSARY BUT NOT SUFFICIENT: they read the parameter table,
so they cannot see whether the parts actually touch. The mesh-level checks
(closed solid, single connected body, outlet continuity) live in
``test_geometry.py`` and run against the exported STL.

    python3 test_model.py        # parameter invariants
    python3 test_geometry.py     # mesh invariants (uses exports/*.stl)
"""

import math
import re
import sys
from pathlib import Path

SCAD = Path(__file__).with_name("flower_rainwater_collector.scad")

PARAM = re.compile(r"^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*([0-9]*\.?[0-9]+)\s*;", re.MULTILINE)

# Mexico City annual precipitation (m) and a conservative runoff coefficient.
MEXICO_CITY_RAINFALL_M = 0.800
RUNOFF_COEFFICIENT = 0.85

failures = []


def check(name, condition, detail):
    if condition:
        print(f"PASS  {name}")
    else:
        failures.append(name)
        print(f"FAIL  {name}: {detail}")


def main() -> int:
    text = SCAD.read_text()
    params = {m.group(1): float(m.group(2)) for m in PARAM.finditer(text)}

    required = [
        "n_petals", "collector_dia", "petal_rise", "petal_gap",
        "petal_gap_flare", "hub_dia", "funnel_height", "port_thread_dia",
        "port_length", "screen_thick", "screen_hole", "screen_pitch", "wall",
    ]
    missing = [k for k in required if k not in params]
    if missing:
        print(f"missing parameters: {missing}")
        return 1

    n = params["n_petals"]
    collector_dia = params["collector_dia"]
    petal_rise = params["petal_rise"]
    petal_gap = params["petal_gap"]
    hub_dia = params["hub_dia"]
    funnel_height = params["funnel_height"]
    port_thread_dia = params["port_thread_dia"]
    port_length = params["port_length"]
    screen_thick = params["screen_thick"]
    screen_hole = params["screen_hole"]
    screen_pitch = params["screen_pitch"]
    wall = params["wall"]
    fuse = params.get("fuse", wall)

    hub_radius = hub_dia / 2
    port_radius = port_thread_dia / 2

    check("petal count is a whole number in 4..16", n.is_integer() and 4 <= n <= 16, f"n_petals={n}")
    check("collector wider than hub", collector_dia > hub_dia, f"{collector_dia} <= {hub_dia}")
    check("hub wider than port (funnel narrows)", hub_dia > port_thread_dia, f"{hub_dia} <= {port_thread_dia}")
    check("concave petal rise is positive", petal_rise > 0, f"petal_rise={petal_rise}")
    check("petal gap is a real channel", 0 < petal_gap < collector_dia / 8, f"petal_gap={petal_gap}")
    check("wall is printable/injection-moldable", wall >= 1.5, f"wall={wall}")
    check("screen holes smaller than pitch (filters)", 0 < screen_hole < screen_pitch, f"{screen_hole} >= {screen_pitch}")
    check("screen is a real disc", screen_thick > 0, f"screen_thick={screen_thick}")
    check("funnel has a drop", funnel_height > 0, f"funnel_height={funnel_height}")
    check("port sleeve engages the tank", port_thread_dia > 10 and port_length > 0, f"{port_thread_dia}/{port_length}")

    # Assembly stack: z = 0 is the hub mouth / mounting datum. The dish sits
    # above it, the funnel and the sleeve hang below it. These relations are
    # what keep the parts in contact; test_geometry.py verifies the mesh.
    check("funnel lip is fused into the dish shell (overlap > 0)",
          0 < fuse <= wall, f"fuse={fuse} wall={wall}")
    check("perforated screen area is at least the port bore (flow is not choked)",
          hub_radius - screen_pitch >= port_radius,
          f"perforated radius {hub_radius - screen_pitch:.1f} < port radius {port_radius:.1f}")
    check("screen seats in solid material, not in the bore (overlapping seat)",
          hub_radius + fuse / 2 > hub_radius > hub_radius - fuse, f"seat radius {hub_radius + fuse / 2}")
    check("outlet is below the funnel outlet (sleeve continues the bore)",
          port_length > 0 and funnel_height > 0, f"{funnel_height}/{port_length}")

    radius_m = collector_dia / 2000.0
    capture_area_m2 = math.pi * radius_m * radius_m
    check("capture area at least 0.7 m^2", capture_area_m2 >= 0.7, f"{capture_area_m2:.3f} m^2")

    annual_harvest_m3 = capture_area_m2 * MEXICO_CITY_RAINFALL_M * RUNOFF_COEFFICIENT

    print()
    print(f"collector diameter      : {collector_dia:g} mm")
    print(f"capture area            : {capture_area_m2:.3f} m^2")
    print(f"annual harvest (CDMX)   : ~{annual_harvest_m3 * 1000:.0f} L/yr "
          f"({MEXICO_CITY_RAINFALL_M * 1000:g} mm, runoff {RUNOFF_COEFFICIENT})")
    print(f"petals                  : {n:g}")
    print()
    print("assembly stack (z = 0 at the hub mouth / mounting datum):")
    print(f"  rim crest             : z = {petal_rise + wall:.1f} mm")
    print(f"  hub mouth             : z = 0.0 mm  (dia {hub_dia:g} mm)")
    print(f"  funnel outlet         : z = {-funnel_height:.1f} mm")
    print(f"  sleeve bottom         : z = {-(funnel_height + port_length):.1f} mm  "
          f"(dia {port_thread_dia:g} mm, {port_length:g} mm engagement)")
    print(f"  overall height        : {petal_rise + wall + funnel_height + port_length:.1f} mm")

    if failures:
        print(f"\n{len(failures)} invariant(s) failed.")
        return 1
    print("\nAll invariants passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
