import importlib.util
from pathlib import Path
import sys
import unittest


PATH = Path(__file__).with_name("verify_open_competition_v2_beta3_interfaces.py")
sys.path.insert(0, str(PATH.parent))
SPEC = importlib.util.spec_from_file_location("verify_open_competition_v2_beta3_interfaces", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


class InterfaceTests(unittest.TestCase):
    def test_discovery_search_is_structural_and_case_sensitive_to_protocol(self):
        self.assertTrue(MODULE.contains_beta3({"tools": ["inspect_open_competition_v2"]}))
        self.assertTrue(MODULE.contains_beta3("agent-bounties/open-competition-v2-beta3"))
        self.assertFalse(MODULE.contains_beta3({"tools": ["inspect_open_competition_v1"]}))

    def test_unwraps_standard_mcp_json_tool_envelope(self):
        release = {"release": {"protocol_version": "agent-bounties/open-competition-v2-beta3"}}
        response = {
            "content": [
                {
                    "type": "json",
                    "json": {"http_status": 200, "body": release},
                }
            ]
        }
        self.assertEqual(MODULE.unwrap_mcp_release(response), release)

    def test_rejects_unexpected_mcp_tool_envelope(self):
        with self.assertRaisesRegex(MODULE.InterfaceError, "invalid tool envelope"):
            MODULE.unwrap_mcp_release({"content": []})


if __name__ == "__main__":
    unittest.main()
