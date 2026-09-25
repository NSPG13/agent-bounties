#!/usr/bin/env python3
"""Regenerate and check every export from flower_tinaco_collector.scad.

    python3 export.py            # STL, STEP, DXF, SVG views, drawing, evidence
    python3 export.py --check    # only reopen and verify the committed exports

Needs `openscad` (2021.01 or newer) on PATH. The STEP export and all reopen
checks use OpenCASCADE through `pip install cadquery-ocp`.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import struct
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCAD = HERE / "flower_tinaco_collector.scad"
OUT = HERE / "exports"
STL = OUT / "flower_tinaco_collector.stl"
STEP = OUT / "flower_tinaco_collector.step"
ADAPTER_STL = OUT / "adapter.stl"
DXF = OUT / "petal_flat.dxf"
VIEWS = {"section_a": OUT / "section_a.svg", "section_b": OUT / "section_b.svg",
         "plan": OUT / "plan.svg", "petal_flat": OUT / "petal_flat.svg"}
DRAWING = OUT / "assembly_drawing.svg"
DIMS = OUT / "dimensions.json"
EVIDENCE = OUT / "EVIDENCE.md"


def openscad(out: Path, part: str | None = None) -> str:
    cmd = ["openscad", "--hardwarnings", "-o", str(out)]
    if part:
        cmd += ["-D", f'part="{part}"']
    cmd.append(str(SCAD))
    proc = subprocess.run(cmd, capture_output=True, text=True)
    log = proc.stdout + proc.stderr
    if proc.returncode != 0 or "WARNING" in log:
        sys.exit(f"openscad failed for {out.name}:\n{log[-2000:]}")
    return log


def parse_dims(log: str) -> dict[str, float]:
    dims: dict[str, float] = {}
    for line in log.splitlines():
        if 'ECHO: "DIM ' in line:
            for key, val in re.findall(r"(\w+)=([-0-9.eE]+)", line):
                dims[key] = float(val)
    return dims


# ------------------------------------------------------------------ STL tools
def read_stl(path: Path) -> list[tuple[tuple[float, float, float], ...]]:
    data = path.read_bytes()
    tris = []
    if data[:5] == b"solid" and b"facet" in data[:400]:
        verts = [tuple(map(float, m.split())) for m in
                 re.findall(rb"vertex\s+([^\n]+)", data)]
        tris = [tuple(verts[i:i + 3]) for i in range(0, len(verts), 3)]
    else:
        n = struct.unpack("<I", data[80:84])[0]
        for i in range(n):
            v = struct.unpack("<12f", data[84 + 50 * i: 84 + 50 * i + 48])
            tris.append((v[3:6], v[6:9], v[9:12]))
    return tris


def stl_stats(path: Path) -> dict[str, object]:
    tris = read_stl(path)
    edges: dict[tuple, int] = {}
    vol = 0.0
    lo = [1e18] * 3
    hi = [-1e18] * 3
    for a, b, c in tris:
        vol += (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0
        for p, q in ((a, b), (b, c), (c, a)):
            k = (p, q) if p < q else (q, p)
            edges[k] = edges.get(k, 0) + 1
        for v in (a, b, c):
            for i in range(3):
                lo[i] = min(lo[i], v[i])
                hi[i] = max(hi[i], v[i])
    bad = sum(1 for n in edges.values() if n != 2)
    return {"triangles": len(tris), "volume_mm3": round(vol, 1),
            "non_manifold_edges": bad, "bbox_min": [round(x, 2) for x in lo],
            "bbox_max": [round(x, 2) for x in hi]}


# ------------------------------------------------------------------ STEP tools
def stl_to_step(stl: Path, step: Path) -> None:
    from OCP.BRep import BRep_Builder
    from OCP.BRepBuilderAPI import BRepBuilderAPI_MakeSolid, BRepBuilderAPI_Sewing
    from OCP.IFSelect import IFSelect_RetDone
    from OCP.Interface import Interface_Static
    from OCP.RWStl import RWStl
    from OCP.ShapeFix import ShapeFix_Solid
    from OCP.ShapeUpgrade import ShapeUpgrade_UnifySameDomain
    from OCP.STEPControl import STEPControl_AsIs, STEPControl_Writer
    from OCP.TopAbs import TopAbs_SHELL
    from OCP.TopExp import TopExp_Explorer
    from OCP.TopoDS import TopoDS, TopoDS_Compound
    from OCP.BRepBuilderAPI import BRepBuilderAPI_MakePolygon, BRepBuilderAPI_MakeFace
    from OCP.gp import gp_Pnt

    tri = RWStl.ReadFile_s(str(stl))
    sew = BRepBuilderAPI_Sewing(1e-3)
    for i in range(1, tri.NbTriangles() + 1):
        n1, n2, n3 = tri.Triangle(i).Get()
        pts = [tri.Node(n) for n in (n1, n2, n3)]
        poly = BRepBuilderAPI_MakePolygon(pts[0], pts[1], pts[2], True)
        if not poly.IsDone():
            continue
        face = BRepBuilderAPI_MakeFace(poly.Wire(), True)
        if face.IsDone():
            sew.Add(face.Face())
    sew.Perform()
    sewed = sew.SewedShape()
    builder = BRep_Builder()
    comp = TopoDS_Compound()
    builder.MakeCompound(comp)
    exp = TopExp_Explorer(sewed, TopAbs_SHELL)
    while exp.More():
        mk = BRepBuilderAPI_MakeSolid(TopoDS.Shell(exp.Current()) if hasattr(TopoDS, "Shell") else TopoDS.Shell_s(exp.Current()))
        fix = ShapeFix_Solid(mk.Solid())
        fix.Perform()
        builder.Add(comp, fix.Solid())
        exp.Next()
    unify = ShapeUpgrade_UnifySameDomain(comp, True, True, False)
    unify.Build()
    shape = unify.Shape()
    writer = STEPControl_Writer()
    Interface_Static.SetCVal_s("write.step.schema", "AP214")
    Interface_Static.SetCVal_s("write.step.unit", "MM")
    writer.Transfer(shape, STEPControl_AsIs)
    if writer.Write(str(step)) != IFSelect_RetDone:
        sys.exit("STEP write failed")


def write_step(dims: dict[str, float], step: Path) -> None:
    """Exact B-rep (cones, cylinders, planes) rebuilt from the same parameters."""
    from OCP.IFSelect import IFSelect_RetDone
    from OCP.Interface import Interface_Static
    from OCP.STEPControl import STEPControl_AsIs, STEPControl_Writer
    from step_model import build

    shape = build(dims)
    writer = STEPControl_Writer()
    Interface_Static.SetCVal_s("write.step.schema", "AP214")
    Interface_Static.SetCVal_s("write.step.unit", "MM")
    writer.Transfer(shape, STEPControl_AsIs)
    if writer.Write(str(step)) != IFSelect_RetDone:
        sys.exit("STEP write failed")


def step_stats(step: Path) -> dict[str, object]:
    from OCP.BRepCheck import BRepCheck_Analyzer
    from OCP.BRepGProp import BRepGProp
    from OCP.GProp import GProp_GProps
    from OCP.IFSelect import IFSelect_RetDone
    from OCP.STEPControl import STEPControl_Reader
    from OCP.TopAbs import TopAbs_FACE, TopAbs_SOLID
    from OCP.TopExp import TopExp_Explorer

    reader = STEPControl_Reader()
    if reader.ReadFile(str(step)) != IFSelect_RetDone:
        return {"reopened": False}
    reader.TransferRoots()
    shape = reader.OneShape()
    props = GProp_GProps()
    BRepGProp.VolumeProperties_s(shape, props)

    def count(kind: int) -> int:
        exp, n = TopExp_Explorer(shape, kind), 0
        while exp.More():
            n += 1
            exp.Next()
        return n

    return {"reopened": True, "valid": bool(BRepCheck_Analyzer(shape).IsValid()),
            "solids": count(TopAbs_SOLID), "faces": count(TopAbs_FACE),
            "volume_mm3": round(props.Mass(), 1)}


# ------------------------------------------------------------------ drawing
def svg_path(view: Path) -> tuple[str, list[float]]:
    text = view.read_text()
    vb = [float(x) for x in re.search(r'viewBox="([^"]+)"', text).group(1).split()]
    d = " ".join(re.findall(r'<path d="([^"]+)"', text))
    return re.sub(r"\s+", " ", d), vb


def build_drawing(d: dict[str, float], stats: dict[str, object]) -> str:
    W, H = 841, 594  # A1 landscape, mm
    parts: list[str] = []
    add = parts.append

    def text(x, y, s, size=3.5, anchor="start", weight="normal", fill="#000"):
        add(f'<text x="{x:.2f}" y="{y:.2f}" font-size="{size}" text-anchor="{anchor}" '
            f'font-weight="{weight}" fill="{fill}" font-family="DejaVu Sans, Arial">{s}</text>')

    def line(x1, y1, x2, y2, w=0.25, dash=None, color="#000", marker=False):
        extra = f' stroke-dasharray="{dash}"' if dash else ""
        mk = ' marker-start="url(#a)" marker-end="url(#a)"' if marker else ""
        add(f'<line x1="{x1:.2f}" y1="{y1:.2f}" x2="{x2:.2f}" y2="{y2:.2f}" stroke="{color}" '
            f'stroke-width="{w}"{extra}{mk}/>')

    def view(name, ox, oy, k, clip=None):
        dpath, _ = svg_path(VIEWS[name])
        cid = ""
        if clip:
            x0, y0, x1, y1 = clip
            add(f'<clipPath id="c_{name}_{int(k*100)}"><rect x="{x0}" y="{y0}" width="{x1-x0}" '
                f'height="{y1-y0}"/></clipPath>')
            cid = f' clip-path="url(#c_{name}_{int(k*100)})"'
        add(f'<g transform="translate({ox:.2f},{oy:.2f}) scale({k})"{cid}>'
            f'<path d="{dpath}" fill="#cfd8dc" stroke="#000" stroke-width="{0.3 / k:.2f}" '
            f'fill-rule="evenodd"/></g>')

    def hdim(ox, oy, k, x1, x2, z, label, off=8):
        y = oy - z * k - off
        line(ox + x1 * k, oy - z * k, ox + x1 * k, y - 1.5, 0.18)
        line(ox + x2 * k, oy - z * k, ox + x2 * k, y - 1.5, 0.18)
        line(ox + x1 * k, y, ox + x2 * k, y, 0.18, marker=True)
        text(ox + (x1 + x2) / 2 * k, y - 1.2, label, 3.2, "middle")

    def zlevel(ox, oy, k, x_from, x_to, z, label):
        line(ox + x_from * k, oy - z * k, ox + x_to * k, oy - z * k, 0.15, "1.5,1")
        text(ox + x_to * k + 1.5, oy - z * k + 1.1, label, 2.8)

    add(f'<rect x="10" y="10" width="{W-20}" height="{H-20}" fill="none" stroke="#000" stroke-width="0.7"/>')

    # Section A-A (1:4) with drainage route
    k, ox, oy = 0.25, 225, 150
    text(30, 30, "SECTION A-A (between ribs, through a petal tip and a vent) · scale 1:4", 5, weight="bold")
    view("section_a", ox, oy, k)
    R, zt = d["tip_radius_mm"], d["z_tip_top"]
    hdim(ox, oy, k, -R, R, zt, f"Ø{2*R:.0f} petal tips (plan)", 10)
    for z, lab in ((zt, f"z {zt:.0f} petal tip top"), (d["z_funnel_lip"], f"z {d['z_funnel_lip']:.0f} funnel lip"),
                   (d["z_funnel_bottom"], f"z {d['z_funnel_bottom']:.0f} funnel to outlet tube"),
                   (0, "z 0 = top of tinaco lid neck"), (d["outlet_bottom_z"], f"z {d['outlet_bottom_z']:.0f} tube end in tank")):
        zlevel(ox, oy, k, 0, R + 30, z, lab)
    # flow arrows
    flow = [(-R + 40, zt - 12), (-d["funnel_lip_radius_mm"], d["z_funnel_lip"] + 4),
            (-40, d["z_funnel_bottom"] + 12), (0, d["z_funnel_bottom"]), (0, d["outlet_bottom_z"] - 30)]
    pts = " ".join(f"{ox + x * k:.2f},{oy - z * k:.2f}" for x, z in flow)
    add(f'<polyline points="{pts}" fill="none" stroke="#1565c0" stroke-width="0.8" '
        f'stroke-dasharray="3,1.5" marker-end="url(#f)"/>')
    text(30, 37, "Drainage (blue): rain → petal (15° fall) → petal edge laps 40 mm inside the funnel lip → "
         "funnel (40°) → Ø110 tube → tank. Funnel and petals are carried by 4 ribs (B-B).", 3.2, fill="#1565c0")

    # Section B-B (1:4)
    oy2 = 305
    text(30, 190, "SECTION B-B (through two support ribs) · scale 1:4", 5, weight="bold")
    view("section_b", ox, oy2, k)
    text(30, 197, f"4 ribs × 8 mm tie the funnel wall and petal underside to the adapter plate and socket; "
         f"rib height {d['rib_height_at_reach']:.0f} mm at r 520.", 3.2)
    hdim(ox, oy2, k, -520, 520, d["z_tip_top"], "rib reach Ø1040", 4)

    # Detail C: adapter and outlet (1:2)
    k3, ox3, oy3 = 0.5, 190, 505
    text(30, 345, "DETAIL C: tinaco adapter and outlet (from A-A) · scale 1:2", 5, weight="bold")
    view("section_a", ox3, oy3, k3, clip=(-300, -250, 300, 175))
    ad = d["adapter_outer_d"]
    hdim(ox3, oy3, k3, -ad / 2, ad / 2, d["z_plate_top"], f"adapter plate Ø{ad:.0f} × {d['z_plate_top'] - d['z_plate_bottom']:.0f}", 45)
    si = d["skirt_inner_d"]
    line(ox3 - si / 2 * k3, oy3 + 34, ox3 + si / 2 * k3, oy3 + 34, 0.18, marker=True)
    text(ox3, oy3 + 39, f"skirt bore Ø{si:.0f} = measured neck OD + 2 × 3 mm; band clamp in groove, 8 relief slots", 3, "middle")
    line(ox3 - 55 * k3, oy3 + 60, ox3 + 55 * k3, oy3 + 60, 0.18, marker=True)
    text(ox3, oy3 + 66, "outlet PVC Ø110 in socket Ø111.5 × 60 (EPDM lip seal)", 3, "middle")
    for z, lab in ((d["z_socket_top"], f"z {d['z_socket_top']:.0f} socket top"),
                   (d["z_plate_top"], f"z {d['z_plate_top']:.0f} plate top; z {d['z_plate_bottom']:.0f}–0 EPDM gasket on neck rim"),):
        zlevel(ox3, oy3, k3, 60, 310, z, lab)

    # Plan (1:10)
    k4, ox4, oy4 = 0.1, 585, 115
    text(505, 30, "PLAN · scale 1:10", 5, weight="bold")
    view("plan", ox4, oy4, k4)
    text(ox4, oy4 + 84, f"8 petals: tips Ø{d['tip_radius_mm']*2:.0f}, notches Ø{d['valley_radius_mm']*2:.0f}", 3, "middle")

    # Developed petal (1:10)
    k5, ox5, oy5 = 0.1, 680, 115
    text(690, 30, "PETAL FLAT PATTERN · 1:10", 5, weight="bold")
    view("petal_flat", ox5, oy5, k5)
    text(690, oy5 + 50, f"petal_flat.dxf · cut 8", 3)
    text(690, oy5 + 55, f"inner R {d['petal_flat_inner_rho']:.1f} · tip R {d['petal_flat_outer_rho']:.1f}", 3)
    text(690, oy5 + 60, "rolls to the 15° cone (developed)", 3)
    text(690, oy5 + 65, "30 mm seam lap · Ø6.5 holes (M6)", 3)
    text(690, oy5 + 70, "Ø10 tie hole at the tip", 3)

    # Parts list and site measurements
    y = 222
    text(505, y, "PARTS LIST (default parameters)", 4.5, weight="bold")
    bom = [
        "1  Petal, 3 mm UV-stabilised HDPE, from petal_flat.dxf ........ 8",
        "2  Funnel Ø360 → Ø110, 40°, with 60 mm sloped flange .......... 1",
        f"3  Outlet tube PVC sanitario Ø110 × {d['z_funnel_bottom'] - d['outlet_bottom_z']:.0f} mm (z −150 … +92) ... 1",
        "4  Leaf screen Ø220 × 3, 24 × 6 mm slots + 0.5 mm steel mesh .. 1",
        f"5  Adapter: plate Ø{ad:.0f} × 8, skirt × 60, socket, 4 ribs ......... 1",
        "6  EPDM gasket Ø485 × 18 × 4 (cut to measured neck) ........... 1",
        "7  Stainless worm-drive band for the skirt groove ............. 1",
        "8  EPDM lip seal for Ø110 socket ................................ 1",
        "9  M6 stainless bolt + 2 washers + nyloc: 24 seam + 24 flange .. 48",
        "10 L-brackets + M6 at rib tops (petal and funnel; not modelled) 8",
        "11 Stainless mesh discs over the 4 Ø30 vents .................. 4",
        "12 Guy lines to roof anchors, from Ø10 tip tie holes ........... 4",
    ]
    for i, row in enumerate(bom):
        text(505, y + 7 + i * 5, row, 3, fill="#000")
    y2 = y + 7 + len(bom) * 5 + 6
    text(505, y2, "MEASURE BEFORE FABRICATION → parameter", 4.5, weight="bold")
    meas = [
        "M1 Lid-neck outside Ø at the rim, 3 readings 60° apart → neck_od_mm",
        "M2 Neck height above tank shoulder (≥ 60 mm free) → skirt_h_mm",
        "M3 Rim width / flatness under the gasket → gasket_w_mm",
        "M4 Float valve and inlet position under the neck → drop_tube_mm",
        "M5 Free height above tank (≥ 400 mm) and radius (≥ 850 mm)",
        "M6 Tank overflow pipe present and clear (collector has none)",
        "M7 Anchor points for 4 guy lines on parapet / roof slab",
        "M8 Lid vent location (replaced by the 4 screened vents)",
    ]
    for i, row in enumerate(meas):
        text(505, y2 + 7 + i * 5, row, 3)

    # Title block
    add('<rect x="561" y="514" width="270" height="70" fill="none" stroke="#000" stroke-width="0.5"/>')
    text(566, 524, "Flower rainwater collector for a tinaco (#1505)", 4.5, weight="bold")
    text(566, 531, "Source: flower_tinaco_collector.scad (all views generated by export.py)", 3)
    text(566, 537, f"Default parameters · units mm · z 0 = top of lid neck · tip top z {d['z_tip_top']:.0f}", 3)
    text(566, 543, f"Plan catchment {d['plan_catchment_area_m2']:.3f} m² (model outline; yield is an assumption)", 3)
    sv = stats.get("stl", {})
    text(566, 549, f"STL {sv.get('triangles', '?')} tris, volume {float(sv.get('volume_mm3', 0))/1e6:.2f} dm³ (all parts)", 3)
    text(566, 555, "Concept design: not a certified roof or potable-water product.", 3)
    text(566, 561, "Wind loads not analysed: guy lines required; remove before storms.", 3)
    text(566, 572, "Sheet A1 · 1/1", 3)

    defs = ('<defs><marker id="a" viewBox="0 0 10 10" refX="5" refY="5" markerWidth="3" markerHeight="3" '
            'orient="auto-start-reverse"><path d="M0,1 L10,5 L0,9 z"/></marker>'
            '<marker id="f" viewBox="0 0 10 10" refX="5" refY="5" markerWidth="4" markerHeight="4" orient="auto">'
            '<path d="M0,1 L10,5 L0,9 z" fill="#1565c0"/></marker></defs>')
    return (f'<?xml version="1.0" encoding="UTF-8"?>\n<svg xmlns="http://www.w3.org/2000/svg" '
            f'width="{W}mm" height="{H}mm" viewBox="0 0 {W} {H}">\n{defs}\n'
            f'<rect width="{W}" height="{H}" fill="#fff"/>\n' + "\n".join(parts) + "\n</svg>\n")


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def check(stats_out: dict[str, object]) -> bool:
    ok = True
    s = stl_stats(STL)
    stats_out["stl"] = s
    if s["non_manifold_edges"] or s["volume_mm3"] <= 0:
        print("STL not watertight:", s)
        ok = False
    stats_out["adapter_stl"] = stl_stats(ADAPTER_STL)
    st = step_stats(STEP)
    stats_out["step"] = st
    if not (st.get("reopened") and st.get("valid") and st.get("solids", 0) >= 1):
        print("STEP reopen/validity failed:", st)
        ok = False
    else:
        rel = abs(st["volume_mm3"] - s["volume_mm3"]) / s["volume_mm3"]
        stats_out["step_vs_stl_volume_rel_diff"] = round(rel, 6)
        if rel > 0.005:
            print(f"STEP volume differs from STL by {rel:.3%}")
            ok = False
    dxf = DXF.read_text()
    stats_out["dxf"] = {"entities_lwpolyline_or_line": dxf.count("\nLWPOLYLINE") + dxf.count("\nLINE"),
                        "closed_loops": dxf.count("\nLWPOLYLINE")}
    return ok


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()
    OUT.mkdir(exist_ok=True)
    stats: dict[str, object] = {}
    if not args.check:
        log = openscad(STL)
        dims = parse_dims(log)
        DIMS.write_text(json.dumps(dims, indent=2, sort_keys=True) + "\n")
        openscad(ADAPTER_STL, "adapter")
        openscad(DXF, "petal_flat")
        for part, path in VIEWS.items():
            openscad(path, part)
        write_step(json.loads(DIMS.read_text()), STEP)
    dims = json.loads(DIMS.read_text())
    ok = check(stats)
    if not args.check:
        DRAWING.write_text(build_drawing(dims, stats))
        files = [SCAD, STL, STEP, ADAPTER_STL, DXF, DRAWING, *VIEWS.values(), DIMS]
        ver = subprocess.run(["openscad", "--version"], capture_output=True, text=True)
        lines = ["# Export evidence", "",
                 f"Generated by `python3 export.py` with {(ver.stdout + ver.stderr).strip()} "
                 "and OpenCASCADE (cadquery-ocp) for STEP and the reopen checks.", "",
                 "## Reopen checks", "", "```json", json.dumps(stats, indent=2), "```", "",
                 "## SHA-256", "", "| File | SHA-256 |", "| --- | --- |"]
        lines += [f"| `{p.relative_to(HERE)}` | `{sha256(p)}` |" for p in files]
        EVIDENCE.write_text("\n".join(lines) + "\n")
    print(json.dumps(stats, indent=2))
    print("OK" if ok else "FAILED")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
