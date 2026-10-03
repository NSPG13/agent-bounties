"""Exact B-rep rebuild of flower_tinaco_collector.scad for the STEP export.

Every dimension comes from the parameters OpenSCAD echoes (exports/dimensions.json),
and each function mirrors the SCAD module of the same name. export.py checks
that this solid matches the OpenSCAD STL (volume and bounding box).
"""
from __future__ import annotations

import math

from OCP.BRepAlgoAPI import BRepAlgoAPI_Common, BRepAlgoAPI_Cut, BRepAlgoAPI_Fuse
from OCP.BRepBuilderAPI import (BRepBuilderAPI_MakeFace, BRepBuilderAPI_MakePolygon,
                                BRepBuilderAPI_Transform)
from OCP.BRepPrimAPI import (BRepPrimAPI_MakeBox, BRepPrimAPI_MakeCylinder,
                             BRepPrimAPI_MakePrism, BRepPrimAPI_MakeRevol)
from OCP.gp import gp_Ax1, gp_Ax2, gp_Dir, gp_Pnt, gp_Trsf, gp_Vec
try:  # OCP >= 7.9 / 8.x
    from OCP.collections import List_TopoDS_Shape as _ShapeList
except ImportError:  # older cadquery-ocp
    from OCP.TopTools import TopTools_ListOfShape as _ShapeList

Z = gp_Ax1(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1))


def _face(pts3):
    poly = BRepBuilderAPI_MakePolygon()
    for p in pts3:
        poly.Add(gp_Pnt(*p))
    poly.Close()
    return BRepBuilderAPI_MakeFace(poly.Wire(), True).Face()


def revolve(rz):
    return BRepPrimAPI_MakeRevol(_face([(r, 0.0, z) for r, z in rz]), Z, 2 * math.pi).Shape()


def prism_xy(pts, z0, h):
    return BRepPrimAPI_MakePrism(_face([(x, y, z0) for x, y in pts]), gp_Vec(0, 0, h)).Shape()


def spline_prism_xy(pts, z0, h):
    """Periodic B-spline through the outline points (one smooth side face)."""
    from OCP.BRepBuilderAPI import BRepBuilderAPI_MakeEdge, BRepBuilderAPI_MakeWire
    from OCP.GeomAPI import GeomAPI_Interpolate
    try:
        from OCP.collections import HArray1_gp_Pnt as TColgp_HArray1OfPnt
    except ImportError:
        from OCP.TColgp import TColgp_HArray1OfPnt

    arr = TColgp_HArray1OfPnt(1, len(pts))
    for i, (x, y) in enumerate(pts, start=1):
        arr.SetValue(i, gp_Pnt(x, y, z0))
    interp = GeomAPI_Interpolate(arr, True, 1e-6)
    interp.Perform()
    edge = BRepBuilderAPI_MakeEdge(interp.Curve()).Edge()
    wire = BRepBuilderAPI_MakeWire(edge).Wire()
    face = BRepBuilderAPI_MakeFace(wire, True).Face()
    return BRepPrimAPI_MakePrism(face, gp_Vec(0, 0, h)).Shape()


def cyl(r, h, x=0.0, y=0.0, z=0.0):
    return BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(x, y, z), gp_Dir(0, 0, 1)), r, h).Shape()


def box(x, y, z, dx, dy, dz):
    return BRepPrimAPI_MakeBox(gp_Pnt(x, y, z), dx, dy, dz).Shape()


def rot(shape, deg):
    t = gp_Trsf()
    t.SetRotation(Z, math.radians(deg))
    return BRepBuilderAPI_Transform(shape, t, True).Shape()


def _list(shapes):
    lst = _ShapeList()
    for s in shapes:
        lst.Append(s)
    return lst


def cut(base, tools):
    op = BRepAlgoAPI_Cut()
    op.SetArguments(_list([base]))
    op.SetTools(_list(tools))
    op.SetFuzzyValue(1e-4)
    op.Build()
    return op.Shape()


