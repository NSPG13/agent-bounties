#!/usr/bin/env python3
"""Report candidate bounty supply and verified claimable earning inventory.

The threshold applies only to fail-closed output from the portable skill's
canonical inventory verifier. GitHub issue labels remain candidate telemetry.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

NON_ACTIONABLE_LABELS = frozenset(
    {
        "duplicate",
        "invalid",
        "wontfix",
        "won't fix",
        "not-actionable",
        "not actionable",
        "spam",
        "closed",
        "activation-blocked",
    }
)

ADDRESS = re.compile(r"^0x[0-9a-fA-F]{40}$")
BYTES32 = re.compile(r"^0x[0-9a-fA-F]{64}$")
CLAIMABLE_EVIDENCE = "confirmed_canonical_autonomous_bounty"
MAX_REPORT_AGE_SECONDS = 900
STANDING_META_SCHEMA = "agent-bounties/standing-meta-bounty-v2"
STANDING_META_CLASS = "post_bounty_third_party_completion"
STANDING_META_PROTOCOL = "agent-bounties/independent-child-v2"
STANDING_META_VERIFIER = "0xe573cb4f471d38b5bf10ce82237251ac902c9867"
STANDING_META_VERIFIER_CODE_HASH = (
    "0xe3b6e82880edee69b1f30560506ac80a46b4ebcc6c083cfa8207e3673eede26c"
)
STANDING_META_ACCEPTANCE_HASH = (
    "0x25c41d7d51e2c807754b901733de17cdb1778dbd353f86347ff33e10289fcb54"
)


@dataclass
class InventoryReport:
    repository: str
    threshold: int
    meta_threshold: int
    meta_replenishment_target: int
    open_bounty_count: int
    verified_claimable_count: int
    verified_meta_claimable_count: int
    missing_count: int
    meta_missing_count: int
    meta_replenishment_count: int
    below_threshold: bool
    meta_below_threshold: bool
    meta_replenishment_required: bool
    issue_urls: list[str]
    claimable_bounty_ids: list[str]
    meta_claimable_bounty_ids: list[str]
    excluded_count: int
    protocol_status: str
    inventory_evidence_valid: bool
    suggested_next_action: str
    disclaimer: str

    def to_markdown(self) -> str:
        status = "BELOW THRESHOLD" if self.below_threshold else "OK"
        lines = [
            f"# Bounty inventory guard - {status}",
            "",
            f"- Repository: `{self.repository}`",
            f"- Open actionable `bounty` issues (candidate supply): **{self.open_bounty_count}**",
            f"- Verified canonical claimable bounties: **{self.verified_claimable_count}**",
            f"- Claimable threshold: **{self.threshold}**",
            f"- Missing claimable bounties: **{self.missing_count}**",
            f"- Verified standing meta-bounties: **{self.verified_meta_claimable_count}**",
            f"- Standing meta-bounty floor: **{self.meta_threshold}**",
            f"- Standing meta-bounty replenishment target: **{self.meta_replenishment_target}**",
            f"- Missing from hard meta floor: **{self.meta_missing_count}**",
            f"- Missing from meta replenishment target: **{self.meta_replenishment_count}**",
            f"- Excluded (non-actionable labels): **{self.excluded_count}**",
            f"- Protocol status: `{self.protocol_status}`",
            f"- Inventory evidence valid: **{str(self.inventory_evidence_valid).lower()}**",
            "",
            "## Issue URLs",
        ]
        if self.issue_urls:
            lines.extend(f"- {url}" for url in self.issue_urls)
        else:
            lines.append("- _(none)_")
        lines.extend(["", "## Verified claimable bounty IDs"])
        if self.claimable_bounty_ids:
            lines.extend(f"- `{bounty_id}`" for bounty_id in self.claimable_bounty_ids)
        else:
            lines.append("- _(none)_")
        lines.extend(["", "## Verified standing meta-bounty IDs"])
        if self.meta_claimable_bounty_ids:
            lines.extend(f"- `{bounty_id}`" for bounty_id in self.meta_claimable_bounty_ids)
        else:
            lines.append("- _(none)_")
        lines.extend(
            [
                "",
                "## Suggested next action",
                "",
                self.suggested_next_action,
                "",
                "## Disclaimer",
                "",
                self.disclaimer,
                "",
            ]
        )
        return "\n".join(lines)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument(
        "--repository",
        default=os.environ.get("GITHUB_REPOSITORY", "NSPG13/agent-bounties"),
        help="owner/repo (default: GITHUB_REPOSITORY or NSPG13/agent-bounties)",
    )
    p.add_argument(
        "--threshold",
        type=int,
        default=int(os.environ.get("BOUNTY_INVENTORY_THRESHOLD", "5")),
        help="Minimum verified canonical claimable bounties (default 5)",
    )
    p.add_argument(
        "--meta-threshold",
        type=int,
        default=int(os.environ.get("META_BOUNTY_INVENTORY_THRESHOLD", "1")),
        help="Hard floor for funded, claimable standing meta-bounties (default 1)",
    )
    p.add_argument(
        "--meta-replenishment-target",
        type=int,
        default=int(os.environ.get("META_BOUNTY_REPLENISHMENT_TARGET", "2")),
        help="Target that creates a buffer above the standing meta-bounty floor (default 2)",
    )
    p.add_argument(
        "--claimable-report",
        type=Path,
        default=None,
        help="JSON output from skills/agent-bounties/scripts/check-in.mjs",
    )
    p.add_argument(
        "--fixture",
        type=Path,
        default=None,
        help="Optional JSON file of issue objects (offline / tests)",
    )
    p.add_argument(
        "--json-out",
        type=Path,
        default=None,
        help="Write JSON report to this path",
    )
    p.add_argument(
        "--md-out",
        type=Path,
        default=None,
        help="Write Markdown report to this path",
    )
    p.add_argument(
        "--fail-below",
        action="store_true",
        help="Exit with code 2 when count is below threshold",
    )
    return p.parse_args(argv)


def label_names(issue: dict[str, Any]) -> set[str]:
    labels = issue.get("labels") or []
    names: set[str] = set()
    for lab in labels:
        if isinstance(lab, str):
            names.add(lab.lower())
        elif isinstance(lab, dict) and lab.get("name"):
            names.add(str(lab["name"]).lower())
    return names


def is_actionable_bounty(issue: dict[str, Any]) -> bool:
    if issue.get("state") and str(issue["state"]).lower() != "open":
        return False
    if issue.get("pull_request") is not None:
        return False
    names = label_names(issue)
    if "bounty" not in names:
        return False
    if names & NON_ACTIONABLE_LABELS:
        return False
    return True


def issue_url(issue: dict[str, Any], repository: str) -> str:
    if issue.get("html_url"):
        return str(issue["html_url"])
    number = issue.get("number")
    return f"https://github.com/{repository}/issues/{number}"


def _credential_free_https(value: object) -> bool:
    try:
        url = urllib.parse.urlparse(str(value))
        return (
            url.scheme == "https"
            and bool(url.hostname)
            and url.username is None
            and url.password is None
        )
    except ValueError:
        return False


def verified_claimable_entries(report: object) -> tuple[list[dict[str, Any]], bool, str]:
    if not isinstance(report, dict):
        return [], False, "unavailable"
    protocol_status = str(report.get("protocol_status") or "unavailable")
    try:
        observed_at = datetime.fromisoformat(
            str(report.get("observed_at") or "").replace("Z", "+00:00")
        )
        if observed_at.tzinfo is None:
            return [], False, protocol_status
        age_seconds = (datetime.now(timezone.utc) - observed_at).total_seconds()
    except ValueError:
        return [], False, protocol_status
    if age_seconds < -60 or age_seconds > MAX_REPORT_AGE_SECONDS:
        return [], False, protocol_status
    raw_warnings = report.get("warnings") or []
    if not isinstance(raw_warnings, list) or not all(
        isinstance(item, str) for item in raw_warnings
    ):
        return [], False, protocol_status
    warnings = set(raw_warnings)
    source = report.get("protocol_source")
    hosted_source = source == "hosted_indexed_feed"
    direct_source = source == "direct_safe_chain"
    common_invalid = (
        protocol_status != "active"
        or not ADDRESS.fullmatch(str(report.get("active_factory") or ""))
        or "autonomous_protocol_not_active" in warnings
    )
    hosted_invalid = hosted_source and (
        report.get("hosted_api_healthy") is not True
        or bool(
            warnings
            & {"hosted_api_health_not_confirmed", "autonomous_feed_unavailable"}
        )
    )
    direct_block = report.get("direct_chain_observed_block")
    direct_status = report.get("direct_chain_status")
    direct_invalid = direct_source and (
        direct_status not in {"verified", "no_claimable_bounties"}
        or not isinstance(direct_block, dict)
        or direct_block.get("tag") != "safe"
        or not isinstance(direct_block.get("number"), int)
        or isinstance(direct_block.get("number"), bool)
        or direct_block.get("number") <= 0
        or not BYTES32.fullmatch(str(direct_block.get("hash") or ""))
        or "direct_safe_chain_verification_failed" in warnings
    )
    if common_invalid or hosted_invalid or direct_invalid or not (
        hosted_source or direct_source
    ):
        return [], False, protocol_status

    items = report.get("verified_claimable_bounties")
    if not isinstance(items, list):
        return [], False, protocol_status
    if direct_source and (
        (direct_status == "verified" and not items)
        or (direct_status == "no_claimable_bounties" and items)
    ):
        return [], False, protocol_status
    ids: set[str] = set()
    contracts: set[str] = set()
    verified: list[dict[str, Any]] = []
    for item in items:
        if not isinstance(item, dict):
            return [], False, protocol_status
        bounty_id = str(item.get("id") or "").lower()
        contract = str(item.get("contract") or "").lower()
        solver_reward = item.get("solver_reward_minor")
        claim_bond = item.get("claim_bond_minor")
        common_valid = (
            BYTES32.fullmatch(bounty_id)
            and ADDRESS.fullmatch(contract)
            and bounty_id not in ids
            and contract not in contracts
            and item.get("status") == "claimable"
            and item.get("evidence") == CLAIMABLE_EVIDENCE
            and item.get("currency") == "usdc"
            and isinstance(solver_reward, int)
            and not isinstance(solver_reward, bool)
            and solver_reward > 0
            and isinstance(claim_bond, int)
            and not isinstance(claim_bond, bool)
            and claim_bond > 0
            and _credential_free_https(item.get("terms_url"))
        )
        hosted_valid = hosted_source and _credential_free_https(
            item.get("claim_plan_url")
        )
        direct_valid = direct_source and (
            item.get("evidence_source") == "direct_safe_chain"
            and item.get("observed_block_number") == direct_block["number"]
            and item.get("observed_block_hash") == direct_block["hash"]
            and ADDRESS.fullmatch(str(item.get("claim_contract") or ""))
            and item.get("claim_contract", "").lower() == contract
            and isinstance(item.get("claim_plan"), dict)
            and _credential_free_https(item.get("source_url"))
        )
        valid = common_valid and (hosted_valid or direct_valid)
        if not valid:
            return [], False, protocol_status
        ids.add(bounty_id)
        contracts.add(contract)
        verified.append(item)
    return verified, True, protocol_status


def standing_meta_entries(
    claimable: list[dict[str, Any]],
    *,
    direct_block: object = None,
) -> tuple[list[dict[str, Any]], bool]:
    """Return exact-code canonical child-loop inventory; reject malformed claims."""
    direct = direct_block if isinstance(direct_block, dict) else None
    meta: list[dict[str, Any]] = []
    for item in claimable:
        descriptor = item.get("standing_meta_bounty")
        if descriptor is None:
            continue
        if not isinstance(descriptor, dict):
            return [], False
        block_number = descriptor.get("observed_block_number")
        block_hash = str(descriptor.get("observed_block_hash") or "").lower()
        valid = (
            descriptor.get("schema_version") == STANDING_META_SCHEMA
            and descriptor.get("inventory_class") == STANDING_META_CLASS
            and descriptor.get("verifier_protocol") == STANDING_META_PROTOCOL
            and str(descriptor.get("verifier_module") or "").lower()
            == STANDING_META_VERIFIER
            and str(descriptor.get("verifier_runtime_code_hash") or "").lower()
            == STANDING_META_VERIFIER_CODE_HASH
            and str(descriptor.get("acceptance_criteria_hash") or "").lower()
            == STANDING_META_ACCEPTANCE_HASH
            and descriptor.get("requires_funded_canonical_child") is True
            and descriptor.get("requires_different_solver_wallet") is True
            and descriptor.get("required_child_status") == "settled"
            and item.get("verification_mode") == "deterministic_module"
            and str(item.get("verifier_module") or "").lower()
            == STANDING_META_VERIFIER
            and item.get("verification_ready") is True
            and isinstance(block_number, int)
            and not isinstance(block_number, bool)
            and block_number > 0
            and BYTES32.fullmatch(block_hash)
        )
        if direct is not None:
            valid = valid and (
                block_number == direct.get("number")
                and block_hash == str(direct.get("hash") or "").lower()
            )
        if not valid:
            return [], False
        meta.append(item)
    return meta, True


def fetch_open_issues(repository: str, token: str | None) -> list[dict[str, Any]]:
    """Paginate GitHub REST issues with label bounty, state open."""
    owner, _, repo = repository.partition("/")
    if not owner or not repo:
        raise SystemExit(f"invalid repository: {repository!r}")

    headers = {
        "Accept": "application/vnd.github+json",
        "User-Agent": "agent-bounties-inventory-guard",
    }
    if token:
        headers["Authorization"] = f"Bearer {token}"

    issues: list[dict[str, Any]] = []
    page = 1
    while page <= 20:
        qs = urllib.parse.urlencode(
            {
                "state": "open",
                "labels": "bounty",
                "per_page": "100",
                "page": str(page),
            }
        )
        url = f"https://api.github.com/repos/{owner}/{repo}/issues?{qs}"
        req = urllib.request.Request(url, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=30) as resp:
                batch = json.loads(resp.read().decode("utf-8"))
        except urllib.error.HTTPError as exc:
            body = exc.read().decode("utf-8", errors="replace")
            raise SystemExit(f"GitHub API error {exc.code}: {body[:500]}") from exc
        if not isinstance(batch, list) or not batch:
            break
        issues.extend(batch)
        if len(batch) < 100:
            break
        page += 1
    return issues


def build_report(
    repository: str,
    threshold: int,
    meta_threshold: int,
    meta_replenishment_target: int,
    issues: list[dict[str, Any]],
    claimable_report: object = None,
) -> InventoryReport:
    actionable: list[dict[str, Any]] = []
    excluded = 0
    for issue in issues:
        if is_actionable_bounty(issue):
            actionable.append(issue)
        else:
            # Only count exclusions among items that had bounty-ish intent
            if "bounty" in label_names(issue) or issue.get("state") == "open":
                excluded += 1

    urls = [issue_url(i, repository) for i in actionable]
    issue_count = len(actionable)
    claimable, evidence_valid, protocol_status = verified_claimable_entries(claimable_report)
    direct_block = (
        claimable_report.get("direct_chain_observed_block")
        if isinstance(claimable_report, dict)
        and claimable_report.get("protocol_source") == "direct_safe_chain"
        else None
    )
    meta_claimable, meta_evidence_valid = standing_meta_entries(
        claimable, direct_block=direct_block
    )
    evidence_valid = evidence_valid and meta_evidence_valid
    claimable_count = len(claimable)
    meta_claimable_count = len(meta_claimable)
    missing = max(0, threshold - claimable_count)
    meta_missing = max(0, meta_threshold - meta_claimable_count)
    meta_replenishment = max(0, meta_replenishment_target - meta_claimable_count)
    meta_below = not evidence_valid or meta_claimable_count < meta_threshold
    below = not evidence_valid or claimable_count < threshold or meta_below
    if not evidence_valid:
        action = (
            "Restore a fresh, active protocol and canonical inventory feed before "
            "counting liquidity. Candidate issues cannot substitute for missing or "
            "invalid on-chain evidence."
        )
    elif meta_below:
        action = (
            f"Activate, fully fund, and canonically index at least {meta_missing} "
            "standing meta-bounty contract(s). Each must use the exact canonical "
            "child-loop verifier and pay only after a different wallet completes "
            "the solver-created child bounty and receives canonical settlement."
        )
    elif claimable_count < threshold:
        action = (
            f"Activate, fund, and canonically index at least {missing} more claimable "
            f"bounty contract(s). Open GitHub issues are candidate supply and do not "
            f"satisfy this liquidity threshold."
        )
    elif meta_replenishment:
        action = (
            f"The hard standing meta-bounty floor is satisfied. Replenish "
            f"{meta_replenishment} additional standing meta-bounty contract(s) to "
            "restore the buffer before the live one is claimed."
        )
    else:
        action = (
            "Verified canonical claimable inventory is at or above threshold. Keep "
            "monitoring funding, claims, deadlines, and settlement events."
        )
    disclaimer = (
        "The GitHub candidate issue count does not imply funding or claimability. "
        "Claimable inventory requires an active canonical factory plus matching "
        "terms, economics, funding, and canonical indexed events or exact safe-block "
        "state. Only a confirmed canonical "
        "BountySettled event proves payout."
    )
    return InventoryReport(
        repository=repository,
        threshold=threshold,
        meta_threshold=meta_threshold,
        meta_replenishment_target=meta_replenishment_target,
        open_bounty_count=issue_count,
        verified_claimable_count=claimable_count,
        verified_meta_claimable_count=meta_claimable_count,
        missing_count=missing,
        meta_missing_count=meta_missing,
        meta_replenishment_count=meta_replenishment,
        below_threshold=below,
        meta_below_threshold=meta_below,
        meta_replenishment_required=meta_replenishment > 0,
        issue_urls=urls,
        claimable_bounty_ids=[str(item["id"]) for item in claimable],
        meta_claimable_bounty_ids=[str(item["id"]) for item in meta_claimable],
        excluded_count=excluded,
        protocol_status=protocol_status,
        inventory_evidence_valid=evidence_valid,
        suggested_next_action=action,
        disclaimer=disclaimer,
    )


def load_fixture(path: Path) -> list[dict[str, Any]]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if isinstance(data, dict) and "issues" in data:
        data = data["issues"]
    if not isinstance(data, list):
        raise SystemExit("fixture must be a JSON list of issues or {issues: [...]}")
    return data


def load_claimable_report(path: Path | None) -> object:
    if path is None:
        return None
    return json.loads(path.read_text(encoding="utf-8-sig"))


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if args.threshold < 0:
        raise SystemExit("threshold must be >= 0")
    if args.meta_threshold < 0:
        raise SystemExit("meta-threshold must be >= 0")
    if args.meta_replenishment_target < args.meta_threshold:
        raise SystemExit("meta-replenishment-target must be >= meta-threshold")

    if args.fixture:
        issues = load_fixture(args.fixture)
    else:
        token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
        issues = fetch_open_issues(args.repository, token)

    report = build_report(
        args.repository,
        args.threshold,
        args.meta_threshold,
        args.meta_replenishment_target,
        issues,
        load_claimable_report(args.claimable_report),
    )
    payload = asdict(report)
    md = report.to_markdown()

    print(md)
    print("--- JSON ---")
    print(json.dumps(payload, indent=2))

    if args.json_out:
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    if args.md_out:
        args.md_out.parent.mkdir(parents=True, exist_ok=True)
        args.md_out.write_text(md, encoding="utf-8")

    if args.fail_below and report.below_threshold:
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
