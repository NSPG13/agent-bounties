# Verification evidence — flower-shaped rainwater collector (#1340)

Everything below was produced from `flower_rainwater_collector.scad` in this
directory. Commands are copy-pasteable from here.

## Environment

| Tool | Version / source |
|---|---|
| OpenSCAD | 2021.01 (Linux AppImage, x86_64) |
| Python (parameter + mesh tests) | CPython 3.14, standard library only |
| OpenCascade bindings | `cadquery-ocp` 8.0.1 (OCCT), Python 3.14 venv |
| Host | Debian, x86_64 |

## Exports (generated from this source)

| Artifact | Bytes | sha256 | Command |
|---|---|---|---|
| `exports/flower_rainwater_collector.stl` | 2 894 441 | `5f89ee2c69add9b98b31cd5dd53a1f6ba90c9156bac582df00f079fac90b52e8` | `openscad --hardwarnings -o exports/flower_rainwater_collector.stl flower_rainwater_collector.scad` |
| `exports/flower_rainwater_collector.step.gz` | 6 389 838 | `09e990df2514c13738051b093e61cb71ad11ed96bfcb324346eb149ad6f57f6f` | `python3 export_step.py` then `gzip -9` (uncompressed: 39 904 185 B, sha256 `c6232af9076c5d597ffaad4b8e8a0b6b7860f53b9c7cc2b664ef609d19d43e50`) |
| `exports/assembly_drawing.svg` | 2 388 650 | `a949250850c03ad528945c32018a8e4af219b668845375d895510c1587c89e80` | `openscad --hardwarnings -D render_assembly=false -o exports/assembly_drawing.svg assembly_drawing.scad` |

The STEP is committed **gzipped** because the tessellated B-rep is ~38 MB
uncompressed (18 624 planar faces, one per mesh triangle). `gunzip` it to open
it in CAD software. It is regenerated in ~75 s by `python3 export_step.py`.

Reproducibility — measured, not assumed. Re-rendering this source reproduces the
**mesh**, not the **bytes**. Three renders of the committed source
(`5f89ee2c69add9b9…` committed, `f332fe70316031bd…` and `6448ba18d0920f7e…` from
fresh runs, all 2 894 441 B) are identical as a facet *set* — 18 624 facets each,
facet multiset equal, zero facets unique to any one file, 8 088 welded vertices,
volume 2 862.9 cm³, every topology and outlet-probe check passing — but the
facets appear in a **different order** each run. OpenSCAD emits the same
triangles in a run-dependent sequence, so an STL sha256 is **not** a
source-equivalence token; the facet-set comparison plus the topology/volume/probe
checks are (section 6). The STEP bytes are not reproducible either — the STEP
header carries a creation timestamp (committed `c6232af9076c5d59…` vs a fresh
export `cecf240ecf9832b8…`, both 39 904 185 B, identical validity results).

## 1. Parameter invariants — `python3 test_model.py`

```
PASS  petal count is a whole number in 4..16
PASS  collector wider than hub
PASS  hub wider than port (funnel narrows)
PASS  concave petal rise is positive
PASS  petal gap is a real channel
PASS  wall is printable/injection-moldable
PASS  screen holes smaller than pitch (filters)
PASS  screen is a real disc
PASS  funnel has a drop
PASS  port sleeve engages the tank
PASS  funnel lip is fused into the dish shell (overlap > 0)
PASS  perforated screen area is at least the port bore (flow is not choked)
PASS  screen seats in solid material, not in the bore (overlapping seat)
PASS  outlet is below the funnel outlet (sleeve continues the bore)
PASS  capture area at least 0.7 m^2

assembly stack (z = 0 at the hub mouth / mounting datum):
  rim crest             : z = 102.5 mm
  hub mouth             : z = 0.0 mm  (dia 180 mm)
  funnel outlet         : z = -90.0 mm
  sleeve bottom         : z = -130.0 mm  (dia 50.8 mm, 40 mm engagement)
  overall height        : 232.5 mm

All invariants passed.
```

## 2. Mesh invariants — `python3 test_geometry.py`

Runs against the committed STL (no third-party dependency):

