#!/usr/bin/env python3
"""Save public growth evidence without credentials, uploads, or tracking writes.

Use operator tooling for private Search Console/GitHub provider snapshots.
This companion consumes the existing public aggregate contracts only.
"""
from __future__ import annotations

import argparse
import json
import math
from datetime import datetime, timedelta, timezone
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import HTTPRedirectHandler, Request, build_opener

API = "https://api.agentizable.ai"
SOURCES = {
    "site_28_days": ("/v1/analytics/site?window_hours=672", "agent-bounties/site-analytics-v2"),
    "site_calendar_history": ("/v1/analytics/site?window_hours=1368", "agent-bounties/site-analytics-v2"),
    "discoverability": ("/v1/discoverability/summary", "agent-bounties/discoverability-summary-v1"),
    "conversion_28_days": ("/v1/opportunities/conversion-funnel?window_hours=672", "agent-bounties/opportunity-conversion-funnel-v1"),
}
ADDITIVE_EVENTS = ("page_views", "market_views", "funded_bounty_clicks", "canonical_posts_confirmed", "funding_starts", "claims_confirmed")


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def count(value):
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ValueError("invalid aggregate count")
    return value


def calendar_comparison(payload):
    """Compare additive events on two complete 28-day UTC calendar windows.

    Missing dates are unavailable, never zero-filled. Daily visitors/sessions
    cannot be summed into deduplicated period totals and are excluded.
    """
    generated = datetime.fromisoformat(payload["generated_at"].replace("Z", "+00:00"))
    started = datetime.fromisoformat(payload["window_started_at"].replace("Z", "+00:00"))
    if generated.utcoffset() is None or started.utcoffset() is None:
        raise ValueError("timestamps require time zones")
    today = generated.astimezone(timezone.utc).date()
    rows = {}
    for row in payload["daily"]:
        day = datetime.strptime(row["day"], "%Y-%m-%d").date()
        if day in rows:
            raise ValueError("duplicate daily aggregate")
        rows[day] = row
    result = {"timezone": "UTC", "metric_unit": "events, not people or deduplicated sessions", "windows": {}}
    for label, offset in (("previous", 56), ("current", 28)):
        start = today - timedelta(days=offset)
        end = start + timedelta(days=27)
        days = [start + timedelta(days=i) for i in range(28)]
        complete = (started <= datetime.combine(start, datetime.min.time(), tzinfo=timezone.utc)
                    and all(day in rows for day in days))
        window = {"start": str(start), "end": str(end), "status": "available" if complete else "unavailable", "metrics": None}
        if complete:
            window["metrics"] = {key: sum(count(rows[day][key]) for day in days) for key in ADDITIVE_EVENTS}
        result["windows"][label] = window
    return result


def validate(payload, schema):
    if not isinstance(payload, dict) or payload.get("schema_version") != schema:
        raise ValueError("unexpected response schema")
    if schema.endswith("site-analytics-v2"):
        for key in ("overview", "daily", "rates", "channels", "interfaces", "generated_at", "window_started_at"):
            if key not in payload:
                raise ValueError("incomplete site aggregate")
        for key in ("unique_visitors", "sessions", "page_views"):
            count(payload["overview"][key])
        for rate in payload["rates"]:
            numerator = count(rate["numerator_sessions"])
            denominator = count(rate["denominator_sessions"])
            value = rate["value"]
            if numerator > denominator or (denominator == 0 and value is not None):
                raise ValueError("invalid conversion denominator")
            if denominator and (not isinstance(value, (float, int)) or isinstance(value, bool)
                                or not math.isfinite(value)
                                or abs(value - numerator / denominator) > 1e-9):
                raise ValueError("inconsistent conversion rate")
    return payload


def collect():
    opener = build_opener(NoRedirect)
    evidence = {"schema_version": "agentizable/growth-baseline-v1", "observed_at": datetime.now(timezone.utc).isoformat(), "sources": {}}
    failures = []
    for name, (path, schema) in SOURCES.items():
        source = {"url": API + path, "status": "unavailable", "response": None}
        try:
            request = Request(API + path, headers={"Accept": "application/json", "User-Agent": "Agentizable-ReadOnly-Growth-Audit/1.0"})
            with opener.open(request, timeout=25) as response:
                source["http_status"] = response.status
                if response.status != 200:
                    raise ValueError("expected a complete HTTP 200 aggregate response")
                raw = response.read(4_000_001)
                if len(raw) > 4_000_000:
                    raise ValueError("aggregate exceeds bounded response size")
                payload = validate(json.loads(raw), schema)
                source.update(status="retrieved", http_status=response.status, response=payload)
        except Exception as error:
            source["error_type"] = type(error).__name__
            if isinstance(error, HTTPError):
                source["http_status"] = error.code
            failures.append(name)
        evidence["sources"][name] = source
    history = evidence["sources"]["site_calendar_history"]["response"]
    if history is not None:
        try:
            evidence["calendar_comparison"] = calendar_comparison(history)
        except (ValueError, KeyError, TypeError) as error:
            evidence["calendar_comparison"] = {"status": "unavailable", "error_type": type(error).__name__}
            failures.append("calendar_comparison")
    evidence["limitations"] = [
        "Public aggregates do not identify verified people or independent agents or fully separate bots and unexcluded operator probes.",
        "Rolling 28-day totals contain partial boundary days; calendar comparisons use complete UTC days and only additive event counts.",
        "Retrieved discoverability data can itself report unavailable; null metrics never mean zero.",
        "First-party interface events are not canonical settlement. The unfunded-publication cohort is not the whole marketplace.",
        "Search Console, GA4, GitHub traffic and social analytics require separate authorized reads and their own date/coverage receipts.",
    ]
    evidence["retrieval_failures"] = failures
    return evidence


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    evidence = collect()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    print("growth_baseline saved; retrieval_failures=" + str(len(evidence["retrieval_failures"])))
    return int(bool(evidence["retrieval_failures"]))


if __name__ == "__main__":
    raise SystemExit(main())
