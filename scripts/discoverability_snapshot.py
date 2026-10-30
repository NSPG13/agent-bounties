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
import math
import os
import sys
from dataclasses import dataclass
from datetime import date, datetime, timedelta, timezone
from typing import Any, Callable
from urllib.parse import quote

PROVIDERS = ("search_console", "github", "first_party")
GSC_SCOPE = "https://www.googleapis.com/auth/webmasters.readonly"
DEFAULT_API = "https://api.agentizable.ai"
DEFAULT_REPOSITORY = "NSPG13/agent-bounties"
DEFAULT_PROPERTY = "https://agentizable.ai/"
CREDENTIAL_NAMES = (
    "DISCOVERABILITY_INGEST_TOKEN",
    "REPOSITORY_TRAFFIC_TOKEN",
)
GSC_CREDENTIAL_NAMES = ("GSC_ACCESS_TOKEN", "GSC_SERVICE_ACCOUNT_JSON")


def iso_z(value: datetime) -> str:
    return value.astimezone(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def canonical_json(value: Any) -> bytes:
    return json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False
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
        if any(value.utcoffset() is None for value in (
            self.observed_at, self.window_started_at, self.window_ended_at, self.data_through
        )):
            raise ValueError("snapshot timestamps require time zones")
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
    service_account_json: str = "",
    access_token: str = "",
    property_url: str,
    observed_at: datetime,
) -> Snapshot:
    try:
        from google.auth.transport.requests import AuthorizedSession
        from google.oauth2 import credentials as oauth_credentials, service_account
    except ImportError as error:  # pragma: no cover - workflow installs dependency
        raise RuntimeError("google-auth is required for Search Console collection") from error

    if access_token:
        # A workflow-minted token has no local key or refresh credential. A
        # rejected/expired token must fail, never retry with the legacy key.
        credentials = oauth_credentials.Credentials(token=access_token)
    else:
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
        response = session.post(endpoint, json=body, timeout=60, allow_redirects=False)
        if 300 <= response.status_code < 400:
            raise RuntimeError("unexpected Search Console redirect")
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
    completed_at = datetime.now(timezone.utc)
    if completed_at < observed_at:
        raise ValueError("collection clock moved backwards")
    if (not isinstance(report, dict)
            or report.get("schema_version") != "agent-bounties/site-analytics-v2"
            or report.get("window_hours") != 672
            or not isinstance(report.get("overview"), dict)):
        raise ValueError("unexpected first-party aggregate contract")
    for key in ("event_counts", "channels", "rates", "daily"):
        if not isinstance(report.get(key), list):
            raise ValueError("missing first-party aggregate collection")

    def keyed_rows(rows: list[Any], key: str) -> dict[str, dict[str, Any]]:
        result = {}
        for row in rows:
            if (not isinstance(row, dict) or not isinstance(row.get(key), str)
                    or not row[key] or row[key] in result):
                raise ValueError("invalid or duplicate first-party aggregate row")
            result[row[key]] = row
        return result

    event_counts = keyed_rows(report["event_counts"], "event_name")
    for row in event_counts.values():
        nonnegative_int(row.get("events"), "event count")
    channels = report["channels"]
    channel_keys = set()
    for row in channels:
        if (not isinstance(row, dict) or not isinstance(row.get("source"), str)
                or not row["source"] or not (row.get("campaign") is None
                                             or isinstance(row["campaign"], str))):
            raise ValueError("invalid first-party acquisition row")
        channel_key = (row["source"], row.get("campaign"))
        if channel_key in channel_keys:
            raise ValueError("duplicate first-party acquisition row")
        channel_keys.add(channel_key)
        nonnegative_int(row.get("visitors"), "channel visitors")
    chatgpt_referrals = sum(
        row["visitors"]
        for row in channels
        if channel_is_chatgpt(row["source"])
    )
    rates = keyed_rows(report["rates"], "metric")
    rate = rates.get("market_to_funded_bounty_click", {})
    numerator = nonnegative_int(rate.get("numerator_sessions"), "market numerator")
    denominator = nonnegative_int(rate.get("denominator_sessions"), "market denominator")
    # The existing ingestion contract requires a numeric rate. An undefined
    # rate makes this provider unavailable; zero would invent evidence.
    ctr = bounded_rate(rate.get("value"), "market click-through")
    if (denominator == 0 or numerator > denominator
            or not math.isclose(ctr, numerator / denominator, rel_tol=0, abs_tol=1e-9)):
        raise ValueError("undefined or inconsistent market click-through")
    totals = {
        "captured_chatgpt_referrals": chatgpt_referrals,
        "opportunity_feed_clicks": nonnegative_int(
            event_counts.get("opportunity_feed_click", {}).get("events", 0),
            "opportunity feed clicks",
        ),
        "market_views": nonnegative_int(
            event_counts.get("market_view", {}).get("events", 0), "market views"
        ),
        "funded_bounty_clicks": nonnegative_int(
            event_counts.get("funded_bounty_click", {}).get("events", 0),
            "funded bounty clicks",
        ),
        "market_to_funded_opportunity_ctr": ctr,
    }
    try:
        started = datetime.fromisoformat(report["window_started_at"].replace("Z", "+00:00"))
        generated = datetime.fromisoformat(report["generated_at"].replace("Z", "+00:00"))
    except (KeyError, AttributeError, TypeError) as error:
        raise ValueError("missing first-party timestamps") from error
    payload = {
        "totals": totals,
        "daily": report["daily"],
        "acquisition": channels,
        "coverage": {
            "headline_window_days": 28,
            "first_event_at": report.get("overview", {}).get("first_event_at"),
            "last_event_at": report.get("overview", {}).get("last_event_at"),
        },
    }
    snapshot = Snapshot("first_party", completed_at, started, generated, generated, payload)
    snapshot.as_request()
    return snapshot


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
    observed_at = datetime.now(timezone.utc)
    missing_configuration = [name for name in CREDENTIAL_NAMES if not os.environ.get(name, "").strip()]
    gsc_credentials = {name: os.environ.get(name, "").strip() for name in GSC_CREDENTIAL_NAMES}
    if not any(gsc_credentials.values()):
        missing_configuration.extend(GSC_CREDENTIAL_NAMES)
    print(
        "discoverability_configuration "
        f"mode={'dry_run' if args.dry_run else 'upload'} "
        f"missing={','.join(missing_configuration) or 'none'}"
    )
    if not args.dry_run and "DISCOVERABILITY_INGEST_TOKEN" in missing_configuration:
        return 1
    ingest_token = None if args.dry_run else required_env("DISCOVERABILITY_INGEST_TOKEN")
    collectors: list[tuple[str, Callable[[], Snapshot]]] = []
    collectors.append(
        (
            "search_console",
            lambda: collect_search_console(
                service_account_json=gsc_credentials["GSC_SERVICE_ACCOUNT_JSON"],
                access_token=gsc_credentials["GSC_ACCESS_TOKEN"],
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
    provider_credentials = {
        "search_console": "GSC_SERVICE_ACCOUNT_JSON",
        "github": "REPOSITORY_TRAFFIC_TOKEN",
    }
    for provider, collect in collectors:
        if provider_credentials.get(provider) in missing_configuration:
            failures.append(provider)
            print(f"discoverability_provider provider={provider} available=false reason=missing_configuration")
            continue
        stage = "collect"
        try:
            snapshot = collect()
            stage = "validate"
            snapshot.as_request()
            snapshots.append(snapshot)
        except Exception as error:
            failures.append(provider)
            reason = "invalid_evidence" if isinstance(error, ValueError) else "collection_failed"
            print(f"discoverability_provider provider={provider} available=false stage={stage} reason={reason}")
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
    assert ingest_token is not None
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
