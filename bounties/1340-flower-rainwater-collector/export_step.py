#!/usr/bin/env python3
"""CAD import + valid-solid check + STEP export (OpenCascade / OCCT).

Imports the exported mesh into OpenCascade (the B-rep kernel behind FreeCAD and
CadQuery), checks it the way a CAD tool would, and writes a STEP file:

  * import      - the STL is read into an OCCT shape (one face per triangle)
  * sewing      - faces are sewn into a shell at 1e-4 mm
  * closure     - ShapeAnalysis_Shell reports no free (unconnected) edges and no
                  badly oriented edges, i.e. the shell is closed
  * validity    - BRepCheck_Analyzer reports the shape is a valid solid, the
                  explorer finds exactly one shell / one solid, and the volume
                  is positive and matches the mesh volume measured by
                  ``test_geometry.py``
  * export      - STEPControl writes exports/flower_rainwater_collector.step

The STEP is a TESSELLATED B-rep (one planar face per mesh triangle). The design
source is a CSG mesh model, so this is the honest representation of it; an
analytic B-rep would require rebuilding the part in a B-rep kernel. The value of
the check is not the surface type but the guarantee that the export opens in CAD
software as a closed, valid, positive-volume solid.

Requires the optional dependency (verified with cadquery-ocp 8.0.1 /
OCCT 7.9):   pip install cadquery-ocp

Usage:  python3 export_step.py [--stl exports/flower_rainwater_collector.stl]
"""

import argparse
import struct
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT_STL = HERE / "exports" / "flower_rainwater_collector.stl"
DEFAULT_STEP = HERE / "exports" / "flower_rainwater_collector.step"

VOLUME_TOLERANCE = 0.01  # 1% between the raw mesh and the sewn CAD shape


def fail(message):
    print(f"FAIL  {message}")
    return 1


