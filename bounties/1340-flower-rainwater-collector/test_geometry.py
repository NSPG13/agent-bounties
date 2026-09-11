#!/usr/bin/env python3
"""Geometry-level checks for the flower-shaped rainwater collector.

`test_model.py` checks the parameter table. It cannot see the mesh, and the
mesh is where a CAD assembly actually fails: a part can satisfy every numeric
invariant and still be assembled from pieces that never touch. This script
therefore reads the exported mesh - the artifact a CAD kernel, a slicer or a
reviewer would import - and asserts the properties that matter geometrically:

  1. valid closed solid  - triangle-consistent, every edge shared by exactly
                           two triangles, no boundary or non-manifold edges
  2. one connected body  - dish, funnel, port sleeve and screen overlap in
                           real volume, i.e. the assembly is a single part
  3. dimensions          - signed volume is positive and the bounding box
                           matches the parametric envelope
  4. outlet continuity   - vertical probes from above the hub mouth to below
                           the threaded outlet cross ZERO surfaces (the water
                           path is open end to end), with control probes
                           through a petal and through the funnel wall that
                           cross exactly two (proof the probe can tell solid
                           from void, and that the shell is not perforated)

Usage:
    python3 test_geometry.py                 # check the committed export
    python3 test_geometry.py --render        # re-render the .scad first
    python3 test_geometry.py --stl FILE      # check a specific mesh

Requires only the Python standard library. `--render` additionally needs an
OpenSCAD binary (set OPENSCAD_BIN or put `openscad` on PATH).
"""

import argparse
import hashlib
import math
import re
import shutil
import struct
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCAD = HERE / "flower_rainwater_collector.scad"
DEFAULT_STL = HERE / "exports" / "flower_rainwater_collector.stl"

# Welding precision: vertices closer than this are the same point. OpenSCAD
# writes exact duplicates for shared CSG edges, so 1 micrometre is generous.
WELD_DECIMALS = 3

failures = []


def check(name, condition, detail):
    if condition:
        print(f"PASS  {name}")
    else:
        failures.append(name)
        print(f"FAIL  {name}: {detail}")
    return bool(condition)


# ---------------------------------------------------------------- STL reading

def read_stl(path):
    """Return (vertices, triangles) from a binary or ASCII STL.

    OpenSCAD writes ASCII STL by default, but binary files are common from
    other tools, so both are accepted and auto-detected.
    """
    data = path.read_bytes()
    if len(data) >= 84:
        (count,) = struct.unpack_from("<I", data, 80)
        if len(data) == 84 + count * 50:
            vertices = []
            triangles = []
            off = 84
            for _ in range(count):
                values = struct.unpack_from("<12f", data, off)
                v = [tuple(float(x) for x in values[3:6]),
                     tuple(float(x) for x in values[6:9]),
                     tuple(float(x) for x in values[9:12])]
                base = len(vertices)
                vertices.extend(v)
                triangles.append((base, base + 1, base + 2))
                off += 50
            return vertices, triangles

    text = data.decode("ascii", errors="replace")
    if "facet" not in text:
        raise ValueError(
            f"{path}: not a readable STL ({len(data)} bytes; neither a valid binary "
            "STL nor an ASCII STL with facet records)"
        )
    vertices = []
    triangles = []
    pending = []
    for line in text.splitlines():
        line = line.strip()
        if not line.startswith("vertex"):
            continue
        parts = line.split()
        if len(parts) != 4:
            raise ValueError(f"{path}: malformed vertex line {line!r}")
        pending.append(tuple(float(x) for x in parts[1:4]))
        if len(pending) == 3:
            base = len(vertices)
            vertices.extend(pending)
            triangles.append((base, base + 1, base + 2))
            pending = []
    if pending:
        raise ValueError(f"{path}: {len(pending)} dangling vertices (incomplete facet)")
    return vertices, triangles


def weld(vertices):
    """Snap vertices to a grid and return (remap, canonical_vertices).

    `remap[i]` is the canonical index of raw vertex `i`; `canonical_vertices`
    holds the snapped coordinates. Two vertices closer than the tolerance
    collapse to one, which is what makes shared CSG edges connect.
    """
    index = {}
    canonical = []
    remap = []
    for v in vertices:
        key = tuple(round(c, WELD_DECIMALS) for c in v)
        i = index.get(key)
        if i is None:
            i = len(canonical)
            index[key] = i
            canonical.append(key)
        remap.append(i)
    return remap, canonical


# ---------------------------------------------------------------- topology

def edge_manifold_report(triangles):
    """Count undirected edge uses and directed-edge consistency."""
    undirected = {}
    directed = {}
    for a, b, c in triangles:
        for u, v in ((a, b), (b, c), (c, a)):
            key = (u, v) if u < v else (v, u)
            undirected[key] = undirected.get(key, 0) + 1
            directed[(u, v)] = directed.get((u, v), 0) + 1
    boundary = sum(1 for n in undirected.values() if n == 1)
    non_manifold = sum(1 for n in undirected.values() if n > 2)
    bad_winding = sum(1 for n in directed.values() if n != 1)
    return boundary, non_manifold, bad_winding, len(undirected)


