"""
Automated verification test suite for flower-shaped rainwater collector and adjustable tinaco adapter.
Validates parametric OpenSCAD parameters, physical engineering invariants, STEP/STL CAD file integrity,
dimensioned SVG assembly drawings, and Mexico City hydrologic catchment performance.
"""

from pathlib import Path
import math
import re
import struct
import sys
import unittest


ROOT_DIR = Path(__file__).parent
SCAD_FILE = ROOT_DIR / "flower_rainwater_collector.scad"
FLOWER_STEP = ROOT_DIR / "flower_rainwater_collector.step"
FLOWER_STL = ROOT_DIR / "flower_rainwater_collector.stl"
ADAPTER_STEP = ROOT_DIR / "tinaco_adapter.step"
ADAPTER_STL = ROOT_DIR / "tinaco_adapter.stl"
ASSEMBLY_SVG = ROOT_DIR / "assembly_drawing.svg"
README_FILE = ROOT_DIR / "README.md"

MEXICO_CITY_ANNUAL_RAINFALL_M = 0.800
CONSERVATIVE_RUNOFF_COEFFICIENT = 0.85
PARAM_PATTERN = re.compile(
    r"^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*([0-9]*\.?[0-9]+)\s*;",
    re.MULTILINE,
)


class FlowerRainwaterCollectorTests(unittest.TestCase):
    """
    Test suite for the flower rainwater collector CAD models, exports, and documentation.
    """

    def setUp(self) -> None:
        """
        Parse parametric variables from the OpenSCAD source file.
        """
        self.assertTrue(SCAD_FILE.exists(), f"Missing OpenSCAD source file: {SCAD_FILE}")
        text = SCAD_FILE.read_text(encoding="utf-8")
        self.params = {
            match.group(1): float(match.group(2))
            for match in PARAM_PATTERN.finditer(text)
        }

    def test_parametric_definitions_present(self) -> None:
        """
        Verify all mandatory design parameters exist in the OpenSCAD source.
        """
        required_keys = [
            "n_petals",
            "collector_dia",
            "petal_rise",
            "petal_gap",
            "petal_gap_flare",
            "hub_dia",
            "funnel_height",
            "outlet_dia",
            "adapter_collar_min_dia",
            "adapter_collar_max_dia",
            "adapter_height",
            "screen_thick",
            "screen_hole",
            "screen_pitch",
            "wall",
        ]
        for key in required_keys:
            self.assertIn(key, self.params, f"Missing required parameter: {key}")

    def test_geometric_invariants(self) -> None:
        """
        Validate physical and engineering invariants for gravity flow and mechanical fit.
        """
        petals = self.params["n_petals"]
        self.assertTrue(petals.is_integer(), "Petal count must be an integer")
        self.assertGreaterEqual(petals, 4.0, "Petal count must be at least 4")
        self.assertLessEqual(petals, 16.0, "Petal count must not exceed 16")

        collector_dia = self.params["collector_dia"]
        hub_dia = self.params["hub_dia"]
        outlet_dia = self.params["outlet_dia"]
        self.assertGreater(collector_dia, hub_dia, "Collector diameter must exceed hub diameter")
        self.assertGreater(hub_dia, outlet_dia, "Hub diameter must exceed outlet diameter for funnel narrowing")

        petal_rise = self.params["petal_rise"]
        self.assertGreater(petal_rise, 0.0, "Petal rise must be positive to ensure inward gravity drainage")

        petal_gap = self.params["petal_gap"]
        self.assertGreater(petal_gap, 0.0, "Petal gap must be positive for water runoff channels")
        self.assertLess(petal_gap, collector_dia / 8.0, "Petal gap must remain small relative to collector scale")

        wall = self.params["wall"]
        self.assertGreaterEqual(wall, 2.5, "Wall thickness must be at least 2.5mm for structural integrity")

        screen_hole = self.params["screen_hole"]
        screen_pitch = self.params["screen_pitch"]
        self.assertGreater(screen_hole, 0.0, "Screen hole must be positive")
        self.assertLess(screen_hole, screen_pitch, "Screen hole must be smaller than pitch to provide filtering")

        min_collar = self.params["adapter_collar_min_dia"]
        max_collar = self.params["adapter_collar_max_dia"]
        self.assertLessEqual(min_collar, 450.0, "Adapter min collar must accommodate 450mm tinaco rims")
        self.assertGreaterEqual(max_collar, 600.0, "Adapter max collar must accommodate 600mm tinaco rims")

    def test_hydrologic_catchment_capacity(self) -> None:
        """
        Validate horizontal rainwater catchment area and projected annual yield in Mexico City.
        """
        radius_m = (self.params["collector_dia"] / 2.0) / 1000.0
        catchment_area_m2 = math.pi * (radius_m**2)
        self.assertGreaterEqual(catchment_area_m2, 0.70, "Catchment area must exceed 0.70 m2")

        annual_yield_m3 = (
            catchment_area_m2
            * MEXICO_CITY_ANNUAL_RAINFALL_M
            * CONSERVATIVE_RUNOFF_COEFFICIENT
        )
        annual_yield_liters = annual_yield_m3 * 1000.0
        self.assertGreaterEqual(
            annual_yield_liters,
            700.0,
            "Annual harvest yield in Mexico City must exceed 700 Liters for standard 1200mm diameter",
        )

    def test_step_exports_conformance(self) -> None:
        """
        Verify that STEP CAD files exist and conform to ISO 10303-21 standard syntax.
        """
        for step_path in [FLOWER_STEP, ADAPTER_STEP]:
            self.assertTrue(step_path.exists(), f"STEP file missing: {step_path}")
            self.assertGreater(step_path.stat().st_size, 5000, f"STEP file unexpectedly small: {step_path}")
            header_sample = step_path.read_text(encoding="utf-8", errors="ignore")[:2000]
            self.assertIn("ISO-10303-21;", header_sample, f"Missing ISO-10303-21 header in {step_path.name}")
            self.assertIn("HEADER;", header_sample, f"Missing HEADER block in {step_path.name}")
            self.assertIn("DATA;", header_sample, f"Missing DATA block in {step_path.name}")
            tail_sample = step_path.read_text(encoding="utf-8", errors="ignore")[-2000:]
            self.assertIn("ENDSEC;", tail_sample, f"Missing ENDSEC marker in {step_path.name}")

    def test_stl_exports_conformance(self) -> None:
        """
        Verify that STL files exist and contain valid binary or ASCII triangular facets.
        """
        for stl_path in [FLOWER_STL, ADAPTER_STL]:
            self.assertTrue(stl_path.exists(), f"STL file missing: {stl_path}")
            self.assertGreater(stl_path.stat().st_size, 10000, f"STL file unexpectedly small: {stl_path}")
            raw_bytes = stl_path.read_bytes()
            triangle_count = struct.unpack("<I", raw_bytes[80:84])[0]
            self.assertGreater(triangle_count, 100, f"Insufficient facet density in STL: {stl_path.name}")
            expected_size = 84 + (triangle_count * 50)
            self.assertEqual(
                len(raw_bytes),
                expected_size,
                f"Binary STL file size mismatch in {stl_path.name}",
            )

    def test_assembly_drawing_svg(self) -> None:
        """
        Verify the dimensioned vector SVG assembly drawing exists and contains required annotations.
        """
        self.assertTrue(ASSEMBLY_SVG.exists(), f"Assembly drawing missing: {ASSEMBLY_SVG}")
        content = ASSEMBLY_SVG.read_text(encoding="utf-8")
        self.assertIn("<svg", content)
        self.assertIn("SECTION A-A", content)
        self.assertIn("1200", content)
        self.assertIn("180", content)
        self.assertIn("BOM CALLOUT LEGEND", content)
        self.assertIn("s6pa1rta3n-lab", content)

    def test_readme_documentation(self) -> None:
        """
        Ensure README.md covers all required engineering sections and criteria.
        """
        self.assertTrue(README_FILE.exists(), "README.md must exist")
        text = README_FILE.read_text(encoding="utf-8")
        required_sections = [
            "Bill of Materials",
            "Pre-Fabrication Measurement Checklist",
            "Tinaco Retrofit Compatibility",
            "Inward Gravity Drainage",
            "Adjustable Tinaco Adapter",
            "ISO 10303-21 STEP",
            "STL Mesh",
            "Mexico City",
        ]
        for section in required_sections:
            self.assertIn(section, text, f"README.md missing section or topic: {section}")


def run_standalone_diagnostics() -> int:
    """
    Execute tests and output clean diagnostic summary.

    Returns:
        Exit code 0 if all tests succeed, 1 otherwise.
    """
    suite = unittest.TestLoader().loadTestsFromTestCase(FlowerRainwaterCollectorTests)
    runner = unittest.TextTestRunner(verbosity=2)
    result = runner.run(suite)
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    sys.exit(run_standalone_diagnostics())
