#!/usr/bin/env python3
"""Immutable verifier for the attributed Glama onboarding audit canary."""

from __future__ import annotations

from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
from urllib.parse import urlparse


ROOT = Path(os.environ.get("WORKSPACE_ROOT", "/workspace"))
EVIDENCE_PATH = ROOT / "glama-onboarding-audit.json"
REPORT_PATH = ROOT / "glama-onboarding-audit.md"
SCHEMA = "agent-bounties/glama-onboarding-audit-evidence-v1"
MCP_ENDPOINT = "https://mcp.agentbounties.app/r/glama/mcp"
INSTALL_URL = "https://agentbounties.app/install/glama/"
REVIEW_ORIGIN = "https://agentbounties.app"
PROTOCOL_VERSION = "2025-06-18"
TX_RE = re.compile(r"^0x[0-9a-f]{64}$")
ADDRESS_RE = re.compile(r"^0x[0-9a-f]{40}$")
DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
SIGNED_ATTRIBUTION_RE = re.compile(r"aba1_[0-9a-f]{64}\.[0-9a-f]{64}", re.IGNORECASE)


def fail(message: str) -> None:
    raise SystemExit(message)


def exact_keys(value: object, expected: set[str], label: str) -> dict[str, object]:
    if not isinstance(value, dict):
        fail(f"{label} must be an object")
    actual = set(value)
    if actual != expected:
        missing = sorted(expected - actual)
        extra = sorted(actual - expected)
        fail(f"{label} keys mismatch; missing={missing} extra={extra}")
    return value


def text(value: object, label: str, maximum: int = 4_000) -> str:
    if not isinstance(value, str) or not value.strip():
        fail(f"{label} must be a non-empty string")
    if len(value.encode("utf-8")) > maximum:
        fail(f"{label} exceeds {maximum} bytes")
    return value


def utc_instant(value: object, label: str) -> datetime:
    raw = text(value, label, 64)
    if not raw.endswith("Z"):
        fail(f"{label} must be an RFC 3339 UTC instant ending in Z")
    try:
        parsed = datetime.fromisoformat(raw[:-1] + "+00:00")
    except ValueError as error:
        fail(f"{label} is not a valid RFC 3339 instant: {error}")
    if parsed.tzinfo != timezone.utc:
        fail(f"{label} must use UTC")
    return parsed


def public_https(value: object, label: str, *, hosts: set[str] | None = None) -> str:
    raw = text(value, label, 2_000)
    parsed = urlparse(raw)
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password:
        fail(f"{label} must be a public HTTPS URL without embedded credentials")
    if parsed.hostname in {"localhost", "127.0.0.1", "::1"}:
        fail(f"{label} must not use a loopback host")
    if hosts is not None and parsed.hostname not in hosts:
        fail(f"{label} must use one of {sorted(hosts)}")
    return raw


def sha256_file(path: Path) -> str:
    return f"sha256:{hashlib.sha256(path.read_bytes()).hexdigest()}"


def transcript_evidence(value: object, label: str) -> dict[str, object]:
    record = exact_keys(
        value,
        {"captured_at", "request_sha256", "response_sha256", "redacted"},
        label,
    )
    utc_instant(record["captured_at"], f"{label}.captured_at")
    for field in ("request_sha256", "response_sha256"):
        if not DIGEST_RE.fullmatch(str(record[field])):
            fail(f"{label}.{field} must be sha256:<64 lowercase hex>")
    if record["redacted"] is not True:
        fail(f"{label}.redacted must be true")
    return record


def event_proof(value: object, name: str) -> dict[str, object]:
    record = exact_keys(
        value,
        {"event", "transaction_hash", "block_number", "log_index", "explorer_url"},
        f"lifecycle.{name}",
    )
    if record["event"] != name:
        fail(f"lifecycle.{name}.event must be {name}")
    if not TX_RE.fullmatch(str(record["transaction_hash"])):
        fail(f"lifecycle.{name}.transaction_hash must be a lowercase transaction hash")
    if not isinstance(record["block_number"], int) or record["block_number"] <= 0:
        fail(f"lifecycle.{name}.block_number must be a positive integer")
    if not isinstance(record["log_index"], int) or record["log_index"] < 0:
        fail(f"lifecycle.{name}.log_index must be a non-negative integer")
    expected = f"https://basescan.org/tx/{record['transaction_hash']}"
    if record["explorer_url"] != expected:
        fail(f"lifecycle.{name}.explorer_url must be {expected}")
    return record


if not EVIDENCE_PATH.is_file():
    fail("missing glama-onboarding-audit.json")
if not REPORT_PATH.is_file():
    fail("missing glama-onboarding-audit.md")
if EVIDENCE_PATH.stat().st_size > 64 * 1024:
    fail("glama-onboarding-audit.json exceeds 64 KiB")
if REPORT_PATH.stat().st_size > 128 * 1024:
    fail("glama-onboarding-audit.md exceeds 128 KiB")

try:
    evidence = json.loads(EVIDENCE_PATH.read_text(encoding="utf-8"))
except (UnicodeDecodeError, json.JSONDecodeError) as error:
    fail(f"glama-onboarding-audit.json is invalid UTF-8 JSON: {error}")

evidence = exact_keys(
    evidence,
    {
        "schema_version",
        "rail",
        "measurement_exclusion",
        "mcp",
        "wallet_boundary",
        "lifecycle",
        "report",
    },
    "evidence",
)
if evidence["schema_version"] != SCHEMA:
    fail(f"schema_version must be {SCHEMA}")
if evidence["rail"] != "glama":
    fail("rail must be glama")
