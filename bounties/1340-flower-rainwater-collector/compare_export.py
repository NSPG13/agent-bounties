#!/usr/bin/env python3
"""Replayable source -> export equivalence check.

`test_geometry.py --render` writes the STL that gets committed, and OpenSCAD
re-renders of one source produce the same triangle SET in a different ORDER
(three renders of this part gave three sha256s with an identical facet
multiset). An STL hash is therefore NOT a source-equivalence token, and the
"the facet multiset matches" claim in `exports/EVIDENCE.md` needs a check that
somebody else can re-run:

    python3 compare_export.py

  * renders the .scad to a TEMPORARY file - never over the committed export,
  * records the committed export's sha256 before and after the run and fails if
    it changed (the reviewed bytes stay the reviewed bytes),
  * compares the two meshes as sorted facet MULTISETS, so triangle order and
    float formatting cannot decide the result,
  * compares welded-vertex count, edge counts, winding consistency, connected
    bodies, signed volume and bounding box,
  * exits nonzero on any difference.

RED control (runs in seconds, needs no OpenSCAD):

    python3 compare_export.py --selftest

writes temporary copies of the committed export, mutates one copy two ways
(nudge one vertex, drop one facet) and asserts the comparator passes the
identical pair and FAILS both mutated pairs - proof the comparison can fail.

Usage:
    python3 compare_export.py [--export FILE] [--scad FILE] [--keep-temp]
"""

import argparse
import collections
import hashlib
import shutil
import sys
import tempfile
from pathlib import Path

import test_geometry as tg

HERE = Path(__file__).resolve().parent
DEFAULT_EXPORT = HERE / "exports" / "flower_rainwater_collector.stl"

# Facet coordinates are rounded before comparison: OpenSCAD writes ASCII floats
# whose last digits vary with the run's internal ordering.
ROUND = 4


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def facet_multiset(path):
    """Map every facet to its multiplicity, independent of order and winding."""
    raw_vertices, raw_triangles = tg.read_stl(Path(path))
    counts = collections.Counter()
    for a, b, c in raw_triangles:
        corners = tuple(sorted(
            tuple(round(x, ROUND) for x in raw_vertices[i]) for i in (a, b, c)
        ))
        counts[corners] += 1
    return counts


def mesh_summary(path):
    raw_vertices, raw_triangles = tg.read_stl(Path(path))
    remap, vertices = tg.weld(raw_vertices)
    triangles = [(remap[a], remap[b], remap[c]) for a, b, c in raw_triangles]
    boundary, non_manifold, bad_winding, n_edges = tg.edge_manifold_report(triangles)
    n_bodies, sizes = tg.connected_components(len(vertices), triangles)
    volume = tg.signed_volume(vertices, triangles)
    lo, hi = tg.bounding_box(vertices)
    return {
        "triangles": len(triangles),
        "welded_vertices": len(vertices),
        "edges": n_edges,
        "boundary_edges": boundary,
        "non_manifold_edges": non_manifold,
        "bad_winding": bad_winding,
        "bodies": n_bodies,
        "body_sizes": sizes,
        "volume_mm3": volume,
        "bbox_lo": lo,
        "bbox_hi": hi,
    }


def compare(path_a, path_b, label_a="A", label_b="B", verbose=True):
    """Compare two meshes. Returns (ok, lines); never raises on a mismatch."""
    lines = []
    ok = True

    set_a = facet_multiset(path_a)
    set_b = facet_multiset(path_b)
    n_a, n_b = sum(set_a.values()), sum(set_b.values())
    only_a = set_a - set_b
    only_b = set_b - set_a
    n_only_a = sum(only_a.values())
    n_only_b = sum(only_b.values())
    facets_match = (n_a == n_b) and not only_a and not only_b
    ok &= facets_match
    lines.append(f"facets: {n_a} ({label_a}) vs {n_b} ({label_b}) - "
                 f"multisets identical: {'yes' if facets_match else 'NO'}")
    if not facets_match:
        lines.append(f"  facets only in {label_a}: {n_only_a}; "
                     f"only in {label_b}: {n_only_b}")
        for corners in list(only_a)[:3]:
            lines.append(f"  example only in {label_a}: {corners}")
        for corners in list(only_b)[:3]:
            lines.append(f"  example only in {label_b}: {corners}")

    a = mesh_summary(path_a)
    b = mesh_summary(path_b)
    exact = ("triangles", "welded_vertices", "edges", "boundary_edges",
             "non_manifold_edges", "bad_winding", "bodies", "body_sizes")
    for key in exact:
        same = a[key] == b[key]
        ok &= same
        lines.append(f"{key}: {a[key]} vs {b[key]}{'' if same else '  <-- DIFFERS'}")
    volume_delta = abs(a["volume_mm3"] - b["volume_mm3"])
    volume_same = volume_delta <= max(1e-6, abs(a["volume_mm3"]) * 1e-9)
    ok &= volume_same
    lines.append(f"volume: {a['volume_mm3']:.4f} vs {b['volume_mm3']:.4f} mm^3 "
                 f"(delta {volume_delta:.6f}){'' if volume_same else '  <-- DIFFERS'}")
    for lo_key, hi_key in (("bbox_lo", "bbox_hi"),):
        deltas = max(
            max(abs(x - y) for x, y in zip(a[lo_key], b[lo_key])),
            max(abs(x - y) for x, y in zip(a[hi_key], b[hi_key])),
        )
        bbox_same = deltas <= 1e-3
        ok &= bbox_same
        lines.append(f"bounding box: max coordinate delta {deltas:.6f} mm"
                     f"{'' if bbox_same else '  <-- DIFFERS'}")
    if verbose:
        for line in lines:
            print(line)
    return bool(ok), lines


