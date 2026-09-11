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

Reproducibility note: the STL is byte-stable for this source (`5f89ee2c…`), but
the STEP file's bytes are **not** — the STEP header carries a creation timestamp,
so two runs of the same geometry produce different file hashes with identical
volume, face count and validity results.

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

## What is NOT verified

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
