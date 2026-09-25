#!/usr/bin/env python3
\n"""
\nExport OpenSCAD design to DXF for collision testing.
"""
\n
\nimport ezdxf
\nimport subprocess
\nimport sys

\ndef export_dxf():
\n    """Export SCAD design to DXF using OpenSCAD."""
\n    cmd = """
\n    openscad -Dexport_dxf=true -o tinaco_collector.dxf \
\n    cad/flower_tinaco_collector.scad
\n    """
\n    
    result = subprocess.run(cmd, shell=True, capture_output=True, text=True)
\n    if result.returncode != 0:
\n        print(f"DXF export failed: {result.stderr}", file=sys.stderr)
\n        sys.exit(1)

\n    return "tinaco_collector.dxf"

\ndef check_collision():
\n    """Check for collisions with tinaco bounding box."""
\n    doc = ezdxf.readfile(export_dxf())
\n    modelspace = doc.modelspace()

\n    # Tinaco bounding box (1.2m diameter)
\n    tinaco_radius = 0.6

\n    # Check all entities
\n    for entity in modelspace:
\n        if hasattr(entity, 'dxfattribs'):
\n            bbox = entity.boundingbox
\n            if bbox:
\n                distance = (bbox[0][0] - bbox[1][0]) / 2  # Simplified check
\n                if distance > tinaco_radius * 0.95:  # 95% of tinaco radius
\n                    print(f"Collision detected with entity: {entity.dxftype()}")
\n                    return False

\n    return True

\nif __name__ == "__main__":
\n    if "--check_collision" in sys.argv:
\n        success = check_collision()
\n        sys.exit(0 if success else 1)
\n    else:
\n        export_dxf()