def mutate_vertex(text, index=100, delta=0.5):
    """Nudge one coordinate of the index-th vertex line: a real geometry change."""
    out = []
    seen = 0
    for line in text.splitlines(keepends=True):
        stripped = line.strip()
        if stripped.startswith("vertex"):
            if seen == index:
                parts = stripped.split()
                x = float(parts[1]) + delta
                line = line.replace(parts[1], f"{x:.6f}", 1)
            seen += 1
        out.append(line)
    assert seen > index, "mutate_vertex found no vertex to change"
    return "".join(out)


def drop_facet(text):
    """Remove one whole `facet ... endfacet` block: a missing triangle."""
    lines = text.splitlines(keepends=True)
    start = next(i for i, line in enumerate(lines) if line.strip().startswith("facet"))
    end = next(i for i, line in enumerate(lines) if i > start and line.strip().startswith("endfacet"))
    assert end > start, "drop_facet found no facet block"
    return "".join(lines[:start] + lines[end + 1:])


def selftest(export):
    """RED/GREEN controls for the comparator itself."""
    text = Path(export).read_text()
    tmp = Path(tempfile.mkdtemp(prefix="compare_export_selftest_"))
    try:
        same = tmp / "same.stl"
        nudged = tmp / "nudged.stl"
        dropped = tmp / "dropped.stl"
        same.write_text(text)
        nudged.write_text(mutate_vertex(text))
        dropped.write_text(drop_facet(text))
        print("selftest: the comparator must PASS the identical copy and FAIL "
              "both mutated ones")
        print()
        controls = [
            ("identical copy", same, True),
            ("one vertex nudged by 0.5 mm", nudged, False),
            ("one facet removed", dropped, False),
        ]
        all_ok = True
        for name, path, expect_pass in controls:
            print(f"--- control: {name} (expect {'PASS' if expect_pass else 'FAIL'})")
            ok, _ = compare(export, path, label_a="committed", label_b="control")
            behaved = ok is expect_pass
            all_ok &= behaved
            print(f"    comparator {'PASSED' if ok else 'FAILED'} -> "
                  f"{'as expected' if behaved else 'UNEXPECTED'}")
            print()
        print("selftest: " + ("all three controls behaved as expected"
                              if all_ok else "A CONTROL DID NOT BEHAVE AS EXPECTED"))
        return 0 if all_ok else 1
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--export", type=Path, default=DEFAULT_EXPORT,
                    help="preserved export to compare against (never written to)")
    ap.add_argument("--scad", type=Path, default=tg.SCAD,
                    help="source to render (default: flower_rainwater_collector.scad)")
    ap.add_argument("--keep-temp", action="store_true",
                    help="keep the temporary render for inspection")
    ap.add_argument("--selftest", action="store_true",
                    help="run the RED/GREEN controls on the comparator itself")
    args = ap.parse_args()

    if not args.export.exists():
        raise SystemExit(f"FAIL  preserved export not found: {args.export}")
    if args.selftest:
        return selftest(args.export)

    before = sha256(args.export)
    print(f"preserved export: {args.export} (sha256 {before})")
    tmp = Path(tempfile.mkdtemp(prefix="compare_export_"))
    render_path = tmp / "render.stl"
    try:
        tg.SCAD = args.scad
        tg.render_stl(render_path)
        after = sha256(args.export)
        print(f"fresh render:     {render_path} (sha256 {sha256(render_path)})")
        print(f"preserved export unchanged by the render: "
              f"{'yes' if before == after else 'NO'}"
              f"{'' if before == after else f' (now {after})'}")
        print()
        ok, _ = compare(args.export, render_path,
                        label_a="committed", label_b="fresh render")
        if ok and before != after:
            ok = False
        print()
        print("PASS  source -> export equivalence (facet multiset, topology, volume)"
              if ok else
              "FAIL  the fresh render does NOT match the committed export")
        return 0 if ok else 1
    finally:
        if args.keep_temp:
            print(f"temporary render kept at {render_path}")
        else:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
