import importlib.util
from pathlib import Path
import unittest


PATH = Path(__file__).with_name("verify_open_competition_v2_beta3_interfaces.py")
SPEC = importlib.util.spec_from_file_location("verify_open_competition_v2_beta3_interfaces", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


class InterfaceTests(unittest.TestCase):
    def test_discovery_search_is_structural_and_case_sensitive_to_protocol(self):
        self.assertTrue(MODULE.contains_beta3({"tools": ["inspect_open_competition_v2"]}))
        self.assertTrue(MODULE.contains_beta3("agent-bounties/open-competition-v2-beta3"))
        self.assertFalse(MODULE.contains_beta3({"tools": ["inspect_open_competition_v1"]}))


if __name__ == "__main__":
    unittest.main()
