# Flower-shaped rainwater collector for a tinaco

Parametric CAD concept for a flower-shaped rainwater collector that retrofits
onto a rooftop **tinaco** (rotomolded water tank) in Mexico City.

## Concept

A shallow, flower-like funnel sits on top of the tinaco and captures rain over
an area far larger than the tank's own fill opening. Eight concave **petals**
tilt downward toward a central hub, so every drop that lands on a petal runs
inward. Radial gaps between petals act as drainage channels that carry water to
a **screened central funnel**, which narrows to a **smooth sleeve** that drops
into the tinaco's fill port. The sleeve is not a threaded fitting: the model
carries no screw geometry, and the joint to the actual tank port is made with a
gasket plus a strap clamp (or an off-the-shelf threaded adapter slipped over the
smooth sleeve) — see [Port interface](#port-interface) and the BOM. A
debris/mosquito screen across the hub mouth keeps leaves, bird droppings, and
insect breeding out of the stored water.

The whole part is a single watertight shell, so it can be rotomolded, vacuum
formed, or FDM printed (with a food-safe liner when potable water is intended).

### How it works

1. Rain falls on the petals (rim high, hub low) and runs toward the center.
2. Water passes through the debris screen at the hub mouth.
3. The funnel narrows from the hub to the port and drops into the tank.
4. A first-flush diverter (not modeled here, documented below) discards the
   dirtiest first few litres before the tank starts filling.

## Parametric model

`flower_rainwater_collector.scad` is a self-contained OpenSCAD model. Edit the
constants at the top to fit any tinaco and any desired capture area.

| Parameter | Default | Unit | Meaning |
|---|---|---|---|
| `n_petals` | 8 | – | Number of petals (4–16) |
| `collector_dia` | 1200 | mm | Overall collector diameter |
| `petal_rise` | 100 | mm | Rim-to-hub concave depth (drain slope) |
| `petal_gap` | 6 | mm | Radial drainage channel width |
| `petal_gap_flare` | 2.0 | – | Rim valley radius as a multiple of `petal_gap` |
| `hub_dia` | 180 | mm | Central funnel mouth diameter |
| `funnel_height` | 90 | mm | Funnel drop from hub to port |
| `port_thread_dia` | 50.8 | mm | Tinaco fill-port size being adapted to (2" nominal) — the **sleeve is smooth**. It sets the sleeve's **bore** (inside diameter), i.e. the water path and the dimension the tank port is matched against — *not* the outside diameter; see [Port interface](#port-interface) |
| `port_length` | 40 | mm | Sleeve engagement depth below the funnel outlet (smooth, not a screw thread) |
| `screen_thick` | 3 | mm | Debris screen thickness |
| `screen_hole` | 2.5 | mm | Square screen hole side |
| `screen_pitch` | 6 | mm | Screen hole grid pitch |
| `wall` | 2.5 | mm | Material wall thickness |

Derived from those (not free parameters): the sleeve's outside diameter is
`port_od = port_thread_dia + 2 × wall` = **55.8 mm** at the defaults, because
`port_sleeve()` adds the wall on *both* sides of the bore. Any gasket, clamp or
adapter fits that 55.8 mm outside surface — there is no 50.8 mm outside surface
on the part.

### Sizing to a tinaco

Common Mexico City tinacos and the `port_thread_dia` (sleeve bore) to target:

| Tinaco | Typical diameter | Fill port |
|---|---|---|
| 450 L | 800 mm | 1½"–2" |
| 750 L | 920 mm | 2" |
| 1100 L | 1080 mm | 2" |
| 2500 L | 1550 mm | 2"–4" lid |

A 1200 mm collector captures ~1.13 m². At Mexico City's ~800 mm/yr mean
rainfall and an 0.85 runoff coefficient, that is ~770 L/yr per collector —
meaningful against a 750–1100 L tinaco.

## Assembly stack

`z = 0` is the hub-mouth / mounting datum. The dish sits **above** it, the funnel
and the sleeve hang **below** it, and every joint is a real volumetric
overlap (the funnel lip rises `fuse` into the dish shell, the screen rim sinks
into the funnel wall), so the assembly renders as **one** closed solid:

| Feature | z | Notes |
|---|---|---|
| rim crest | +102.5 mm | `petal_rise + wall` |
| hub mouth (datum) | 0.0 mm | ⌀180 mm, screen across it |
| funnel outlet | −90.0 mm | `funnel_height` |
| sleeve bottom | −130.0 mm | `port_length` = 40 mm of sleeve engagement |
| overall height | 232.5 mm | rim → outlet |

## Port interface

The sleeve that enters the tinaco is **smooth** — the model carries no screw
geometry, and neither the STL nor the STEP contains a thread. What the sleeve
does provide is the 40 mm engagement depth, a **50.8 mm bore** and a **55.8 mm
outside diameter**, so a standard adapter can be fitted on site:

| Surface | Value at the defaults | Where it comes from |
|---|---|---|
| Bore (water path, inside diameter) | **50.8 mm** | `port_thread_dia` — the nominal tinaco fill-port size, 2" |
| Outside diameter | **55.8 mm** | `port_od = port_thread_dia + 2 × wall` (`port_sleeve()` adds `wall = 2.5 mm` on both sides of the bore radius) |

The distinction matters when ordering hardware: the tank port is matched against
the **bore** figure, but every gasket, strap clamp or slip-over adapter grips the
**55.8 mm outside surface**. A part described as "50.8 mm OD" would not fit this
sleeve, and no 50.8 mm outside surface exists on it.

| Option | How it makes the seal |
|---|---|
| Gasket + strap clamp | EPDM/silicone gasket over the smooth sleeve, stainless worm-drive or spring clamp over the tank's port collar (this is the default in the BOM) |
| Threaded adapter over the sleeve | Off-the-shelf PVC/NPT or tinaco-thread female adapter slipped over the smooth sleeve and solvent-welded or clamped, then screwed into the tank port |
| Rotomoulded port collar | If this is tooled as one part, the mould can carry the tank's own thread instead — that is a tooling decision, not modelled here |

Adding modelled threads is deliberately out of scope for this revision; the
smoothed-out interface is honest about what the CAD actually contains.

## Verification

Five checks, in increasing order of what they can catch, plus the equivalence
check that ties the committed export back to the source:

```bash
python3 test_model.py                  # 1. parameter-table invariants (stdlib only)
python3 test_geometry.py               # 2. mesh invariants against exports/*.stl (stdlib only)
python3 export_step.py                 # 3. CAD import + valid-solid check + STEP (needs cadquery-ocp)
python3 test_required_dependencies.py  # 4. controls: generation fails when its tool is missing (stdlib only)
python3 compare_export.py --selftest   # 5. RED/GREEN controls for the comparator (stdlib only, seconds)
python3 compare_export.py              # 5. render to a TEMP file, compare facet multiset vs the committed STL (needs OpenSCAD)
```

An explicitly requested render or export never falls back to a stale artifact
and never exits `0` on a missing tool:

- `test_geometry.py --render` needs OpenSCAD (`OPENSCAD_BIN`, `openscad` on
  `PATH`, or a known install path). Without it the script stops with
  `FAIL --render requires OpenSCAD` and a nonzero exit — it does not silently
  check the STL that happens to be on disk.
- `export_step.py` needs the OpenCascade bindings (`pip install cadquery-ocp`).
  Without them it stops with `FAIL OpenCascade bindings not installed` and exit
  `1` — it does not print `SKIP` and report success.
- `test_required_dependencies.py` is the regression control for both: it forces
  each tool to be unavailable and asserts the nonzero exit, so a future edit
  cannot quietly restore the silent fallback.

1. `test_model.py` reads the parametric constants and asserts the engineering
   invariants: the flower drains inward, the funnel narrows to the port, the
   screen filters instead of blocking, the wall is moldable, the capture area is
   meaningful, and the parts overlap in the assembly stack. It also prints the
   derived capture figures and the assembly stack above.

   These checks read the *parameter table* — they cannot see whether the parts
   actually touch, which is why layer 2 exists.

2. `test_geometry.py` reads the **exported mesh** (the artifact a CAD kernel, a
   slicer or a reviewer imports) and asserts what only the geometry can show:
   closed shell (no boundary or non-manifold edges), consistent winding, **one
   connected body**, positive volume with a bounding box matching the parametric
   envelope, and **outlet continuity** — vertical probes down the bore reach the
   outlet through zero surfaces, with control probes through a petal and through
   the fused hub wall crossing exactly two (proof the probe can tell solid from
   void). `python3 test_geometry.py --render` re-renders the STL from the `.scad`
   first and then checks *that* mesh; it requires OpenSCAD and fails with a
   nonzero exit if OpenSCAD is unavailable (see layer 4 below).

3. `export_step.py` imports the STL into OpenCascade (OCCT), sews it at 1e-4 mm,
   and checks the way a CAD tool does: zero free edges, zero multiple edges, one
   shell promoted to one valid solid, positive volume matching the mesh volume,
   then writes the STEP. The STEP is a *tessellated* B-rep (one planar face per
   triangle) — the honest representation of a CSG mesh source; an analytic B-rep
   would require rebuilding the part in a B-rep kernel.

4. `test_required_dependencies.py` is the regression control for 2 and 3: it
   forces the tool to be unavailable and asserts the nonzero exit, so a future
   edit cannot quietly restore a silent `SKIP`-and-report-success path.

5. `compare_export.py` answers "is the committed STL still the file this source
   produces?" — a question an STL hash cannot answer, because OpenSCAD re-renders
   of one source emit the same triangle *set* in a different *order* (three
   renders of this part gave three sha256s with an identical facet multiset). It
   renders the `.scad` to a **temporary** file (never over the committed export),
   verifies the committed export's sha256 did not change, and compares facet
   multisets plus vertex/edge counts, winding, connected bodies, volume and
   bounding box. `--selftest` proves the comparison can fail: it runs the
   comparator against an identical copy (must PASS) and against a copy with one
   vertex nudged 0.5 mm and one facet deleted (both must FAIL).

## Exports (`exports/`)

Generated from the same source, not drawn separately:

| Artifact | Generated by |
|---|---|
| `flower_rainwater_collector.stl` | `OPENSCAD_BIN=openscad python3 test_geometry.py --render` (or `openscad --hardwarnings -o exports/flower_rainwater_collector.stl flower_rainwater_collector.scad`) |
| `flower_rainwater_collector.step` | `python3 export_step.py` (OpenCascade, after the STL; committed gzipped as `.step.gz`) |
| `assembly_drawing.svg` | `openscad --hardwarnings -D render_assembly=false -o exports/assembly_drawing.svg assembly_drawing.scad` |

`assembly_drawing.scad` `include`s the part file and projects it, so SECTION A-A
(1:4, cut through a petal centre) and PLAN (1:8) and every dimension label come
from the model's own variables — nothing is redrawn by hand.

## Bill of materials (concept)

| # | Item | Qty | Material | Notes |
|---|---|---|---|---|
| 1 | Collector shell — dish, funnel, port sleeve and integral debris screen | 1 | UV-stabilised food-grade PP/LLDPE (rotomould or vacuum-form); PETG for FDM prototypes | single fused part; ~2.9 L of material at the default parameters (see `test_geometry.py` volume report) |
| 2 | Port gasket + clamp set | 1 | EPDM or silicone gasket, stainless strap clamp | adapts the sleeve to the actual tinaco port; the model's 40 mm sleeve is the interchangeable interface |
| 3 | First-flush diverter | 1 | PP/PVC body, ½" tap | discards the first ~1 L per storm; documented, **not modelled** |
| 4 | Tie-down fasteners | 4 | stainless self-tapping screws + bonded washers | wind uplift on a 1.2 m dish |
| 5 | Food-safe liner / coating | as needed | NSF-61 rated liner or epoxy | only if the stored water is for drinking |
| — | Debris screen | — | integral in this model; a removable stainless mesh is an acceptable substitute | holes 2.5 mm @ 6 mm pitch |

## On-site dimensions to confirm before fabrication

Measured at the actual installation; the defaults are **nominal** and this model
is a concept, not a fabrication drawing:

- **Fill-port size (bore)**: the tank's own port diameter, thread major diameter
  and pitch. The default assumes 2" nominal = 50.8 mm **bore**. Note what that
  number is matched against: the model's bore is 50.8 mm and its outside
  diameter is 55.8 mm (`port_od`), so the sleeve's outside will not pass through
  a 50.8 mm restriction — a 2" PVC/NPT fitting is ~60.3 mm OD, so measure. This
  is the one dimension that must be right.
- **Port height above the tank top** and its centre offset from the tank centre.
- **Tank top / lid ring diameter** — clearance for the 1.2 m dish overhang and
  for the lid to still be serviceable.
- **Roof clearance**: 232.5 mm overall height (102.5 mm of it above the datum)
  plus a maintenance gap; keep the rim clear of walls and roof structure.
- **Overflow position and diameter** — must stay clear and unblocked.
- **Inlet/downspout position** feeding the collector, and the roof slope.
- **Wind exposure** at the installation (for the tie-down count).

## Deployment notes (real-world retrofit)

- **First-flush diverter:** add a removable cap or a small side tap below the
  hub so the first ~1 L per storm can be discarded before collection begins.
- **Overflow:** size the tinaco's own overflow for the collector area; an
  unvented collector must not pressurize a sealed tank.
- **Materials:** UV-stabilized, food-grade polypropylene or LLDPE. For FDM
  prototypes use PETG and a food-safe liner if water will be drunk.
- **Seal:** the sleeve is smooth, so the seal is made by a gasket plus a strap
  clamp over the tank's port collar (the BOM default), or by an off-the-shelf
  threaded adapter fitted over the sleeve. `port_thread_dia` sets the sleeve
  **bore** (50.8 mm) and the wall adds 2.5 mm on each side, so the outside
  surface that hardware grips is `port_od` = 55.8 mm; the adapter is the
  interchangeable part, per the [Port interface](#port-interface) section.