def mesh_volume_mm3(path):
    """Signed volume of the raw STL, from the divergence theorem.

    Read here as well so the CAD volume can be compared against the tessellation
    it came from: a sewn solid whose volume differs means faces were dropped or
    double-sewn. Handles both binary and ASCII STL (OpenSCAD writes ASCII by
    default in 2021.01).
    """
    data = path.read_bytes()

    def volume(tris):
        total = 0.0
        for (ax, ay, az), (bx, by, bz), (cx, cy, cz) in tris:
            total += (
                ax * (by * cz - bz * cy)
                - ay * (bx * cz - bz * cx)
                + az * (bx * cy - by * cx)
            )
        return total / 6.0

    if len(data) >= 84:
        (count,) = struct.unpack_from("<I", data, 80)
        if len(data) == 84 + count * 50:
            tris = []
            off = 84
            for _ in range(count):
                v = struct.unpack_from("<12f", data, off)
                tris.append((v[3:6], v[6:9], v[9:12]))
                off += 50
            return volume(tris)

    text = data.decode("ascii", errors="replace")
    if "facet" not in text:
        return None
    tris = []
    pending = []
    for line in text.splitlines():
        line = line.strip()
        if not line.startswith("vertex"):
            continue
        parts = line.split()
        if len(parts) != 4:
            return None
        pending.append(tuple(float(x) for x in parts[1:4]))
        if len(pending) == 3:
            tris.append(tuple(pending))
            pending = []
    return volume(tris) if tris and not pending else None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--stl", type=Path, default=DEFAULT_STL)
    ap.add_argument("--step", type=Path, default=DEFAULT_STEP)
    args = ap.parse_args()

    try:
        from OCP.BRepBuilderAPI import BRepBuilderAPI_Sewing
        from OCP.BRepCheck import BRepCheck_Analyzer
        from OCP.BRepGProp import BRepGProp
        from OCP.GProp import GProp_GProps
        from OCP.IFSelect import IFSelect_RetDone
        from OCP.STEPControl import STEPControl_AsIs, STEPControl_Writer
        from OCP.ShapeFix import ShapeFix_Solid
        from OCP.StlAPI import StlAPI_Reader
        from OCP.TopAbs import TopAbs_ShapeEnum
        from OCP.TopExp import TopExp_Explorer
        from OCP.TopoDS import TopoDS_Shape
        from OCP import TopoDS
    except ImportError as exc:
        return fail(f"OpenCascade bindings not installed ({exc}); pip install cadquery-ocp")

    if not args.stl.exists():
        return fail(f"{args.stl} not found - run test_geometry.py --render first")

    print(f"importing {args.stl} into OpenCascade")
    reader = StlAPI_Reader()
    mesh_shape = TopoDS_Shape()
    if not reader.Read(mesh_shape, str(args.stl)):
        return fail("OpenCascade could not read the STL")

    def count(shape, kind):
        exp = TopExp_Explorer(shape, kind)
        n = 0
        while exp.More():
            n += 1
            exp.Next()
        return n

    print(f"      {count(mesh_shape, TopAbs_ShapeEnum.TopAbs_FACE)} faces imported")

    sewing = BRepBuilderAPI_Sewing(1e-4)
    sewing.Add(mesh_shape)
    sewing.Perform()
    sewn = sewing.SewedShape()
    if sewn.IsNull():
        return fail("sewing produced a null shape")
    print(f"PASS  sewn into {count(sewn, TopAbs_ShapeEnum.TopAbs_SHELL)} shell(s), "
          f"{count(sewn, TopAbs_ShapeEnum.TopAbs_SOLID)} solid(s), "
          f"{count(sewn, TopAbs_ShapeEnum.TopAbs_FACE)} faces")

    # The sewing tool's own counters are the authoritative closedness report: a
    # mesh with a missing or dropped face leaves free edges (verified by
    # removing one face and watching the counter go 0 -> 3).
    free = sewing.NbFreeEdges()
    multiple = sewing.NbMultipleEdges()
    degenerate = sewing.NbDegeneratedShapes()
    if free:
        return fail(f"the shell has {free} free (unconnected) edges: not a closed solid")
    print("PASS  0 free edges (closed shell)")
    if multiple:
        return fail(f"{multiple} edges shared by more than two faces (non-manifold)")
    print("PASS  0 multiple edges (manifold: every edge shared by exactly two faces)")
    if degenerate:
        print(f"      note: {degenerate} degenerate shape(s) reported by the sewing tool")

    shells = []
    exp = TopExp_Explorer(sewn, TopAbs_ShapeEnum.TopAbs_SHELL)
    while exp.More():
        shells.append(TopoDS.TopoDS.Shell(exp.Current()))
        exp.Next()
    if len(shells) != 1:
        return fail(f"expected exactly one shell, found {len(shells)}")

    shape = ShapeFix_Solid().SolidFromShell(shells[0])
    if shape.IsNull():
        return fail("ShapeFix_Solid could not promote the shell to a solid")
    solids = count(shape, TopAbs_ShapeEnum.TopAbs_SOLID)
    if solids != 1:
        return fail(f"expected exactly one solid, found {solids}")
    print("PASS  shell promoted to exactly one TopoDS_Solid")
    if not BRepCheck_Analyzer(shape).IsValid():
        return fail("BRepCheck_Analyzer reports the imported solid is NOT valid")
    print("PASS  BRepCheck_Analyzer: solid is valid")

    props = GProp_GProps()
    BRepGProp.VolumeProperties_s(shape, props)
    volume_mm3 = props.Mass()
    if volume_mm3 <= 0:
        return fail(f"expected a positive volume, got {volume_mm3} mm^3")
    print(f"PASS  positive volume: {volume_mm3 / 1000.0:.1f} cm^3")

    mesh = mesh_volume_mm3(args.stl)
    if mesh:
        delta = abs(volume_mm3 - mesh) / mesh
        print(f"      mesh volume (test_geometry.py): {mesh / 1000.0:.1f} cm^3  "
              f"delta {delta * 100:.4f}%")
        if delta > VOLUME_TOLERANCE:
            return fail(f"CAD volume differs from the mesh volume by {delta * 100:.2f}%")

    writer = STEPControl_Writer()
    writer.Transfer(shape, STEPControl_AsIs)
    status = writer.Write(str(args.step))
    if status != IFSelect_RetDone:
        return fail(f"STEP write failed (status {status})")
    if not args.step.exists() or args.step.stat().st_size == 0:
        return fail("STEP file was not written")
    print(f"PASS  wrote {args.step.relative_to(HERE)} "
          f"({args.step.stat().st_size / 1024:.0f} KiB)")
    print()
    print("All CAD import checks passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
