# Flower-Shaped Tinaco Rainwater Collector (Parametric CAD Concept)

A parametric OpenSCAD concept for a **flower-shaped rainwater collector** that
retrofits onto the lid opening of a Mexican **tinaco** (Rotoplas-style
fibreglass/plastic water storage tank). Captured roof water funnels through a
petaled, self-draining bowl into a central outlet stub that seats inside the
existing tank lid — no cutting, no sealing modification, gravity-fed.

This folder is the CAD-concept deliverable for the autonomous bounty
"Flower-shaped rainwater collector for a tinaco".

## Why flower-shaped?

- **More catchment per same footprint than a plain cone.** Each petal lobe
  extends the swept perimeter and slows runoff, helping the bowl collect
  splash-back from the lid instead of letting it wash off.
- **Self-cleaning.** The shallow bowl slopes continuously toward the centre
  outlet, so sediment, leaves, and debris are carried into (or out of) the
  tank rather than pooling in flat corners.
- **Retrofit-first.** A simple collar/skirt seats around the existing lid
  opening. Parameters default to a 470 mm lid (1100 L Rotoplas tinaco) but
  scale up (e.g. 590 mm / 2500 L).

## Files

| File | Purpose |
| ---- | ------- |
| `flower_collector.scad` | Parametric OpenSCAD source. Edit constants or override with `-D`. |
| `catchment_calc.py` | Deterministic catchment math (area m^2, litres per mm rainfall, CDMX monthly normals). |
| `test_catchment_calc.py` | Self-contained `unittest` tests for the calculator. |

## Parameters (`flower_collector.scad`)

| Parameter | Default | Meaning |
| --------- | ------- | ------- |
| `tinaco_lid_diameter` | 470 mm | Existing lid opening the bowl seats onto. |
| `flower_radius` | 300 mm | Bowl outer radius, centre to petal tip. |
| `flower_niche_radius` | 210 mm | Bowl radius at the petal notches. |
| `petal_count` | 8 | Number of flower lobes around the rim. |
| `bowl_depth` | 60 mm | Centre-to-rim drop that drives the drain slope. |
| `wall_thickness` | 3 mm | Bowl/flange wall thickness. |
| `rim_lip` | 12 mm | Outer rim lip height above the bowl floor. |
| `flange_depth` | 45 mm | Skirt that seats inside the tinaco lid opening. |
| `outlet_diameter` | 30 mm | Central outlet stub diameter. |
| `outlet_height` | 25 mm | Outlet stub height below the bowl. |

Override any value from the CLI, e.g.:

```sh
openscad -D tinaco_radius=0.75 -D petal_count=8 -o collect-2500L.stl flower_collector.scad
openscad -D '$fn=64' -o low-res-preview.stl flower_collector.scad
```

## Catchment math

The petaled bowl is modelled as a flat circular disk of radius
`flower_radius` (its projected catchment area). Since 1 mm of rain over 1 m^2
collects exactly 1 L:

```
litres = pi * (flower_radius_mm / 1000)^2 * rainfall_mm
```

Try it:

```sh
python3 catchment_calc.py --flower-radius-mm 300 --rainfall-mm 161
python3 test_catchment_calc.py
```

Example (300 mm radius collector):

- Projected catchment area ≈ 0.283 m^2
- Wettest CDMX month (~161 mm, Aug) ≈ ~45.5 L
- CDMX average annual (2001–2020 vision, Tacubaya) ≈ ~198 L

## Build / verify

1. Install [OpenSCAD](https://openscad.org/) (CLI: `openscad`).
2. `openscad -o flower_collector.stl flower_collector.scad`
3. Inspect/fillet the STL in OpenSCAD or a slicer; adjust `petal_count`,
   `flower_radius`, `bowl_depth` to taste.
4. Re-run `python3 test_catchment_calc.py` after any parametric change.

## Note on evidence

Per the repository's autonomous-protocol rules, this on-disk deliverable is
the engineering/CAD evidence. Settlement requires the canonical bounty flow:
publish the evidence preimage, then the `signed_quorum` verifier signs and the
`BountySettled` event records the payout. No amount is settled by this repo
alone.
