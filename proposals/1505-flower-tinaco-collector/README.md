# Flower-shaped rainwater collector for a tinaco (#1505)

Continuation of #1340. Parametric catchment that sits on a rooftop tinaco lid
neck (Mexico City 450 mm / 600 mm covers) and does not drill the tank body.

Colector pluvial en forma de flor para tinaco de azotea. Sujeción al cuello
de la tapa; no se taladra el cuerpo del tinaco.

This is a field-fit concept, not a certified roof product. Confirm local
plumbing, potable-water, and wind rules before cutting material. Catchment
water is non-potable unless treated separately.

## Files

| File | Role |
| --- | --- |
| `flower_tinaco_collector.scad` | Parametric assembly (STL) and flat petal (DXF) |
| `README.md` | Sizing, BOM, assembly, checks |

```
openscad -o flower_tinaco_collector.stl flower_tinaco_collector.scad
openscad -o petal_pattern.dxf -D export_mode='\"petal_dxf\"' flower_tinaco_collector.scad
openscad -o adapter_ring.stl -D export_mode='\"adapter\"' flower_tinaco_collector.scad
```

Switch lid size with `-D lid_neck_mm=450` (default 600).

## Parameters

| Parameter | Default | Range | Notes |
| --- | --- | --- | --- |
| `petal_count` | 8 | 6–12 | even counts sit more evenly on the lid |
| `collector_diameter_mm` | 1500 | 1000–2000 | projected catchment diameter |
| `throat_diameter_mm` | 120 | 100–200 | outlet into the adapter / first-flush tee |
| `petal_slope_deg` | 18 | 12–30 | steeper sheds debris; shallower catches more wind |
| `petal_overlap_mm` | 15 | 0–30 | seam lap for bolts, sealant, and expansion |
| `material_thickness_mm` | 2.0 | 1.2–4.0 | UV-stabilized HDPE or galvanized sheet |
| `lid_neck_mm` | 600 | 450 or 600 | inner bore of the clamp ring |
| `adapter_outer_mm` | 620 | lid + ~20 | flange over the lid neck |
| `adapter_height_mm` | 40 | 30–50 | clamp band lands on this collar |
| `gasket_thickness_mm` | 4 | 3–6 | EPDM against the lid lip |

Derived at the defaults (also echoed by OpenSCAD):

| Quantity | Value |
| --- | --- |
| Projected area | 1.767 m² |
| Dish rise (`tan(18°) × 630 mm`) | ~205 mm |
| Central cone (30° , 240 → 120 mm) | ~35 mm |
| Height above lid (dish + cone + adapter) | ~280 mm |
| Peak flow at 50 mm/h | ~88 L/h |
| 20 mm storm yield | ~35 L |
| First 2 mm (should divert) | ~3.5 L |

## Geometry

- Each petal is a rounded sector of the catchment cone, tilted
  `petal_slope_deg` toward the throat. Adjacent petals lap by
  `petal_overlap_mm` so seams can take M6 bolts without a gap.
- Central cone: 2 × `throat_diameter_mm` down to `throat_diameter_mm`,
  steeper than the petals so grit does not sit in the neck.
- Adapter: collar `adapter_outer_mm` / `lid_neck_mm`, EPDM gasket,
  four stainless worm-drive clamps on the lid neck. For 450 mm lids,
  either rebuild with `lid_neck_mm=450` or drop in a 600→450 reducing
  insert (same flange, 20 mm step).
- Debris screen: 0.5–1.0 mm stainless dome (18–20 mesh) at the throat.
  1 mm alone is too open for Aedes; do not skip the finer mesh.
- First-flush: tee under the throat into a **4 L** bottle (2 mm on 1.77 m²
  is ~3.5 L; a 3 L bottle is short). Auto-reset float or manual drain.
- Overflow: two 25 mm ports at 80% of dish height, hoses led off the roof
  edge — not down the building wall.
- Four strap lugs on the rim for guy lines. Lines go to the parapet, never
  to the tinaco body or the plumbing.

## Hydrology (Mexico City, order-of-magnitude)

Design intensity 50 mm/h on 1.767 m² with runoff coefficient 0.9 for smooth
HDPE: ~80 L/h. A 50 mm PVC outlet is far above that. A 20 mm event puts
~32–35 L into a 750 L / 1100 L tinaco.

These numbers size the throat, first-flush, and overflow. They are not a
yield guarantee; surrounding roof, splash, and wind all cut capture.

## Bill of materials (default 8-petal, 1500 mm, 600 mm lid)

- 8 petals, 2 mm UV-HDPE or galvanized — unfold from `petal_pattern.dxf`
- 1 central cone, 1 adapter collar, 1 EPDM gasket, 1 reducing insert if
  the lid is 450 mm
- 1 mesh dome (0.5–1.0 mm stainless)
- 1 first-flush tee + 4 L bottle + drain valve
- 2 × 25 mm overflow elbows and hose
- M6 stainless bolts/nuts/washers × 32, polyurethane sheet-metal sealant
- 4 worm-drive clamps (lid neck)
- 4 guy straps / rope and parapet anchors

Wet mass of the plastic assembly is under 12 kg on the lid neck. Remove
the collector if a hurricane warning is posted.

## Assembly

1. Lay petals on the ground, lap to `petal_overlap_mm`, drill, bolt, seal.
   Work from one seam around; do not snug all bolts until the flower is round.
2. Fasten the central cone and mesh dome. Bolt the adapter under the cone.
3. Set the gasket on the tinaco lid lip. Seat the collar. Tighten the four
   clamps evenly. Confirm the tank vent is still open.
4. Tee the first-flush bottle below the throat. Run overflow hoses away
   from walls and electrical.
5. Guy four straps from the rim lugs to the parapet. Target hold-down for
   gusts above 60 km/h. Do not guy to the tank.

## Sanitary / safety

- Mesh + first-flush + a dark enclosed throat cut debris and mosquito
  habitat. Empty the bottle after each storm; rinse the mesh monthly in
  the rainy season.
- Cut galvanized edges rust; paint or tape them. HDPE must be UV-stabilized
  or it will go brittle on a Mexico City roof in one season.
- Do not seal the tinaco vent. Keep both overflow ports clear so the lid
  cannot pressurize.
- The lid neck takes the load, not the tank wall. If the lid is cracked or
  the neck is oval, replace the lid first.

## Verification

- [ ] `openscad -o /tmp/flower_tinaco_collector.stl flower_tinaco_collector.scad` exits 0
- [ ] Console echo of capture area is ~1.767 m² at the default diameter
- [ ] `lid_neck_mm=450` and `lid_neck_mm=600` both build an adapter
- [ ] DXF mode emits a single developed petal with seam bolt holes
- [ ] First-flush volume is documented as 4 L (~2 mm on the design area)
- [ ] No fastener is specified through the tinaco body
