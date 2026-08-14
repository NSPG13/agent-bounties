#!/usr/bin/env python3
"""Fail closed when Agent Bounties advertises unsupported A2A interoperability."""

from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def validate_agent_discovery_contract(root: Path) -> None:
    api = (root / "crates" / "api" / "src" / "main.rs").read_text(encoding="utf-8")
    public = (root / "crates" / "web-public" / "src" / "lib.rs").read_text(
        encoding="utf-8"
    )
    quickstart = (root / "docs" / "agent-quickstart.md").read_text(encoding="utf-8")
    status = (root / "docs" / "a2a-status.md").read_text(encoding="utf-8")

    forbidden = {
        "API well-known A2A route": "/.well-known/agent-card.json",
        "API A2A handler": "async fn agent_card",
    }
    for label, marker in forbidden.items():
        if marker in api:
            raise SystemExit(
                f"{label} is advertised without a conforming A2A server; "
                "implement and test every required A2A core operation before restoring it"
            )

    if "pub agent_card" in public or "/.well-known/agent-card.json" in public:
        raise SystemExit("public discovery manifest advertises an unsupported A2A Agent Card")
    if "A2A Agent Card" in quickstart or "/.well-known/agent-card.json" in quickstart:
        raise SystemExit("agent quickstart advertises an unsupported A2A Agent Card")

    retired_artifacts = (
        root / "fixtures" / "a2a-agent-card.json",
        root / "docs" / "a2a-direct-api-binding-v1.md",
        root / "scripts" / "check-a2a-agent-card.py",
    )
    for path in retired_artifacts:
        if path.exists():
            raise SystemExit(
                f"retired unsupported A2A artifact remains: {path.relative_to(root)}"
            )

    if "does not currently implement the Agent2Agent (A2A) protocol" not in status:
        raise SystemExit("A2A status must state that the protocol is not implemented")
    if "a2aproject/A2A/blob/main/docs/specification.md" not in status:
        raise SystemExit("A2A status must link the current primary specification")

    required = {
        "API generic discovery route": "/.well-known/agent-bounties.json",
        "API versioned discovery route": "/v1/discovery",
    }
    for label, marker in required.items():
        if marker not in api:
            raise SystemExit(f"missing {label}")
    if "/.well-known/agent-bounties.json" not in quickstart:
        raise SystemExit("agent quickstart must retain the generic discovery manifest")


def main() -> int:
    validate_agent_discovery_contract(ROOT)
    print(
        "Agent discovery contract passed: generic manifest retained; "
        "unsupported A2A surface absent and status documented"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
