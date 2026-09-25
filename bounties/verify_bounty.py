#!/usr/bin/env python3
"""
Bounty verification script for flower-shaped tinaco rainwater collector.
Validates parametric OpenSCAD compilation and geometry checks.
"""

import os
import subprocess
import sys


def run_command(cmd):
    """Execute shell command and return stdout/stderr."""
    result = subprocess.run(cmd, shell=True, capture_output=True, text=True)
    if result.returncode != 0:
        raise RuntimeError(f"Command failed: {cmd}\nOutput: {result.stderr or result.stdout}")
    return result.stdout


def verify_parametric_compilation():
    """Verify that OpenSCAD compiles default and boundary parametric combinations."""
    scad_file = os.path.join("cad", "flower_tinaco_collector.scad")
    if not os.path.exists(scad_file):
        print(f"Error: {scad_file} not found", file=sys.stderr)
        return False

    test_cases = [
        {"petals": 3, "diameter": 300, "adapter": 1},
        {"petals": 6, "diameter": 500, "adapter": 1},
        {"petals": 12, "diameter": 600, "adapter": 2},
    ]

    for tc in test_cases:
        out_file = os.path.join("cad", f"test_collector_{tc['petals']}p_{tc['diameter']}mm.stl")
        cmd = f"openscad --hardwarnings -Dpetals={tc['petals']} -Ddiameter={tc['diameter']} -Dadapter_type={tc['adapter']} -o {out_file} {scad_file}"
        print(f"Testing compilation: {tc['petals']} petals, {tc['diameter']}mm, adapter {tc['adapter']}...")
        try:
            run_command(cmd)
            if os.path.exists(out_file) and os.path.getsize(out_file) > 1000:
                print(f"  -> Generated {out_file} ({os.path.getsize(out_file)} bytes)")
                os.remove(out_file)
            else:
                print(f"  -> Failed to generate valid mesh: {out_file}", file=sys.stderr)
                return False
        except RuntimeError as e:
            print(f"  -> Compilation failed: {e}", file=sys.stderr)
            return False

    return True


if __name__ == "__main__":
    print("Running tinaco collector CAD verification...")
    if verify_parametric_compilation():
        print("All parametric CAD tests PASSED successfully.")
        sys.exit(0)
    else:
        print("CAD verification FAILED.", file=sys.stderr)
        sys.exit(1)