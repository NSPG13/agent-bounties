# Flower-shaped rainwater collector for a tinaco

Parametric CAD concept for a flower-shaped rainwater collector that retrofits
onto a rooftop **tinaco** (rotomolded water tank) in Mexico City.

## Concept

A shallow, flower-like funnel sits on top of the tinaco and captures rain over
an area far larger than the tank's own fill opening. Eight concave **petals**
tilt downward toward a central hub, so every drop that lands on a petal runs
inward. Radial gaps between petals act as drainage channels that carry water to
a **screened central funnel**, which narrows to a threaded sleeve that engages
the tinaco's fill port. A debris/mosquito screen across the hub mouth keeps
leaves, bird droppings, and insect breeding out of the stored water.

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
| `port_thread_dia` | 50.8 | mm | Tinaco fill-port thread (2" nominal) |
| `port_length` | 40 | mm | Threaded sleeve engagement |
| `screen_thick` | 3 | mm | Debris screen thickness |
| `screen_hole` | 2.5 | mm | Square screen hole side |
| `screen_pitch` | 6 | mm | Screen hole grid pitch |
| `wall` | 2.5 | mm | Material wall thickness |

### Sizing to a tinaco

Common Mexico City tinacos and the `port_thread_dia` to target:

| Tinaco | Typical diameter | Fill port |
|---|---|---|
| 450 L | 800 mm | 1½"–2" |
| 750 L | 920 mm | 2" |
| 1100 L | 1080 mm | 2" |
| 2500 L | 1550 mm | 2"–4" lid |

A 1200 mm collector captures ~1.13 m². At Mexico City's ~800 mm/yr mean
rainfall and an 0.85 runoff coefficient, that is ~770 L/yr per collector —
meaningful against a 750–1100 L tinaco.

## Verification

```bash
python3 test_model.py
```

The script reads the parametric constants from the `.scad` file and asserts the
engineering invariants: the flower drains inward, the funnel narrows to the
port, the screen filters instead of blocking, the wall is moldable, and the
capture area is meaningful. It also prints derived capture figures.

Render the concept locally (optional, needs OpenSCAD):

```bash
openscad -o flower_rainwater_collector.stl flower_rainwater_collector.scad
```

## Deployment notes (real-world retrofit)

- **First-flush diverter:** add a removable cap or a small side tap below the
  hub so the first ~1 L per storm can be discarded before collection begins.
- **Overflow:** size the tinaco's own overflow for the collector area; an
  unvented collector must not pressurize a sealed tank.
- **Materials:** UV-stabilized, food-grade polypropylene or LLDPE. For FDM
  prototypes use PETG and a food-safe liner if water will be drunk.
- **Seal:** a gasketed thread or a strap clamp adapts `port_thread_dia` to the
  specific tinaco lid; the sleeve is the interchangeable interface.
