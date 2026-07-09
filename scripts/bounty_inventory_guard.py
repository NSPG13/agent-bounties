#!/usr/bin/env python3
"""Count open public GitHub issues labeled `bounty` and report inventory health.

Does not claim any issue is funded, claimable, accepted, payable, or paid.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import asdict, dataclass
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
    }
)


@dataclass
class InventoryReport:
    repository: str
    threshold: int
    open_bounty_count: int
    missing_count: int
    below_threshold: bool
    issue_urls: list[str]
    excluded_count: int
    suggested_next_action: str
    disclaimer: str

    def to_markdown(self) -> str:
        status = "BELOW THRESHOLD" if self.below_threshold else "OK"
        lines = [
            f"# Bounty inventory guard — {status}",
            "",
            f"- Repository: `{self.repository}`",
            f"- Open actionable `bounty` issues: **{self.open_bounty_count}**",
            f"- Threshold: **{self.threshold}**",
            f"- Missing to threshold: **{self.missing_count}**",
            f"- Excluded (non-actionable labels): **{self.excluded_count}**",
            "",
            "## Issue URLs",
        ]
        if self.issue_urls:
            lines.extend(f"- {url}" for url in self.issue_urls)
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
        help="Minimum open actionable bounty issues (default 5)",
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
    issues: list[dict[str, Any]],
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
    count = len(actionable)
    missing = max(0, threshold - count)
    below = count < threshold
    if below:
        action = (
            f"Open or fund at least {missing} more public actionable bounty issue(s) "
            f"(label `bounty`, keep open, avoid non-actionable labels) so organic "
            f"solvers have inventory. This report does not mark any issue as funded."
        )
    else:
        action = (
            "Inventory is at or above threshold. Keep monitoring; this report does "
            "not imply any listed issue is funded or claimable."
        )
    disclaimer = (
        "Counts open GitHub issues labeled `bounty` only. This report does not "
        "imply any bounty is funded, claimable, accepted, payable, or paid unless "
        "verified platform payment evidence exists separately."
    )
    return InventoryReport(
        repository=repository,
        threshold=threshold,
        open_bounty_count=count,
        missing_count=missing,
        below_threshold=below,
        issue_urls=urls,
        excluded_count=excluded,
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


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if args.threshold < 0:
        raise SystemExit("threshold must be >= 0")

    if args.fixture:
        issues = load_fixture(args.fixture)
    else:
        token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
        issues = fetch_open_issues(args.repository, token)

    report = build_report(args.repository, args.threshold, issues)
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
