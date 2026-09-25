#!/usr/bin/env python3
"""
Validation and export helper for tinaco collector CAD geometries.
"""

import os
import subprocess
import sys


def export_stl(output_path="cad/tinaco_collector_export.stl", scad_path="cad/flower_tinaco_collector.scad"):
    """Export OpenSCAD design to STL mesh."""
    cmd = f"openscad --hardwarnings -o {output_path} {scad_path}"
    result = subprocess.run(cmd, shell=True, capture_output=True, text=True)
    if result.returncode != 0:
        print(f"Export failed: {result.stderr}", file=sys.stderr)
        return False
    return os.path.exists(output_path) and os.path.getsize(output_path) > 0


if __name__ == "__main__":
    if len(sys.argv) > 1:
        target = sys.argv[1]
    else:
        target = "cad/tinaco_collector_export.stl"
    success = export_stl(output_path=target)
    sys.exit(0 if success else 1)