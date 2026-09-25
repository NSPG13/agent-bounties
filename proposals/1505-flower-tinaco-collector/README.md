# Flower-shaped rainwater collector for a tinaco (#1505)

Continuation of #1340. This is a parametric concept for a flower-shaped
catchment that fits a rooftop tinaco in Mexico City. It drains through a
central funnel and a 110 mm outlet tube into the tank. The adapter replaces
the tank lid, clamps around the measured lid neck and carries the collector
on four ribs. Nothing is drilled into the tank body.

*Colector pluvial en forma de flor para tinaco: los pétalos drenan a un
embudo central y a un tubo de 110 mm que entra al tinaco; el adaptador
sustituye la tapa y se sujeta al cuello medido con una abrazadera.*

This is a concept design, not a certified roof or potable-water product.
Rainwater from it is non-potable unless you treat it separately.

## Files

| File | Content |
| --- | --- |
| `flower_tinaco_collector.scad` | Editable parametric source (OpenSCAD). Every export comes from it. |
| `export.py` | Regenerates every export, then reopens and checks them (`--check` only verifies). |
| `step_model.py` | Rebuilds the same parts as an exact B-rep (cones, cylinders, planes) for STEP, using the parameters echoed by the SCAD. |
| `exports/flower_tinaco_collector.stl` | Assembly mesh (watertight, one solid). |
| `exports/flower_tinaco_collector.step` | Assembly STEP (AP214, mm, one valid solid). |
| `exports/adapter.stl` | Adapter on its own (plate, skirt, socket, ribs). |
| `exports/petal_flat.dxf` / `.svg` | Developed flat petal with seam lap and bolt holes (cut 8). |
| `exports/assembly_drawing.svg` | A1 sheet: sections A-A and B-B, adapter detail, plan, flat pattern, parts list, site measurements. |
| `exports/section_a.svg`, `section_b.svg`, `plan.svg` | Raw OpenSCAD views used by the drawing. |
| `exports/dimensions.json` | Parameters and derived dimensions echoed by OpenSCAD. |
| `exports/EVIDENCE.md` | Reopen-check results and SHA-256 of every file. |

## Build and verify

```sh
openscad --hardwarnings -o /tmp/collector.stl flower_tinaco_collector.scad
pip install cadquery-ocp        # only for STEP and the reopen checks
python3 export.py               # regenerate all exports + drawing + evidence
python3 export.py --check       # reopen and verify committed exports only
```

`export.py` fails if OpenSCAD prints any warning. It also fails if the STL is
not watertight, or if the STEP doesn't reopen as a valid solid whose volume
is within 0.5 % of the STL. Current results are in `exports/EVIDENCE.md`: STL
has 10,560 triangles and 0 non-manifold edges. The STEP reopens as 1 valid
solid with 482 faces, and its volume differs from the STL by 0.07 %.

Other lid sizes: `-D neck_od_mm=450`. Other layouts: `-D petal_count=10`,
`-D collector_diameter_mm=1200`. The asserts at the top of the SCAD stop
combinations that no longer fit, for example ribs outside the petal notch or
vents that no longer clear the socket.

## Drainage route and adapter (default parameters)

Heights are from the top rim of the tinaco lid neck (z = 0).

1. The petals form a 15° cone. A petal tip is at z 355, 750 mm from the axis.
2. The petal inner edge is at r 140. It overhangs 40 mm inside the funnel lip
   (r 180, z 199.6), so water drips into the funnel and can't run under the
   joint. The petals also bolt onto a 60 mm funnel flange that has the same
   15° slope.
3. The funnel falls at 40° from the lip down to the 110 mm PVC outlet tube at
   z 92.
4. The tube goes through the adapter socket (Ø111.5 × 60 mm, EPDM lip
   seal) and the plate. It ends 150 mm below the neck rim, inside the tank.
5. The adapter plate (Ø503 × 8 mm) sits on an EPDM gasket on the neck rim.
   A 60 mm skirt around the neck has a Ø491 bore (measured neck OD +
   2 × 3 mm clearance), 8 relief slots and a groove for a worm-drive band.
   Tightening the band closes the skirt onto the neck. To fit a different
   tank, set `neck_od_mm` to the measured value.
6. Four 8 mm ribs join the plate and socket to the funnel wall and the petal
   underside out to r 520 (rib height 279 mm there). Section B-B on the
   drawing shows them. Four screened Ø30 vents sit between the ribs and
   replace the lid vent.

Section A-A on the drawing traces this route in blue.

## Parameters

