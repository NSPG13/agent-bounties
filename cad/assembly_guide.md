# Flower-Shaped Tinaco Rainwater Collector

## Engineering & Assembly Guide

This document specifies the technical geometry, dimensional construction envelopes, mechanical interfaces, and assembly procedures for the parametric flower-shaped rainwater collector retrofitted onto standard Mexican tinacos (e.g., Rotoplas, Citijal, Eureka 450L–1100L tanks).

---

### 1. Dimensioned Geometry & Continuous Flow Interfaces

The collector integrates an angled catchment petal array feeding into a central conical funnel and downspout with a **continuous, unobstructed internal bore** down to an adjustable tinaco mounting collar.

```
       \  Petal R=162.5mm (concave catchment trough) /
        \________________                         /
                         \                       /
                          \  Catchment Funnel   /
                           \   f_h = 120mm     /
                            \                 /
                            |                 |
                            |  Downspout Tube |
                            |  Outer:  80 mm  |
                            |  Inner:  74 mm  |  <--- Continuous Open Bore (3mm wall)
                            |  t_h = 200 mm   |
                            |                 |
                         ___|                 |___
                        [___   Mounting Ring   ___] (4x M8 bolt pads, embedded 8mm)
                            |  Tinaco Adapter |
                            |  90mm or 110mm  |  <--- Engages into tank access port
                            |_________________|
```

#### 1.1 Petal-to-Funnel Rim Junction Detail

Water collected along each petal is guided inward by gravity along a 20° downward incline. To eliminate any retention lip, stagnation pool, or perimeter damming:

```
          Petal Trough Floor (z = 322 mm) \
                                           \  Slope = 20° downward
   Outer Funnel Wall (z = 320 mm) --------->\ 
                                             \
  [==== Funnel Rim Wall ====]                 \ ===> Direct Gravity Drop
         (z = 320 mm)                          |     into Open Bore (d = 219 mm)
                                               |
  Petal Base Embedded 1mm (z = 319 mm) --------+
                                               |
                                        Inner Funnel Bore
```

- **Funnel Top Rim Elevation**: `z = t_h + f_h = 320.0 mm`
- **Petal Base Translation**: `z = t_h + f_h - 1 = 319.0 mm` (1.0 mm overlap for seamless 2-manifold solid boolean union)
- **Petal Trough Inflow Lip**: Floor sits at `z = 322.0 mm` (2.0 mm above the outer funnel rim), discharging directly into the `219.0 mm` diameter open inner funnel throat.
- **Top Bore Clearance**: Inner bore cutter extends continuously through `z = 321.0 mm`, ensuring complete elimination of any internal obstruction or retaining wall.

#### 1.2 Adapter Collar & Mounting Bracket Pad Engagement

The bottom adapter transitions from the standard tinaco tank inlet opening to the central downspout:

```
        Collar Wall at z = 10 mm (r = 44.17 mm)
                       |
                       v
         +-------------+-------------------------+  z = 18 mm (Pad Top)
         |             |      Mounting Pad       |
         |  COLLAR     |      (20mm wide)        |
         |  WALL       |                         |
         | (Embedded)  |       [ M8 Hole ]       |
         |             |       (d = 8.0mm)       |
         |             |                         |
         +-------------+-------------------------+  z = 10 mm (Pad Bottom)
         <-- 7.17mm -->|<-------- 25.83mm ------->
         r = 37.0 mm   r = 44.17 mm              r = 70.0 mm
```

- **Collar Taper**: Linearly tapers from `adapt_out_d` (90 mm or 110 mm) at `z = 0` to `80.0 mm` at `z = 60.0 mm`.
- **Collar Wall Radius at Bracket Plane (`z = 10 mm`)**:
  - Size 1 (90mm collar): `r = 45.0 - (10/60) * 5.0 = 44.17 mm`
  - Size 2 (110mm collar): `r = 55.0 - (10/60) * 15.0 = 52.50 mm`
- **Pad Inset & Embedding**: Mounting pads originate at `r = adapt_out_d / 2 - 8.0 mm` (`37.0 mm` for 90mm collar; `47.0 mm` for 110mm collar). This embeds each bracket pad `7.17 mm` into the solid collar wall, completely eliminating floating gaps and stress-concentration seams.
- **Pad Dimensions**: Length = 33.0 mm (radial span from r=37mm to r=70mm for 90mm preset), Width = 20.0 mm, Height = 8.0 mm (spanning `z = 10.0 mm` to `z = 18.0 mm`).

#### 1.3 Bolt Pattern & Fastener Clearance

- **Fastener Configuration**: 4 radial bracket pads positioned symmetrically at 90° intervals (0°, 90°, 180°, 270°).
- **Bolt Hole Diameter**: 8.0 mm through-holes sized for M8 stainless steel hardware.
- **Bolt Circle Diameter (BCD)**:
  - Size 1 (90mm preset): Center radius `r = 45.0 + 15.0 = 60.0 mm` -> **BCD = 120.0 mm**
  - Size 2 (110mm preset): Center radius `r = 55.0 + 15.0 = 70.0 mm` -> **BCD = 140.0 mm**
- **Cutter Penetration**: Bolt hole cutters are centered at `z = 14.0 mm` with height `h = 30.0 mm` (cutting from `z = -1.0 mm` to `z = +29.0 mm`), guaranteeing complete, clean penetration through both upper (`z = 18 mm`) and lower (`z = 10 mm`) pad faces.

---

### 2. Geometric Construction Envelopes & Footprint

The dimensions below define the **maximum bounding construction envelopes** used for installation clearance checks (e.g., clearance against adjacent roof coping, access ladders, or overflow plumbing) rather than rigid aerodynamic boundaries:

