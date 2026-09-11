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
| `exports/assembly_drawing.svg` | 247 070 | `46c930b847a018e6f93feea589927ceabde380cca9dc1c595e7f03404cdf1373` | `openscad --hardwarnings -D render_assembly=false -o exports/assembly_drawing.svg assembly_drawing.scad` |

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
   Contours:      127
```

`--hardwarnings` turns every OpenSCAD warning into a failure, so this is a
positive check that no dimension label is an undefined string operation.
Control (RED) check: a probe containing `text("a" + "b")` exits 1 with
`WARNING: undefined operation (string + string)` under the same flag.

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
Ran 2 tests in 0.034s - OK
```

Nothing regressed: `test_model.py` passes all 15 invariants, `test_geometry.py`
passes every mesh check on the committed STL, and `export_step.py` under OCCT
reproduces the section-3 receipt (18 624 faces, 0 free edges, 0 multiple edges,
one valid `TopoDS_Solid`, 2 862.9 cm³, delta 0.0000 %, STEP 38 969 KiB).

## 6. Source → export equivalence — fresh render from source

Exact command (from this directory; `OPENSCAD_BIN` points at the extracted
OpenSCAD 2021.01 AppImage on this host — any 2021.01 `openscad` works):

```
$ OPENSCAD_BIN=/home/claw1/portfolio/tools/squashfs-root/AppRun python3 test_geometry.py --render
rendering flower_rainwater_collector.scad with …/AppRun -> exports/flower_rainwater_collector.stl (this takes a few minutes)
mesh: exports/flower_rainwater_collector.stl
      18624 triangles, 8088 welded vertices (from 55872 raw), sha256:f332fe70316031bd…
<all 11 mesh checks PASS, volume 2862.9 cm^3>
```

Result: the re-render reproduces the committed mesh **exactly as a facet set**
(18 624 facets; comparison against `exports/flower_rainwater_collector.stl`
reports `multiset equal: True`, 0 facets unique to either file) with identical
welded-vertex count, volume and every topology/outlet-probe verdict — while the
file **bytes** differ (`f332fe70316031bd…` vs the committed `5f89ee2c69add9b9…`),
because OpenSCAD orders the same triangles differently between runs (section
"Reproducibility — measured, not assumed"; a second fresh run gave
`6448ba18d0920f7e…` with the same facet multiset).

The committed STL was therefore **not** replaced by the re-render: the artifact
in this PR stays exactly the bytes that the maintainer's independent check
hashed. `gunzip -c exports/flower_rainwater_collector.step.gz` also still
reproduces 39 904 185 B / sha256 `c6232af9076c5d59…`, matching this file's
manifest.

## What is NOT verified

- **Byte-level export reproduction.** Re-rendering reproduces the facet set, not
  the file: the STL hash differs run to run (facet ordering) and the STEP hash
  differs even more (header timestamp). Nothing here claims bit-identical
  artifacts — section 6 states exactly what was compared.
- **Fabrication.** Nothing has been printed, moulded or measured; the model is a
  concept, not a fabrication drawing.
- **On-site fit.** The port thread is an assumption (2" nominal = 50.8 mm). The
  real tinaco port must be measured — see the "On-site dimensions" section of
  the README.
- **Material, filtration and fit claims** are concept assumptions: no UV, impact,
  food-safety or flow testing has been run, and the first-flush diverter is
  documented but not modelled.
- **STEP surface type.** The STEP is a tessellated B-rep (planar faces from the
  mesh), not an analytic B-rep.
- **Mesh density.** `$fn = 120` is a rendering choice; the tests check the model
  as exported at that density, not mesh-convergence at other densities.