```
mesh: exports/flower_rainwater_collector.stl
      18624 triangles, 8088 welded vertices (from 55872 raw), sha256:5f89ee2c69add9b9...

PASS  no boundary edges (closed shell)
PASS  no non-manifold edges (each edge shared by 2 triangles)
PASS  winding consistent (every directed edge used once)
PASS  assembly is ONE connected body (dish + funnel + sleeve + screen fused)
PASS  positive signed volume (outward-facing solid)
PASS  height matches the parametric stack (rim -> outlet)
PASS  collector diameter matches collector_dia
PASS  outlet reaches port_length below the funnel outlet
      solid volume: 2862.9 cm^3 (2.863 L of material)
PASS  outlet continuity: 45 vertical probes through screen holes reach the outlet with zero surface crossings
PASS  control probe through a petal crosses exactly two surfaces (probe can tell solid from void; petal shell is sound)
PASS  control probe through the fused hub wall crosses exactly two surfaces

All geometry checks passed.
```

## 3. CAD import + valid-solid check + STEP — `python3 export_step.py`

```
      18624 faces imported
PASS  sewn into 1 shell(s), 0 solid(s), 18624 faces
PASS  0 free edges (closed shell)
PASS  0 multiple edges (manifold: every edge shared by exactly two faces)
PASS  shell promoted to exactly one TopoDS_Solid
PASS  BRepCheck_Analyzer: solid is valid
PASS  positive volume: 2862.9 cm^3
      mesh volume (test_geometry.py): 2862.9 cm^3  delta 0.0000%
PASS  wrote exports/flower_rainwater_collector.step (38969 KiB)

All CAD import checks passed.
```

## 4. Drawing renders warning-free — `assembly_drawing.scad`

```
$ openscad --hardwarnings -D render_assembly=false -o exports/assembly_drawing.svg assembly_drawing.scad
OPENSCAD_EXIT=0 WARNINGS=0 UNDEF=0
   Top level object is a 2D object:
   Contours:      964
```

`--hardwarnings` turns every OpenSCAD warning into a failure, so this is a
positive check that no dimension label is an undefined string operation.
Control (RED) check: a probe containing `text("a" + "b")` exits 1 with
`WARNING: undefined operation (string + string)` under the same flag.

### 4.1 Readability — checked on the rendered sheet, not on the exit code

Review item on PR #1345 (NSPG13) asked for a drawing revision and then for an
eye check of the rendered SVG, because "a successful export alone does not catch
the overlap". The SVG was rasterised at 192 dpi (4664 × 3530 px) and read region
by region. Fixed in this revision:

| Defect (cited by the review or found by the eye check) | Fix |
|---|---|
| `square([300, 185])` painted a filled rectangle (line 134) — in OpenSCAD 2D an overlapping fill is a **union**, so it absorbed the section view it sat on | border drawn as four `hline`/`vline` rules; block moved to `y = -300`, below both views (top edge `-115`, section's lowest dimension line `-4.5`, plan's lowest label `-65`) |
| table rule at line 137 applied its `y` twice (translate **and** `hline(...,158,...)`) → drew at `y = 316` | one local coordinate system: `translate([5, 158]) hline(0, 290, 0, 0.4)` |
| column divider at line 151 applied its `x`/`y` twice → drew at `(310, 316)` | `vline(x2, 158, -1, 0.4)` in block-local coordinates |
| `arrowhead()` ignored its `x`/`y` arguments, so every arrow was drawn at the drawing origin | `translate([x, y])` before `rotate`; the horizontal chain's angles now match the vertical chain's convention (tip on the extension line, body inside) |
| view title at `S*30` sat inside the dish, and the hub-mouth dimension line at `S*45` ran through the text | title and note moved above the part (part tops out at `S*(petal_rise + wall) = 25.6`) |
| "100 mm" (rim-to-hub) label ran into the hub-mouth diameter text | `vdim(..., side = 1)`: label placed to the right of its dimension line |
| the plan's hub-mouth dimension was drawn **inside** the filled silhouette, where the 2D union absorbs it (it survived only as fragments across the white radial channels — 4 path vertices over its whole length, none of its label glyphs) | dimension removed from the plan (the hub is a real edge and IS dimensioned in the section); the plan caption now reads `8 petals, hub 180 mm dia (see section)` |