| Geometric Envelope Dimension | Nominal Value | Envelope Description |
| :--- | :---: | :--- |
| **Opposite-Petal Y-Span** | **520.4 mm – 527.4 mm** | Maximum projected horizontal envelope across opposing petal tips |
| **3-Petal Chord / X-Span** | **439.8 mm – 461.4 mm** | Transverse horizontal envelope across adjacent lateral petal lobes |
| **Total Assembly Z-Height** | **378.0 mm** | Overall vertical construction height from adapter base to petal tip |
| **Downspout Tube Section** | 200.0 mm | Height of vertical transition section (`z = 0` to `z = 200 mm`) |
| **Conical Funnel Section** | 120.0 mm | Height of expanding collection funnel (`z = 200` to `z = 320 mm`) |
| **Petal Vertical Rise** | 58.0 mm | Angular vertical rise of angled catchment leaves (`z = 320` to `z = 378 mm`) |
| **Effective Catchment Diameter** | 500.0 mm | Nominal projected circular collection envelope (`~0.20 m²`) |

---

### 3. Deliverables & Presets

The repository includes complete source code and pre-compiled, verified solid models under `cad/`:

1. **Parametric Source (`cad/flower_tinaco_collector.scad`)**: Full OpenSCAD source with parametric overrides for petal count, diameter, heights, and adapter sizing.
2. **Standard 90mm STL (`cad/flower_tinaco_collector_90mm.stl` & `cad/flower_tinaco_collector_default.stl`)**:
   - Manifold Status: **100% 2-manifold solid** (`hasNonManifolds=False`, `isSolid=True`, 0 non-manifold edges)
   - Facet Count: 3,968 facets | Vertices: 1,976
3. **Large 110mm STL (`cad/flower_tinaco_collector_110mm.stl`)**:
   - Manifold Status: **100% 2-manifold solid** (`hasNonManifolds=False`, `isSolid=True`, 0 non-manifold edges)
   - Facet Count: 3,904 facets | Vertices: 1,944
4. **Standard STEP Solid Model (`cad/flower_tinaco_collector.step`)**:
   - Export standard: STEP AP214 automotive/mechanical solid protocol
   - Entity Count: 77,382 entities (Single closed topological solid, valid volume)
   - Fully compatible with FreeCAD, SolidWorks, Autodesk Fusion, and commercial CAM software.

---

### 4. Bill of Materials (BOM)

| Item | Specification | Quantity | Purpose |
| :--- | :--- | :---: | :--- |
| **Collector Body** | PETG / UV-stabilized ASA (3D printed, 4 perimeters, 25% gyroid) OR roto-molded HDPE | 1 | Main rainwater catchment assembly |
| **Mounting Fasteners** | M8 × 35 mm Stainless Steel 304 Hex Head Bolts | 4 | Secures pads to tinaco lid rim |
| **Fastener Washers** | M8 × 24 mm Stainless Steel Flat Washers | 8 | Load distribution across polymer pads |
| **Vibration Dampeners**| M8 EPDM Rubber Washers (2 mm thick) | 8 | Prevents mechanical abrasion on tank wall |
| **Locking Nuts** | M8 Stainless Steel 304 Nylon-Insert Locknuts (Nyloc) | 4 | Resists loosening from thermal cycling |
| **Debris Screen** | 100 mm Stainless Steel 304 Mesh (1.5 mm aperture) | 1 | Pre-filtration of leaves and coarse debris |
| **Sanitary Sealant** | Food-grade or neutral-cure exterior silicone (RTV) | 1 cartridge | Seals collar joint against insect/dust entry |

---

### 5. Step-by-Step Installation Procedure

1. **Pre-Installation Sizing Verification**:
   - Inspect the existing tinaco inspection port or inlet bulkhead.
   - Verify nominal inner diameter: select the **90 mm collar** preset for standard 3" tank openings, or the **110 mm collar** preset for 4" openings.
   - Ensure the rim landing surface has at least 15 mm of flat engagement around the perimeter.

2. **Debris Filter Installation**:
   - Seat the circular 100 mm stainless steel debris filter into the throat of the central downspout at the funnel transition (`z = 200 mm`).
   - Verify seating by pressing gently around the perimeter.

3. **Adapter Engagement & Hole Marking**:
   - Lower the collector adapter collar squarely into the tank opening until the 4 bracket pads rest flush on the tank rim.
   - Using an automatic center punch or marker through the 8 mm bolt clearance holes, mark the 4 hole locations on the tank rim.
   - Lift the collector and drill four 8.5 mm holes through the tank rim using a sharp high-speed steel (HSS) bit at moderate RPM to prevent polymer melting.

4. **Fastening & Weatherproofing**:
   - Apply a continuous 5 mm bead of neutral-cure silicone around the adapter shoulder where it contacts the tank inlet.
   - Place an EPDM washer between each bracket pad and the tank surface.
   - Insert M8 bolts with stainless and EPDM washers from the top.
   - From underneath the tank rim, install stainless flat washers and tighten M8 Nyloc nuts to finger-tight, then torque evenly in a cross pattern to `6 N·m` (do not over-tighten on plastic tanks).
   - Clean any excess silicone bead.

---

### 6. Engineering Scope & Mechanical Validation Protocol

- **Scope of Representation**: The calculations and CAD geometries provided represent nominal kinematic and geometric construction envelopes for pre-fabrication and mechanical interfacing.
- **Empirical Testing Required**:
  - *No untested aerodynamic drag, wind load survival, snow load capacity, or CFD fluid velocity claims are asserted.*
  - Local wind loading depends heavily on building elevation, parapet geometry, and roof exposure. Installers must perform site-specific anchoring evaluations.
  - Periodic inspection of the mesh filter and bolt torque is recommended prior to each rainy season.