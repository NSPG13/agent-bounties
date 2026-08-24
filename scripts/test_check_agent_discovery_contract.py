from __future__ import annotations

import importlib.util
import json
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
    def create_repository(
        self, root: Path, *, advertise_a2a: bool, conforming_a2a: bool = False
    ) -> None:
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
            (
                "Agent Bounties implements a public, read-only Agent2Agent (A2A) "
                "1.0 HTTP+JSON interface.\n"
                if conforming_a2a
                else "Agent Bounties does not currently implement the Agent2Agent "
                "(A2A) protocol.\n"
            )
            + "https://github.com/a2aproject/A2A/blob/main/docs/specification.md\n",
            encoding="utf-8",
        )
        if conforming_a2a:
            a2a = root / "crates" / "api" / "src" / "a2a.rs"
            a2a.write_text(
                'const A2A_PROTOCOL_VERSION: &str = "1.0";\n'
                'const MEDIA: &str = "application/a2a+json";\n'
                'route("/.well-known/agent-card.json", get(agent_card));\n'
                'route("/a2a/v1/message:send", post(send_message));\n'
                'route("/a2a/v1/tasks", get(list_tasks));\n'
                "DefaultBodyLimit::max(65536);\n"
                "fn agent_card() {}\nfn send_message() {}\nfn get_task() {}\n"
                "fn list_tasks() {}\nfn cancel_task() {}\n",
                encoding="utf-8",
            )
            with api.open("a", encoding="utf-8") as handle:
                handle.write(
                    "mod a2a;\n.merge(a2a::router())\n"
                    "fn a2a_router_serves_card_and_core_task_operations() {}\n"
                )
            card = {
                "supportedInterfaces": [
                    {
                        "url": "https://api.agentbounties.app/a2a/v1",
                        "protocolBinding": "HTTP+JSON",
                        "protocolVersion": "1.0",
                    }
                ],
                "capabilities": {
                    "streaming": False,
                    "pushNotifications": False,
                    "extendedAgentCard": False,
                },
                "skills": [{"id": "discover"}],
            }
            card_bytes = (json.dumps(card, indent=2) + "\n").encode()
            for relative in (
                "crates/api/fixtures/agent-card.json",
                "site/.well-known/agent-card.json",
            ):
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(card_bytes)
            docs = root / "docs" / "a2a.md"
            docs.write_text("# A2A\n", encoding="utf-8")

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

    def test_conforming_a2a_advertisement_passes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.create_repository(root, advertise_a2a=True, conforming_a2a=True)
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
        with self.assertRaisesRegex(SystemExit, "intentionally removed route objective.html"):
            guard.validate_entrypoint_text(
                "guide.md",
                "Open https://agentbounties.app/objective.html\n",
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

    def test_static_agent_entrypoint_must_resolve(self) -> None:
        discovery = {
            "endpoints": {
                "agent_mode": "https://agentbounties.app/agent/",
                "agent_mode_markdown": "https://agentbounties.app/agent/index.md",
            }
        }
        with self.assertRaisesRegex(SystemExit, "retained Markdown entrypoint"):
            guard.validate_static_discovery_urls(discovery)

    def test_static_agent_entrypoint_alignment_passes(self) -> None:
        endpoint = "https://agentbounties.app/agent/index.md"
        guard.validate_static_discovery_urls(
            {"endpoints": {"agent_mode": endpoint, "agent_mode_markdown": endpoint}}
        )


if __name__ == "__main__":
    unittest.main()
