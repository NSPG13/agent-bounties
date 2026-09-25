# Flower-Shaped Tinaco Rainwater Collector

## Engineering & Assembly Guide

This document describes the design, pre-fabrication measurements, and assembly procedure for the parametric flower-shaped rainwater collector retrofitted onto standard Mexican tinacos (e.g. Rotoplas, Citijal, 450L–1100L tanks).

---

### 1. Dimensioned Design & Internal Flow Path

The collector features an angled flower-petal catchment funnel feeding directly into a central downspout with a **continuous, unobstructed internal bore** down to an adjustable tinaco mounting collar.

```
       \  Petal R=162mm (concave catchment trough)  /
        \________________                         /
                         \                       /
                          \  Catchment Funnel   /
                           \   f_h = 120mm     /
                            \                 /
                            |                 |
                            |  Downspout Tube |
                            |  Outer:  80 mm  |
                            |  Inner:  74 mm  |  <--- Continuous Open Bore
                            |  t_h = 200 mm   |
                            |                 |
                         ___|                 |___
                        [___   Mounting Ring   ___] (4x M8 bolt points)
                            |  Tinaco Adapter |
                            |  90mm or 110mm  |  <--- Reaches into tank inlet
                            |_________________|
```

#### Pre-Fabrication Measurement Matrix

| Parameter | Default (Standard) | Minimum Range | Maximum Range | Notes |
| :--- | :---: | :---: | :---: | :--- |
| **Petal Count (`petals`)** | 6 | 3 | 12 | Parametric loop in OpenSCAD |
| **Collector Diameter (`diameter`)** | 500 mm | 300 mm | 600 mm | Full catchment span |
| **Funnel Height (`funnel_height`)** | 120 mm | 80 mm | 160 mm | Guides water to throat |
| **Downspout Height (`tube_height`)** | 200 mm | 100 mm | 300 mm | Vertical clearance |
| **Downspout Outer Diameter** | 80 mm | 75 mm | 90 mm | Matches standard 3" PVC |
| **Downspout Inner Bore** | 74 mm | 69 mm | 84 mm | 3mm wall thickness, hollow |
| **Adapter Collar Option 1** | 90 mm | — | — | Standard 3" tank inlet |
| **Adapter Collar Option 2** | 110 mm | — | — | Standard 4" tank inlet |
| **Mounting Fasteners** | 4× M8 holes | — | — | 8mm clearance at 90° intervals |

---

### 2. Standard Tinaco Adapter Sizing

Mexican tinacos typically utilize standard nominal pipe and lid access diameters:
1. **Size 1 (90 mm Collar)**: Direct slip fit into nominal 3-inch female tank inlets and standard tank access ports.
2. **Size 2 (110 mm Collar)**: Direct slip fit into nominal 4-inch female tank inlets or bulkhead overflow adapters.

Switching between adapter sizes in OpenSCAD:
```bash
# Export with 90mm adapter
openscad -Dadapter_type=1 -o cad/flower_tinaco_collector_90mm.stl cad/flower_tinaco_collector.scad

# Export with 110mm adapter
openscad -Dadapter_type=2 -o cad/flower_tinaco_collector_110mm.stl cad/flower_tinaco_collector.scad
```

Pre-rendered STL exports are included in the repository under `cad/`:
- `cad/flower_tinaco_collector_default.stl`
- `cad/flower_tinaco_collector_90mm.stl`
- `cad/flower_tinaco_collector_110mm.stl`

---

### 3. Bill of Materials (BOM)

- **Main Collector Body**: 3D printed PETG / UV-resistant ASA or molded recycled HDPE / 1.5mm galvanized steel sheets.
- **Fasteners**: 4× M8 × 35mm stainless steel 304 bolts, washers, and nylon locking nuts.
- **Pre-Filtration**: 100mm circular stainless steel or nylon mesh (1.5mm aperture) positioned in the collection throat to prevent leaf debris from entering the tinaco.
- **Gasket / Sealant**: EPDM rubber gasket or neutral-cure exterior silicone sealant around the tinaco rim interface.

---

### 4. Step-by-Step Assembly Procedure

1. **Pre-Fit Verification**:
   - Measure the tinaco inlet opening diameter (verify whether 90mm or 110mm adapter is required).
   - Ensure a clean, horizontal landing rim for the 4 mounting bracket pads.
2. **Mesh Filter Placement**:
   - Insert the stainless steel debris screen into the upper throat of the 80mm central downspout.
3. **Mounting Collar Attachment**:
   - Lower the collector adapter into the tinaco inlet.
   - Align the 4 mounting brackets with the tank rim.
   - Mark and drill four 8.5mm holes for the M8 mounting bolts.
4. **Fastening & Sealing**:
   - Place EPDM rubber washers between brackets and tank body to prevent vibration.
   - Fasten with M8 stainless bolts and nylon locking nuts.
   - Apply a continuous bead of silicone around the adapter seam to prevent dust ingress.

---

### 5. Design Assumptions & Engineering Notes

- **Rainfall Inflow Assumption**: Estimated nominal collection area of ~0.20 m² (at 500mm diameter). Theoretical inflow rate scales directly with local rainfall intensity.
- **Wind & Environmental Resistance**: Designed with symmetrical curved petal profiles to minimize aerodynamic drag during gusting winds. The 3mm nominal wall thickness provides adequate rigidity for rooftop residential installations.
- **Maintenance**: The open flower geometry allows visual inspection from ground level; the removable mesh filter should be flushed before and after each wet season.