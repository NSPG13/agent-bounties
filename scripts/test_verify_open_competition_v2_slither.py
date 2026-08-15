import importlib.util
from pathlib import Path
import unittest


PATH = Path(__file__).with_name("verify_open_competition_v2_slither.py")
SPEC = importlib.util.spec_from_file_location("verify_open_competition_v2_slither", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


def finding(impact, check, signature):
    return {
        "impact": impact,
        "check": check,
        "elements": [{
            "type_specific_fields": {"signature": signature},
            "source_mapping": {"filename_relative": MODULE.SOURCE},
        }],
    }


def report():
    return {
        "success": True,
        "results": {"detectors": [finding(*item) for item in MODULE.TRIAGED]},
    }


class SlitherTriageTests(unittest.TestCase):
    def test_exact_triage_passes(self):
        self.assertTrue(MODULE.verify(report())["passed"])

    def test_new_high_finding_fails(self):
        value = report()
        value["results"]["detectors"].append(finding("High", "suicidal", "destroy()"))
        with self.assertRaises(ValueError):
            MODULE.verify(value)

    def test_removed_fingerprint_requires_retriage(self):
        value = report()
        value["results"]["detectors"].pop()
        with self.assertRaises(ValueError):
            MODULE.verify(value)


if __name__ == "__main__":
    unittest.main()
