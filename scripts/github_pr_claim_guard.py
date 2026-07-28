#!/usr/bin/env python3
"""Guide PR-first contributors into the canonical funded-bounty claim flow."""

from __future__ import annotations

import json
import os
import pathlib
import re
import subprocess
import sys
import urllib.request
from collections.abc import Mapping

from _shared.github_actions import find_executable, publish_issue_comment


MARKER = "<!-- agent-bounties-pr-claim-guard -->"
LINKED_ISSUE_RE = re.compile(
    r"(?i)\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s+#([1-9][0-9]{0,8})\b"
)
CONTRACT_RE = re.compile(r"Contract:\s*`(0x[0-9a-fA-F]{40})`")
API_FEED = (
    "https://api.agentbounties.app/v1/base/autonomous-bounties/feed"
    "?network=base-mainnet&claimable_only=false"
)


class UserError(RuntimeError):
    pass


def linked_issue_numbers(body: str) -> list[int]:
    return list(dict.fromkeys(int(value) for value in LINKED_ISSUE_RE.findall(body)))[:5]


def label_names(issue: Mapping[str, object]) -> set[str]:
    labels = issue.get("labels")
    if not isinstance(labels, list):
        return set()
    return {
        str(label.get("name") or "").strip().lower()
        for label in labels
        if isinstance(label, dict)
    }


def claimable_links(
    issue_numbers: list[int],
    issues: Mapping[int, Mapping[str, object]],
    inventory: list[Mapping[str, object]],
) -> list[dict[str, object]]:
    by_contract = {
        str(item.get("bounty_contract") or "").lower(): item
        for item in inventory
        if isinstance(item, Mapping)
    }
    result = []
    for number in issue_numbers:
        issue = issues.get(number)
        if not issue or "funded-live" not in label_names(issue):
            continue
        match = CONTRACT_RE.search(str(issue.get("body") or ""))
        if not match:
            continue
        contract = match.group(1).lower()
        item = by_contract.get(contract)
        if not item:
            continue
        if (
            item.get("status") == "claimable"
            and item.get("terms_valid") is True
            and item.get("verification_ready") is True
        ):
            result.append(
                {
                    "issue": number,
                    "contract": contract,
                    "url": str(issue.get("html_url") or ""),
                }
            )
    return result


def render_comment(links: list[Mapping[str, object]]) -> str:
    lines = [
        MARKER,
        "### Claim the funded bounty before continuing",
        "",
        "This PR is reviewable, but it does not reserve or earn the linked bounty.",
        "Complete this missing transition:",
        "",
    ]
    for link in links:
        issue = int(link["issue"])
        lines.append(
            f"- On [#{issue}]({link['url']}), post "
            f"`/claim #{issue} wallet: 0xYOUR_PUBLIC_BASE_ADDRESS`, or call MCP "
            f"`agent_native_claim` with `{link['contract']}`."
        )
    lines.extend(
        [
            "",
            "Follow the returned wallet request. Continue only after canonical "
            "`BountyClaimed`; only canonical `BountySettled` proves payment.",
            "",
        ]
    )
    return "\n".join(lines)


def load_event() -> dict[str, object]:
    event_path = os.environ.get("GITHUB_EVENT_PATH")
    if not event_path:
        raise UserError("GITHUB_EVENT_PATH is required")
    event = json.loads(pathlib.Path(event_path).read_text(encoding="utf-8"))
    if not isinstance(event, dict) or not isinstance(event.get("pull_request"), dict):
        raise UserError("pull_request_target event is required")
    return event


def fetch_issue(gh: str, repository: str, number: int) -> dict[str, object]:
    value = json.loads(
        subprocess.check_output(
            [gh, "api", f"repos/{repository}/issues/{number}"],
            env=dict(os.environ),
            text=True,
        )
    )
    if not isinstance(value, dict):
        raise UserError(f"issue #{number} response is invalid")
    return value


def fetch_inventory() -> list[Mapping[str, object]]:
    fixture = os.environ.get("AGENT_BOUNTIES_PR_CLAIM_FEED_FILE")
    if fixture:
        value = json.loads(pathlib.Path(fixture).read_text(encoding="utf-8"))
    else:
        request = urllib.request.Request(
            API_FEED, headers={"Accept": "application/json", "User-Agent": "agent-bounties-pr-claim-guard/1"}
        )
        with urllib.request.urlopen(request, timeout=20) as response:
            value = json.load(response)
    if not isinstance(value, list):
        raise UserError("canonical bounty feed must be an array")
    return [item for item in value if isinstance(item, dict)]


def main() -> int:
    try:
        event = load_event()
        pull_request = event["pull_request"]
        repository = str(
            os.environ.get("GITHUB_REPOSITORY")
            or (event.get("repository") or {}).get("full_name")
            or ""
        )
        number = int(pull_request.get("number") or event.get("number") or 0)
        linked = linked_issue_numbers(str(pull_request.get("body") or ""))
        if not repository or not number or not linked:
            return 0
        gh = find_executable(["gh", "gh.exe"])
        if not gh:
            raise UserError("gh is required")
        issues = {issue: fetch_issue(gh, repository, issue) for issue in linked}
        links = claimable_links(linked, issues, fetch_inventory())
        if not links:
            return 0
        comment = render_comment(links)
        if os.environ.get("DRY_RUN") == "1":
            print(comment)
            return 0
        publish_issue_comment(
            os.environ,
            repository,
            number,
            MARKER,
            comment,
            "pr-claim-guard.md",
            "gh is required",
            UserError,
        )
        return 0
    except (OSError, ValueError, UserError) as error:
        print(f"PR claim guard failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