def connected_components(n_vertices, triangles):
    """Union-find over welded vertices joined by triangle edges."""
    parent = list(range(n_vertices))

    def find(x):
        while parent[x] != x:
            parent[x] = parent[parent[x]]
            x = parent[x]
        return x

    def union(x, y):
        rx, ry = find(x), find(y)
        if rx != ry:
            parent[ry] = rx

    for a, b, c in triangles:
        union(a, b)
        union(b, c)

    roots = {}
    for a, b, c in triangles:
        r = find(a)
        roots[r] = roots.get(r, 0) + 1
    sizes = sorted(roots.values(), reverse=True)
    return len(roots), sizes


def signed_volume(vertices, triangles):
    total = 0.0
    for a, b, c in triangles:
        ax, ay, az = vertices[a]
        bx, by, bz = vertices[b]
        cx, cy, cz = vertices[c]
        total += (
            ax * (by * cz - bz * cy)
            - ay * (bx * cz - bz * cx)
            + az * (bx * cy - by * cx)
        )
    return total / 6.0


def bounding_box(vertices):
    xs = [v[0] for v in vertices]
    ys = [v[1] for v in vertices]
    zs = [v[2] for v in vertices]
    return (min(xs), min(ys), min(zs)), (max(xs), max(ys), max(zs))


# ---------------------------------------------------------------- ray casting

def ray_crossings(vertices, triangles, origin, direction):
    """Count DISTINCT surface crossings of the ray (Moller-Trumbore, t > 0).

    Hits are deduplicated by distance: a ray that passes exactly through an
    edge or vertex is reported by every triangle that shares it, and counting
    those twice would make a two-wall shell look like a four-wall one.
    """
    ox, oy, oz = origin
    dx, dy, dz = direction
    hits = []
    for a, b, c in triangles:
        v0 = vertices[a]
        v1 = vertices[b]
        v2 = vertices[c]
        e1 = (v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2])
        e2 = (v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2])
        p = (dy * e2[2] - dz * e2[1], dz * e2[0] - dx * e2[2], dx * e2[1] - dy * e2[0])
        det = e1[0] * p[0] + e1[1] * p[1] + e1[2] * p[2]
        if abs(det) < 1e-12:
            continue
        inv = 1.0 / det
        tv = (ox - v0[0], oy - v0[1], oz - v0[2])
        u = (tv[0] * p[0] + tv[1] * p[1] + tv[2] * p[2]) * inv
        if u < 0.0 or u > 1.0:
            continue
        q = (tv[1] * e1[2] - tv[2] * e1[1], tv[2] * e1[0] - tv[0] * e1[2], tv[0] * e1[1] - tv[1] * e1[0])
        v = (dx * q[0] + dy * q[1] + dz * q[2]) * inv
        if v < 0.0 or u + v > 1.0:
            continue
        hit_t = (e2[0] * q[0] + e2[1] * q[1] + e2[2] * q[2]) * inv
        if hit_t > 1e-6:
            hits.append(hit_t)
    hits.sort()
    distinct = 0
    last = None
    for t in hits:
        if last is None or abs(t - last) > 1e-3:
            distinct += 1
        last = t
    return distinct


# ---------------------------------------------------------------- parameters

PARAM = re.compile(r"^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*([0-9]*\.?[0-9]+)\s*;", re.MULTILINE)


def read_params():
    text = SCAD.read_text()
    return {m.group(1): float(m.group(2)) for m in PARAM.finditer(text)}


