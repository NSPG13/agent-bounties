#!/usr/bin/env python3
"""Publish the canonical public bounty inventory as a GitHub issue mirror.

Dry-run is the default. The writer is intentionally non-authoritative: it can
create or update GitHub issues, but it cannot fund, claim, verify, settle, or
otherwise call a bounty contract.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import asdict, dataclass, replace
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Mapping


USER_AGENT = "agent-bounties-github-discovery/1"
PROJECTION_SCHEMA = "agent-bounties/github-bounty-discovery-v1"
POLICY_SCHEMA = "agent-bounties/github-bounty-discovery-policy-v1"
LANDING_COPY_SCHEMA = "agent-bounties/github-bounty-landing-copy-v1"
REVIEWED_BETA3_SOURCE_PATHS = frozenset(
    {
        "ops/open-competition-v2-forward-gmv-candidate-pool-v2.json",
        "ops/open-competition-v2-forward-gmv-reward-cohort-v1.json",
    }
)
CORE_DISCOVERY_PROTOCOLS = frozenset(
    {
        "agent-bounties/autonomous-v1",
        "agent-bounties/open-competition-v1",
    }
)
BETA3_PROTOCOL = "agent-bounties/open-competition-v2-beta3"
SUPPORTED_PROTOCOLS = frozenset({*CORE_DISCOVERY_PROTOCOLS, BETA3_PROTOCOL})
LIFECYCLE_STATES = frozenset(
    {
        "funding_needed",
        "ready_to_earn",
        "in_progress",
        "verification_pending",
        "unavailable",
        "expired",
        "settled",
        "cancelled",
    }
)
KNOWN_AUTONOMOUS_STATUSES = frozenset(
    {"open", "claimable", "claimed", "submitted", "paid", "cancelled"}
)
NONTERMINAL_STATES = frozenset(
    {"funding_needed", "ready_to_earn", "in_progress", "verification_pending", "unavailable"}
)
ADDRESS = re.compile(r"^0x[0-9a-f]{40}$")
TX_HASH = re.compile(r"^0x[0-9a-f]{64}$")
DISCOVERY_ID = re.compile(r"^eip155:[0-9]+:agent-bounties/[a-z0-9-]+:0x[0-9a-f]{40}$")
MANAGED_START = "<!-- agent-bounties/github-discovery-v1:start -->"
MANAGED_END = "<!-- agent-bounties/github-discovery-v1:end -->"
IDENTITY_MARKER_RE = re.compile(
    r"<!-- agent-bounties/github-discovery-v1 (\{[^\r\n]*\}) -->"
)
SETTLEMENT_RECEIPT_MARKER = "<!-- agent-bounties-canonical-settlement -->"
COMMON_LABELS = frozenset({"bounty", "payments"})
SKILL_LABELS = frozenset(
    {"skill:rust", "skill:python", "skill:web", "skill:research", "skill:browser"}
)
MANAGED_LABELS = frozenset(
    {
        *COMMON_LABELS,
        "ai-agent-welcome",
        *SKILL_LABELS,
        "funding-needed",
        "funded-live",
        "ready-to-earn",
        "claimable-live",
        "open-competition",
        "verifier",
        "claimed-live",
        "in-progress",
        "verification-pending",
        "verification-unavailable",
        "refund-available",
        "expired",
        "cancelled",
        "settled-paid",
        "good-first-agent-bounty",
    }
)
LABEL_DEFINITIONS = {
    "bounty": ("0e8a16", "Work with an explicit outcome or reward"),
    "ai-agent-welcome": ("7057ff", "AI agents are welcome to participate"),
    "payments": ("1d76db", "Payment or escrow related"),
    "funding-needed": ("d4c5f9", "Canonical bounty still needs funding"),
    "funded-live": ("0e8a16", "Canonical bounty is fully funded"),
    "ready-to-earn": ("a2eeef", "Public funded work accepting an eligible agent action"),
    "claimable-live": ("2da44e", "Compatibility discovery label for live earning work"),
    "open-competition": ("5319e7", "First valid confirmed reveal wins"),
    "verifier": ("006b75", "Uses an explicitly identified verifier"),
    "claimed-live": ("fbca04", "Exclusive claim is in progress"),
    "in-progress": ("fbca04", "Work or reveal recovery is in progress"),
    "verification-pending": ("f9d0c4", "Canonical submission awaits verification"),
    "verification-unavailable": ("b60205", "Approved verification is unavailable"),
    "refund-available": ("c5def5", "A wallet-scoped pull recovery action remains"),
    "expired": ("ededed", "The canonical participation window expired"),
    "cancelled": ("ededed", "The canonical bounty was cancelled"),
    "settled-paid": ("0e8a16", "Canonical BountySettled payment evidence exists"),
    "good-first-agent-bounty": ("bfdadc", "Explicitly graded as suitable introductory agent work"),
    "skill:rust": ("dea584", "Primary work skill: Rust"),
    "skill:python": ("3572a5", "Primary work skill: Python"),
    "skill:web": ("1f6feb", "Primary work skill: web development"),
    "skill:research": ("8250df", "Primary work skill: research"),
    "skill:browser": ("c5def5", "Primary work skill: browser operation"),
}
BOUNDARIES = (
    "GitHub is a discovery mirror, not a funding, verification, or settlement authority.",
    "A missing record never authorizes label removal or issue closure.",
    "Only confirmed canonical BountySettled or CompetitionSettledV2 evidence proves payment.",
)


class LabelReconciliationError(RuntimeError):
    pass


@dataclass(frozen=True)
class HttpResult:
    status: int
    body: Any
    headers: Mapping[str, str]


@dataclass(frozen=True)
class SettlementReceipt:
    fingerprint: str
    body: str


@dataclass(frozen=True)
class IssuePlan:
    discovery_id: str
    protocol_version: str
    lifecycle_state: str
    competition_mode: str
    issue_number: int | None
    issue_url: str | None
    mapping_action: str
    create_eligible: bool
    title: str
    original_title: str
    original_body: str
    desired_body: str
    current_managed_labels: list[str]
    desired_managed_labels: list[str]
    add_labels: list[str]
    remove_labels: list[str]
    desired_state: str
    desired_state_reason: str | None
    current_state: str | None
    current_state_reason: str | None
    settlement_receipt: SettlementReceipt | None
    receipt_action: str
    receipt_comment_id: int | None
    publication_lag_seconds: int | None


HttpRequest = Callable[[str, str, Any | None, Mapping[str, str] | None], HttpResult]


def normalize_api_base_url(value: str) -> str:
    parsed = urllib.parse.urlsplit(value.strip())
    if parsed.scheme not in {"http", "https"} or not parsed.netloc:
        raise LabelReconciliationError("API base URL must be an absolute http(s) URL")
    if parsed.query or parsed.fragment or parsed.username or parsed.password:
        raise LabelReconciliationError("API base URL cannot contain credentials, query, or fragment")
    host = (parsed.hostname or "").lower()
    if parsed.scheme != "https" and host not in {"localhost", "127.0.0.1", "::1"}:
        raise LabelReconciliationError("non-local API execution requires https")
    return urllib.parse.urlunsplit((parsed.scheme, parsed.netloc, parsed.path.rstrip("/"), "", ""))


def validate_repository(value: str) -> str:
    owner, separator, repo = value.strip().partition("/")
    if (
        not separator
        or not owner
        or not repo
        or "/" in repo
        or not re.fullmatch(r"[A-Za-z0-9_.-]+", owner)
        or not re.fullmatch(r"[A-Za-z0-9_.-]+", repo)
    ):
        raise LabelReconciliationError(f"invalid repository: {value!r}")
    return f"{owner}/{repo}"


def decode_response(raw: str, content_type: str) -> Any:
    if "json" in content_type.lower() or raw.lstrip().startswith(("{", "[")):
        try:
            return json.loads(raw)
        except json.JSONDecodeError:
            pass
    return raw


def default_http_request(
    method: str,
    url: str,
    body: Any | None,
    headers: Mapping[str, str] | None,
) -> HttpResult:
    request_headers = {
        "Accept": "application/vnd.github+json, application/json",
        "User-Agent": USER_AGENT,
    }
    if headers:
        request_headers.update(headers)
    data = None
    if body is not None:
        data = json.dumps(body, separators=(",", ":")).encode("utf-8")
        request_headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=data, headers=request_headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            raw = response.read().decode("utf-8")
            return HttpResult(
                response.status,
                decode_response(raw, response.headers.get("Content-Type", "")),
                dict(response.headers.items()),
            )
    except urllib.error.HTTPError as error:
        raw = error.read().decode("utf-8", errors="replace")
        return HttpResult(
            error.code,
            decode_response(raw, error.headers.get("Content-Type", "")),
            dict(error.headers.items()),
        )
    except urllib.error.URLError as error:
        raise LabelReconciliationError(f"request failed for {url}: {error.reason}") from error


def request_with_retry(
    request: HttpRequest,
    method: str,
    url: str,
    body: Any | None = None,
    headers: Mapping[str, str] | None = None,
    *,
    sleep: Callable[[float], None] = time.sleep,
) -> HttpResult:
    result: HttpResult | None = None
    for attempt in range(3):
        result = request(method, url, body, headers)
        if result.status not in {429, 500, 502, 503, 504}:
            return result
        if attempt < 2:
            retry_after = next(
                (value for key, value in result.headers.items() if key.lower() == "retry-after"),
                None,
            )
            delay = min(5.0, float(retry_after)) if str(retry_after or "").isdigit() else float(2**attempt)
            sleep(delay)
    assert result is not None
    return result


def github_headers(token: str | None) -> dict[str, str]:
    headers = {
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    }
    if token:
        headers["Authorization"] = f"Bearer {token}"
    return headers


def parse_instant(value: Any, field: str) -> datetime:
    if not isinstance(value, str):
        raise LabelReconciliationError(f"{field} must be an RFC3339 string")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise LabelReconciliationError(f"{field} must be RFC3339") from error
    if parsed.tzinfo is None:
        raise LabelReconciliationError(f"{field} must include a timezone")
    return parsed.astimezone(timezone.utc)


def require_unsigned(value: Any, field: str) -> int:
    if isinstance(value, int) and not isinstance(value, bool) and value >= 0:
        return value
    if isinstance(value, str) and re.fullmatch(r"0|[1-9][0-9]*", value):
        return int(value)
    raise LabelReconciliationError(f"invalid unsigned field {field}")


def load_policy(path: Path, repository: str, network: str) -> dict[str, Any]:
    try:
        policy = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise LabelReconciliationError(f"cannot load activation policy: {error}") from error
    if not isinstance(policy, dict) or policy.get("schema_version") != POLICY_SCHEMA:
        raise LabelReconciliationError("activation policy schema is not supported")
    if policy.get("repository") != repository or policy.get("network") != network:
        raise LabelReconciliationError("activation policy repository or network mismatch")
    if require_unsigned(policy.get("chain_id"), "policy.chain_id") <= 0:
        raise LabelReconciliationError("activation policy chain id must be positive")
    activation = policy.get("activation")
    if not isinstance(activation, dict):
        raise LabelReconciliationError("activation policy lacks activation evidence")
    parse_instant(activation.get("timestamp"), "policy.activation.timestamp")
    if require_unsigned(activation.get("safe_block"), "policy.activation.safe_block") <= 0:
        raise LabelReconciliationError("activation block must be positive")
    if not TX_HASH.fullmatch(str(activation.get("safe_block_hash") or "").lower()):
        raise LabelReconciliationError("activation safe block hash is invalid")
    required = policy.get("required_backfill_discovery_ids")
    if not isinstance(required, list) or not all(isinstance(value, str) for value in required):
        raise LabelReconciliationError("required backfill identities are malformed")
    if len(required) != len(set(required)):
        raise LabelReconciliationError("required backfill identities are duplicated")
    trial = policy.get("open_competition_compatibility_trial")
    if not isinstance(trial, dict):
        raise LabelReconciliationError("compatibility trial policy is missing")
    if parse_instant(trial.get("ends_at"), "trial.ends_at") <= parse_instant(
        trial.get("starts_at"), "trial.starts_at"
    ):
        raise LabelReconciliationError("compatibility trial interval is invalid")
    return policy


def load_landing_copy(path: Path, repository: str) -> dict[str, dict[str, Any]]:
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise LabelReconciliationError(f"cannot load reviewed landing copy: {error}") from error
    if (
        not isinstance(manifest, dict)
        or manifest.get("schema_version") != LANDING_COPY_SCHEMA
        or manifest.get("repository") != repository
        or not isinstance(manifest.get("entries"), dict)
    ):
        raise LabelReconciliationError("reviewed landing-copy manifest is malformed")
    entries: dict[str, dict[str, Any]] = {}
    action_verbs = {
        "add",
        "analyze",
        "build",
        "create",
        "document",
        "fix",
        "implement",
        "improve",
        "migrate",
        "produce",
        "remove",
        "restore",
        "ship",
        "test",
        "update",
        "verify",
    }
    for identity, raw in manifest["entries"].items():
        if not isinstance(identity, str) or not DISCOVERY_ID.fullmatch(identity):
            raise LabelReconciliationError("landing-copy discovery identity is invalid")
        if not isinstance(raw, dict):
            raise LabelReconciliationError(f"landing copy is not an object: {identity}")
        title = str(raw.get("outcome_title") or "").strip()
        title_words = re.findall(r"[A-Za-z0-9]+", title)
        if (
            len(title_words) < 5
            or title_words[0].lower() not in action_verbs
            or len(title) > 160
            or title.lower() in {"bounty", "task", "help needed", "fix bug"}
        ):
            raise LabelReconciliationError(f"landing title is not outcome-specific: {identity}")
        intent = str(raw.get("intent_summary") or "").strip()
        sentences = [part.strip() for part in re.split(r"[.!?](?:\s+|$)", intent) if part.strip()]
        if len(sentences) != 2 or len(intent) > 500:
            raise LabelReconciliationError(f"landing intent must contain exactly two sentences: {identity}")
        skills = raw.get("skills")
        if (
            not isinstance(skills, list)
            or not 1 <= len(skills) <= 3
            or any(f"skill:{skill}" not in SKILL_LABELS for skill in skills)
            or len(skills) != len(set(skills))
        ):
            raise LabelReconciliationError(f"landing skills are not allowlisted: {identity}")
        criteria = raw.get("acceptance_criteria")
        if (
            not isinstance(criteria, list)
            or not 2 <= len(criteria) <= 8
            or any(not isinstance(value, str) or not value.strip() or len(value) > 300 for value in criteria)
        ):
            raise LabelReconciliationError(f"landing acceptance criteria are malformed: {identity}")
        canonical_url = require_public_https_url(
            raw.get("canonical_opportunity_url"), f"{identity}.canonical_opportunity_url"
        )
        safe_start = raw.get("safe_start")
        if not isinstance(safe_start, dict):
            raise LabelReconciliationError(f"landing safe start is missing: {identity}")
        for key in ("label", "instructions"):
            value = safe_start.get(key)
            if not isinstance(value, str) or not value.strip() or len(value) > 300:
                raise LabelReconciliationError(f"landing safe start {key} is malformed: {identity}")
        require_public_https_url(safe_start.get("url"), f"{identity}.safe_start.url")
        if not isinstance(raw.get("issue_number"), int) or raw["issue_number"] <= 0:
            raise LabelReconciliationError(f"landing issue number is invalid: {identity}")
        parse_instant(raw.get("reviewed_at"), f"{identity}.reviewed_at")
        if not isinstance(raw.get("reviewed_by"), str) or not raw["reviewed_by"].strip():
            raise LabelReconciliationError(f"landing reviewer is missing: {identity}")
        entries[identity] = {
            **raw,
            "outcome_title": title,
            "intent_summary": intent,
            "canonical_opportunity_url": canonical_url,
            "skills": list(skills),
            "acceptance_criteria": [value.strip() for value in criteria],
        }
    return entries


def attach_landing_copy(
    items: list[dict[str, Any]],
    entries: Mapping[str, Mapping[str, Any]],
    repository: str,
) -> list[dict[str, Any]]:
    attached: list[dict[str, Any]] = []
    for source in items:
        item = dict(source)
        if item.get("lifecycle_state") == "ready_to_earn":
            identity = str(item["discovery_id"])
            landing = entries.get(identity)
            if (
                landing is None
                and item.get("protocol_version") == BETA3_PROTOCOL
                and item.get("competition_mode") == "best_score"
                and is_same_repository_reviewed_beta3_artifact(
                    item.get("source_url"), repository
                )
            ):
                scoring_window = item.get("scoring_window")
                if not isinstance(scoring_window, dict):
                    raise LabelReconciliationError(
                        f"reviewed Beta3 landing copy lacks a scoring window: {identity}"
                    )
                public_url = require_public_https_url(
                    item.get("public_url"), f"{identity}.public_url"
                )
                title = f"Generate qualifying GMV for {str(item.get('title') or '').strip()}"
                if len(title) > 160:
                    raise LabelReconciliationError(
                        f"reviewed Beta3 landing title is too long: {identity}"
                    )
                landing = {
                    "issue_number": None,
                    "outcome_title": title,
                    "intent_summary": (
                        "Create and fund useful marketplace demand that settles canonically inside the exact scoring window. "
                        "Produce the highest eligible externally funded GMV score without counting excluded wallets or contracts."
                    ),
                    "skills": ["research", "browser"],
                    "canonical_opportunity_url": public_url,
                    "acceptance_criteria": [
                        "Use the same Base wallet for qualifying child-bounty funding and the competition entry.",
                        "Have a different eligible wallet complete the useful child bounty.",
                        f"Reach confirmed canonical child settlement between {scoring_window.get('starts_at')} and {scoring_window.get('ends_at')}.",
                        "Preserve the contract-bound funding and settlement evidence required by the published snapshot and verifier.",
                    ],
                    "safe_start": {
                        "label": "Open the contract-specific participation page",
                        "url": public_url,
                        "instructions": "Read the exact UTC window, complete economics, exclusions, and current next action before funding anything.",
                    },
                    "reviewed_by": "reviewed Beta3 public metadata",
                    "reviewed_at": item.get("updated_at"),
                }
            if landing is None:
                item["_landing_action_required"] = [
                    "outcome_title",
                    "intent_summary",
                    "skills",
                    "canonical_opportunity_url",
                    "acceptance_criteria",
                    "safe_start",
                ]
            else:
                if landing.get("canonical_opportunity_url") != item.get("public_url"):
                    raise LabelReconciliationError(
                        f"reviewed canonical opportunity URL drifted from projection: {identity}"
                    )
                source_issue = parse_same_repository_issue(item.get("source_url"), repository)
                landing_issue = landing.get("issue_number")
                if source_issue is not None and source_issue != landing_issue:
                    raise LabelReconciliationError(
                        f"reviewed landing copy maps the wrong source issue: {identity}"
                    )
                if source_issue is None and landing_issue is not None:
                    raise LabelReconciliationError(
                        f"artifact-backed landing copy must not preassign an issue: {identity}"
                    )
                item["_landing"] = dict(landing)
        attached.append(item)
    return attached


def validate_projection(payload: Any, network: str, policy: Mapping[str, Any]) -> list[dict[str, Any]]:
    if not isinstance(payload, dict) or payload.get("schema_version") != PROJECTION_SCHEMA:
        raise LabelReconciliationError("discovery projection schema is not supported")
    if payload.get("network") != network or payload.get("chain_id") != policy.get("chain_id"):
        raise LabelReconciliationError("discovery projection network or chain mismatch")
    safe = payload.get("safe_block")
    if (
        payload.get("degraded") is not False
        or not isinstance(safe, dict)
        or safe.get("fresh") is not True
        or require_unsigned(safe.get("number"), "safe_block.number") <= 0
        or not TX_HASH.fullmatch(str(safe.get("hash") or "").lower())
    ):
        raise LabelReconciliationError("discovery projection is degraded or stale")
    partial_protocols_raw = payload.get("partial_protocols", [])
    if (
        not isinstance(partial_protocols_raw, list)
        or not all(isinstance(protocol, str) for protocol in partial_protocols_raw)
        or len(set(partial_protocols_raw)) != len(partial_protocols_raw)
        or not set(partial_protocols_raw).issubset({BETA3_PROTOCOL})
    ):
        raise LabelReconciliationError("partial projection protocols are malformed")
    partial_protocols = set(partial_protocols_raw)
    sources = payload.get("source_statuses")
    if not isinstance(sources, list) or {
        source.get("protocol_version") for source in sources if isinstance(source, dict)
    } != SUPPORTED_PROTOCOLS:
        raise LabelReconciliationError("discovery projection protocol adapters are incomplete")
    for source in sources:
        if not isinstance(source, dict):
            raise LabelReconciliationError("a canonical projection source is malformed")
        protocol = str(source.get("protocol_version") or "")
        if protocol in partial_protocols:
            if (
                source.get("available") is not False
                or source.get("fresh") is not False
                or require_unsigned(source.get("item_count"), f"{protocol}.item_count") != 0
                or not str(source.get("error") or "").strip()
            ):
                raise LabelReconciliationError("a partial projection source is malformed")
        elif source.get("available") is not True or source.get("fresh") is not True:
            raise LabelReconciliationError("a canonical projection source is degraded")
    items = payload.get("items")
    if not isinstance(items, list) or not all(isinstance(item, dict) for item in items):
        raise LabelReconciliationError("discovery projection items are malformed")
    seen: set[str] = set()
    for item in items:
        identity = str(item.get("discovery_id") or "")
        protocol = str(item.get("protocol_version") or "")
        contract = str(item.get("bounty_contract") or "").lower()
        lifecycle = str(item.get("lifecycle_state") or "")
        mode = str(item.get("competition_mode") or "")
        if (
            not DISCOVERY_ID.fullmatch(identity)
            or identity in seen
            or protocol not in SUPPORTED_PROTOCOLS
            or protocol in partial_protocols
            or lifecycle not in LIFECYCLE_STATES
            or mode not in {"exclusive_claim", "first_valid_submission", "best_score"}
            or not ADDRESS.fullmatch(contract)
            or item.get("network") != network
            or item.get("chain_id") != policy.get("chain_id")
            or not isinstance(item.get("title"), str)
            or not str(item.get("title")).strip()
            or not isinstance(item.get("summary"), str)
            or not isinstance(item.get("categories"), list)
            or not isinstance(item.get("skills"), list)
        ):
            raise LabelReconciliationError(f"malformed discovery record: {identity or '<missing>'}")
        if item.get("visibility") != "public":
            raise LabelReconciliationError(f"private record reached public projection: {identity}")
        require_public_https_url(item.get("public_url"), f"{identity}.public_url")
        if item.get("source_url") is not None:
            require_public_https_url(item.get("source_url"), f"{identity}.source_url")
        action = item.get("next_action")
        if not isinstance(action, dict):
            raise LabelReconciliationError(f"next action is malformed: {identity}")
        require_public_https_url(action.get("url"), f"{identity}.next_action.url")
        if protocol == "agent-bounties/open-competition-v1" and mode != "first_valid_submission":
            raise LabelReconciliationError(f"Open Competition mode mismatch: {identity}")
        if protocol == BETA3_PROTOCOL and mode not in {"first_valid_submission", "best_score"}:
            raise LabelReconciliationError(f"Open Competition mode mismatch: {identity}")
        if protocol == "agent-bounties/autonomous-v1" and mode != "exclusive_claim":
            raise LabelReconciliationError(f"autonomous-v1 mode mismatch: {identity}")
        for field in (
            "reward_usdc_base_units",
            "verifier_reward_usdc_base_units",
            "bond_usdc_base_units",
            "funded_usdc_base_units",
            "funding_target_usdc_base_units",
        ):
            require_unsigned(item.get(field), f"{identity}.{field}")
        parse_instant(item.get("created_at"), f"{identity}.created_at")
        parse_instant(item.get("updated_at"), f"{identity}.updated_at")
        require_unsigned(item.get("created_block"), f"{identity}.created_block")
        if lifecycle == "settled":
            validate_settlement(item)
        elif item.get("settlement_evidence") is not None:
            raise LabelReconciliationError(f"non-settled record exposes payment evidence: {identity}")
        if item.get("ready_to_earn") is True and (
            lifecycle != "ready_to_earn"
            or item.get("funded") is not True
            or item.get("verification_ready") is not True
        ):
            raise LabelReconciliationError(f"unsafe ready-to-earn record: {identity}")
        seen.add(identity)
    for source in sources:
        protocol = str(source["protocol_version"])
        actual = sum(item.get("protocol_version") == protocol for item in items)
        if require_unsigned(source.get("item_count"), f"{protocol}.item_count") != actual:
            raise LabelReconciliationError(f"projection source count mismatch: {protocol}")
    required = set(policy["required_backfill_discovery_ids"])
    missing_required = required - seen
    if missing_required:
        raise LabelReconciliationError(
            "required backfill identities are missing: " + ", ".join(sorted(missing_required))
        )
    return items


def validate_settlement(item: Mapping[str, Any]) -> Mapping[str, Any]:
    identity = str(item.get("discovery_id") or "")
    evidence = item.get("settlement_evidence")
    if not isinstance(evidence, dict):
        raise LabelReconciliationError(f"settled record lacks evidence: {identity}")
    event_name = evidence.get("event_name")
    if event_name not in {"BountySettled", "CompetitionSettledV2"} or evidence.get("confirmed_canonical") is not True:
        raise LabelReconciliationError(f"settlement is not canonical: {identity}")
    if (
        str(evidence.get("bounty_contract") or "").lower()
        != str(item.get("bounty_contract") or "").lower()
        or not TX_HASH.fullmatch(str(evidence.get("transaction_hash") or "").lower())
        or not ADDRESS.fullmatch(str(evidence.get("solver_wallet") or "").lower())
    ):
        raise LabelReconciliationError(f"settlement identity is malformed: {identity}")
    solver_reward = require_unsigned(evidence.get("solver_reward"), "settlement.solver_reward")
    if event_name == "CompetitionSettledV2":
        keeper_reward = require_unsigned(evidence.get("keeper_reward"), "settlement.keeper_reward")
        if (
            str(item.get("protocol_version")) != BETA3_PROTOCOL
            or not ADDRESS.fullmatch(str(evidence.get("keeper_wallet") or "").lower())
            or solver_reward != require_unsigned(
                item.get("reward_usdc_base_units"), "reward_usdc_base_units"
            )
            or keeper_reward != require_unsigned(
                item.get("verifier_reward_usdc_base_units"),
                "verifier_reward_usdc_base_units",
            )
        ):
            raise LabelReconciliationError(f"CompetitionSettledV2 payout is inconsistent: {identity}")
        return evidence
    returned_bond = require_unsigned(evidence.get("returned_bond"), "settlement.returned_bond")
    bonus = require_unsigned(evidence.get("completion_bonus"), "settlement.completion_bonus")
    payout = require_unsigned(evidence.get("solver_payout"), "settlement.solver_payout")
    require_unsigned(evidence.get("verifier_reward"), "settlement.verifier_reward")
    if payout != solver_reward + returned_bond + bonus:
        raise LabelReconciliationError(f"settlement payout is inconsistent: {identity}")
    return evidence


def fetch_projection(request: HttpRequest, api_base_url: str, network: str) -> dict[str, Any]:
    health = request_with_retry(request, "GET", f"{api_base_url}/health")
    if health.status != 200 or str(health.body).strip() != "ok":
        raise LabelReconciliationError("hosted API health is not confirmed")
    query = urllib.parse.urlencode({"network": network})
    result = request_with_retry(
        request, "GET", f"{api_base_url}/v1/github/bounty-discovery-v1?{query}"
    )
    if result.status != 200 or not isinstance(result.body, dict):
        raise LabelReconciliationError(f"discovery projection returned HTTP {result.status}")
    return result.body


def fetch_json_object(request: HttpRequest, url: str, label: str) -> dict[str, Any]:
    result = request_with_retry(request, "GET", url)
    if result.status != 200 or not isinstance(result.body, dict):
        raise LabelReconciliationError(f"{label} returned HTTP {result.status}")
    return result.body


def opportunity_amount(opportunity: Mapping[str, Any], field: str) -> int:
    amount = opportunity.get(field)
    if not isinstance(amount, dict):
        raise LabelReconciliationError(f"Beta3 opportunity lacks {field}")
    if amount.get("currency") != "USDC" or amount.get("unit") != "base_units":
        raise LabelReconciliationError(f"Beta3 opportunity has invalid {field} units")
    return require_unsigned(amount.get("amount"), f"Beta3 opportunity {field}")


def beta3_settlement_evidence(
    opportunity: Mapping[str, Any],
    competition: Mapping[str, Any],
    events: list[dict[str, Any]],
    safe_block: int,
) -> dict[str, Any] | None:
    state = str(competition.get("state") or "")
    settled = [
        event
        for event in events
        if event.get("kind") == "competition_settled"
        and require_unsigned(event.get("block_number"), "Beta3 event block_number") <= safe_block
    ]
    if state != "settled":
        if settled:
            raise LabelReconciliationError("non-settled Beta3 competition exposes settlement")
        return None
    if len(settled) != 1:
        raise LabelReconciliationError("settled Beta3 competition lacks one canonical settlement")
    event = settled[0]
    data = event.get("data")
    if not isinstance(data, dict):
        raise LabelReconciliationError("Beta3 settlement data is malformed")
    contract = str(competition.get("competition") or "").lower()
    bounty_id = str(competition.get("bounty_id") or "").lower()
    solver = str(data.get("solver") or "").lower()
    keeper = str(data.get("keeper") or "").lower()
    solver_reward = require_unsigned(data.get("solver_reward"), "Beta3 solver_reward")
    keeper_reward = require_unsigned(data.get("keeper_reward"), "Beta3 keeper_reward")
    if (
        str(event.get("contract_address") or "").lower() != contract
        or str(event.get("bounty_id") or "").lower() != bounty_id
        or solver != str(competition.get("winner") or "").lower()
        or solver_reward != opportunity_amount(opportunity, "reward")
        or solver_reward != require_unsigned(competition.get("solver_reward"), "solver_reward")
        or keeper_reward != opportunity_amount(opportunity, "completion_bonus")
        or keeper_reward != require_unsigned(competition.get("keeper_reward"), "keeper_reward")
        or not ADDRESS.fullmatch(solver)
        or not ADDRESS.fullmatch(keeper)
        or not TX_HASH.fullmatch(str(event.get("tx_hash") or "").lower())
    ):
        raise LabelReconciliationError("Beta3 settlement does not match canonical inventory")
    return {
        "event_name": "CompetitionSettledV2",
        "bounty_id": bounty_id,
        "bounty_contract": contract,
        "transaction_hash": str(event["tx_hash"]).lower(),
        "block_number": require_unsigned(event.get("block_number"), "settlement block_number"),
        "log_index": require_unsigned(event.get("log_index"), "settlement log_index"),
        "solver_wallet": solver,
        "solver_reward": str(solver_reward),
        "keeper_wallet": keeper,
        "keeper_reward": str(keeper_reward),
        "confirmed_canonical": True,
    }


def beta3_lifecycle(state: str, verification_ready: bool) -> str:
    if state == "settled":
        return "settled"
    if state == "cancelled":
        return "cancelled"
    if state == "expired":
        return "expired"
    if state == "funding":
        return "funding_needed"
    if state == "active" and verification_ready:
        return "ready_to_earn"
    return "unavailable"


def beta3_discovery_competition_mode(winner_mode: object) -> str:
    modes = {
        "first_proven": "first_valid_submission",
        "best_score": "best_score",
    }
    try:
        return modes[str(winner_mode)]
    except KeyError as error:
        raise LabelReconciliationError(
            "GitHub discovery cannot represent this Beta3 winner mode safely"
        ) from error


def augment_projection_with_beta3(
    request: HttpRequest,
    api_base_url: str,
    network: str,
    repository: str,
    projection: Mapping[str, Any],
) -> dict[str, Any]:
    try:
        return _augment_projection_with_beta3(
            request, api_base_url, network, repository, projection
        )
    except LabelReconciliationError as error:
        return beta3_unavailable_projection(projection, str(error))


def beta3_unavailable_projection(
    projection: Mapping[str, Any], error: str
) -> dict[str, Any]:
    result = dict(projection)
    existing_items = result.get("items")
    existing_sources = result.get("source_statuses")
    if not isinstance(existing_items, list) or not isinstance(existing_sources, list):
        raise LabelReconciliationError("core discovery projection is malformed")
    result["items"] = [
        item
        for item in existing_items
        if isinstance(item, dict) and item.get("protocol_version") != BETA3_PROTOCOL
    ]
    result["source_statuses"] = [
        source
        for source in existing_sources
        if isinstance(source, dict) and source.get("protocol_version") != BETA3_PROTOCOL
    ]
    result["source_statuses"].append(
        {
            "source_type": "open_competition_v2",
            "protocol_version": BETA3_PROTOCOL,
            "factory_contract": None,
            "available": False,
            "fresh": False,
            "item_count": 0,
            "persisted_cursor_block": 0,
            "error": f"beta3_projection_unavailable: {error}",
        }
    )
    result["partial_protocols"] = [BETA3_PROTOCOL]
    return result


def _augment_projection_with_beta3(
    request: HttpRequest,
    api_base_url: str,
    network: str,
    repository: str,
    projection: Mapping[str, Any],
) -> dict[str, Any]:
    query = urllib.parse.urlencode({"network": network})
    inventory = fetch_json_object(
        request,
        f"{api_base_url}/v1/base/open-competition-v2-beta3/inventory?{query}",
        "Beta3 inventory",
    )
    event_feed = fetch_json_object(
        request,
        f"{api_base_url}/v1/base/open-competition-v2-beta3/events?{query}",
        "Beta3 events",
    )
    opportunities = fetch_json_object(
        request,
        f"{api_base_url}/v1/opportunities?{urllib.parse.urlencode({'network': network, 'source_type': 'canonical_base', 'limit': 300})}",
        "opportunity inventory",
    )
    opportunity_sources = opportunities.get("source_statuses")
    canonical_sources = (
        [
            source
            for source in opportunity_sources
            if isinstance(source, dict) and source.get("source_type") == "canonical_base"
        ]
        if isinstance(opportunity_sources, list)
        else []
    )
    if len(canonical_sources) != 1:
        raise LabelReconciliationError("Beta3 opportunity source status is malformed")
    canonical_error = str(canonical_sources[0].get("error") or "")
    canonical_error_codes = set(canonical_error.split("+")) if canonical_error else set()
    known_canonical_errors = {
        "canonical_read_model_unavailable",
        "open_competition_read_model_unavailable",
        "open_competition_v2_read_model_unavailable",
    }
    if not canonical_error_codes.issubset(known_canonical_errors):
        raise LabelReconciliationError("Beta3 opportunity source status is malformed")
    if "open_competition_v2_read_model_unavailable" in canonical_error_codes:
        raise LabelReconciliationError("Beta3 opportunity projection is degraded")
    expected_available = not canonical_error_codes
    if canonical_sources[0].get("available") is not expected_available:
        raise LabelReconciliationError("Beta3 opportunity source status is inconsistent")
    inventory_factory = str(inventory.get("factory_contract") or "").lower()
    event_factory = str(event_feed.get("factory_contract") or "").lower()
    if (
        inventory.get("network") != network
        or inventory.get("protocol_version") != BETA3_PROTOCOL
        or event_feed.get("network") != network
        or event_feed.get("protocol_version") != BETA3_PROTOCOL
        or not ADDRESS.fullmatch(inventory_factory)
        or inventory_factory != event_factory
    ):
        raise LabelReconciliationError("Beta3 canonical views disagree on protocol or network")
    competitions = inventory.get("competitions")
    events = event_feed.get("events")
    opportunity_items = opportunities.get("items")
    if (
        not isinstance(competitions, list)
        or not isinstance(events, list)
        or not isinstance(opportunity_items, list)
        or not all(isinstance(value, dict) for value in competitions + events + opportunity_items)
    ):
        raise LabelReconciliationError("Beta3 canonical views are malformed")
    records: dict[str, tuple[dict[str, Any], int, str]] = {}
    factories: set[str] = set()
    for wrapped in competitions:
        record = wrapped.get("record")
        canonical = record.get("projection") if isinstance(record, dict) else None
        if not isinstance(record, dict) or not isinstance(canonical, dict):
            raise LabelReconciliationError("Beta3 inventory record is malformed")
        contract = str(canonical.get("competition") or "").lower()
        factory = str(record.get("factory_contract") or "").lower()
        safe_number = require_unsigned(record.get("safe_block_number"), "Beta3 safe block")
        safe_hash = str(record.get("safe_block_hash") or "").lower()
        if (
            not ADDRESS.fullmatch(contract)
            or contract in records
            or not ADDRESS.fullmatch(factory)
            or not TX_HASH.fullmatch(safe_hash)
            or record.get("network") != network
        ):
            raise LabelReconciliationError("Beta3 inventory identity is malformed or duplicated")
        records[contract] = (canonical, safe_number, safe_hash)
        factories.add(factory)
    if factories != {inventory_factory}:
        raise LabelReconciliationError("Beta3 inventory does not have one factory")
    selected = []
    chain_id = require_unsigned(projection.get("chain_id"), "projection.chain_id")
    for opportunity in opportunity_items:
        requirements = opportunity.get("evidence_requirements")
        if not isinstance(requirements, dict) or requirements.get("protocol_version") != BETA3_PROTOCOL:
            continue
        source_url = opportunity.get("source_url")
        if (
            parse_same_repository_issue(source_url, repository) is None
            and not is_same_repository_reviewed_beta3_artifact(source_url, repository)
        ):
            continue
        contract = str(opportunity.get("source_id") or "").lower()
        record = records.get(contract)
        if record is None:
            raise LabelReconciliationError("public Beta3 opportunity is absent from canonical inventory")
        canonical, safe_number, _ = record
        bounty_id = str(canonical.get("bounty_id") or "").lower()
        relevant = [
            event
            for event in events
            if str(event.get("contract_address") or "").lower() == contract
            and str(event.get("bounty_id") or "").lower() == bounty_id
            and require_unsigned(event.get("block_number"), "Beta3 event block_number")
            <= safe_number
        ]
        if not relevant:
            raise LabelReconciliationError("public Beta3 opportunity lacks canonical events")
        state = str(canonical.get("state") or "")
        verification_ready = opportunity.get("verification_ready") is True
        lifecycle = beta3_lifecycle(state, verification_ready)
        competition_mode = beta3_discovery_competition_mode(
            opportunity.get("winner_mode") or canonical.get("winner_mode")
        )
        participation_phase = requirements.get("participation_phase")
        scoring_window = requirements.get("scoring_window")
        scoring_formula = requirements.get("scoring_formula")
        qualifying_action = requirements.get("qualifying_action")
        cash_economics = opportunity.get("cash_economics")
        if competition_mode == "best_score":
            if participation_phase not in {"upcoming", "scoring", "proof"}:
                raise LabelReconciliationError("best-score Beta3 participation phase is malformed")
            if not isinstance(scoring_window, dict):
                raise LabelReconciliationError("best-score Beta3 scoring window is missing")
            starts_at = parse_instant(scoring_window.get("starts_at"), "scoring_window.starts_at")
            ends_at = parse_instant(scoring_window.get("ends_at"), "scoring_window.ends_at")
            if starts_at >= ends_at:
                raise LabelReconciliationError("best-score Beta3 scoring window is malformed")
            if not isinstance(scoring_formula, str) or not scoring_formula.strip():
                raise LabelReconciliationError("best-score Beta3 scoring formula is missing")
            if not isinstance(qualifying_action, dict) or not str(
                qualifying_action.get("objective") or ""
            ).strip():
                raise LabelReconciliationError("best-score Beta3 qualifying action is missing")
            if not isinstance(cash_economics, dict):
                raise LabelReconciliationError("best-score Beta3 cash economics are missing")
        target = opportunity_amount(opportunity, "funding_target")
        settlement = beta3_settlement_evidence(opportunity, canonical, relevant, safe_number)
        created_block = min(
            require_unsigned(event.get("block_number"), "Beta3 event block_number")
            for event in relevant
        )
        next_action = opportunity.get("next_action")
        if not isinstance(next_action, dict):
            raise LabelReconciliationError("public Beta3 opportunity lacks a next action")
        next_action_kind = str(next_action.get("action") or "")
        next_action_label = {
            "prepare_open_competition_v2_score": "Prepare scoring work",
            "generate_open_competition_v2_score": "Generate a qualifying score",
            "inspect_open_competition_v2_snapshot": "Inspect the scoring snapshot",
            "quote_open_competition_v2_proof": "Enter competition",
            "inspect_open_competition_v2_settlement": "Inspect settlement",
        }.get(next_action_kind, "Enter competition")
        selected.append(
            {
                "discovery_id": f"eip155:{chain_id}:{BETA3_PROTOCOL}:{contract}",
                "network": network,
                "chain_id": chain_id,
                "protocol_version": BETA3_PROTOCOL,
                "source_id": contract,
                "visibility": "public",
                "bounty_id": bounty_id,
                "bounty_contract": contract,
                "created_at": opportunity.get("created_at"),
                "created_block": created_block,
                "updated_at": opportunity.get("updated_at"),
                "title": opportunity.get("title"),
                "summary": opportunity.get("goal") or "",
                "categories": opportunity.get("categories") or [],
                "skills": opportunity.get("skills") or [],
                "difficulty": None,
                "public_url": opportunity.get("public_url"),
                "source_url": opportunity.get("source_url"),
                "competition_mode": competition_mode,
                "lifecycle_state": lifecycle,
                "funded": lifecycle in {"ready_to_earn", "settled"},
                "verification_ready": verification_ready,
                "ready_to_earn": lifecycle == "ready_to_earn",
                "reward_usdc_base_units": str(opportunity_amount(opportunity, "reward")),
                "verifier_reward_usdc_base_units": str(
                    opportunity_amount(opportunity, "completion_bonus")
                ),
                "bond_usdc_base_units": str(opportunity_amount(opportunity, "bond")),
                "funded_usdc_base_units": str(
                    target
                    if lifecycle == "settled"
                    else opportunity_amount(opportunity, "funded_amount")
                ),
                "funding_target_usdc_base_units": str(target),
                "deadline": opportunity.get("deadline"),
                "deadline_kind": opportunity.get("deadline_kind"),
                "entry_count": require_unsigned(opportunity.get("entry_count"), "entry_count"),
                "max_entries": None,
                "verifier": {
                    "profile_id": opportunity.get("verifier_profile_id"),
                    "display_name": opportunity.get("verifier_profile_name"),
                    "method": opportunity.get("verification_method"),
                    "ready": verification_ready,
                },
                "next_action": {
                    "kind": next_action_kind,
                    "label": "Inspect settlement" if lifecycle == "settled" else next_action_label,
                    "method": next_action.get("method"),
                    "url": next_action.get("url"),
                    "instructions": next_action.get("instructions"),
                },
                "participation_phase": participation_phase,
                "scoring_window": scoring_window,
                "scoring_formula": scoring_formula,
                "qualifying_action": qualifying_action,
                "cash_economics": cash_economics,
                "recovery_action_available": False,
                "identity_warning": "One wallet does not prove one independent person.",
                "settlement_evidence": settlement,
                "evidence_boundary": "Only CompetitionSettledV2 proves solver payment.",
            }
        )
    result = dict(projection)
    existing_items = result.get("items")
    existing_sources = result.get("source_statuses")
    if not isinstance(existing_items, list) or not isinstance(existing_sources, list):
        raise LabelReconciliationError("core discovery projection is malformed")
    result["items"] = [*existing_items, *selected]
    result["source_statuses"] = [
        *existing_sources,
        {
            "source_type": "open_competition_v2",
            "protocol_version": BETA3_PROTOCOL,
            "factory_contract": next(iter(factories)),
            "available": True,
            "fresh": True,
            "item_count": len(selected),
            "persisted_cursor_block": max((safe for _, safe, _ in records.values()), default=0),
            "error": None,
        },
    ]
    result.pop("partial_protocols", None)
    return result


# The claim-comment workflow still consumes the autonomous-v1 full and earning
# feeds for its exclusive-claim handoff. Keep this compatibility reader here so
# both automations share the same strict transport and identity checks while the
# GitHub publisher itself uses only the lifecycle-complete projection above.
def fetch_canonical_feeds(
    request: HttpRequest, api_base_url: str, network: str
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    health = request_with_retry(request, "GET", f"{api_base_url}/health")
    if health.status != 200 or str(health.body).strip() != "ok":
        raise LabelReconciliationError("hosted API health is not confirmed")
    full_query = urllib.parse.urlencode({"network": network})
    earning_query = urllib.parse.urlencode({"network": network, "claimable_only": "true"})
    results: list[list[dict[str, Any]]] = []
    for url in (
        f"{api_base_url}/v1/base/autonomous-bounties/feed?{full_query}",
        f"{api_base_url}/v1/base/autonomous-bounties/feed?{earning_query}",
    ):
        result = request_with_retry(request, "GET", url)
        if result.status != 200 or not isinstance(result.body, list) or not all(
            isinstance(record, dict) for record in result.body
        ):
            raise LabelReconciliationError(f"canonical feed returned HTTP {result.status}")
        results.append(result.body)
    return results[0], results[1]


def require_amount(item: Mapping[str, Any], field: str) -> int:
    try:
        return require_unsigned(item.get(field), field)
    except LabelReconciliationError as error:
        raise LabelReconciliationError(f"canonical item has invalid {field}") from error


def source_issue_url(item: Mapping[str, Any], repository: str) -> str | None:
    terms = item.get("terms")
    document = terms.get("document") if isinstance(terms, dict) else None
    source = document.get("source_url") if isinstance(document, dict) else None
    number = parse_same_repository_issue(source, repository)
    return f"https://github.com/{repository}/issues/{number}" if number else None


def validate_autonomous_state_evidence(
    item: Mapping[str, Any], status: str, contract: str
) -> None:
    expected = {
        "claimed": {"bounty_claimed"},
        "submitted": {"bounty_claimed", "submission_added"},
        "paid": {"bounty_settled"},
    }.get(status)
    if expected is None:
        return
    events = item.get("events")
    if not isinstance(events, list):
        raise LabelReconciliationError(f"canonical {status} item lacks an event list: {contract}")
    observed = {
        str(event.get("kind"))
        for event in events
        if isinstance(event, dict)
        and str(event.get("contract_address") or "").lower() == contract
        and TX_HASH.fullmatch(str(event.get("tx_hash") or "").lower())
    }
    if not expected.issubset(observed):
        raise LabelReconciliationError(f"canonical {status} item lacks confirmed event evidence")


def canonical_records(
    full_feed: list[dict[str, Any]],
    claimable_feed: list[dict[str, Any]],
    repository: str,
) -> tuple[dict[str, dict[str, Any]], set[tuple[str, str]]]:
    by_contract: dict[str, dict[str, Any]] = {}
    candidates: dict[str, list[dict[str, Any]]] = {}
    for item in full_feed:
        contract = str(item.get("bounty_contract") or "").lower()
        status = str(item.get("status") or "").lower()
        if not ADDRESS.fullmatch(contract) or status not in KNOWN_AUTONOMOUS_STATUSES:
            raise LabelReconciliationError("canonical full feed has an invalid contract or status")
        if contract in by_contract:
            raise LabelReconciliationError(f"duplicate canonical contract: {contract}")
        target = require_amount(item, "target_amount")
        funded = require_amount(item, "funded_amount")
        if target <= 0 or funded > target:
            raise LabelReconciliationError(f"invalid canonical economics: {contract}")
        if status in {"claimable", "claimed", "submitted", "paid"} and funded != target:
            raise LabelReconciliationError(f"canonical {status} item is not fully funded: {contract}")
        validate_autonomous_state_evidence(item, status, contract)
        source = source_issue_url(item, repository)
        normalized = dict(item)
        normalized.update(
            {"bounty_contract": contract, "status": status, "_source_issue_url": source}
        )
        by_contract[contract] = normalized
        if source:
            candidates.setdefault(source, []).append(normalized)

    by_issue: dict[str, dict[str, Any]] = {}
    for source, records in candidates.items():
        if len(records) == 1:
            by_issue[source] = records[0]
            continue
        ready = [
            record
            for record in records
            if record["status"] in {"claimable", "claimed", "submitted", "paid"}
            and record.get("terms_valid") is True
            and record.get("verification_ready") is True
        ]
        if len(ready) != 1:
            raise LabelReconciliationError(
                f"multiple canonical contracts reference {source} without one unique ready record"
            )
        by_issue[source] = ready[0]

    earning: set[tuple[str, str]] = set()
    for item in claimable_feed:
        contract = str(item.get("bounty_contract") or "").lower()
        source = source_issue_url(item, repository)
        counterpart = by_contract.get(contract)
        pair = (source or "", contract)
        if counterpart is None or not (
            source == counterpart["_source_issue_url"]
            and counterpart["status"] == "claimable"
            and counterpart.get("terms_valid") is True
            and counterpart.get("verification_ready") is True
            and str(item.get("status") or "").lower() == "claimable"
            and item.get("terms_valid") is True
            and item.get("verification_ready") is True
        ):
            raise LabelReconciliationError(
                f"earning feed item is not an exact executable full-feed record: {contract}"
            )
        if pair in earning:
            raise LabelReconciliationError(f"duplicate earning feed item: {contract}")
        earning.add(pair)
    return by_issue, earning


def next_page(headers: Mapping[str, str]) -> str | None:
    link = next((value for key, value in headers.items() if key.lower() == "link"), "")
    for part in link.split(","):
        match = re.match(r'\s*<([^>]+)>;\s*rel="next"', part)
        if match:
            return match.group(1)
    return None


def fetch_paginated(
    request: HttpRequest,
    url: str,
    token: str | None,
    resource: str,
) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    seen_urls: set[str] = set()
    while url:
        if url in seen_urls:
            raise LabelReconciliationError(f"GitHub {resource} pagination looped")
        seen_urls.add(url)
        result = request_with_retry(request, "GET", url, headers=github_headers(token))
        if result.status != 200 or not isinstance(result.body, list):
            raise LabelReconciliationError(f"GitHub {resource} returned HTTP {result.status}")
        records.extend(record for record in result.body if isinstance(record, dict))
        url = next_page(result.headers)
    return records


def fetch_github_issues(request: HttpRequest, repository: str, token: str | None) -> list[dict[str, Any]]:
    # Listing every issue also recovers a managed issue whose `bounty` label was
    # manually removed, preventing a duplicate mirror on the next run.
    query = urllib.parse.urlencode({"state": "all", "per_page": "100"})
    return fetch_paginated(
        request,
        f"https://api.github.com/repos/{repository}/issues?{query}",
        token,
        "bounty issue listing",
    )


def parse_same_repository_issue(source_url: Any, repository: str) -> int | None:
    if source_url is None:
        return None
    try:
        parsed = urllib.parse.urlsplit(str(source_url))
    except ValueError as error:
        raise LabelReconciliationError("source URL is malformed") from error
    if parsed.scheme != "https" or parsed.hostname != "github.com":
        return None
    if parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise LabelReconciliationError("GitHub source URL must be exact and credential-free")
    match = re.fullmatch(rf"/{re.escape(repository)}/issues/([1-9][0-9]*)/?", parsed.path)
    return int(match.group(1)) if match else None


def is_same_repository_reviewed_beta3_artifact(source_url: Any, repository: str) -> bool:
    """Admit only immutable, reviewed Beta3 source artifacts from this repository."""
    if source_url is None:
        return False
    try:
        parsed = urllib.parse.urlsplit(str(source_url))
    except ValueError as error:
        raise LabelReconciliationError("source URL is malformed") from error
    if parsed.scheme != "https" or parsed.hostname != "github.com":
        return False
    if parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise LabelReconciliationError("GitHub source URL must be exact and credential-free")
    prefix = f"/{repository}/blob/"
    if not parsed.path.startswith(prefix):
        return False
    remainder = parsed.path[len(prefix) :]
    revision, separator, path = remainder.partition("/")
    return bool(
        separator
        and re.fullmatch(r"[0-9a-f]{40}", revision)
        and path in REVIEWED_BETA3_SOURCE_PATHS
    )


def fetch_linked_source_issues(
    request: HttpRequest,
    repository: str,
    token: str | None,
    items: list[dict[str, Any]],
    listed: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    by_number = {
        issue.get("number"): issue
        for issue in listed
        if isinstance(issue.get("number"), int) and "pull_request" not in issue
    }
    for number in sorted(
        {
            number
            for item in items
            if (number := parse_same_repository_issue(item.get("source_url"), repository))
        }
    ):
        if number in by_number:
            continue
        result = request_with_retry(
            request,
            "GET",
            f"https://api.github.com/repos/{repository}/issues/{number}",
            headers=github_headers(token),
        )
        if result.status != 200 or not isinstance(result.body, dict) or "pull_request" in result.body:
            raise LabelReconciliationError(f"linked source issue #{number} is unavailable")
        by_number[number] = result.body
    return list(by_number.values())


def fetch_issue_comments(
    request: HttpRequest, repository: str, issue_number: int, token: str | None
) -> list[dict[str, Any]]:
    query = urllib.parse.urlencode({"per_page": "100"})
    return fetch_paginated(
        request,
        f"https://api.github.com/repos/{repository}/issues/{issue_number}/comments?{query}",
        token,
        f"comments for issue #{issue_number}",
    )


def label_names(issue: Mapping[str, Any]) -> set[str]:
    names: set[str] = set()
    for label in issue.get("labels") or []:
        if isinstance(label, str):
            names.add(label.lower())
        elif isinstance(label, dict) and label.get("name"):
            names.add(str(label["name"]).lower())
    return names


def issue_marker(issue: Mapping[str, Any]) -> str | None:
    body = str(issue.get("body") or "")
    starts = body.count(MANAGED_START)
    ends = body.count(MANAGED_END)
    markers = IDENTITY_MARKER_RE.findall(body)
    if starts != ends or starts > 1 or ends > 1 or len(markers) > 1:
        raise LabelReconciliationError(f"issue #{issue.get('number')} has malformed managed markers")
    if starts == 1 and len(markers) != 1:
        raise LabelReconciliationError(f"issue #{issue.get('number')} lacks one discovery identity")
    if markers and starts != 1:
        raise LabelReconciliationError(f"issue #{issue.get('number')} has an unmanaged discovery identity")
    if not markers:
        return None
    try:
        payload = json.loads(markers[0])
    except json.JSONDecodeError as error:
        raise LabelReconciliationError(f"issue #{issue.get('number')} marker is invalid JSON") from error
    identity = payload.get("discovery_id") if isinstance(payload, dict) else None
    if not isinstance(identity, str) or not DISCOVERY_ID.fullmatch(identity):
        raise LabelReconciliationError(f"issue #{issue.get('number')} marker identity is invalid")
    return identity


def add_attribution(url: str, discovery_id: str) -> str:
    parsed = urllib.parse.urlsplit(url)
    if parsed.scheme != "https" or not parsed.netloc or parsed.username or parsed.password:
        raise LabelReconciliationError(f"public discovery URL is invalid: {discovery_id}")
    query = dict(urllib.parse.parse_qsl(parsed.query, keep_blank_values=True))
    query.update(
        {
            "utm_source": "github",
            "utm_medium": "issue",
            "utm_campaign": "bounty-discovery-v1",
            "discovery_id": discovery_id,
        }
    )
    return urllib.parse.urlunsplit(
        (parsed.scheme, parsed.netloc, parsed.path, urllib.parse.urlencode(query), parsed.fragment)
    )


def require_public_https_url(value: Any, field: str) -> str:
    try:
        parsed = urllib.parse.urlsplit(str(value or ""))
    except ValueError as error:
        raise LabelReconciliationError(f"{field} is malformed") from error
    if (
        parsed.scheme != "https"
        or not parsed.netloc
        or parsed.username
        or parsed.password
        or any(character in str(value) for character in ("\n", "\r"))
    ):
        raise LabelReconciliationError(f"{field} must be a public credential-free HTTPS URL")
    return str(value)


def format_usdc(amount: Any) -> str:
    value = require_unsigned(amount, "USDC amount")
    whole, fraction = divmod(value, 1_000_000)
    decimals = f"{fraction:06d}".rstrip("0")
    return f"{whole}.{decimals.ljust(2, '0')}" if decimals else f"{whole}.00"


def render_managed_block(item: Mapping[str, Any]) -> str:
    identity = str(item["discovery_id"])
    marker = json.dumps({"discovery_id": identity}, separators=(",", ":"), sort_keys=True)
    canonical_public_url = str(item["public_url"])
    public_url = add_attribution(canonical_public_url, identity)
    landing = item.get("_landing")
    action_required = item.get("_landing_action_required")
    next_action = item.get("next_action")
    if not isinstance(next_action, dict) or not isinstance(next_action.get("label"), str):
        raise LabelReconciliationError(f"next action is malformed: {identity}")
    action_url = add_attribution(str(next_action.get("url")), identity)
    competition_mode = str(item["competition_mode"])
    competition_state = competition_mode
    if competition_mode == "first_valid_submission" and item["lifecycle_state"] == "ready_to_earn":
        competition_state = "accepting_entries"
    elif competition_mode == "best_score":
        competition_state = str(item.get("participation_phase") or "best_score")
    lines = [
        MANAGED_START,
        f"<!-- agent-bounties/github-discovery-v1 {marker} -->",
        "## Canonical bounty discovery",
        "",
        str(landing.get("intent_summary") if isinstance(landing, dict) else item["summary"]).strip(),
        "",
        f"- **Current work state:** `{item['lifecycle_state']}`",
        f"- **Current payment state:** `{'paid' if item['lifecycle_state'] == 'settled' else 'escrowed' if item.get('funded') is True else 'unfunded'}`",
        f"- **Current competition state:** `{competition_state}`",
        f"- **Solver reward:** {format_usdc(item['reward_usdc_base_units'])} USDC",
        f"- **Entry/claim bond:** {format_usdc(item['bond_usdc_base_units'])} USDC",
        f"- **Funding:** {format_usdc(item['funded_usdc_base_units'])} / {format_usdc(item['funding_target_usdc_base_units'])} USDC",
    ]
    verifier = item.get("verifier")
    if isinstance(verifier, dict):
        lines.append(
            f"- **Verifier:** {verifier.get('display_name', 'Unspecified')} "
            f"(`{verifier.get('method', 'unknown')}`; ready: `{str(verifier.get('ready') is True).lower()}`)"
        )
    if item.get("max_entries") is not None:
        lines.append(f"- **Capacity:** {item.get('entry_count', 0)} / {item['max_entries']} entries")
    elif item.get("entry_count") is not None:
        lines.append(f"- **Accepted entries:** {item['entry_count']}")
    if item.get("deadline"):
        lines.append(f"- **{str(item.get('deadline_kind') or 'Deadline').replace('_', ' ').title()}:** `{item['deadline']}`")
    if action_required:
        lines.extend(
            [
                "",
                "### Action required before solver invitation",
                "",
                "Canonical state is still shown above, but this issue is intentionally excluded from solver-invitation labels until reviewed landing copy supplies: "
                + ", ".join(f"`{field}`" for field in action_required)
                + ".",
                "",
                f"[Canonical opportunity URL]({canonical_public_url})",
                "",
                "> This publication gate does not change canonical funding or lifecycle state. Do not start, claim, sign, or spend from this incomplete mirror.",
                MANAGED_END,
            ]
        )
        return "\n".join(lines)
    if isinstance(landing, dict):
        lines.extend(
            [
                f"- **Allowlisted skills:** {', '.join(f'`skill:{skill}`' for skill in landing['skills'])}",
                "",
                "### Replayable acceptance criteria",
                "",
                *[f"- {criterion}" for criterion in landing["acceptance_criteria"]],
                "",
                "### One safe start",
                "",
                f"**[{landing['safe_start']['label']}]({add_attribution(str(landing['safe_start']['url']), identity)})** — {landing['safe_start']['instructions']}",
                "",
                f"[Canonical opportunity URL]({canonical_public_url}) · [Open with GitHub attribution]({public_url})",
            ]
        )
    if item["competition_mode"] == "first_valid_submission":
        lines.extend(
            [
                "",
                "### Open Competition rules",
                "",
                "First valid confirmed reveal wins. Each wallet may enter once; an entry does not prove one independent person. Save the local commitment recovery envelope because the API never stores its plaintext salt.",
            ]
        )
    elif item["competition_mode"] == "best_score":
        scoring_window = item.get("scoring_window")
        qualifying_action = item.get("qualifying_action")
        cash_economics = item.get("cash_economics")
        if (
            not isinstance(scoring_window, dict)
            or not isinstance(qualifying_action, dict)
            or not isinstance(cash_economics, dict)
        ):
            raise LabelReconciliationError(f"best-score discovery evidence is missing: {identity}")
        exclusions = qualifying_action.get("excluded")
        if not isinstance(exclusions, list) or not all(
            isinstance(value, str) and value.strip() for value in exclusions
        ):
            raise LabelReconciliationError(f"best-score exclusions are malformed: {identity}")
        lines.extend(
            [
                "",
                "### Best-score competition rules",
                "",
                str(qualifying_action.get("objective") or "").strip(),
                "",
                f"- **Scoring window:** `{scoring_window['starts_at']}` to `{scoring_window['ends_at']}`",
                f"- **Scoring formula:** `{item['scoring_formula']}`",
                f"- **Excluded:** {'; '.join(exclusions)}",
                f"- **Hosted proof and relay cost:** {format_usdc(opportunity_amount(cash_economics, 'required_external_spend'))} USDC",
                "- **Child funding:** user-selected, paid to the child solver after settlement, and still spent if this competition entry loses.",
                "- **What counts:** useful child-bounty demand funded by the entrant wallet and canonically settled to a different eligible solver inside the scoring window.",
                "- **What does not count:** a `/claim` comment, GitHub PR, plan, signature, transaction hash, unfunded draft, self-deal, or settlement outside the scoring window.",
                "- **Entry timing:** accepted entries normally remain at zero during scoring; proof entry starts after the window closes and the exact dual-attested snapshot is available.",
                "- **Commercial value:** define one concrete digital deliverable with binary acceptance tests. Canonical settlement proves counted GMV, not the deliverable's business quality.",
                "- **Decision rule:** use the contract-specific page to calculate win, loss, break-even, and expected cash result before funding.",
            ]
        )
    payment_event = (
        "CompetitionSettledV2"
        if item.get("protocol_version") == BETA3_PROTOCOL
        else "BountySettled"
    )
    lines.extend(
        [
            "",
            "### Next action",
            "",
            f"**[{next_action['label']}]({action_url})** — {next_action.get('instructions', '')}",
            "",
            f"[Open the public bounty page]({public_url})",
        ]
    )
    if item.get("source_url"):
        lines.append(f"[Original source]({item['source_url']})")
    lines.extend(
        [
            "",
            f"> GitHub mirrors canonical public state and cannot fund, claim, verify, settle, or prove payment. Only a confirmed canonical `{payment_event}` receipt below proves solver payment.",
            MANAGED_END,
        ]
    )
    return "\n".join(lines)


def replace_managed_block(body: str, managed: str) -> str:
    starts = body.count(MANAGED_START)
    ends = body.count(MANAGED_END)
    if starts != ends or starts > 1:
        raise LabelReconciliationError("cannot update malformed managed issue body")
    if starts == 0:
        return f"{body.rstrip()}\n\n{managed}\n" if body.strip() else f"{managed}\n"
    start = body.index(MANAGED_START)
    end = body.index(MANAGED_END, start) + len(MANAGED_END)
    return f"{body[:start]}{managed}{body[end:]}"


def has_current_discovery_block(body: str) -> bool:
    """Return whether an existing managed block already uses the current layout."""
    starts = body.count(MANAGED_START)
    ends = body.count(MANAGED_END)
    if starts != ends or starts > 1:
        raise LabelReconciliationError("cannot inspect malformed managed issue body")
    if starts == 0:
        return False
    start = body.index(MANAGED_START)
    end = body.index(MANAGED_END, start) + len(MANAGED_END)
    managed = body[start:end]
    return (
        "- **Current work state:**" in managed
        and "- **Current payment state:**" in managed
        and "- **Current competition state:**" in managed
    )


def trial_claimable_enabled(policy: Mapping[str, Any], generated_at: datetime) -> bool:
    trial = policy["open_competition_compatibility_trial"]
    ends_at = parse_instant(trial["ends_at"], "trial.ends_at")
    return generated_at <= ends_at or trial.get("post_trial_action") == "hold_for_day_30_decision"


def desired_labels(item: Mapping[str, Any], policy: Mapping[str, Any], generated_at: datetime) -> set[str]:
    labels = set(COMMON_LABELS)
    state = str(item["lifecycle_state"])
    mode = str(item["competition_mode"])
    if state == "funding_needed":
        labels.add("funding-needed")
    elif state == "ready_to_earn":
        labels.add("funded-live")
        landing = item.get("_landing")
        if item.get("_landing_action_required") is None:
            labels.update({"ai-agent-welcome", "ready-to-earn", "claimable-live"})
            if isinstance(landing, dict):
                labels.update(f"skill:{skill}" for skill in landing["skills"])
            if mode in {"first_valid_submission", "best_score"}:
                labels.update({"open-competition", "verifier"})
                if not trial_claimable_enabled(policy, generated_at):
                    labels.discard("claimable-live")
    elif state == "in_progress":
        labels.add("funded-live")
        labels.add("claimed-live" if mode == "exclusive_claim" else "in-progress")
        if mode in {"first_valid_submission", "best_score"}:
            labels.add("open-competition")
    elif state == "verification_pending":
        labels.update({"funded-live", "verification-pending"})
        if mode in {"first_valid_submission", "best_score"}:
            labels.add("open-competition")
    elif state == "unavailable":
        if item.get("funded") is True:
            labels.add("funded-live")
        if item.get("verification_ready") is not True:
            labels.add("verification-unavailable")
        else:
            labels.add("in-progress")
        if mode in {"first_valid_submission", "best_score"}:
            labels.add("open-competition")
    elif state in {"cancelled", "expired"}:
        labels.add(state)
        if item.get("recovery_action_available") is True:
            labels.add("refund-available")
    elif state == "settled":
        labels.add("settled-paid")
    difficulty = item.get("difficulty")
    if (
        state == "ready_to_earn"
        and item.get("_landing_action_required") is None
        and isinstance(difficulty, str)
        and difficulty.strip()
    ):
        labels.add("good-first-agent-bounty")
    return labels


def settlement_transaction_url(network: str, tx_hash: str) -> str:
    origins = {"base-mainnet": "https://basescan.org", "base-sepolia": "https://sepolia.basescan.org"}
    try:
        return f"{origins[network]}/tx/{tx_hash}"
    except KeyError as error:
        raise LabelReconciliationError(f"unsupported settlement network: {network}") from error


def build_settlement_receipt(item: Mapping[str, Any]) -> SettlementReceipt:
    evidence = validate_settlement(item)
    tx_hash = str(evidence["transaction_hash"]).lower()
    fingerprint = hashlib.sha256(
        json.dumps(evidence, sort_keys=True, separators=(",", ":")).encode("utf-8")
    ).hexdigest()
    lines = [
        SETTLEMENT_RECEIPT_MARKER,
        "## Canonical payout confirmed",
        "",
        f"- Bounty ID: `{evidence['bounty_id']}`",
        f"- Contract: `{evidence['bounty_contract']}`",
        f"- Settlement: [`{tx_hash}`]({settlement_transaction_url(str(item['network']), tx_hash)})",
        f"- Solver wallet: `{evidence['solver_wallet']}`",
        f"- Solver reward: **{format_usdc(evidence['solver_reward'])} USDC**",
    ]
    if evidence["event_name"] == "CompetitionSettledV2":
        attribution_query = urllib.parse.urlencode(
            {"network": item["network"], "competition_contract": evidence["bounty_contract"]}
        )
        lines.extend(
            [
                f"- Keeper wallet: `{evidence['keeper_wallet']}`",
                f"- Keeper reward: **{format_usdc(evidence['keeper_reward'])} USDC**",
                f"- Hosted proof and relay attribution: [inspect evidence](https://api.agentbounties.app/v1/base/open-competition-v2-beta3/proof-attribution?{attribution_query})",
            ]
        )
    else:
        lines.extend(
            [
                f"- Returned bond: **{format_usdc(evidence['returned_bond'])} USDC**",
                f"- Completion bonus: **{format_usdc(evidence['completion_bonus'])} USDC**",
                f"- Total solver transfer: **{format_usdc(evidence['solver_payout'])} USDC**",
                f"- Verifier reward: **{format_usdc(evidence['verifier_reward'])} USDC**",
            ]
        )
    lines.extend(
        [
            f"- Receipt fingerprint: `{fingerprint}`",
            "",
            f"Only this confirmed canonical `{evidence['event_name']}` event proves solver payment. This GitHub comment reports the event; it did not authorize or execute settlement.",
        ]
    )
    body = "\n".join(lines)
    return SettlementReceipt(fingerprint=fingerprint, body=body)


def created_after_activation(item: Mapping[str, Any], policy: Mapping[str, Any]) -> bool:
    activation = policy["activation"]
    return require_unsigned(item["created_block"], "created_block") >= require_unsigned(
        activation["safe_block"], "activation.safe_block"
    ) and parse_instant(item["created_at"], "created_at") >= parse_instant(
        activation["timestamp"], "activation.timestamp"
    )


def create_allowed(item: Mapping[str, Any], policy: Mapping[str, Any]) -> tuple[bool, str]:
    identity = str(item["discovery_id"])
    if identity in policy["required_backfill_discovery_ids"]:
        return True, "required_backfill"
    state = str(item["lifecycle_state"])
    if state in NONTERMINAL_STATES or item.get("recovery_action_available") is True:
        return True, "current_nonterminal_backfill"
    if created_after_activation(item, policy):
        return True, "post_activation_record"
    return False, "historical_terminal_without_existing_issue"


def mapping_rank(item: Mapping[str, Any]) -> tuple[int, int, str]:
    state = str(item["lifecycle_state"])
    priority = 0 if state == "ready_to_earn" else 1 if state in NONTERMINAL_STATES else 2
    return (priority, -require_unsigned(item["created_block"], "created_block"), str(item["discovery_id"]))


def build_plans(
    projection: Mapping[str, Any],
    issues: list[dict[str, Any]],
    policy: Mapping[str, Any],
    repository: str,
    comments_by_issue: Mapping[int, list[dict[str, Any]]] | None = None,
    landing_entries: Mapping[str, Mapping[str, Any]] | None = None,
) -> list[IssuePlan]:
    items = validate_projection(projection, str(policy["network"]), policy)
    if landing_entries is not None:
        items = attach_landing_copy(items, landing_entries, repository)
    generated_at = parse_instant(projection["generated_at"], "projection.generated_at")
    issue_by_number: dict[int, dict[str, Any]] = {}
    marker_to_issue: dict[str, dict[str, Any]] = {}
    for issue in issues:
        if "pull_request" in issue:
            continue
        number = issue.get("number")
        if not isinstance(number, int) or number <= 0 or number in issue_by_number:
            raise LabelReconciliationError("GitHub issue listing has an invalid or duplicate number")
        issue_by_number[number] = issue
        marker = issue_marker(issue)
        if marker:
            if marker in marker_to_issue:
                raise LabelReconciliationError(f"duplicate discovery_id mapping: {marker}")
            marker_to_issue[marker] = issue

    known_ids = {str(item["discovery_id"]) for item in items}
    for marker in marker_to_issue:
        if marker not in known_ids:
            continue  # Preserve disappeared records exactly as they are.

    source_candidates: dict[int, list[dict[str, Any]]] = {}
    for item in items:
        identity = str(item["discovery_id"])
        if identity in marker_to_issue:
            continue
        number = parse_same_repository_issue(item.get("source_url"), repository)
        if number is not None:
            source_candidates.setdefault(number, []).append(item)
    source_winners = {
        number: sorted(candidates, key=mapping_rank)[0]["discovery_id"]
        for number, candidates in source_candidates.items()
        if number in issue_by_number and issue_marker(issue_by_number[number]) is None
    }

    used_issues: set[int] = set()
    plans: list[IssuePlan] = []
    for item in sorted(items, key=lambda value: str(value["discovery_id"])):
        identity = str(item["discovery_id"])
        issue = marker_to_issue.get(identity)
        mapping_action = "reuse_marker" if issue else "create_mirror"
        if issue is None:
            source_number = parse_same_repository_issue(item.get("source_url"), repository)
            if source_number is not None and source_winners.get(source_number) == identity:
                issue = issue_by_number[source_number]
                mapping_action = "reuse_source"
        if issue is not None:
            number = int(issue["number"])
            if number in used_issues:
                raise LabelReconciliationError(f"ambiguous issue mapping for #{number}")
            used_issues.add(number)
            eligible, reason = True, mapping_action
        else:
            eligible, reason = create_allowed(item, policy)
            if not eligible:
                mapping_action = "excluded_historical_terminal"
            else:
                mapping_action = reason
        if item.get("_landing_action_required"):
            mapping_action = "action_required_landing_copy"
            if issue is None:
                eligible = False

        lifecycle_state = str(item["lifecycle_state"])
        preserve_closed_historical = bool(
            issue
            and issue.get("state") == "closed"
            and lifecycle_state in {"settled", "cancelled", "expired"}
        )
        if preserve_closed_historical:
            eligible = False
            mapping_action = "preserve_closed_historical"

        original_body = str(issue.get("body") or "") if issue else ""
        post_activation_source = bool(
            issue
            and issue_marker(issue) is None
            and created_after_activation(item, policy)
        )
        content_reformat_allowed = bool(
            issue is None
            or lifecycle_state == "ready_to_earn"
            or has_current_discovery_block(original_body)
            or post_activation_source
        )
        title_reformat_allowed = bool(
            issue is None
            or lifecycle_state == "ready_to_earn"
            or post_activation_source
        )
        managed = render_managed_block(item)
        desired_body = (
            replace_managed_block(original_body, managed)
            if eligible and content_reformat_allowed
            else original_body
        )
        desired = desired_labels(item, policy, generated_at) if eligible else set()
        current_all = label_names(issue) if issue else set()
        current_managed = current_all & MANAGED_LABELS
        state = lifecycle_state
        should_close = state == "settled" or (
            state in {"cancelled", "expired"} and item.get("recovery_action_available") is not True
        )
        desired_state = "closed" if should_close else "open"
        desired_reason = "completed" if state == "settled" else "not_planned" if should_close else None
        receipt = build_settlement_receipt(item) if state == "settled" and eligible else None
        issue_number = int(issue["number"]) if issue else None
        comments = list((comments_by_issue or {}).get(issue_number, [])) if issue_number else []
        trusted = [
            comment
            for comment in comments
            if SETTLEMENT_RECEIPT_MARKER in str(comment.get("body") or "")
            and str((comment.get("user") or {}).get("login") or "").lower()
            in {"github-actions[bot]", "nspg13"}
        ]
        if len(trusted) > 1:
            raise LabelReconciliationError(f"issue #{issue_number} has duplicate trusted receipts")
        receipt_action = "none"
        receipt_comment_id = None
        if receipt:
            if not trusted:
                receipt_action = "create"
            else:
                receipt_comment_id = trusted[0].get("id")
                if not isinstance(receipt_comment_id, int):
                    raise LabelReconciliationError(f"issue #{issue_number} receipt lacks an id")
                if str(trusted[0].get("body") or "") != receipt.body:
                    receipt_action = "update"
        created_at = issue.get("created_at") if issue else None
        lag = None
        if created_at:
            lag = max(
                0,
                int((parse_instant(created_at, f"issue #{issue_number}.created_at") - parse_instant(item["created_at"], "created_at")).total_seconds()),
            )
        landing = item.get("_landing")
        desired_title = (
            str(issue.get("title") or item["title"])
            if issue
            and (
                preserve_closed_historical
                or item.get("_landing_action_required")
                or not title_reformat_allowed
            )
            else str(landing["outcome_title"])
            if isinstance(landing, dict)
            else f"[Bounty] {str(item['title']).strip()}"
        )[:256]
        plans.append(
            IssuePlan(
                discovery_id=identity,
                protocol_version=str(item["protocol_version"]),
                lifecycle_state=state,
                competition_mode=str(item["competition_mode"]),
                issue_number=issue_number,
                issue_url=str(issue.get("html_url")) if issue else None,
                mapping_action=mapping_action,
                create_eligible=eligible,
                title=desired_title,
                original_title=str(issue.get("title") or "") if issue else "",
                original_body=original_body,
                desired_body=desired_body,
                current_managed_labels=sorted(current_managed),
                desired_managed_labels=sorted(desired),
                add_labels=sorted(desired - current_managed),
                remove_labels=sorted(current_managed - desired),
                desired_state=desired_state,
                desired_state_reason=desired_reason,
                current_state=str(issue.get("state")) if issue else None,
                current_state_reason=issue.get("state_reason") if issue else None,
                settlement_receipt=receipt,
                receipt_action=receipt_action,
                receipt_comment_id=receipt_comment_id,
                publication_lag_seconds=lag,
            )
        )
    mapped_numbers = [plan.issue_number for plan in plans if plan.issue_number is not None]
    if len(mapped_numbers) != len(set(mapped_numbers)):
        raise LabelReconciliationError("one GitHub issue maps to multiple discovery records")
    return plans


def plan_has_write(plan: IssuePlan) -> bool:
    if not plan.create_eligible:
        return False
    return (
        plan.issue_number is None
        or plan.original_title != plan.title
        or plan.original_body != plan.desired_body
        or bool(plan.add_labels)
        or bool(plan.remove_labels)
        or plan.receipt_action != "none"
        or plan.current_state != plan.desired_state
        or (plan.desired_state == "closed" and plan.current_state_reason != plan.desired_state_reason)
    )


def provision_labels(request: HttpRequest, repository: str, token: str) -> list[str]:
    query = urllib.parse.urlencode({"per_page": "100"})
    current = fetch_paginated(
        request,
        f"https://api.github.com/repos/{repository}/labels?{query}",
        token,
        "label listing",
    )
    existing = {str(label.get("name") or "").lower() for label in current}
    created: list[str] = []
    for name in sorted(MANAGED_LABELS - existing):
        color, description = LABEL_DEFINITIONS[name]
        result = request_with_retry(
            request,
            "POST",
            f"https://api.github.com/repos/{repository}/labels",
            {"name": name, "color": color, "description": description},
            github_headers(token),
        )
        if result.status not in {201, 422}:
            raise LabelReconciliationError(f"failed to provision label {name}: HTTP {result.status}")
        created.append(name)
    return created


def patch_issue_core(
    request: HttpRequest,
    repository: str,
    token: str,
    issue_number: int,
    title: str,
    body: str,
    labels: list[str],
) -> dict[str, Any]:
    result = request_with_retry(
        request,
        "PATCH",
        f"https://api.github.com/repos/{repository}/issues/{issue_number}",
        {"title": title, "body": body, "labels": labels},
        github_headers(token),
    )
    if result.status != 200 or not isinstance(result.body, dict):
        raise LabelReconciliationError(f"issue #{issue_number} update failed: HTTP {result.status}")
    return result.body


def execute_plans(
    plans: list[IssuePlan],
    repository: str,
    token: str,
    request: HttpRequest,
) -> tuple[list[dict[str, Any]], list[str]]:
    provisioned = provision_labels(request, repository, token)
    results: list[dict[str, Any]] = []
    for original_plan in plans:
        if not plan_has_write(original_plan):
            continue
        plan = original_plan
        issue_number = plan.issue_number
        current_all = set(plan.current_managed_labels)
        if issue_number is None:
            result = request_with_retry(
                request,
                "POST",
                f"https://api.github.com/repos/{repository}/issues",
                {
                    "title": plan.title,
                    "body": plan.desired_body,
                    "labels": plan.desired_managed_labels,
                },
                github_headers(token),
            )
            if result.status != 201 or not isinstance(result.body, dict) or not isinstance(result.body.get("number"), int):
                raise LabelReconciliationError(f"issue creation failed for {plan.discovery_id}: HTTP {result.status}")
            issue_number = int(result.body["number"])
            current_all = label_names(result.body)
            plan = replace(
                plan,
                issue_number=issue_number,
                issue_url=str(result.body.get("html_url") or ""),
                current_state=str(result.body.get("state") or "open"),
                current_state_reason=result.body.get("state_reason"),
            )
        else:
            fetched = request_with_retry(
                request,
                "GET",
                f"https://api.github.com/repos/{repository}/issues/{issue_number}",
                headers=github_headers(token),
            )
            if fetched.status != 200 or not isinstance(fetched.body, dict):
                raise LabelReconciliationError(f"issue #{issue_number} refresh failed")
            current_all = label_names(fetched.body)
            desired_all = sorted((current_all - MANAGED_LABELS) | set(plan.desired_managed_labels))
            if (
                str(fetched.body.get("title") or "") != plan.title
                or str(fetched.body.get("body") or "") != plan.desired_body
                or current_all != set(desired_all)
            ):
                patch_issue_core(
                    request,
                    repository,
                    token,
                    issue_number,
                    plan.title,
                    plan.desired_body,
                    desired_all,
                )

        if plan.settlement_receipt and plan.receipt_action == "create":
            result = request_with_retry(
                request,
                "POST",
                f"https://api.github.com/repos/{repository}/issues/{issue_number}/comments",
                {"body": plan.settlement_receipt.body},
                github_headers(token),
            )
            if result.status != 201:
                raise LabelReconciliationError(f"settlement receipt creation failed for #{issue_number}")
        elif plan.settlement_receipt and plan.receipt_action == "update":
            result = request_with_retry(
                request,
                "PATCH",
                f"https://api.github.com/repos/{repository}/issues/comments/{plan.receipt_comment_id}",
                {"body": plan.settlement_receipt.body},
                github_headers(token),
            )
            if result.status != 200:
                raise LabelReconciliationError(f"settlement receipt update failed for #{issue_number}")

        if plan.current_state != plan.desired_state or (
            plan.desired_state == "closed" and plan.current_state_reason != plan.desired_state_reason
        ):
            payload: dict[str, Any] = {"state": plan.desired_state}
            if plan.desired_state_reason:
                payload["state_reason"] = plan.desired_state_reason
            result = request_with_retry(
                request,
                "PATCH",
                f"https://api.github.com/repos/{repository}/issues/{issue_number}",
                payload,
                github_headers(token),
            )
            if result.status != 200:
                raise LabelReconciliationError(f"issue #{issue_number} lifecycle update failed")

        verified = request_with_retry(
            request,
            "GET",
            f"https://api.github.com/repos/{repository}/issues/{issue_number}",
            headers=github_headers(token),
        )
        if verified.status != 200 or not isinstance(verified.body, dict):
            raise LabelReconciliationError(f"issue #{issue_number} verification failed")
        if (
            issue_marker(verified.body) != plan.discovery_id
            or str(verified.body.get("title") or "") != plan.title
            or label_names(verified.body) & MANAGED_LABELS != set(plan.desired_managed_labels)
            or verified.body.get("state") != plan.desired_state
        ):
            raise LabelReconciliationError(f"issue #{issue_number} did not converge")
        results.append(
            {
                "discovery_id": plan.discovery_id,
                "issue_number": issue_number,
                "issue_url": verified.body.get("html_url"),
                "mapping_action": plan.mapping_action,
                "receipt_action": plan.receipt_action,
                "state": plan.desired_state,
            }
        )
    return results, provisioned


def load_fixture(path: Path) -> tuple[dict[str, Any], list[dict[str, Any]], dict[int, list[dict[str, Any]]]]:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise LabelReconciliationError(f"cannot load fixture: {error}") from error
    if not isinstance(payload, dict) or not isinstance(payload.get("projection"), dict) or not isinstance(payload.get("issues"), list):
        raise LabelReconciliationError("fixture must contain projection and issues")
    comments = {
        int(number): value
        for number, value in (payload.get("comments_by_issue") or {}).items()
        if isinstance(value, list)
    }
    return payload["projection"], payload["issues"], comments


def percentile_95(values: list[int]) -> int | None:
    if not values:
        return None
    ordered = sorted(values)
    return ordered[max(0, (95 * len(ordered) + 99) // 100 - 1)]


def render_markdown(report: Mapping[str, Any]) -> str:
    lines = [
        "# GitHub bounty discovery reconciliation",
        "",
        f"- Mode: `{report['mode']}`",
        f"- Projection records: `{report['projection_record_count']}`",
        f"- Covered records: `{report['covered_record_count']}`",
        f"- Coverage: `{report['coverage_percent']:.2f}%`",
        f"- Duplicate mappings: `{report['duplicate_mapping_count']}`",
        f"- Planned or executed writes: `{report['write_count']}`",
        f"- Publication lag P95: `{report['publication_lag_p95_seconds']}` seconds",
        "",
        "Only canonical `BountySettled` evidence permits `settled-paid` and completed closure.",
    ]
    return "\n".join(lines) + "\n"


def write_report(report: Mapping[str, Any], json_out: Path | None, md_out: Path | None) -> None:
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if json_out:
        json_out.parent.mkdir(parents=True, exist_ok=True)
        json_out.write_text(rendered, encoding="utf-8")
    else:
        print(rendered, end="")
    if md_out:
        md_out.parent.mkdir(parents=True, exist_ok=True)
        md_out.write_text(render_markdown(report), encoding="utf-8")


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--api-base-url", default="https://api.agentbounties.app")
    parser.add_argument("--network", default="base-mainnet")
    parser.add_argument("--repository", default="NSPG13/agent-bounties")
    parser.add_argument("--policy", type=Path, default=Path("ops/github-bounty-discovery-policy.json"))
    parser.add_argument(
        "--landing-copy",
        type=Path,
        default=Path("ops/github-bounty-landing-copy.json"),
    )
    parser.add_argument("--fixture", type=Path)
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--confirm-repository")
    parser.add_argument("--json-out", type=Path)
    parser.add_argument("--md-out", type=Path)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None, request: HttpRequest = default_http_request) -> int:
    args = parse_args(argv)
    repository = validate_repository(args.repository)
    api_base_url = normalize_api_base_url(args.api_base_url)
    policy = load_policy(args.policy, repository, args.network)
    landing_entries = load_landing_copy(args.landing_copy, repository)
    token = os.getenv("GITHUB_TOKEN") or os.getenv("GH_TOKEN") or ""
    if args.execute:
        if args.fixture:
            raise LabelReconciliationError("fixture mode cannot execute GitHub writes")
        if not token:
            raise LabelReconciliationError("GITHUB_TOKEN or GH_TOKEN is required for --execute")
        if args.confirm_repository != repository:
            raise LabelReconciliationError("--confirm-repository must exactly match --repository")

    if args.fixture:
        projection, issues, comments_by_issue = load_fixture(args.fixture)
    else:
        projection = fetch_projection(request, api_base_url, args.network)
        projection = augment_projection_with_beta3(
            request, api_base_url, args.network, repository, projection
        )
        items = validate_projection(projection, args.network, policy)
        issues = fetch_github_issues(request, repository, token or None)
        issues = fetch_linked_source_issues(request, repository, token or None, items, issues)
        comments_by_issue = {}

    enforced_landing_entries = None if args.fixture else landing_entries
    plans = build_plans(
        projection,
        issues,
        policy,
        repository,
        comments_by_issue,
        enforced_landing_entries,
    )
    if not args.fixture:
        for plan in plans:
            if plan.lifecycle_state == "settled" and plan.issue_number is not None:
                comments_by_issue[plan.issue_number] = fetch_issue_comments(
                    request, repository, plan.issue_number, token or None
                )
        plans = build_plans(
            projection,
            issues,
            policy,
            repository,
            comments_by_issue,
            enforced_landing_entries,
        )
    eligible = [plan for plan in plans if plan.create_eligible]
    excluded = [plan for plan in plans if not plan.create_eligible]
    writes = [plan for plan in plans if plan_has_write(plan)]
    execution_results: list[dict[str, Any]] = []
    provisioned_labels: list[str] = []
    if args.execute:
        execution_results, provisioned_labels = execute_plans(plans, repository, token, request)
    lags = [plan.publication_lag_seconds for plan in plans if plan.publication_lag_seconds is not None]
    report = {
        "schema_version": "agent-bounties/github-bounty-reconciliation-report-v1",
        "mode": "execute" if args.execute else "dry-run",
        "repository": repository,
        "network": args.network,
        "api_base_url": api_base_url,
        "projection_schema_version": projection.get("schema_version"),
        "projection_generated_at": projection.get("generated_at"),
        "projection_safe_block": projection.get("safe_block"),
        "partial_protocols": projection.get("partial_protocols", []),
        "source_statuses": projection.get("source_statuses", []),
        "projection_record_count": len(plans),
        "mapped_issue_count_before_reconciliation": sum(
            plan.issue_number is not None for plan in eligible
        ),
        "planned_create_count": sum(plan.issue_number is None for plan in eligible),
        "covered_record_count": len(eligible),
        "covered_record_count_after_successful_reconciliation": len(eligible),
        "excluded_historical_terminal_count": len(excluded),
        "coverage_percent": 100.0,
        "duplicate_mapping_count": 0,
        "write_count": len(writes),
        "publication_lag_p95_seconds": percentile_95([int(value) for value in lags]),
        "publication_lag_target_seconds": int(policy.get("publication_lag_target_minutes_p95", 10)) * 60,
        "provisioned_labels": provisioned_labels,
        "settlement_authority": False,
        "plans": [asdict(plan) for plan in plans],
        "execution_results": execution_results,
        "boundaries": list(BOUNDARIES),
    }
    write_report(report, args.json_out, args.md_out)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except LabelReconciliationError as error:
        print(f"GitHub bounty discovery reconciliation blocked: {error}", file=sys.stderr)
        raise SystemExit(2) from error
