from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
SCRIPT = SCRIPT_DIR / "check-agent-discovery-contract.py"
SPEC = importlib.util.spec_from_file_location("check_agent_discovery_contract", SCRIPT)
assert SPEC and SPEC.loader
guard = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = guard
SPEC.loader.exec_module(guard)


class AgentDiscoveryContractTests(unittest.TestCase):
    def create_repository(self, root: Path, *, advertise_a2a: bool) -> None:
        api = root / "crates" / "api" / "src" / "main.rs"
        api.parent.mkdir(parents=True)
        api.write_text(
            '.route("/.well-known/agent-bounties.json", get(discovery))\n'
            '.route("/v1/discovery", get(discovery))\n'
            + (
                '.route("/.well-known/agent-card.json", get(agent_card))\n'
                "async fn agent_card() {}\n"
                if advertise_a2a
                else ""
            ),
            encoding="utf-8",
        )
        public = root / "crates" / "web-public" / "src" / "lib.rs"
        public.parent.mkdir(parents=True)
        public.write_text(
            "pub struct DiscoveryEndpoints {}\n"
            + (
                'const CARD: &str = "/.well-known/agent-card.json";\n'
                if advertise_a2a
                else ""
            ),
            encoding="utf-8",
        )
        quickstart = root / "docs" / "agent-quickstart.md"
        quickstart.parent.mkdir(parents=True)
        quickstart.write_text(
            "Read /.well-known/agent-bounties.json.\n"
            + (
                "Read the A2A Agent Card at /.well-known/agent-card.json.\n"
                if advertise_a2a
                else ""
            ),
            encoding="utf-8",
        )
        status = root / "docs" / "a2a-status.md"
        status.write_text(
            "Agent Bounties does not currently implement the Agent2Agent (A2A) "
            "protocol.\n"
            "https://github.com/a2aproject/A2A/blob/main/docs/specification.md\n",
            encoding="utf-8",
        )

    def test_generic_discovery_without_a2a_passes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.create_repository(root, advertise_a2a=False)
            guard.validate_agent_discovery_contract(root)

    def test_unsupported_a2a_advertisement_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.create_repository(root, advertise_a2a=True)
            with self.assertRaisesRegex(SystemExit, "without a conforming A2A server"):
                guard.validate_agent_discovery_contract(root)

    def test_retired_agent_card_fixture_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.create_repository(root, advertise_a2a=False)
            fixture = root / "fixtures" / "a2a-agent-card.json"
            fixture.parent.mkdir(parents=True)
            fixture.write_text("{}\n", encoding="utf-8")
            with self.assertRaisesRegex(SystemExit, "retired unsupported A2A artifact"):
                guard.validate_agent_discovery_contract(root)

    def test_concise_actionable_entrypoint_passes(self) -> None:
        guard.validate_entrypoint_text(
            "guide.md",
            "server/discover\nget_bounty_feed\nOnly `BountySettled` proves payment.\n",
            max_lines=4,
            max_chars=200,
            required=("server/discover", "get_bounty_feed", "BountySettled"),
        )

    def test_removed_public_route_fails(self) -> None:
        with self.assertRaisesRegex(SystemExit, "intentionally removed route earn.html"):
            guard.validate_entrypoint_text(
                "guide.md",
                "Open https://agentbounties.app/earn.html\n",
                max_lines=4,
                max_chars=200,
                required=(),
            )

    def test_entrypoint_length_budget_fails(self) -> None:
        with self.assertRaisesRegex(SystemExit, "too long"):
            guard.validate_entrypoint_text(
                "guide.md",
                "one\ntwo\nthree\n",
                max_lines=2,
                max_chars=200,
                required=(),
            )


if __name__ == "__main__":
    unittest.main()
