#!/usr/bin/env python3
"""Refresh public aggregates over read-only HTTPS, without gh or Actions.

Only the aggregate is written. Raw GitHub records and the read-only token stay
in memory. A failed collection never replaces the last successful snapshot.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import re
import sys
import urllib.error
import urllib.request

import github_audience_audit as audit

ROOT = Path(__file__).resolve().parents[1]
PREFIX = "https://api.github.com/repos/NSPG13/agent-bounties/"
# GitHub's Link header uses this repository's stable numeric ID for pagination.
PAGINATION_PREFIX = "https://api.github.com/repositories/1293030696/"


class NoRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ValueError("GitHub metrics redirects are not permitted")


class Reader:
    def __init__(self, token, opener=None):
        self.token = token
        self.opener = opener or urllib.request.build_opener(NoRedirects())

    def page(self, url, accept=None):
        if not url.startswith((PREFIX, PAGINATION_PREFIX)) or any(c in url for c in ("\r", "\n", "#", "\\")):
            raise ValueError("Unexpected GitHub metrics URL")
        request = urllib.request.Request(url, headers={
            "Accept": accept or "application/vnd.github+json",
            "Authorization": "Bearer " + self.token,
            "User-Agent": "AgentBounties-PublicMetrics/1.0",
            "X-GitHub-Api-Version": "2022-11-28",
        })
        with self.opener.open(request, timeout=30) as response:
            if response.status != 200:
                raise ValueError("GitHub metrics request did not succeed")
            data = response.read(16 * 1024 * 1024 + 1)
            if len(data) > 16 * 1024 * 1024:
                raise ValueError("GitHub metrics page exceeds limit")
            return json.loads(data), response.headers.get("Link", "")

    def records(self, repository, suffix, *, accept=None):
        if repository != "NSPG13/agent-bounties":
            raise ValueError("Unexpected metrics repository")
        url = PREFIX + suffix.lstrip("/")
        records, visited = [], set()
        while url:
            if url in visited or len(visited) >= 1000:
                raise ValueError("GitHub metrics pagination did not terminate")
            visited.add(url)
            page, links = self.page(url, accept)
            if not isinstance(page, list) or any(not isinstance(row, dict) for row in page):
                raise ValueError("Malformed GitHub metrics page")
            records.extend(page)
            match = re.search(r'<([^>]+)>;\s*rel="next"', links)
            url = match.group(1) if match else None
        return records

    def traffic(self, repository):
        # This credential deliberately has no repository-administration access.
        # Private clone/visitor analytics remain explicitly unavailable.
        return {"status": "unavailable"}


def refresh(destination, token):
    reader = Reader(token)
    audit.gh_api = reader.records
    audit.collect_repository_traffic = reader.traffic
    snapshot = audit.collect_snapshot("NSPG13/agent-bounties", include_enrichment=False,
        activity_since=audit.PLATFORM_LAUNCH_AT, include_repository_traffic=False)
    policy = audit.public_metrics_policy(ROOT / "crates/api/fixtures/public-metrics-policy.json")
    result = audit.build_public_participation_metrics(snapshot, "NSPG13",
        excluded_logins=set(policy["maintainer_github_logins"]))
    if result["coverage"]["status"] != "ready":
        raise ValueError("GitHub metrics collection is incomplete")
    destination.parent.mkdir(parents=True, exist_ok=True)
    pending = destination.with_suffix(".pending")
    pending.write_text(json.dumps(result, indent=2) + "\n")
    pending.replace(destination)
    print("public_metrics_refreshed=" + result["generated_at"])


def main():
    token = os.environ.get("GITHUB_METRICS_TOKEN", "")
    if not token:
        raise ValueError("GITHUB_METRICS_TOKEN is not configured")
    refresh(ROOT / "site/generated/github-participation.json", token)


if __name__ == "__main__":
    try:
        main()
    except Exception:
        # Do not print exception bodies, HTTP responses, raw records or headers.
        print("Public metrics refresh failed; previous snapshot retained.", file=sys.stderr)
        raise SystemExit(1)