Verification of the fix, on the rendered sheet: every dimension label in SECTION
A-A reads (232.5 / 90 / 40 / 100 mm, 180 mm dia, 50.8 mm dia, wall 2.5 mm,
screen 3 mm holes 2.5 mm @ 6 mm), arrowheads are present at both ends of both
horizontal dimension lines, the title is clear of the part and of every
dimension line, the plan shows no dimension inside the silhouette, and the title
block is an unfilled outline with both rules in place and every row legible.

The export grew from 247 070 B (127 contours) to 2 388 650 B (964 contours)
because the old fill had absorbed a large part of the section view into its
union; the sheet now exports the geometry it actually draws.

Those labels were superseded in the next revision (§4.2): the outlet
dimension now reads `50.8 mm bore` and an `OD 55.8 mm` leader points at the
sleeve's outside surface.

### 4.2 Orientation and the bore / OD wording — second review on PR #1345

Review item (2026-09-15, review id 5216867831) asked for two bounded presentation
repairs on revision `5c0783b`. Each one was **verified before it was changed**,
and the verification is recorded here rather than asserted.

**1. Section axis convention (`assembly_drawing.scad`).** The section was
projected through `rotate([90, 0, 0])`, which sends model `+Z` to drawing `-Y`,
while every dimension in the file is written `+Z` → `+Y`. Measured on standalone
`projection(cut = true)` renders of the two options (part file only, no
dimensions), splitting every contour vertex at the datum `y = 0`:

| rotation | half carrying the 1.2 m dish (`max|x|` = 599.79 mm) | other half (`max|x|` = 42.85 mm) |
|---|---|---|
| `[90, 0, 0]` — before | drawn **below** the datum | drawn above the datum |
| `[-90, 0, 0]` — now | drawn **above** the datum | drawn below the datum |

The wide half is the rim/dish (radius 600 mm) and the narrow one the funnel and
sleeve, so the table states which way the part was drawn. Reproduce: write a
`.scad` that `include`s the part and applies `projection(cut = true)` over the
rotation under test, export to SVG, and take `max|x|` per half around the datum
(SVG negates y, so the split is symmetric).

**2. Bore versus outside diameter.** `port_sleeve()` cuts
`cylinder(r = port_radius)` out of `cylinder(r = port_radius + wall)`, so
`port_thread_dia` is the **bore** and the outside is
`port_od = port_thread_dia + 2 × wall`. Measured on the committed mesh (the
exported artifact, not the parameter table), at the sleeve's bottom ring:

```
$ python3 measure_sleeve.py          # reads exports/*.stl with test_geometry.read_stl
sleeve bottom ring  z = -130.0
    r = 25.4000 mm  (dia 50.8000)   x720 vertices
    r = 27.9000 mm  (dia 55.8000)   x720 vertices
  -> bore 50.8000 mm dia, outside 55.8000 mm dia, wall 2.5001 mm
```

Both rings are cylindrical and the inner radius is identical at the top and
bottom of the sleeve, so the bore is constant over the 40 mm engagement.
Corrected in `flower_rainwater_collector.scad` (customizer comment, `port_sleeve()`
comment, new derived `port_od`), in `README.md` (parameter table, a new
"Port interface" table, the on-site-dimension list, the deployment notes) and on
the drawing: the outlet dimension now reads `50.8 mm bore`, an `OD 55.8 mm`
leader points at the sleeve's outside surface, and the title block carries
adjacent `port sleeve bore 50.8 mm` / `port sleeve OD 55.8 mm` rows.

**Rendered sheet for this revision.** Commands and receipts:

```
$ openscad --hardwarnings -D render_assembly=false -o exports/assembly_drawing.svg assembly_drawing.scad
OPENSCAD_EXIT=0 WARNINGS=0 UNDEF=0
   Top level object is a 2D object:
   Contours:     1038
exports/assembly_drawing.svg   2 594 620 B   sha256 53119043f19c6174…
exports/flower_rainwater_collector.stl   sha256 5f89ee2c69add9b9…   (unchanged by the render)
```

