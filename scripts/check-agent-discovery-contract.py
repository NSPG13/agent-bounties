#!/usr/bin/env python3
"""Fail closed when Agent Bounties advertises unsupported A2A interoperability."""

from __future__ import annotations

import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

RETIRED_PUBLIC_ROUTES = (
    "earn.html",
    "post.html",
    "objective.html",
    "create-competition.html",
    "refunds.html",
    "x402.html",
    "prepare-agent.html",
    "agent-budget.html",
    "funding.html",
)

ENTRYPOINT_CONTRACTS = {
    "README.md": (
        180,
        16_000,
        ("service-smoke-spawn", "docs-contract-check", "BountySettled"),
    ),
    "docs/agent-quickstart.md": (
        240,
        18_000,
        (
            "server/discover",
            "2026-07-28",
            "service-smoke-spawn",
            "prepare_agent_to_earn",
            "BountySettled",
        ),
    ),
    "docs/interaction-guide.md": (
        140,
        13_000,
        ("server/discover", "production-smoke", "service-smoke-spawn"),
    ),
    "site/llms.txt": (
        140,
        13_000,
        ("server/discover", "get_bounty_feed", "Only `BountySettled` proves payment."),
    ),
    "site/agent/index.md": (
        90,
        11_000,
        ("server/discover", "No computer use is required", "get_bounty_feed"),
    ),
}


def validate_entrypoint_text(
    label: str,
    text: str,
    *,
    max_lines: int,
    max_chars: int,
    required: tuple[str, ...],
) -> None:
    line_count = len(text.splitlines())
    if line_count > max_lines:
        raise SystemExit(f"{label} is too long: {line_count} lines; limit is {max_lines}")
    if len(text) > max_chars:
        raise SystemExit(f"{label} is too long: {len(text)} characters; limit is {max_chars}")
    for marker in required:
        if marker not in text:
            raise SystemExit(f"{label} is missing actionable marker {marker!r}")
    for route in RETIRED_PUBLIC_ROUTES:
        if route in text:
            raise SystemExit(f"{label} points to intentionally removed route {route}")
    for retired_domain in ("bountyboard.global", "agentbounties.org"):
        if retired_domain in text:
            raise SystemExit(f"{label} uses retired domain {retired_domain}")


def validate_static_discovery_urls(discovery: dict[str, object]) -> None:
    endpoints = discovery.get("endpoints")
    if not isinstance(endpoints, dict):
        raise SystemExit("static discovery endpoints must be an object")
    agent_mode = endpoints.get("agent_mode")
    agent_markdown = endpoints.get("agent_mode_markdown")
    if agent_mode != agent_markdown:
        raise SystemExit(
            "static discovery agent_mode must resolve to the retained Markdown entrypoint"
        )


def validate_public_agent_entrypoints(root: Path) -> None:
    for relative, (max_lines, max_chars, required) in ENTRYPOINT_CONTRACTS.items():
        path = root / relative
        if not path.exists():
            raise SystemExit(f"missing public agent entrypoint {relative}")
        validate_entrypoint_text(
            relative,
            path.read_text(encoding="utf-8"),
            max_lines=max_lines,
            max_chars=max_chars,
            required=required,
        )

    public_source = (root / "crates/web-public/src/lib.rs").read_text(encoding="utf-8")
    for route in RETIRED_PUBLIC_ROUTES:
        retired_url = f'https://agentbounties.app/{route}'
        if retired_url in public_source:
            raise SystemExit(f"shared discovery source advertises removed URL {retired_url}")
    if 'const DISCOVERY_SCHEMA: &str = "https://agentbounties.app/' not in public_source:
        raise SystemExit("shared discovery source must use the agentbounties.app schema identity")

    schema_path = root / "schemas/discovery-manifest.v2.json"
    discovery_path = root / "site/.well-known/agent-bounties.json"
    schema = json.loads(schema_path.read_text(encoding="utf-8"))
    discovery = json.loads(discovery_path.read_text(encoding="utf-8"))
    if schema.get("$id") != "https://agentbounties.app/schemas/discovery-manifest.v2.json":
        raise SystemExit("discovery schema $id must use agentbounties.app")
    if discovery.get("schema") != schema.get("$id"):
        raise SystemExit("static discovery does not identify its published schema")
    unknown = set(discovery) - set(schema.get("properties", {}))
    missing = set(schema.get("required", [])) - set(discovery)
    if unknown or missing:
        raise SystemExit(
            f"static discovery top-level schema mismatch: unknown={sorted(unknown)} "
            f"missing={sorted(missing)}"
        )

    serialized_discovery = json.dumps(discovery, sort_keys=True)
    for route in RETIRED_PUBLIC_ROUTES:
        if route in serialized_discovery:
            raise SystemExit(f"static discovery advertises removed route {route}")
    if discovery.get("website") != "https://agentbounties.app/":
        raise SystemExit("static discovery website must be https://agentbounties.app/")
    validate_static_discovery_urls(discovery)

    protocol = json.loads((root / "site/protocol.json").read_text(encoding="utf-8"))
    if protocol.get("protocol_version") != "agent-bounties/autonomous-v1":
        raise SystemExit("protocol status must identify agent-bounties/autonomous-v1")
    if protocol.get("status") != "active" or protocol.get("chain_id") != 8453:
        raise SystemExit("protocol status must identify the active Base mainnet deployment")


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
    validate_public_agent_entrypoints(ROOT)
    print(
        "Agent discovery contract passed: entrypoints concise; removed routes absent; "
        "schema, protocol, generic discovery, and A2A boundaries valid"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