| Parameter | Default | Notes |
| --- | --- | --- |
| `petal_count` | 8 | 6–12 |
| `collector_diameter_mm` | 1500 | tip to tip, plan view |
| `petal_notch_mm` | 110 | notch depth between petals (notch Ø1280) |
| `petal_slope_deg` | 15 | petal fall to the centre |
| `petal_overlap_mm` | 30 | seam lap in the flat pattern |
| `sheet_mm` | 3 | petal / funnel sheet |
| `funnel_top_d_mm` / `funnel_slope_deg` | 360 / 40 | funnel lip and wall angle |
| `petal_lap_mm` / `flange_w_mm` | 40 / 60 | drip overhang / bolting flange |
| `outlet_od_mm` / `drop_tube_mm` | 110 / 150 | PVC sanitario 110 mm |
| `neck_od_mm` | 485 | **measure on site** |
| `neck_clearance_mm` | 3 | radial, closed by the band clamp |
| `skirt_h_mm` | 60 | needs this much free neck height |
| `rib_count` / `rib_reach_mm` | 4 / 520 | supports |

## Parts list (default parameters)

| # | Part | Qty |
| --- | --- | --- |
| 1 | Petal, 3 mm UV-stabilised HDPE, cut from `petal_flat.dxf` | 8 |
| 2 | Funnel Ø360 → Ø110, 40°, with 60 mm sloped flange | 1 |
| 3 | Outlet tube, PVC sanitario Ø110 × 242 mm (z −150 to +92) | 1 |
| 4 | Leaf screen Ø220 × 3 mm with 24 × 6 mm slots, plus 0.5 mm stainless mesh | 1 |
| 5 | Adapter: plate Ø503 × 8, skirt × 60, socket, 4 ribs | 1 |
| 6 | EPDM gasket Ø485 × 18 × 4 (cut to the measured neck) | 1 |
| 7 | Stainless worm-drive band for the skirt groove | 1 |
| 8 | EPDM lip seal for the Ø110 socket | 1 |
| 9 | M6 stainless bolt, 2 washers, nyloc nut: 24 seam + 24 flange | 48 |
| 10 | L-bracket + M6 at each rib top (petal and funnel; not modelled) | 8 |
| 11 | Stainless mesh disc over each Ø30 vent | 4 |
| 12 | Guy line to a roof anchor, from the Ø10 tip tie holes | 4 |

## Measure before fabrication

| # | Measurement | Sets |
| --- | --- | --- |
| M1 | Lid-neck outside diameter at the rim, 3 readings 60° apart | `neck_od_mm` |
| M2 | Neck height above the tank shoulder (≥ 60 mm free) | `skirt_h_mm` |
| M3 | Rim width and flatness under the gasket | `gasket_w_mm` |
| M4 | Float valve and inlet position under the neck | `drop_tube_mm` |
| M5 | Free height above the tank (≥ 400 mm) and radius (≥ 850 mm) | `collector_diameter_mm` |
| M6 | Tank overflow pipe present and clear (the collector has no overflow of its own) | — |
| M7 | Anchor points for 4 guy lines on the parapet or roof slab | — |
| M8 | Where the current lid vents | vents |

## Assembly

1. Cut 8 petals from `petal_flat.dxf`. Each flat petal is the exact
   development of its part of the 15° cone, so it rolls into shape without
   stretching. Lap the seams 30 mm and bolt them through the seam holes.
2. Bolt the petal ring onto the funnel flange through the 24 flange holes.
3. Bond the outlet tube into the funnel. Seat the leaf screen and mesh in
   the funnel.
4. Remove the tank lid. Lay the gasket on the neck rim, set the adapter over
   the neck and tighten the band in the skirt groove.
5. Lower the funnel and petals so the tube passes through the socket and the
   ribs meet the funnel wall and petal underside. Fix the rib tops with the
   L-brackets.
6. Tie the four guy lines from the petal tip holes to the roof anchors.

## Assumptions (not demonstrated by the model)

- **Catchment and yield.** The 1.522 m² plan area comes from the modelled
  petal outline. Yield assumes a runoff coefficient of 0.8 for HDPE, which
  gives ~1.2 L per mm of rain (~12 L for a 10 mm event). Wind-driven rain and
  splash will reduce this.
- **Outlet capacity.** At an assumed 50 mm/h design intensity, the flow is
  ~61 L/h (~1 L/min). A 110 mm outlet should carry far more than that. This
  is not flow-tested.
- **Mass.** ~9.3 kg if every modelled part were HDPE (0.95 g/cm³, from the
  model volumes), without fasteners. If the screen clogs, the funnel holds
  up to ~5 L below its lip.
- **Loads and fit.** No wind or structural analysis was done. The ribs and
  guy lines are the intended load path, and the neck only locates the
  adapter. Take the collector down before forecast storms. The skirt and
  band fit assumes a round, undamaged neck. If M1 readings differ by more
  than ~4 mm, shim the band or replace the lid neck first.

## Sanitary notes

- The screen, mesh and screened vents keep leaves and mosquitoes out.
  Rinse the screen monthly in the rainy season.
- HDPE must be UV-stabilised or it will embrittle on a roof.
- Keep the tank's own overflow clear. The collector has no overflow.
