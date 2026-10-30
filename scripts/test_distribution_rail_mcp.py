"""Offline controls for the real distribution probe and stable MCP wire key."""

import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "distribution_rail_probe", ROOT / "scripts/check-distribution-rail-mcp.py"
)
probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(probe)

# Published wire identity: the website's hostname is not a protocol migration.
KEY = "agentbounties.app/acquisition"
TOKEN = "aba1_" + "a" * 64 + "." + "b" * 64


def responses(rail="github", kind="dry-run-v1"):
    headers = {
        "x-agent-bounties-acquisition-id": TOKEN,
        "x-agent-bounties-attribution-rail": rail,
        "x-agent-bounties-first-touch-rail": rail,
        "x-agent-bounties-canary": kind,
        "x-agent-bounties-measurement-eligible": "false",
    }
    initialize = (200, dict(headers), {"result": {"serverInfo": {"name": "Agentizable"}}})
    tools = (200, dict(headers), {"result": {
        "tools": [{"name": "prepare_bounty_post"}],
        "_meta": {KEY: {
            "acquisition_id": TOKEN,
            "first_touch_rail": rail,
            "current_rail": rail,
            "measurement_eligible": False,
            "request_header": "x-agent-bounties-acquisition-id",
            "authority": "analytics_only",
        }},
    }})
    return [initialize, tools]


class DistributionRailProbeTests(unittest.TestCase):
    def invoke(self, replies, rail="github", kind="dry-run-v1"):
        with patch.object(probe, "post_json", side_effect=replies) as post:
            probe.check_rail("https://mcp.agentizable.ai", rail, kind)
            self.assertEqual(post.call_count, 2)
            self.assertEqual(
                [call.args[1]["method"] for call in post.call_args_list],
                ["initialize", "tools/list"],
            )
            self.assertEqual(post.call_args_list[1].args[2], {
                "x-agent-bounties-acquisition-id": TOKEN,
                "x-agent-bounties-canary": kind,
            })

    def test_actual_wire_metadata_passes_for_each_rail_and_canary_kind(self):
        for rail in probe.RAILS:
            for kind in ("dry-run-v1", "mainnet-v1"):
                with self.subTest(rail=rail, kind=kind):
                    self.invoke(responses(rail, kind), rail, kind)

    def test_fixture_matches_existing_public_server_wire_identity(self):
        source = (ROOT / "crates/mcp-server/src/chatgpt_app.rs").read_text()
        function = source.split("fn attach_attribution_metadata(", 1)[1].split("\nfn ", 1)[0]
        self.assertIn('"' + KEY + '".to_string()', function)

    def test_missing_or_brand_renamed_namespace_still_fails(self):
        for metadata in ({}, {"agentizable.ai/acquisition": responses()[1][2]["result"]["_meta"][KEY]}):
            with self.subTest(metadata_keys=list(metadata)):
                replies = responses()
                replies[1][2]["result"]["_meta"] = metadata
                with self.assertRaisesRegex(RuntimeError, "attribution evidence is incomplete"):
                    self.invoke(replies)

    def test_altered_or_missing_bound_evidence_still_fails(self):
        variants = [
            ("acquisition_id", "aba1_" + "c" * 64 + "." + "d" * 64),
            ("first_touch_rail", "cursor"),
            ("measurement_eligible", True),
            ("measurement_eligible", "false"),
            ("measurement_eligible", None),
            ("authority", "payment"),
        ]
        for key, value in variants:
            with self.subTest(key=key, value=value):
                replies = responses()
                replies[1][2]["result"]["_meta"][KEY][key] = value
                with self.assertRaisesRegex(RuntimeError, "attribution evidence is incomplete"):
                    self.invoke(replies)
        for key in ("acquisition_id", "first_touch_rail", "measurement_eligible", "authority"):
            with self.subTest(missing=key):
                replies = responses()
                del replies[1][2]["result"]["_meta"][KEY][key]
                with self.assertRaisesRegex(RuntimeError, "attribution evidence is incomplete"):
                    self.invoke(replies)

    def test_missing_required_tool_still_fails(self):
        replies = responses()
        replies[1][2]["result"]["tools"] = [{"name": "some_other_tool"}]
        with self.assertRaisesRegex(RuntimeError, "canonical prepare_bounty_post tool is absent"):
            self.invoke(replies)

    def test_changed_response_token_still_fails(self):
        replies = responses()
        replies[1][1]["x-agent-bounties-acquisition-id"] = "other"
        with self.assertRaisesRegex(RuntimeError, "acquisition was not retry-stable"):
            self.invoke(replies)

    def test_missing_measurement_exclusion_stops_before_catalog_call(self):
        replies = responses()
        replies[0][1]["x-agent-bounties-measurement-eligible"] = "true"
        with patch.object(probe, "post_json", side_effect=replies) as post:
            with self.assertRaisesRegex(RuntimeError, "was not measurement-excluded"):
                probe.check_rail("https://mcp.agentizable.ai", "github", "dry-run-v1")
            self.assertEqual(post.call_count, 1)


if __name__ == "__main__":
    unittest.main()