Rasterised at 192 dpi (4664 × 3530 px) and read region by region, not judged by
the export's exit code:

| Region | What the sheet shows |
|---|---|
| SECTION A-A | dish/rim **above** the datum and the funnel + sleeve hanging **below** it, ending at the outlet dimensions: `232.5 / 90 / 40 mm`, `100 mm`, `180 mm dia`, `wall 2.5 mm`, `screen 3 mm, holes 2.5 mm @ 6 mm`, `50.8 mm bore`, and an `OD 55.8 mm` leader on the sleeve's outside surface |
| view title | above the part, clear of the geometry and of every dimension line |
| PLAN (1:8) | `1200 mm dia` both ways, caption `8 petals, hub 180 mm dia (see section)`, nothing dimensioned inside the silhouette |
| title block | unfilled outline with both rules and the column divider, every row legible — including `port sleeve bore 50.8 mm` and `port sleeve OD 55.8 mm` |

Label placement is measured as well as looked at. Text in the export is glyph
outlines, so `measure_rows.py` splits the SVG into subpaths, takes each subpath's
bounding box, and reports the x-extent of the glyph clusters inside a table row —
two fields that touch appear as one cluster with no gap:

```
subpaths parsed: 1038   (geometries + glyph outlines of the whole sheet)

PARAMETER COLUMN (key left-aligned at -375, value right-aligned at -280):
  n_petals             y= -152.0  key ends  -349.53   value  -283.47.. -280.21   separation =  66.06 mm
  collector_dia        y= -160.0  key ends  -336.84   value  -307.74.. -280.32   separation =  29.10 mm
  petal_rise           y= -168.0  key ends  -345.81   value  -303.97.. -280.32   separation =  41.84 mm
  petal_gap            y= -176.0  key ends  -345.41   value  -296.60.. -280.32   separation =  48.81 mm
  hub_dia              y= -184.0  key ends  -350.77   value  -303.97.. -280.32   separation =  46.80 mm
  funnel_height        y= -192.0  key ends  -334.28   value  -300.40.. -280.32   separation =  33.88 mm
  port_thread_dia      y= -200.0  key ends  -327.40   value  -306.10.. -280.32   separation =  21.30 mm
  port_length          y= -208.0  key ends  -344.84   value  -300.57.. -280.32   separation =  44.28 mm
  screen_thick         y= -216.0  key ends  -340.42   value  -296.69.. -280.32   separation =  43.73 mm
  screen_hole          y= -224.0  key ends  -338.27   value  -302.26.. -280.32   separation =  36.01 mm
  screen_pitch         y= -232.0  key ends  -340.42   value  -296.60.. -280.32   separation =  43.82 mm
  wall                 y= -240.0  key ends  -363.75   value  -302.26.. -280.32   separation =  61.48 mm

DERIVED COLUMN (key at -220, value right-aligned at -125):
  overall height       y= -152.0  key ends  -179.67   value  -154.80.. -125.32   separation =  24.86 mm
  outlet below datum   y= -160.0  key ends  -163.40   value  -151.45.. -125.32   separation =  11.95 mm
  hub mouth            y= -168.0  key ends  -188.31   value  -139.61.. -125.18   separation =  48.70 mm
  rim crest            y= -176.0  key ends  -194.01   value  -165.99.. -125.32   separation =  28.02 mm
  port sleeve bore     y= -184.0  key ends  -171.59   value  -151.10.. -125.32   separation =  20.48 mm
  port sleeve OD       y= -192.0  key ends  -174.99   value  -151.10.. -125.32   separation =  23.88 mm
  capture area         y= -200.0  key ends  -181.83   value  -152.16.. -125.26   separation =  29.67 mm
  material volume      y= -208.0  key ends  -172.35   value  -166.64.. -125.19   separation =   5.71 mm   <-- tight
  BOM                  y= -216.0  key ends  -204.99   value  -187.41.. -125.19   separation =  17.57 mm

rows measured: 21   smallest separation: 5.71 mm   median: 33.88 mm
```