if evidence["measurement_exclusion"] != "synthetic_canary":
    fail("measurement_exclusion must be synthetic_canary")

mcp = exact_keys(
    evidence["mcp"],
    {
        "endpoint",
        "install_url",
        "protocol_version",
        "first_touch_rail",
        "current_rail",
        "measurement_eligible_at_discovery",
        "prepare_bounty_post_discoverable",
        "initialize",
        "tools_list",
    },
    "mcp",
)
if mcp["endpoint"] != MCP_ENDPOINT:
    fail(f"mcp.endpoint must be {MCP_ENDPOINT}")
if mcp["install_url"] != INSTALL_URL:
    fail(f"mcp.install_url must be {INSTALL_URL}")
if mcp["protocol_version"] != PROTOCOL_VERSION:
    fail(f"mcp.protocol_version must be {PROTOCOL_VERSION}")
if mcp["first_touch_rail"] != "glama" or mcp["current_rail"] != "glama":
    fail("MCP first-touch and current rail must both be glama")
if mcp["measurement_eligible_at_discovery"] is not True:
    fail("the discovery session must be measurement eligible before canary exclusion")
if mcp["prepare_bounty_post_discoverable"] is not True:
    fail("prepare_bounty_post must be discoverable")
initialize = transcript_evidence(mcp["initialize"], "mcp.initialize")
tools_list = transcript_evidence(mcp["tools_list"], "mcp.tools_list")
if utc_instant(initialize["captured_at"], "mcp.initialize.captured_at") > utc_instant(
    tools_list["captured_at"], "mcp.tools_list.captured_at"
):
    fail("initialize evidence must not be later than tools/list evidence")

wallet = exact_keys(
    evidence["wallet_boundary"],
    {
        "first_party_review_url",
        "human_approval_observed",
        "agent_received_private_key",
        "agent_received_seed_phrase",
        "agent_received_wallet_signature",
        "agent_received_payout_authority",
    },
    "wallet_boundary",
)
review_url = public_https(wallet["first_party_review_url"], "wallet_boundary.first_party_review_url")
if not review_url.startswith(f"{REVIEW_ORIGIN}/post.html?"):
    fail("wallet review must occur on the first-party Agent Bounties post page")
if wallet["human_approval_observed"] is not True:
    fail("human_approval_observed must be true")
for field in (
    "agent_received_private_key",
    "agent_received_seed_phrase",
    "agent_received_wallet_signature",
    "agent_received_payout_authority",
):
    if wallet[field] is not False:
        fail(f"wallet_boundary.{field} must be false")

lifecycle = exact_keys(
    evidence["lifecycle"],
    {
        "network",
        "chain_id",
        "bounty_contract",
        "bounty_id",
        "created",
        "funded",
        "verifier_evidence",
        "settled",
    },
    "lifecycle",
)
if lifecycle["network"] != "base-mainnet" or lifecycle["chain_id"] != 8453:
    fail("lifecycle must identify Base mainnet chain 8453")
if not ADDRESS_RE.fullmatch(str(lifecycle["bounty_contract"])):
    fail("lifecycle.bounty_contract must be a lowercase EVM address")
if not isinstance(lifecycle["bounty_id"], int) or lifecycle["bounty_id"] < 0:
    fail("lifecycle.bounty_id must be a non-negative integer")
created = event_proof(lifecycle["created"], "CanonicalBountyCreated")
funded = event_proof(lifecycle["funded"], "FundingAdded")
settled = event_proof(lifecycle["settled"], "BountySettled")
if not (created["block_number"] <= funded["block_number"] <= settled["block_number"]):
    fail("canonical lifecycle events must be ordered by block number")
verifier = exact_keys(
    lifecycle["verifier_evidence"],
    {"public_url", "sha256"},
    "lifecycle.verifier_evidence",
)
public_https(verifier["public_url"], "lifecycle.verifier_evidence.public_url")
if not DIGEST_RE.fullmatch(str(verifier["sha256"])):
    fail("lifecycle.verifier_evidence.sha256 must be sha256:<64 lowercase hex>")

report = exact_keys(
    evidence["report"],
    {"public_url", "path", "sha256", "started_at", "completed_at"},
    "report",
)
public_https(report["public_url"], "report.public_url")
if report["path"] != REPORT_PATH.name:
    fail(f"report.path must be {REPORT_PATH.name}")
if report["sha256"] != sha256_file(REPORT_PATH):
    fail("report.sha256 does not match glama-onboarding-audit.md")
if utc_instant(report["started_at"], "report.started_at") > utc_instant(
    report["completed_at"], "report.completed_at"
):
    fail("report.started_at must not be later than report.completed_at")

report_text = REPORT_PATH.read_text(encoding="utf-8")
combined = EVIDENCE_PATH.read_text(encoding="utf-8") + "\n" + report_text
if SIGNED_ATTRIBUTION_RE.search(combined):
    fail("the public evidence bundle exposes a signed acquisition identifier")
normalized_report = " ".join(report_text.split())
for required in (
    MCP_ENDPOINT,
    INSTALL_URL,
    PROTOCOL_VERSION,
    "First-touch rail: glama",
    "prepare_bounty_post",
    "no private key",
    "no seed phrase",
    "no wallet signature",
    "no payout authority",
    "CanonicalBountyCreated",
    "FundingAdded",
    "BountySettled",
    str(created["transaction_hash"]),
    str(funded["transaction_hash"]),
    str(settled["transaction_hash"]),
    str(verifier["public_url"]),
):
    if required not in normalized_report:
        fail(f"public report is missing required evidence: {required}")

print("Glama onboarding audit benchmark passed")
