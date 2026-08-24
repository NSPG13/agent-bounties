#!/usr/bin/env python3
"""Collect privacy-bounded discoverability snapshots and upload them idempotently.

Raw Search Console dimensions and GitHub paths/referrers are sent only to the
operator endpoint. Console output is deliberately limited to provider coverage
and aggregate success state.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from dataclasses import dataclass
from datetime import date, datetime, timedelta, timezone
from typing import Any, Callable
from urllib.parse import quote

PROVIDERS = ("search_console", "github", "first_party")
GSC_SCOPE = "https://www.googleapis.com/auth/webmasters.readonly"
DEFAULT_API = "https://api.agentbounties.app"
DEFAULT_REPOSITORY = "NSPG13/agent-bounties"
DEFAULT_PROPERTY = "https://agentbounties.app/"


def iso_z(value: datetime) -> str:
    return value.astimezone(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def canonical_json(value: Any) -> bytes:
    return json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    ).encode("utf-8")


def payload_checksum(payload: dict[str, Any]) -> str:
    return hashlib.sha256(canonical_json(payload)).hexdigest()


def failure_label(error: Exception) -> str:
    """Return a log-safe failure category without exception text or credentials."""
    return type(error).__name__


def nonnegative_int(value: Any, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ValueError(f"{name} must be a non-negative integer")
    return value


def bounded_rate(value: Any, name: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a number")
    normalized = float(value)
    if not 0.0 <= normalized <= 1.0:
        raise ValueError(f"{name} must be between zero and one")
    return normalized


@dataclass(frozen=True)
class Snapshot:
    provider: str
    observed_at: datetime
    window_started_at: datetime
    window_ended_at: datetime
    data_through: datetime
    payload: dict[str, Any]

    def as_request(self) -> dict[str, Any]:
        if self.provider not in PROVIDERS:
            raise ValueError("unsupported provider")
        if not (
            self.window_started_at
            <= self.data_through
            <= self.window_ended_at
            <= self.observed_at
        ):
            raise ValueError("invalid snapshot window")
        return {
            "provider": self.provider,
            "observed_at": iso_z(self.observed_at),
            "window_started_at": iso_z(self.window_started_at),
            "window_ended_at": iso_z(self.window_ended_at),
            "data_through": iso_z(self.data_through),
            "payload_checksum": payload_checksum(self.payload),
            "payload": self.payload,
        }


def request_json(
    method: str,
    url: str,
    *,
    headers: dict[str, str] | None = None,
    payload: dict[str, Any] | None = None,
    timeout: int = 45,
) -> Any:
    import requests

    response = requests.request(
        method,
        url,
        headers=headers,
        json=payload,
        timeout=timeout,
    )
    response.raise_for_status()
    return response.json()


def search_console_date_ranges(observed_at: datetime) -> tuple[date, date, date]:
    """Return the public 28-day start, private recovery start, and D-3 end."""
    end_date = observed_at.astimezone(timezone.utc).date() - timedelta(days=3)
    return end_date - timedelta(days=27), end_date - timedelta(days=34), end_date


def collect_search_console(
    *,
    service_account_json: str,
    property_url: str,
    observed_at: datetime,
) -> Snapshot:
    try:
        from google.auth.transport.requests import AuthorizedSession
        from google.oauth2 import service_account
    except ImportError as error:  # pragma: no cover - workflow installs dependency
        raise RuntimeError("google-auth is required for Search Console collection") from error

    credentials_info = json.loads(service_account_json)
    credentials = service_account.Credentials.from_service_account_info(
        credentials_info, scopes=[GSC_SCOPE]
    )
    session = AuthorizedSession(credentials)
    headline_start_date, recovery_start_date, end_date = search_console_date_ranges(
        observed_at
    )
    endpoint = (
        "https://searchconsole.googleapis.com/webmasters/v3/sites/"
        f"{quote(property_url, safe='')}/searchAnalytics/query"
    )

    def query(body: dict[str, Any]) -> dict[str, Any]:
        response = session.post(endpoint, json=body, timeout=60)
        response.raise_for_status()
        return response.json()

    totals_response = query(
        {
            "startDate": headline_start_date.isoformat(),
            "endDate": end_date.isoformat(),
            "dataState": "final",
        }
    )
    totals_rows = totals_response.get("rows", [])
    total = totals_rows[0] if totals_rows else {}
    dimension_rows = paginate_search_console_dimensions(
        query=query,
        start_date=recovery_start_date.isoformat(),
        end_date=end_date.isoformat(),
    )

    payload = {
        "totals": {
            "impressions": nonnegative_int(int(total.get("impressions", 0)), "impressions"),
            "clicks": nonnegative_int(int(total.get("clicks", 0)), "clicks"),
            "average_position": max(0.0, float(total.get("position", 0.0))),
        },
        "dimensions": {"query_page_rows": dimension_rows},
        "coverage": {
            "data_state": "final",
            "requested_days": 35,
            "headline_window_days": 28,
            "dimension_rows": len(dimension_rows),
        },
    }
    window_started = datetime.combine(
        recovery_start_date, datetime.min.time(), tzinfo=timezone.utc
    )
    data_through = datetime.combine(
        end_date, datetime.max.time(), tzinfo=timezone.utc
    ).replace(microsecond=0)
    return Snapshot(
        "search_console",
        observed_at,
        window_started,
        data_through,
        data_through,
        payload,
    )


def paginate_search_console_dimensions(
    *,
    query: Callable[[dict[str, Any]], dict[str, Any]],
    start_date: str,
    end_date: str,
    row_limit: int = 25_000,
) -> list[dict[str, Any]]:
    dimension_rows: list[dict[str, Any]] = []
    start_row = 0
    while True:
        page = query(
            {
                "startDate": start_date,
                "endDate": end_date,
                "dataState": "final",
                "dimensions": ["query", "page"],
                "rowLimit": row_limit,
                "startRow": start_row,
            }
        ).get("rows", [])
        dimension_rows.extend(page)
        if len(page) < row_limit:
            break
        start_row += len(page)
    return dimension_rows


def collect_github(
    *, token: str, repository: str, observed_at: datetime
) -> Snapshot:
    headers = {
        "Accept": "application/vnd.github+json",
        "Authorization": f"Bearer {token}",
        "X-GitHub-Api-Version": "2022-11-28",
    }
    root = f"https://api.github.com/repos/{repository}/traffic"
    views = request_json("GET", f"{root}/views?per=day", headers=headers)
    clones = request_json("GET", f"{root}/clones?per=day", headers=headers)
    popular_paths = request_json("GET", f"{root}/popular/paths", headers=headers)
    popular_referrers = request_json("GET", f"{root}/popular/referrers", headers=headers)
    totals = {
        "unique_visitors": nonnegative_int(views.get("uniques"), "unique visitors"),
        "unique_cloners": nonnegative_int(clones.get("uniques"), "unique cloners"),
        "views": nonnegative_int(views.get("count"), "views"),
        "clones": nonnegative_int(clones.get("count"), "clones"),
    }
    data_points = [*views.get("views", []), *clones.get("clones", [])]
    if not data_points:
        raise ValueError("GitHub traffic response contains no dated coverage")
    timestamps = [
        datetime.fromisoformat(point["timestamp"].replace("Z", "+00:00"))
        for point in data_points
    ]
    window_started = min(timestamps)
    data_through = max(timestamps)
    payload = {
        "totals": totals,
        "daily": {"views": views.get("views", []), "clones": clones.get("clones", [])},
        "dimensions": {
            "popular_paths": popular_paths,
            "popular_referrers": popular_referrers,
        },
        "coverage": {"provider_window_days": 14},
    }
    return Snapshot(
        "github",
        observed_at,
        window_started,
        data_through,
        data_through,
        payload,
    )


def channel_is_chatgpt(source: str) -> bool:
    normalized = source.strip().lower().rstrip(".")
    return normalized in {
        "chatgpt",
        "chatgpt.com",
        "chat.openai.com",
        "openai",
        "openai.com",
    } or normalized.endswith(".chatgpt.com") or normalized.endswith(".openai.com")


def collect_first_party(*, api_base: str, observed_at: datetime) -> Snapshot:
    report = request_json("GET", f"{api_base}/v1/analytics/site?window_hours=672")
    event_counts = {
        row.get("event_name"): row for row in report.get("event_counts", [])
    }
    channels = report.get("channels", [])
    chatgpt_referrals = sum(
        nonnegative_int(int(row.get("visitors", 0)), "channel visitors")
        for row in channels
        if channel_is_chatgpt(str(row.get("source", "")))
    )
    rates = {row.get("metric"): row for row in report.get("rates", [])}
    ctr = rates.get("market_to_funded_bounty_click", {}).get("value")
    ctr = 0.0 if ctr is None else bounded_rate(ctr, "market click-through")
    totals = {
        "captured_chatgpt_referrals": chatgpt_referrals,
        "opportunity_feed_clicks": nonnegative_int(
            int(event_counts.get("opportunity_feed_click", {}).get("events", 0)),
            "opportunity feed clicks",
        ),
        "market_views": nonnegative_int(
            int(event_counts.get("market_view", {}).get("events", 0)), "market views"
        ),
        "funded_bounty_clicks": nonnegative_int(
            int(event_counts.get("funded_bounty_click", {}).get("events", 0)),
            "funded bounty clicks",
        ),
        "market_to_funded_opportunity_ctr": ctr,
    }
    started = datetime.fromisoformat(report["window_started_at"].replace("Z", "+00:00"))
    generated = datetime.fromisoformat(report["generated_at"].replace("Z", "+00:00"))
    payload = {
        "totals": totals,
        "daily": report.get("daily", []),
        "acquisition": channels,
        "coverage": {
            "headline_window_days": 28,
            "first_event_at": report.get("overview", {}).get("first_event_at"),
            "last_event_at": report.get("overview", {}).get("last_event_at"),
        },
    }
    return Snapshot("first_party", observed_at, started, generated, generated, payload)


def upload_snapshots(
    *, api_base: str, ingest_token: str, snapshots: list[Snapshot]
) -> dict[str, Any]:
    return request_json(
        "POST",
        f"{api_base}/v1/operator/discoverability/snapshots",
        headers={"X-Agent-Bounties-Discoverability-Ingest": ingest_token},
        payload={"snapshots": [snapshot.as_request() for snapshot in snapshots]},
    )


def required_env(name: str) -> str:
    value = os.environ.get(name, "").strip()
    if not value:
        raise RuntimeError(f"required credential {name} is not configured")
    return value


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--api-base", default=os.environ.get("PRODUCTION_API_BASE_URL", DEFAULT_API))
    parser.add_argument("--repository", default=DEFAULT_REPOSITORY)
    parser.add_argument("--property", default=DEFAULT_PROPERTY)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    observed_at = datetime.now(timezone.utc).replace(microsecond=0)
    ingest_token = required_env("DISCOVERABILITY_INGEST_TOKEN")
    collectors: list[tuple[str, Callable[[], Snapshot]]] = []
    collectors.append(
        (
            "search_console",
            lambda: collect_search_console(
                service_account_json=required_env("GSC_SERVICE_ACCOUNT_JSON"),
                property_url=args.property,
                observed_at=observed_at,
            ),
        )
    )
    collectors.append(
        (
            "github",
            lambda: collect_github(
                token=required_env("REPOSITORY_TRAFFIC_TOKEN"),
                repository=args.repository,
                observed_at=observed_at,
            ),
        )
    )
    collectors.append(
        (
            "first_party",
            lambda: collect_first_party(api_base=args.api_base, observed_at=observed_at),
        )
    )
    snapshots: list[Snapshot] = []
    failures: list[str] = []
    for provider, collect in collectors:
        try:
            snapshots.append(collect())
        except Exception:
            failures.append(provider)
    print(
        "discoverability_collection "
        f"available={','.join(sorted(snapshot.provider for snapshot in snapshots)) or 'none'} "
        f"unavailable={','.join(sorted(failures)) or 'none'}"
    )
    if not snapshots:
        return 1
    if args.dry_run:
        print(f"discoverability_upload dry_run=true snapshots={len(snapshots)}")
        return 0 if not failures else 1
    result = upload_snapshots(
        api_base=args.api_base,
        ingest_token=ingest_token,
        snapshots=snapshots,
    )
    print(
        "discoverability_upload "
        f"accepted={int(result.get('accepted', 0))} "
        f"duplicates={int(result.get('duplicates', 0))} "
        f"coverage_complete={str(not failures).lower()}"
    )
    return 0 if not failures else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"discoverability_snapshot failed={failure_label(error)}", file=sys.stderr)
        raise SystemExit(1)
