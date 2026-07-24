#!/usr/bin/env python3
"""Collect public, configured direct-competitor observations and write a daily report.

This process is intentionally read-only against competitors. It accepts only the
reviewed registry in ops/competitors/, records source hashes rather than page
copies, and treats a failed fetch as an observation—not as evidence that a
competitor disappeared.
"""

from __future__ import annotations

import argparse
import hashlib
import html
import json
import re
import sys
import uuid
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

MAX_RESPONSE_BYTES = 5_000_000
USER_AGENT = "AgentBountiesCompetitorIntel/1.0 (+https://github.com/NSPG13/agent-bounties)"
GITHUB_METRICS = {"github_stars", "github_forks", "github_open_issues", "github_pushed_at"}


@dataclass
class Observation:
    competitor_slug: str
    source_key: str
    source_url: str
    observed_at: str
    http_status: int | None
    content_sha256: str | None
    extracted: dict[str, Any]
    error_kind: str | None = None
    error_message: str | None = None


def utc_now() -> datetime:
    return datetime.now(UTC).replace(microsecond=0)


def canonical_json(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def text_from_html(content: bytes) -> str:
    decoded = content.decode("utf-8", errors="replace")
    without_scripts = re.sub(r"<(script|style)[^>]*>.*?</\1>", " ", decoded, flags=re.I | re.S)
    without_tags = re.sub(r"<[^>]+>", " ", without_scripts)
    return re.sub(r"\s+", " ", html.unescape(without_tags)).strip()


def parse_display_number(value: str) -> int | float:
    match = re.fullmatch(r"([0-9]+(?:\.[0-9]+)?)(?:\s*(k|m|million|b|billion))?", value.replace(",", "").strip().lower())
    if not match:
        raise ValueError(f"unsupported numeric display: {value!r}")
    multiplier = {None: 1, "k": 1_000, "m": 1_000_000, "million": 1_000_000, "b": 1_000_000_000, "billion": 1_000_000_000}[match.group(2)]
    parsed = float(match.group(1)) * multiplier
    return int(parsed) if parsed.is_integer() else parsed


def extract_metric(metric_key: str, body: bytes, content_type: str) -> int | float | str | None:
    if metric_key in GITHUB_METRICS:
        payload = json.loads(body.decode("utf-8"))
        github_field = {
            "github_stars": "stargazers_count",
            "github_forks": "forks_count",
            "github_open_issues": "open_issues_count",
            "github_pushed_at": "pushed_at",
        }[metric_key]
        return payload.get(github_field)

    text = text_from_html(body) if "html" in content_type.lower() else body.decode("utf-8", errors="replace")
    patterns = {
        "bounties_paid": r"\b(?:Bounties paid|Paid)\s+([0-9][0-9,]*)\b",
        "bounties_available": r"\b(?:Bounties available|Available)\s+([0-9][0-9,]*)\b",
        "paid_out_usd": r"\b(?:Money paid in bounties|Paid out)\s+\$([0-9][0-9,]*(?:\.\d+)?)\b",
        "open_value_usd": r"\b(?:Money available in bounties|Open value)\s+\$([0-9][0-9,]*(?:\.\d+)?)\b",
        "bounties_posted": r"#\s*of bounties posted:\s*([0-9][0-9,]*)\b",
        "bounties_posted_usd": r"\$\s*of bounties posted:\s*\$([0-9][0-9,]*(?:\.\d+)?(?:\s*(?:k|m|million|b|billion))?)\b",
    }
    pattern = patterns.get(metric_key)
    if pattern is None:
        raise ValueError(f"unsupported metric {metric_key}")
    matches = re.findall(pattern, text, flags=re.I)
    if len(matches) != 1:
        return None
    return parse_display_number(matches[0])


def fetch_source(competitor_slug: str, source: dict[str, Any], observed_at: datetime) -> Observation:
    request = Request(source["url"], headers={"User-Agent": USER_AGENT, "Accept": "application/json, text/html;q=0.9"})
    try:
        with urlopen(request, timeout=20) as response:  # nosec B310: URLs are reviewed registry data
            status = response.status
            body = response.read(MAX_RESPONSE_BYTES + 1)
            content_type = response.headers.get_content_type()
    except HTTPError as error:
        return Observation(competitor_slug, source["key"], source["url"], observed_at.isoformat(), error.code, None, {}, "network", f"HTTP {error.code}")
    except TimeoutError:
        return Observation(competitor_slug, source["key"], source["url"], observed_at.isoformat(), None, None, {}, "timeout", "request timed out")
    except URLError as error:
        return Observation(competitor_slug, source["key"], source["url"], observed_at.isoformat(), None, None, {}, "network", str(error.reason)[:500])

    if not 200 <= status < 300:
        return Observation(competitor_slug, source["key"], source["url"], observed_at.isoformat(), status, None, {}, "network", f"HTTP {status}")
    if len(body) > MAX_RESPONSE_BYTES:
        return Observation(competitor_slug, source["key"], source["url"], observed_at.isoformat(), status, None, {}, "invalid_response", "response exceeds configured limit")

    extracted: dict[str, Any] = {}
    try:
        for metric_key in source.get("metrics", []):
            value = extract_metric(metric_key, body, content_type)
            if value is not None:
                extracted[metric_key] = value
    except (ValueError, json.JSONDecodeError) as error:
        return Observation(competitor_slug, source["key"], source["url"], observed_at.isoformat(), status, None, {}, "invalid_response", str(error)[:500])
    return Observation(
        competitor_slug, source["key"], source["url"], observed_at.isoformat(), status,
        hashlib.sha256(body).hexdigest(), extracted,
    )


def validate_registry(registry: dict[str, Any]) -> None:
    if registry.get("schema_version") != "agent-bounties/direct-competitor-registry-v1":
        raise ValueError("unsupported competitor registry schema")
    competitors = registry.get("competitors")
    if not isinstance(competitors, list) or not competitors:
        raise ValueError("registry must contain competitors")
    seen: set[str] = set()
    for competitor in competitors:
        slug = competitor.get("slug")
        if not isinstance(slug, str) or not re.fullmatch(r"[a-z0-9][a-z0-9-]{1,62}", slug) or slug in seen:
            raise ValueError(f"invalid or duplicate competitor slug: {slug!r}")
        seen.add(slug)
        for field in ("canonical_url", "inclusion_evidence_url"):
            if not str(competitor.get(field, "")).startswith("https://"):
                raise ValueError(f"{slug} has invalid {field}")
        if not competitor.get("direct_competitor_reason") or not competitor.get("sources"):
            raise ValueError(f"{slug} needs a direct reason and at least one source")
        for source in competitor["sources"]:
            if not str(source.get("url", "")).startswith("https://"):
                raise ValueError(f"{slug}/{source.get('key')} has invalid source URL")
            if not re.fullmatch(r"[a-z0-9][a-z0-9_-]{1,62}", str(source.get("key", ""))):
                raise ValueError(f"{slug} has invalid source key")
            unknown = set(source.get("metrics", [])) - {
                "bounties_paid", "bounties_available", "paid_out_usd", "open_value_usd",
                "bounties_posted", "bounties_posted_usd", *GITHUB_METRICS,
            }
            if unknown:
                raise ValueError(f"{slug}/{source['key']} has unsupported metrics: {sorted(unknown)}")


class PostgresWriter:
    def __init__(self, database_url: str):
        try:
            import psycopg  # type: ignore
        except ImportError as error:
            raise RuntimeError("database persistence requires psycopg; install with pip install 'psycopg[binary]'") from error
        self.connection = psycopg.connect(database_url)

    def previous(self, competitor_slug: str, source_key: str) -> tuple[dict[str, Any] | None, dict[str, Any]]:
        with self.connection.cursor() as cursor:
            cursor.execute("""
                SELECT observation.id, observation.content_sha256, observation.error_kind
                FROM competitor_source_observations observation
                JOIN competitor_intelligence_runs run ON run.id = observation.run_id
                WHERE observation.competitor_slug = %s AND observation.source_key = %s
                  AND run.status IN ('completed', 'completed_with_failures')
                ORDER BY observation.observed_at DESC LIMIT 1
            """, (competitor_slug, source_key))
            source = cursor.fetchone()
            if source is None:
                return None, {}
            cursor.execute("SELECT metric_key, value_numeric, value_text FROM competitor_metric_observations WHERE source_observation_id = %s", (source[0],))
            metrics = {row[0]: (float(row[1]) if row[1] is not None else row[2]) for row in cursor.fetchall()}
        return {"id": str(source[0]), "content_sha256": source[1], "error_kind": source[2]}, metrics

    def begin(self, run_id: uuid.UUID, started_at: datetime, registry_sha256: str, registry: dict[str, Any]) -> None:
        with self.connection.cursor() as cursor:
            cursor.execute("INSERT INTO competitor_intelligence_runs (id, started_at, status, registry_sha256) VALUES (%s, %s, 'running', %s)", (run_id, started_at, registry_sha256))
            for competitor in registry["competitors"]:
                cursor.execute("""
                    INSERT INTO competitors (slug, brand_name, canonical_url, market_type, status, direct_competitor_reason, inclusion_evidence_url, created_at, updated_at)
                    VALUES (%(slug)s, %(brand_name)s, %(canonical_url)s, %(market_type)s, %(status)s, %(direct_competitor_reason)s, %(inclusion_evidence_url)s, %(now)s, %(now)s)
                    ON CONFLICT (slug) DO UPDATE SET brand_name = EXCLUDED.brand_name, canonical_url = EXCLUDED.canonical_url,
                      market_type = EXCLUDED.market_type, status = EXCLUDED.status, direct_competitor_reason = EXCLUDED.direct_competitor_reason,
                      inclusion_evidence_url = EXCLUDED.inclusion_evidence_url, updated_at = EXCLUDED.updated_at
                """, {**competitor, "now": started_at})
                cursor.execute("DELETE FROM competitor_links WHERE competitor_slug = %s", (competitor["slug"],))
                for kind, urls in competitor.get("links", {}).items():
                    for url in urls:
                        cursor.execute("INSERT INTO competitor_links (competitor_slug, link_kind, url) VALUES (%s, %s, %s)", (competitor["slug"], kind, url))
                cursor.execute("DELETE FROM competitor_capabilities WHERE competitor_slug = %s", (competitor["slug"],))
                for capability in competitor.get("capabilities", []):
                    cursor.execute("INSERT INTO competitor_capabilities (competitor_slug, capability_key, description, evidence_url, observed_at) VALUES (%s, %s, %s, %s, %s)", (competitor["slug"], capability["key"], capability["description"], capability["evidence_url"], started_at))
        self.connection.commit()

    def save_observation(self, run_id: uuid.UUID, observation: Observation) -> uuid.UUID:
        observation_id = uuid.uuid4()
        with self.connection.cursor() as cursor:
            cursor.execute("""
                INSERT INTO competitor_source_observations (id, run_id, competitor_slug, source_key, source_url, observed_at, http_status, content_sha256, extracted, error_kind, error_message)
                VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s::jsonb, %s, %s)
            """, (observation_id, run_id, observation.competitor_slug, observation.source_key, observation.source_url,
                  observation.observed_at, observation.http_status, observation.content_sha256, canonical_json(observation.extracted), observation.error_kind, observation.error_message))
            for key, value in observation.extracted.items():
                cursor.execute("""
                    INSERT INTO competitor_metric_observations (source_observation_id, metric_key, value_numeric, value_text, unit, evidence_url)
                    VALUES (%s, %s, %s, %s, %s, %s)
                """, (observation_id, key, value if isinstance(value, (int, float)) else None, value if isinstance(value, str) else None,
                      "timestamp" if key.endswith("_at") else "count" if key.startswith(("bounties_", "github_")) else "usd", observation.source_url))
        self.connection.commit()
        return observation_id

    def save_change(self, run_id: uuid.UUID, observation: Observation, change: dict[str, Any]) -> None:
        with self.connection.cursor() as cursor:
            cursor.execute("""
                INSERT INTO competitor_intelligence_changes (id, run_id, competitor_slug, change_kind, field_path, previous_value, current_value, evidence_url, detected_at)
                VALUES (%s, %s, %s, %s, %s, %s::jsonb, %s::jsonb, %s, %s)
            """, (uuid.uuid4(), run_id, observation.competitor_slug, change["kind"], change["field"],
                  canonical_json(change.get("previous")), canonical_json(change.get("current")), observation.source_url, observation.observed_at))
        self.connection.commit()

    def finish(self, run_id: uuid.UUID, completed_at: datetime, status: str, report: dict[str, Any], markdown: str) -> None:
        with self.connection.cursor() as cursor:
            cursor.execute("UPDATE competitor_intelligence_runs SET completed_at = %s, status = %s, report_json = %s::jsonb, report_markdown = %s WHERE id = %s", (completed_at, status, canonical_json(report), markdown, run_id))
        self.connection.commit()

    def close(self) -> None:
        self.connection.close()


def changes_for(observation: Observation, previous: dict[str, Any] | None, old_metrics: dict[str, Any]) -> list[dict[str, Any]]:
    if previous is None:
        return []
    changes: list[dict[str, Any]] = []
    if observation.error_kind:
        if not previous["error_kind"]:
            changes.append({"kind": "source_failed", "field": f"sources.{observation.source_key}", "previous": "available", "current": observation.error_kind})
        return changes
    if previous["error_kind"]:
        changes.append({"kind": "source_recovered", "field": f"sources.{observation.source_key}", "previous": previous["error_kind"], "current": "available"})
    elif previous["content_sha256"] != observation.content_sha256:
        changes.append({"kind": "source_changed", "field": f"sources.{observation.source_key}.content_sha256", "previous": previous["content_sha256"], "current": observation.content_sha256})
    for key, value in observation.extracted.items():
        if key in old_metrics and old_metrics[key] != value:
            changes.append({"kind": "metric_changed", "field": f"metrics.{key}", "previous": old_metrics[key], "current": value})
    return changes


def report_markdown(report: dict[str, Any]) -> str:
    lines = ["# Direct competitor intelligence", "", f"- Generated: {report['completed_at']}", f"- Registry competitors: {report['competitor_count']}", f"- Sources checked: {report['source_count']}", f"- Source failures: {report['source_failures']}", f"- Detected changes: {len(report['changes'])}", "", "Only reviewed direct bounty-market competitors are included. Source changes prove that a public source changed, not why it changed or whether a platform-reported metric is independently audited."]
    if report["changes"]:
        lines.extend(["", "## Changes", ""])
        for change in report["changes"]:
            lines.append(f"- **{change['competitor']}** — `{change['kind']}` `{change['field']}`: `{change.get('previous')}` → `{change.get('current')}` ([evidence]({change['evidence_url']}))")
    else:
        lines.extend(["", "## Changes", "", "No comparable change was detected. The first successful run establishes a baseline."])
    if report["failures"]:
        lines.extend(["", "## Source failures", ""])
        lines.extend(f"- **{failure['competitor']}** — `{failure['source_key']}`: {failure['error_kind']} ({failure['error_message']})" for failure in report["failures"])
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--registry", type=Path, default=Path("ops/competitors/direct-competitors.json"))
    parser.add_argument("--database-url", default=None)
    parser.add_argument("--require-database", action="store_true")
    parser.add_argument("--json-out", type=Path, required=True)
    parser.add_argument("--markdown-out", type=Path, required=True)
    args = parser.parse_args()
    registry = json.loads(args.registry.read_text(encoding="utf-8"))
    validate_registry(registry)
    if args.require_database and not args.database_url:
        raise SystemExit("--require-database needs --database-url")
    registry_sha256 = hashlib.sha256(canonical_json(registry).encode()).hexdigest()
    started_at = utc_now()
    run_id = uuid.uuid4()
    writer = PostgresWriter(args.database_url) if args.database_url else None
    if writer:
        writer.begin(run_id, started_at, registry_sha256, registry)
    observations: list[Observation] = []
    changes: list[dict[str, Any]] = []
    try:
        for competitor in registry["competitors"]:
            for source in competitor["sources"]:
                previous, old_metrics = writer.previous(competitor["slug"], source["key"]) if writer else (None, {})
                observation = fetch_source(competitor["slug"], source, started_at)
                observations.append(observation)
                for change in changes_for(observation, previous, old_metrics):
                    payload = {**change, "competitor": competitor["brand_name"], "evidence_url": observation.source_url}
                    changes.append(payload)
                    if writer:
                        writer.save_change(run_id, observation, change)
                if writer:
                    writer.save_observation(run_id, observation)
        completed_at = utc_now()
        failures = [{"competitor": item.competitor_slug, "source_key": item.source_key, "error_kind": item.error_kind, "error_message": item.error_message} for item in observations if item.error_kind]
        report = {"schema_version": "agent-bounties/direct-competitor-daily-report-v1", "run_id": str(run_id), "started_at": started_at.isoformat(), "completed_at": completed_at.isoformat(), "registry_sha256": registry_sha256, "competitor_count": len(registry["competitors"]), "source_count": len(observations), "source_failures": len(failures), "changes": changes, "failures": failures, "observations": [{"competitor": item.competitor_slug, "source_key": item.source_key, "source_url": item.source_url, "http_status": item.http_status, "content_sha256": item.content_sha256, "metrics": item.extracted, "error_kind": item.error_kind} for item in observations]}
        markdown = report_markdown(report)
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        args.markdown_out.parent.mkdir(parents=True, exist_ok=True)
        args.markdown_out.write_text(markdown, encoding="utf-8")
        if writer:
            writer.finish(run_id, completed_at, "completed_with_failures" if failures else "completed", report, markdown)
        return 0
    except Exception:
        if writer:
            failed_at = utc_now()
            writer.finish(run_id, failed_at, "failed", {"error": "collector failed; inspect workflow logs"}, "# Direct competitor intelligence\n\nCollector failed; inspect workflow logs.\n")
        raise
    finally:
        if writer:
            writer.close()


if __name__ == "__main__":
    sys.exit(main())