def render_stl(out_path):
    binary = None
    import os
    if os.environ.get("OPENSCAD_BIN"):
        binary = os.environ["OPENSCAD_BIN"]
    else:
        binary = shutil.which("openscad")
        for candidate in (
            "/usr/bin/openscad",
            "/usr/local/bin/openscad",
            str(Path.home() / "Applications/OpenSCAD.AppImage"),
        ):
            if binary is None and Path(candidate).exists():
                binary = candidate
    if not binary:
        print("SKIP  --render: no OpenSCAD binary found (set OPENSCAD_BIN or install openscad)")
        return None
    print(f"rendering {SCAD.name} with {binary} -> {out_path} (this takes a few minutes)")
    out_path.parent.mkdir(parents=True, exist_ok=True)
    result = subprocess.run(
        [binary, "--hardwarnings", "-o", str(out_path), str(SCAD)],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0 or not out_path.exists():
        print(result.stdout[-2000:])
        print(result.stderr[-2000:], file=sys.stderr)
        raise SystemExit(f"OpenSCAD render failed (exit {result.returncode})")
    return out_path


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--render", action="store_true", help="re-render the .scad before checking")
    ap.add_argument("--stl", type=Path, default=DEFAULT_STL)
    args = ap.parse_args()

    stl = args.stl
    if args.render:
        rendered = render_stl(DEFAULT_STL)
        if rendered:
            stl = rendered

    if not stl.exists():
        raise SystemExit(
            f"{stl} not found. Generate it with:\n"
            f"  openscad -o {DEFAULT_STL} {SCAD}\n"
            "or re-run this script with --render."
        )

    raw = read_stl(stl)
    raw_vertices, raw_triangles = raw
    # Topology runs on welded vertices: OpenSCAD writes duplicate vertices for
    # shared CSG edges, and welding absorbs float32/ASCII rounding as well.
    remap, vertices = weld(raw_vertices)
    triangles = [(remap[a], remap[b], remap[c]) for a, b, c in raw_triangles]
    n_vertices = len(vertices)
    digest = hashlib.sha256(stl.read_bytes()).hexdigest()[:16]
    print(f"mesh: {stl}")
    print(f"      {len(triangles)} triangles, {n_vertices} welded vertices "
          f"(from {len(raw_vertices)} raw), sha256:{digest}...")
    print()

    p = read_params()
    hub_radius = p["hub_dia"] / 2
    port_radius = p["port_thread_dia"] / 2

    # 1. closed, consistently wound solid
    boundary, non_manifold, bad_winding, n_edges = edge_manifold_report(triangles)
    check("no boundary edges (closed shell)", boundary == 0, f"{boundary} boundary edges")
    check("no non-manifold edges (each edge shared by 2 triangles)",
          non_manifold == 0, f"{non_manifold} edges with >2 faces")
    check("winding consistent (every directed edge used once)",
          bad_winding == 0, f"{bad_winding} directed edges repeated")

    # 2. one solid body
    n_components, sizes = connected_components(n_vertices, triangles)
    check("assembly is ONE connected body (dish + funnel + sleeve + screen fused)",
          n_components == 1, f"{n_components} disjoint bodies, triangle counts {sizes}")

    # 3. volume and envelope
    volume_mm3 = signed_volume(vertices, triangles)
    check("positive signed volume (outward-facing solid)", volume_mm3 > 0,
          f"{volume_mm3:.0f} mm^3")
    lo, hi = bounding_box(vertices)
    height = hi[2] - lo[2]
    expected_height = p["petal_rise"] + p["wall"] + p["funnel_height"] + p["port_length"]
    expected_dia = p["collector_dia"]
    measured_dia = max(hi[0] - lo[0], hi[1] - lo[1])
    check("height matches the parametric stack (rim -> outlet)",
          abs(height - expected_height) <= 1.0,
          f"measured {height:.1f} mm, expected {expected_height:.1f} mm")
    check("collector diameter matches collector_dia",
          abs(measured_dia - expected_dia) <= 1.5,
          f"measured {measured_dia:.1f} mm, expected {expected_dia:.1f} mm")
    check("outlet reaches port_length below the funnel outlet",
          abs(lo[2] + (p["funnel_height"] + p["port_length"])) <= 0.5,
          f"lowest z {lo[2]:.2f} mm, expected {-(p['funnel_height'] + p['port_length']):.2f} mm")
    print(f"      solid volume: {volume_mm3 / 1000.0:.1f} cm^3 "
          f"({volume_mm3 / 1e6:.3f} L of material)")

    # 4. outlet continuity: open water path from the mouth to the threaded outlet
    top = hi[2] + 50.0
    direction = (0.0, 0.0, -1.0)
    pitch = p["screen_pitch"]
    # A clear vertical path must stay inside the NARROWEST passage, the port
    # bore, or the probe would legitimately hit the funnel wall. Probes sit on
    # screen-hole centres (multiples of screen_pitch), so they pass the screen.
    probes = []
    for i in range(-6, 7):
        for j in range(-6, 7):
            x, y = i * pitch, j * pitch
            if math.hypot(x, y) <= port_radius * 0.9:
                probes.append((x, y))
    blocked = []
    for x, y in probes:
        hits = ray_crossings(vertices, triangles, (x, y, top), direction)
        if hits != 0:
            blocked.append((x, y, hits))
    check(f"outlet continuity: {len(probes)} vertical probes through screen holes "
          "reach the outlet with zero surface crossings",
          not blocked, f"blocked probes (x, y, crossings): {blocked[:6]}")

    control_radius = p["collector_dia"] / 4.0
    angle = math.radians(360.0 / p["n_petals"] / 2.0)  # petal centre, not a channel
    cx, cy = control_radius * math.cos(angle), control_radius * math.sin(angle)
    hits = ray_crossings(vertices, triangles, (cx, cy, top), direction)
    check("control probe through a petal crosses exactly two surfaces "
          "(probe can tell solid from void; petal shell is sound)",
          hits == 2, f"{hits} crossings at (x={cx:.1f}, y={cy:.1f})")

    wall_hits = ray_crossings(
        vertices, triangles,
        ((hub_radius + p["wall"] / 2) * math.cos(angle),
         (hub_radius + p["wall"] / 2) * math.sin(angle), top),
        direction)
    check("control probe through the fused hub wall crosses exactly two surfaces",
          wall_hits == 2, f"{wall_hits} crossings at r = hub + wall/2")

    print()
    if failures:
        print(f"{len(failures)} geometry check(s) failed.")
        return 1
    print("All geometry checks passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