def fuse(shapes):
    op = BRepAlgoAPI_Fuse()
    op.SetArguments(_list(shapes[:1]))
    op.SetTools(_list(shapes[1:]))
    op.SetFuzzyValue(1e-4)
    op.Build()
    return op.Shape()


def build(d: dict, parts: bool = False) -> object:
    n = int(d["petal_count"]); R = d["collector_diameter_mm"] / 2
    notch = d["petal_notch_mm"]; ps = math.radians(d["petal_slope_deg"])
    sheet = d["sheet_mm"]; fs = math.radians(d["funnel_slope_deg"])
    r_oo = d["outlet_od_mm"] / 2; r_oi = r_oo - d["outlet_wall_mm"]
    r_ft = d["funnel_top_d_mm"] / 2; r_pi = r_ft - d["petal_lap_mm"]
    r_fl = r_ft + d["flange_w_mm"]; sector = 360.0 / n
    tp, tf = math.tan(ps), math.tan(fs)
    t_pv, t_fv = sheet / math.cos(ps), sheet / math.cos(fs)
    r_neck_in = d["neck_od_mm"] / 2 + d["neck_clearance_mm"]
    r_plate = r_neck_in + d["skirt_wall_mm"]
    r_sock_in = r_oo + d["socket_clearance_mm"]; r_sock_out = r_sock_in + d["socket_wall_mm"]
    z_plate_b = d["gasket_t_mm"]; z_plate_t = z_plate_b + d["plate_t_mm"]
    z_sock_t = z_plate_t + d["socket_h_mm"]; z_fb = z_sock_t + 20
    z_ft = z_fb + (r_ft - r_oi) * tf
    drop = d["drop_tube_mm"]; skirt_h = d["skirt_h_mm"]; skirt_wall = d["skirt_wall_mm"]
    reach = d["rib_reach_mm"]; r_valley = R - notch

    def z_fin(r): return z_fb + (r - r_oi) * tf
    def z_pbot(r): return z_ft + (r - r_ft) * tp
    def r_edge(a): return R - notch * math.sin(math.radians(n * a / 2)) ** 2

    n_out = n * int(d["flower_steps"])
    flower = [(r_edge(i * 360 / n_out) * math.cos(math.radians(i * 360 / n_out)),
               r_edge(i * 360 / n_out) * math.sin(math.radians(i * 360 / n_out))) for i in range(n_out)]
    seam = [(r_fl + (r_valley - 30 - r_fl) * f, s * sector / 2) for s in (-1, 1) for f in (0.30, 0.55, 0.80)]
    flange = [(r_ft + d["flange_w_mm"] / 2, a) for a in (-sector / 4, 0, sector / 4)]
    tie = (R - 30, 0.0)

    def holes(pts, dia):
        out = []
        for k in range(n):
            for r, a in pts:
                ang = math.radians(a + k * sector)
                out.append(cyl(dia / 2, 2000, r * math.cos(ang), r * math.sin(ang), -1000))
        return out

    # petals
    shell = revolve([(r_pi, z_pbot(r_pi)), (R + 20, z_pbot(R + 20)),
                     (R + 20, z_pbot(R + 20) + t_pv), (r_pi, z_pbot(r_pi) + t_pv)])
    petals = BRepAlgoAPI_Common(shell, prism_xy(flower, z_ft - 200, 800)).Shape()
    petals = cut(petals, holes(seam + flange, 6.5) + holes([tie], 10))

    # funnel
    cone = revolve([(r_oi, z_fb), (r_ft, z_ft), (r_ft + sheet / math.sin(fs), z_ft),
                    (r_oo, z_fb - (r_oo - r_oi) / tf), (r_oo, z_fb)])
    fl = revolve([(r_ft, z_ft - t_pv), (r_fl, z_pbot(r_fl) - t_pv),
                  (r_fl, z_pbot(r_fl) + t_pv / 2), (r_ft, z_ft + t_pv / 2)])
    tube = cut(cyl(r_oo, z_fb + drop, z=-drop), [cyl(r_oi, z_fb + drop + 2, z=-drop - 1)])
    funnel = cut(fuse([cone, fl, tube]), holes(flange, 6.5))

    # screen
    rs = d["screen_d_mm"] / 2; w = d["screen_slot_mm"]; zs = z_fin(rs) - 1
    slots = [rot(box(25, -w / 2, zs - 1, rs - 25 - 12, w, sheet + 2), k * 360 / d["screen_slots"])
             for k in range(int(d["screen_slots"]))]
    screen = cut(cyl(rs, sheet, z=zs), slots + [cyl(15, sheet + 2, z=zs - 1)])

    # adapter
    plate = cyl(r_plate, d["plate_t_mm"], z=z_plate_b)
    skirt = cut(cyl(r_plate, skirt_h, z=z_plate_b - skirt_h),
                [cyl(r_neck_in, skirt_h + 2, z=z_plate_b - skirt_h - 1),
                 cut(cyl(r_plate + 1, 16, z=z_plate_b - skirt_h + skirt_h * 0.35),
                     [cyl(r_plate - 1.5, 16, z=z_plate_b - skirt_h + skirt_h * 0.35)])])
    socket = cyl(r_sock_out, d["socket_h_mm"], z=z_plate_t)
    m = 12
    rib_pts = ([(r_sock_out - 1, z_plate_t - 1), (r_plate, z_plate_t - 1), (reach, z_pbot(reach) + t_pv * 0.4)]
               + [(reach - (reach - r_ft) * i / m, z_pbot(reach - (reach - r_ft) * i / m) + t_pv * 0.4)
                  for i in range(1, m + 1)]
               + [(r_ft - (r_ft - (r_oo + 4)) * i / m, z_fin(r_ft - (r_ft - (r_oo + 4)) * i / m) - t_fv / 2)
                  for i in range(1, m + 1)]
               + [(r_oo + 4, z_sock_t + 4), (r_sock_out - 1, z_sock_t)])
    t = d["rib_t_mm"]
    rib0 = BRepPrimAPI_MakePrism(_face([(r, -t / 2, z) for r, z in rib_pts]), gp_Vec(0, t, 0)).Shape()
    ribs = [rot(rib0, k * 360 / d["rib_count"]) for k in range(int(d["rib_count"]))]
    body = fuse([plate, skirt, socket] + ribs)
    ns = int(d["skirt_slots"])
    tools = [cyl(r_sock_in, d["socket_h_mm"] + d["plate_t_mm"] + 2, z=z_plate_b - 1)]
    tools += [rot(box(r_neck_in - 1, -1.5, z_plate_b - skirt_h - 1, skirt_wall + 2, 3, skirt_h * 0.7 + 1),
                  k * 360 / ns + 180 / ns) for k in range(ns)]
    nv = int(d["vent_count"]); rv = (r_sock_out + r_neck_in - 10) / 2
    tools += [cyl(d["vent_d_mm"] / 2, 100, rv * math.cos(math.radians(k * 360 / nv + 180 / nv)),
                  rv * math.sin(math.radians(k * 360 / nv + 180 / nv)), -50) for k in range(nv)]
    adapter = cut(body, tools)

    gasket = cut(cyl(d["neck_od_mm"] / 2, d["gasket_t_mm"]),
                 [cyl(d["neck_od_mm"] / 2 - d["gasket_w_mm"], d["gasket_t_mm"] + 2, z=-1)])
    if parts:
        return {"petals": petals, "funnel": funnel, "screen": screen, "adapter": adapter, "gasket": gasket}
    shape = petals
    for other in (funnel, adapter, screen, gasket):
        shape = BRepAlgoAPI_Fuse(shape, other).Shape()
    return shape