A note on what this revision did **not** do: the geometry, the STL and the STEP
are untouched (`compare_export.py`: facet multisets identical, topology/volume
/bbox equal, "preserved export unchanged by the render: yes"), no thread was
modelled, and the drawing is still a concept sheet — nothing physical was
measured.

## 5. Requested generation fails when its tool is missing

Review item on PR #1345 (NSPG13): `--render` fell back to the committed STL when
OpenSCAD was absent, and `export_step.py` exited `0` without OCP, so an
explicitly requested render/export could report success having produced nothing.
Fixed in `a66ea53` (author NSPG13, prepared on `collab/pr-1345-rainwater-concept`
as `0e08ead5` and cherry-picked here). The controls below were run with the
dependency genuinely absent, in this directory, with the committed STL present:

```
$ env -i PATH=/usr/bin:/bin HOME=$HOME python3 test_geometry.py --render
FAIL  --render requires OpenSCAD (set OPENSCAD_BIN or install openscad)
exit=1                     # the stale exports/*.stl was NOT checked or touched
                           # sha256 unchanged: 5f89ee2c69add9b9…

$ python3 export_step.py   # CPython without cadquery-ocp
FAIL  OpenCascade bindings not installed (No module named 'OCP'); pip install cadquery-ocp
exit=1

$ python3 test_required_dependencies.py -v
test_requested_render_does_not_use_stale_export ... ok
test_requested_step_export_requires_ocp ... ok
Ran 2 tests in 0.063s - OK
```

Nothing regressed: `test_model.py` passes all 15 invariants, `test_geometry.py`
passes every mesh check on the committed STL, and `export_step.py` under OCCT
reproduces the section-3 receipt (18 624 faces, 0 free edges, 0 multiple edges,
one valid `TopoDS_Solid`, 2 862.9 cm³, delta 0.0000 %, STEP 38 969 KiB).

## 6. Source → export equivalence — `python3 compare_export.py`

Review item on PR #1345 (NSPG13): make the equivalence claim **replayable** —
render to a temporary file, compare its facets with the preserved export, fail
when the geometry changes, and never replace the reviewed artifact. That is now
a committed script; this is its run on this revision (the EXACT command from
this directory; `OPENSCAD_BIN` points at the extracted OpenSCAD 2021.01 AppImage
on this host — any 2021.01 `openscad` works):

```
$ OPENSCAD_BIN=…/AppRun python3 compare_export.py
preserved export: exports/flower_rainwater_collector.stl (sha256 5f89ee2c69add9b98b31cd5dd53a1f6ba90c9156bac582df00f079fac90b52e8)
rendering flower_rainwater_collector.scad with … -> /tmp/compare_export_c8rsmemx/render.stl (this takes a few minutes)
fresh render:     /tmp/compare_export_c8rsmemx/render.stl (sha256 68dfc35c6966c5744dae4b351b2c562af8a8ea8c5fa876076a171c9c3f20d9be)
preserved export unchanged by the render: yes

facets: 18624 (committed) vs 18624 (fresh render) - multisets identical: yes
triangles: 18624 vs 18624
welded_vertices: 8088 vs 8088
edges: 27936 vs 27936
boundary_edges: 0 vs 0
non_manifold_edges: 0 vs 0
bad_winding: 0 vs 0
bodies: 1 vs 1
body_sizes: [18624] vs [18624]
volume: 2862882.6454 vs 2862882.6454 mm^3 (delta 0.000000)
bounding box: max coordinate delta 0.000000 mm

PASS  source -> export equivalence (facet multiset, topology, volume)
exit=0
```

