#!/usr/bin/env python3
"""Fail closed when Agent Bounties advertises unsupported A2A interoperability."""

from __future__ import annotations

import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

RETIRED_PUBLIC_ROUTES = (
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
        (
            "server/discover",
            "get_bounty_feed",
            "Only a confirmed canonical `BountySettled` or `CompetitionSettledV2` event",
        ),
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

    if "a2aproject/A2A/blob/main/docs/specification.md" not in status:
        raise SystemExit("A2A status must link the current primary specification")

    a2a_source_path = root / "crates" / "api" / "src" / "a2a.rs"
    api_advertises_a2a = (
        "/.well-known/agent-card.json" in api
        or "mod a2a;" in api
        or a2a_source_path.exists()
    )
    other_a2a_advertising = any(
        (
            "A2A Agent Card" in quickstart,
            "/.well-known/agent-card.json" in quickstart,
            "pub agent_card" in public,
            "/.well-known/agent-card.json" in public,
            (root / "crates/api/fixtures/agent-card.json").exists(),
            (root / "site/.well-known/agent-card.json").exists(),
        )
    )
    if api_advertises_a2a or other_a2a_advertising:
        validate_a2a_implementation(root, api, status)
    elif "does not currently implement the Agent2Agent (A2A) protocol" not in status:
        raise SystemExit("A2A status must state that the protocol is not implemented")

    required = {
        "API generic discovery route": "/.well-known/agent-bounties.json",
        "API versioned discovery route": "/v1/discovery",
    }
    for label, marker in required.items():
        if marker not in api:
            raise SystemExit(f"missing {label}")
    if "/.well-known/agent-bounties.json" not in quickstart:
        raise SystemExit("agent quickstart must retain the generic discovery manifest")


def validate_a2a_implementation(root: Path, api: str, status: str) -> None:
    source_path = root / "crates/api/src/a2a.rs"
    api_card_path = root / "crates/api/fixtures/agent-card.json"
    site_card_path = root / "site/.well-known/agent-card.json"
    docs_path = root / "docs/a2a.md"
    required_paths = (source_path, api_card_path, site_card_path, docs_path)
    missing = [str(path.relative_to(root)) for path in required_paths if not path.exists()]
    if missing:
        raise SystemExit(
            "A2A is advertised without a conforming A2A server; missing "
            + ", ".join(missing)
        )

    source = source_path.read_text(encoding="utf-8")
    source_markers = {
        "well-known Agent Card route": "/.well-known/agent-card.json",
        "SendMessage route": "/a2a/v1/message:send",
        "GetTask/ListTasks routes": "/a2a/v1/tasks",
        "Agent Card handler": "fn agent_card",
        "SendMessage handler": "fn send_message",
        "GetTask handler": "fn get_task",
        "ListTasks handler": "fn list_tasks",
        "CancelTask handler": "fn cancel_task",
        "A2A response media type": "application/a2a+json",
        "A2A Major.Minor version": 'A2A_PROTOCOL_VERSION: &str = "1.0"',
        "bounded request body": "DefaultBodyLimit::max",
    }
    missing_markers = [label for label, marker in source_markers.items() if marker not in source]
    if missing_markers:
        raise SystemExit(
            "A2A is advertised without a conforming A2A server; missing core operations: "
            + ", ".join(missing_markers)
        )

    api_markers = (
        "mod a2a;",
        ".merge(a2a::router())",
        "a2a_router_serves_card_and_core_task_operations",
    )
    for marker in api_markers:
        if marker not in api:
            raise SystemExit(f"A2A API integration is missing required marker {marker!r}")

    if api_card_path.read_bytes() != site_card_path.read_bytes():
        raise SystemExit("API and website A2A Agent Cards must be byte-for-byte identical")
    try:
        card = json.loads(api_card_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        raise SystemExit(f"A2A Agent Card is not valid JSON: {error}") from error
    interfaces = card.get("supportedInterfaces")
    expected_interface = {
        "url": "https://api.agentbounties.app/a2a/v1",
        "protocolBinding": "HTTP+JSON",
        "protocolVersion": "1.0",
    }
    if not isinstance(interfaces, list) or expected_interface not in interfaces:
        raise SystemExit("A2A Agent Card must advertise the implemented HTTP+JSON 1.0 interface")
    if not isinstance(card.get("skills"), list) or not card["skills"]:
        raise SystemExit("A2A Agent Card must declare at least one implemented skill")
    capabilities = card.get("capabilities")
    if not isinstance(capabilities, dict):
        raise SystemExit("A2A Agent Card capabilities must be an object")
    for unsupported in ("streaming", "pushNotifications", "extendedAgentCard"):
        if capabilities.get(unsupported) is not False:
            raise SystemExit(f"A2A Agent Card must truthfully declare {unsupported}=false")

    normalized_status = " ".join(status.split())
    if "implements a public, read-only Agent2Agent (A2A) 1.0 HTTP+JSON interface" not in normalized_status:
        raise SystemExit("A2A status must describe the implemented read-only HTTP+JSON interface")


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
