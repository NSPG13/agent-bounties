#!/usr/bin/env python3
"""[DIRECT #685] MCP/API tool-registry drift coverage (offline, exits 0)."""
import json, os, sys, unittest
from urllib.parse import urlparse

HERE = os.path.dirname(os.path.abspath(__file__))
REQUIRED_TOOLS = ("list_autonomous_bounties", "list_opportunities",
                  "prepare_agent_to_earn", "prepare_bounty_post")


def load(name):
    with open(os.path.join(HERE, name), encoding="utf-8") as fh:
        return json.load(fh)


def drift_report(registry, manifest):
    """Compare MCP registry against API discovery manifest; fail closed."""
    reg, man = list(registry["tools"]), list(manifest["tools"])
    missing = [t for t in REQUIRED_TOOLS if t not in reg or t not in man]
    dupes = sorted({t for t in reg if reg.count(t) > 1})
    renamed = [t for t in REQUIRED_TOOLS
               if t not in reg and any(t.split("_")[0] in r for r in reg)]
    return {"missing": missing, "duplicates": dupes, "renamed": renamed,
            "ok": not (missing or dupes or renamed)}


class ToolRegistryDriftTests(unittest.TestCase):
    def setUp(self):
        self.reg = load("mcp-tool-registry.json")
        self.man = load("api-discovery-manifest.json")

    def test_committed_state_has_no_drift(self):
        rep = drift_report(self.reg, self.man)
        self.assertTrue(rep["ok"], json.dumps(rep))
        self.assertEqual(rep["missing"], [])
        self.assertEqual(rep["duplicates"], [])
        self.assertEqual(rep["renamed"], [])

    def test_all_required_tools_present_in_both_surfaces(self):
        for tool in REQUIRED_TOOLS:
            self.assertIn(tool, self.reg["tools"])
            self.assertIn(tool, self.man["tools"])

    def test_missing_tool_is_detected(self):
        broken = dict(self.reg); broken["tools"] = [t for t in self.reg["tools"]
                                                    if t != "prepare_bounty_post"]
        rep = drift_report(broken, self.man)
        self.assertFalse(rep["ok"])
        self.assertIn("prepare_bounty_post", rep["missing"])

    def test_duplicate_tool_is_detected(self):
        broken = dict(self.reg); broken["tools"] = self.reg["tools"] + ["list_opportunities"]
        self.assertFalse(drift_report(broken, self.man)["ok"])

    def test_renamed_tool_is_detected(self):
        broken = dict(self.reg)
        broken["tools"] = [("list_opportunity" if t == "list_opportunities" else t)
                           for t in self.reg["tools"]]
        rep = drift_report(broken, self.man)
        self.assertFalse(rep["ok"])
        self.assertIn("list_opportunities", rep["renamed"])

    def test_transport_and_inventory_endpoints_are_distinct(self):
        self.assertNotEqual(self.reg["transport"], self.man["tool_inventory_endpoint"])
        self.assertEqual(urlparse(self.reg["transport"]).netloc, "mcp.agentbounties.app")

    def test_report_is_concise_and_deterministic(self):
        rep = drift_report(self.reg, self.man)
        self.assertEqual(rep, drift_report(self.reg, self.man))
        self.assertEqual(set(rep), {"missing", "duplicates", "renamed", "ok"})


if __name__ == "__main__":
    rc = unittest.main(verbosity=2, exit=False).result
    sys.exit(0 if rc.wasSuccessful() else 1)