The two files have **different bytes** (`68dfc35c6966c574…` fresh vs the
committed `5f89ee2c69add9b9…`) and the same geometry. That is the whole point:
an STL sha256 is not a source-equivalence token (section "Reproducibility —
measured, not assumed"), the facet multiset plus the topology/volume checks are.
The committed STL was **not** replaced by the render: the artifact in this PR
stays exactly the bytes the maintainer's independent check hashed, and the
script verifies that itself (sha256 before == after). `gunzip -c
exports/flower_rainwater_collector.step.gz` also still reproduces 39 904 185 B /
sha256 `c6232af9076c5d59…`, matching this file's manifest.

The comparison is itself controlled (RED), because a comparator that cannot fail
proves nothing. `--selftest` copies the committed export, mutates the copies and
asserts the verdicts (runs in seconds, no OpenSCAD):

```
$ python3 compare_export.py --selftest
selftest: the comparator must PASS the identical copy and FAIL both mutated ones

--- control: identical copy (expect PASS)
facets: 18624 (committed) vs 18624 (control) - multisets identical: yes
triangles: 18624 vs 18624
welded_vertices: 8088 vs 8088
edges: 27936 vs 27936
boundary_edges: 0 vs 0
non_manifold_edges: 0 vs 0
bad_winding: 0 vs 0
bodies: 1 vs 1
body_sizes: [18624] vs [18624]
volume: 2862882.6454 vs 2862882.6454 mm^3 (delta 0.000000)
bounding box: max coordinate delta 0.000000 mm
    comparator PASSED -> as expected

--- control: one vertex nudged by 0.5 mm (expect FAIL)
facets: 18624 (committed) vs 18624 (control) - multisets identical: NO
  facets only in committed: 1; only in control: 1
  example only in committed: ((-3.0, 91.1714, 2.5), (-3.0, 91.1714, 2.7451), (-3.0, 92.4214, 2.5))
  example only in control: ((-3.0, 91.1714, 2.5), (-3.0, 92.4214, 2.5), (-2.5, 91.1714, 2.7451))
triangles: 18624 vs 18624
welded_vertices: 8088 vs 8089  <-- DIFFERS
edges: 27936 vs 27938  <-- DIFFERS
boundary_edges: 0 vs 4  <-- DIFFERS
non_manifold_edges: 0 vs 0
bad_winding: 0 vs 0
bodies: 1 vs 1
body_sizes: [18624] vs [18624]
volume: 2862882.6454 vs 2862882.3850 mm^3 (delta 0.260417)  <-- DIFFERS
bounding box: max coordinate delta 0.000000 mm
    comparator FAILED -> as expected

--- control: one facet removed (expect FAIL)
facets: 18624 (committed) vs 18623 (control) - multisets identical: NO
  facets only in committed: 1; only in control: 0
  example only in committed: ((275.097, 3.0, 36.3089), (588.383, 3.0, 97.7377), (588.383, 3.0, 100.238))
triangles: 18624 vs 18623  <-- DIFFERS
welded_vertices: 8088 vs 8088
edges: 27936 vs 27936
boundary_edges: 0 vs 3  <-- DIFFERS
non_manifold_edges: 0 vs 0
bad_winding: 0 vs 0
bodies: 1 vs 1
body_sizes: [18624] vs [18623]  <-- DIFFERS
volume: 2862882.6454 vs 2863274.2529 mm^3 (delta 391.607500)  <-- DIFFERS
bounding box: max coordinate delta 0.000000 mm
    comparator FAILED -> as expected

selftest: all three controls behaved as expected
exit=0
```

## What is NOT verified

- **Byte-level export reproduction.** Re-rendering reproduces the facet set, not
  the file: the STL hash differs run to run (facet ordering) and the STEP hash
  differs even more (header timestamp). Nothing here claims bit-identical
  artifacts — section 6 states exactly what was compared.
- **Fabrication.** Nothing has been printed, moulded or measured; the model is a
  concept, not a fabrication drawing.
- **On-site fit.** The tank's fill-port size is an assumption (2" nominal =
  50.8 mm). The real tinaco port must be measured — see the "On-site dimensions"
  section of the README.
- **No modelled thread.** The sleeve is smooth by design; the gasket + clamp and
  threaded-adapter options in the README's "Port interface" section are
  described, not modelled. Nothing here claims a threaded joint is modelled or
  that the sleeve screws into any specific tank port.
- **Material, filtration and fit claims** are concept assumptions: no UV, impact,
  food-safety or flow testing has been run, and the first-flush diverter is
  documented but not modelled.
- **STEP surface type.** The STEP is a tessellated B-rep (planar faces from the
  mesh), not an analytic B-rep.
- **Mesh density.** `$fn = 120` is a rendering choice; the tests check the model
  as exported at that density, not mesh-convergence at other densities.
