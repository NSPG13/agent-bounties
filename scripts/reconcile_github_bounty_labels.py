#!/usr/bin/env python3
"""Mirror canonical Base bounty states into non-authoritative GitHub labels.

Dry-run is the default. Execution can only add or remove managed issue labels;
it cannot fund, claim, verify, settle, or otherwise call a bounty contract.
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
from pathlib import Path
from typing import Any, Callable, Mapping


USER_AGENT = "agent-bounties-github-label-reconciler/1"
ADDRESS = re.compile(r"^0x[0-9a-fA-F]{40}$")
TX_HASH = re.compile(r"^0x[0-9a-fA-F]{64}$")
KNOWN_STATUSES = frozenset(
    {"open", "claimable", "claimed", "submitted", "paid", "cancelled"}
)
MANAGED_LABELS = frozenset(
    {
        "funded-live",
        "claimable-live",
        "claimed-live",
        "settled-paid",
        "verification-unavailable",
    }
)
BOUNDARIES = (
    "GitHub labels mirror canonical indexed state for discovery only.",
    "A label cannot fund, claim, verify, accept, release, or settle a bounty.",
    "Only a confirmed canonical BountySettled event proves payment.",
)


class LabelReconciliationError(RuntimeError):
    pass


@dataclass(frozen=True)
class HttpResult:
    status: int
    body: Any
    headers: Mapping[str, str]


@dataclass(frozen=True)
class LabelPlan:
    issue_number: int
    issue_url: str
    issue_state: str
    bounty_contract: str | None
    canonical_status: str | None
    verification_ready: bool | None
    current_managed_labels: list[str]
    desired_managed_labels: list[str]
    add_labels: list[str]
    remove_labels: list[str]


HttpRequest = Callable[[str, str, Any | None, Mapping[str, str] | None], HttpResult]


def normalize_api_base_url(value: str) -> str:
    parsed = urllib.parse.urlsplit(value.strip())
    if parsed.scheme not in {"http", "https"} or not parsed.netloc:
        raise LabelReconciliationError("API base URL must be an absolute http(s) URL")
    if parsed.query or parsed.fragment or parsed.username or parsed.password:
        raise LabelReconciliationError(
            "API base URL cannot contain credentials, query, or fragment"
        )
    host = (parsed.hostname or "").lower()
    if parsed.scheme != "https" and host not in {"localhost", "127.0.0.1", "::1"}:
        raise LabelReconciliationError("non-local API execution requires https")
    return urllib.parse.urlunsplit(
        (parsed.scheme, parsed.netloc, parsed.path.rstrip("/"), "", "")
    )


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
        raise LabelReconciliationError(
            f"request failed for {url}: {error.reason}"
        ) from error


def github_headers(token: str | None) -> dict[str, str]:
    headers = {
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    }
    if token:
        headers["Authorization"] = f"Bearer {token}"
    return headers


def fetch_json_list(request: HttpRequest, url: str) -> list[dict[str, Any]]:
    result = request("GET", url, None, None)
    if result.status != 200 or not isinstance(result.body, list) or not all(
        isinstance(item, dict) for item in result.body
    ):
        raise LabelReconciliationError(f"canonical feed returned HTTP {result.status}")
    return result.body


def fetch_canonical_feeds(
    request: HttpRequest, api_base_url: str, network: str
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    health = request("GET", f"{api_base_url}/health", None, None)
    if health.status != 200 or str(health.body).strip() != "ok":
        raise LabelReconciliationError("hosted API health is not confirmed")
    query = urllib.parse.urlencode({"network": network})
    full = fetch_json_list(
        request, f"{api_base_url}/v1/base/autonomous-bounties/feed?{query}"
    )
    claimable_query = urllib.parse.urlencode(
        {"network": network, "claimable_only": "true"}
    )
    claimable = fetch_json_list(
        request,
        f"{api_base_url}/v1/base/autonomous-bounties/feed?{claimable_query}",
    )
    return full, claimable


def fetch_github_issues(
    request: HttpRequest, repository: str, token: str | None
) -> list[dict[str, Any]]:
    issues: list[dict[str, Any]] = []
    headers = github_headers(token)
    for page in range(1, 21):
        query = urllib.parse.urlencode(
            {"state": "all", "per_page": "100", "page": str(page)}
        )
        url = f"https://api.github.com/repos/{repository}/issues?{query}"
        result = request("GET", url, None, headers)
        if result.status != 200 or not isinstance(result.body, list):
            raise LabelReconciliationError(
                f"GitHub issue listing returned HTTP {result.status}"
            )
        batch = [item for item in result.body if isinstance(item, dict)]
        issues.extend(batch)
        if len(batch) < 100:
            return issues
    raise LabelReconciliationError("GitHub issue listing exceeded 2000 records")


def label_names(issue: Mapping[str, Any]) -> set[str]:
    names: set[str] = set()
    for label in issue.get("labels") or []:
        if isinstance(label, str):
            names.add(label.lower())
        elif isinstance(label, dict) and label.get("name"):
            names.add(str(label["name"]).lower())
    return names


def source_issue_url(item: Mapping[str, Any], repository: str) -> str | None:
    terms = item.get("terms")
    document = terms.get("document") if isinstance(terms, dict) else None
    source = document.get("source_url") if isinstance(document, dict) else None
    if source is None:
        return None
    try:
        parsed = urllib.parse.urlsplit(str(source))
    except ValueError as error:
        raise LabelReconciliationError("canonical source URL is malformed") from error
    expected_prefix = f"/{repository}/issues/"
    if parsed.scheme != "https" or parsed.hostname != "github.com":
        return None
    if parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise LabelReconciliationError("GitHub source URL must be credential-free and exact")
    if not parsed.path.startswith(expected_prefix):
        return None
    suffix = parsed.path[len(expected_prefix) :]
    if not suffix.isdigit() or int(suffix) <= 0:
        raise LabelReconciliationError("GitHub source URL lacks a positive issue number")
    return f"https://github.com/{repository}/issues/{int(suffix)}"


def require_amount(item: Mapping[str, Any], field: str) -> int:
    value = item.get(field)
    if isinstance(value, int) and not isinstance(value, bool) and value >= 0:
        return value
    if isinstance(value, str) and re.fullmatch(r"0|[1-9][0-9]*", value):
        return int(value)
    raise LabelReconciliationError(f"canonical item has invalid {field}")


def validate_state_evidence(item: Mapping[str, Any], status: str, contract: str) -> None:
    if status not in {"claimed", "submitted", "paid"}:
        return
    expected_kinds = {
        "claimed": {"bounty_claimed"},
        "submitted": {"bounty_claimed", "submission_added"},
        "paid": {"bounty_settled"},
    }[status]
    events = item.get("events")
    if not isinstance(events, list):
        raise LabelReconciliationError(
            f"canonical {status} item lacks an event list: {contract}"
        )
    observed_kinds = {
        str(event.get("kind"))
        for event in events
        if isinstance(event, dict)
        and str(event.get("contract_address") or "").lower() == contract
        and TX_HASH.fullmatch(str(event.get("tx_hash") or ""))
    }
    if not expected_kinds.issubset(observed_kinds):
        raise LabelReconciliationError(
            f"canonical {status} item lacks confirmed "
            f"{','.join(sorted(expected_kinds))} evidence: {contract}"
        )


def canonical_records(
    full_feed: list[dict[str, Any]],
    claimable_feed: list[dict[str, Any]],
    repository: str,
) -> tuple[dict[str, dict[str, Any]], set[tuple[str, str]]]:
    by_contract: dict[str, dict[str, Any]] = {}
    by_issue_url: dict[str, dict[str, Any]] = {}
    for item in full_feed:
        contract = str(item.get("bounty_contract") or "").lower()
        status = str(item.get("status") or "").lower()
        if not ADDRESS.fullmatch(contract) or status not in KNOWN_STATUSES:
            raise LabelReconciliationError("canonical full feed has an invalid contract or status")
        if contract in by_contract:
            raise LabelReconciliationError(f"duplicate canonical contract: {contract}")
        target = require_amount(item, "target_amount")
        funded = require_amount(item, "funded_amount")
        if target <= 0 or funded > target:
            raise LabelReconciliationError(f"invalid canonical economics: {contract}")
        if status in {"claimable", "claimed", "submitted", "paid"} and funded != target:
            raise LabelReconciliationError(f"canonical {status} item is not fully funded: {contract}")
        validate_state_evidence(item, status, contract)
        source = source_issue_url(item, repository)
        normalized = dict(item)
        normalized["bounty_contract"] = contract
        normalized["status"] = status
        normalized["_source_issue_url"] = source
        by_contract[contract] = normalized
        if source:
            if source in by_issue_url:
                raise LabelReconciliationError(
                    f"multiple canonical contracts reference {source}"
                )
            by_issue_url[source] = normalized

    earning: set[tuple[str, str]] = set()
    for item in claimable_feed:
        contract = str(item.get("bounty_contract") or "").lower()
        source = source_issue_url(item, repository)
        counterpart = by_contract.get(contract)
        if counterpart is None:
            raise LabelReconciliationError(
                f"earning feed contract is absent from full feed: {contract}"
            )
        pair = (source or "", contract)
        valid = (
            source == counterpart["_source_issue_url"]
            and counterpart["status"] == "claimable"
            and counterpart.get("terms_valid") is True
            and counterpart.get("verification_ready") is True
            and str(item.get("status") or "").lower() == "claimable"
            and item.get("terms_valid") is True
            and item.get("verification_ready") is True
        )
        if not valid:
            raise LabelReconciliationError(
                f"earning feed item is not an exact executable full-feed record: {contract}"
            )
        if pair in earning:
            raise LabelReconciliationError(f"duplicate earning feed item: {contract}")
        earning.add(pair)
    return by_issue_url, earning


def desired_labels(
    record: Mapping[str, Any] | None, earning: set[tuple[str, str]]
) -> set[str]:
    if record is None:
        return set()
    status = str(record["status"])
    contract = str(record["bounty_contract"])
    source = str(record.get("_source_issue_url") or "")
    ready = record.get("verification_ready") is True and record.get("terms_valid") is True
    if status == "paid":
        return {"settled-paid"}
    if status == "claimable":
        labels = {"funded-live"}
        if ready and (source, contract) in earning:
            labels.add("claimable-live")
        else:
            labels.add("verification-unavailable")
        return labels
    if status in {"claimed", "submitted"}:
        labels = {"funded-live", "claimed-live"}
        if not ready:
            labels.add("verification-unavailable")
        return labels
    return set()


def build_plans(
    issues: list[dict[str, Any]],
    full_feed: list[dict[str, Any]],
    claimable_feed: list[dict[str, Any]],
    repository: str,
) -> list[LabelPlan]:
    records, earning = canonical_records(full_feed, claimable_feed, repository)
    plans: list[LabelPlan] = []
    seen_urls: set[str] = set()
    for issue in issues:
        if issue.get("pull_request") is not None:
            continue
        number = issue.get("number")
        url = str(issue.get("html_url") or "")
        if not isinstance(number, int) or number <= 0:
            raise LabelReconciliationError("GitHub issue lacks a positive number")
        expected_url = f"https://github.com/{repository}/issues/{number}"
        if url != expected_url or url in seen_urls:
            raise LabelReconciliationError("GitHub issue listing contains invalid or duplicate URLs")
        seen_urls.add(url)
        current = label_names(issue) & MANAGED_LABELS
        record = records.get(url)
        if record is None and not current:
            continue
        desired = desired_labels(record, earning)
        plans.append(
            LabelPlan(
                issue_number=number,
                issue_url=url,
                issue_state=str(issue.get("state") or "unknown").lower(),
                bounty_contract=(str(record["bounty_contract"]) if record else None),
                canonical_status=(str(record["status"]) if record else None),
                verification_ready=(
                    record.get("verification_ready") is True if record else None
                ),
                current_managed_labels=sorted(current),
                desired_managed_labels=sorted(desired),
                add_labels=sorted(desired - current),
                remove_labels=sorted(current - desired),
            )
        )
    missing = sorted(set(records) - seen_urls)
    if missing:
        raise LabelReconciliationError(
            f"canonical feed references GitHub issues absent from listing: {', '.join(missing)}"
        )
    return plans


def execute_plans(
    plans: list[LabelPlan],
    repository: str,
    token: str,
    request: HttpRequest,
) -> list[dict[str, Any]]:
    headers = github_headers(token)
    results: list[dict[str, Any]] = []
    for plan in plans:
        if not plan.add_labels and not plan.remove_labels:
            continue
        base = f"https://api.github.com/repos/{repository}/issues/{plan.issue_number}"
        for label in plan.remove_labels:
            encoded = urllib.parse.quote(label, safe="")
            response = request("DELETE", f"{base}/labels/{encoded}", None, headers)
            if response.status not in {200, 204, 404}:
                raise LabelReconciliationError(
                    f"failed to remove {label} from issue #{plan.issue_number}: HTTP {response.status}"
                )
        if plan.add_labels:
            response = request(
                "POST", f"{base}/labels", {"labels": plan.add_labels}, headers
            )
            if response.status != 200:
                raise LabelReconciliationError(
                    f"failed to add labels to issue #{plan.issue_number}: HTTP {response.status}"
                )
        verification = request("GET", base, None, headers)
        if verification.status != 200 or not isinstance(verification.body, dict):
            raise LabelReconciliationError(
                f"failed to verify issue #{plan.issue_number}: HTTP {verification.status}"
            )
        actual = label_names(verification.body) & MANAGED_LABELS
        expected = set(plan.desired_managed_labels)
        if actual != expected:
            raise LabelReconciliationError(
                f"post-write labels do not match canonical plan for issue #{plan.issue_number}"
            )
        results.append(
            {
                "issue_number": plan.issue_number,
                "status": "reconciled",
                "managed_labels": sorted(actual),
            }
        )
    return results


def load_fixture(path: Path) -> tuple[list[dict[str, Any]], list[dict[str, Any]], list[dict[str, Any]]]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(payload, dict):
        raise LabelReconciliationError("fixture must be a JSON object")
    values = tuple(payload.get(key) for key in ("issues", "full_feed", "claimable_feed"))
    if not all(
        isinstance(value, list) and all(isinstance(item, dict) for item in value)
        for value in values
    ):
        raise LabelReconciliationError(
            "fixture requires issues, full_feed, and claimable_feed arrays"
        )
    return values  # type: ignore[return-value]


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--repository", default=os.environ.get("GITHUB_REPOSITORY", "NSPG13/agent-bounties")
    )
    parser.add_argument(
        "--api-base-url",
        default=os.environ.get(
            "AGENT_BOUNTIES_API_BASE_URL", "https://agent-bounties-api.onrender.com"
        ),
    )
    parser.add_argument("--network", default="base-mainnet")
    parser.add_argument("--fixture", type=Path)
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--confirm-repository")
    parser.add_argument("--json-out", type=Path)
    parser.add_argument("--md-out", type=Path)
    return parser.parse_args(argv)


def render_markdown(report: Mapping[str, Any]) -> str:
    lines = [
        "# Canonical GitHub bounty-label reconciliation",
        "",
        f"- Mode: `{report['mode']}`",
        f"- Canonical records: **{report['canonical_record_count']}**",
        f"- Managed issues: **{report['managed_issue_count']}**",
        f"- Drifted issues: **{report['drift_count']}**",
        f"- Executed changes: **{len(report['execution_results'])}**",
        "",
        "## Drift",
    ]
    drifted = [
        plan for plan in report["plans"] if plan["add_labels"] or plan["remove_labels"]
    ]
    if not drifted:
        lines.append("- None")
    for plan in drifted:
        lines.append(
            f"- #{plan['issue_number']} `{plan['canonical_status'] or 'unmapped'}`: "
            f"add `{','.join(plan['add_labels']) or '-'}`; "
            f"remove `{','.join(plan['remove_labels']) or '-'}`"
        )
    lines.extend(["", "## Boundaries"])
    lines.extend(f"- {boundary}" for boundary in BOUNDARIES)
    return "\n".join(lines) + "\n"


def write_report(report: Mapping[str, Any], json_out: Path | None, md_out: Path | None) -> None:
    rendered_json = json.dumps(report, indent=2, sort_keys=True) + "\n"
    rendered_md = render_markdown(report)
    sys.stdout.write(rendered_md)
    if json_out:
        json_out.parent.mkdir(parents=True, exist_ok=True)
        json_out.write_text(rendered_json, encoding="utf-8")
    if md_out:
        md_out.parent.mkdir(parents=True, exist_ok=True)
        md_out.write_text(rendered_md, encoding="utf-8")


def main(argv: list[str] | None = None, request: HttpRequest = default_http_request) -> int:
    args = parse_args(argv)
    repository = validate_repository(args.repository)
    api_base_url = normalize_api_base_url(args.api_base_url)
    if args.execute:
        if args.fixture:
            raise LabelReconciliationError("--execute cannot use a fixture")
        if validate_repository(args.confirm_repository or "") != repository:
            raise LabelReconciliationError(
                "--confirm-repository must exactly match --repository"
            )
    if args.fixture:
        issues, full_feed, claimable_feed = load_fixture(args.fixture)
    else:
        full_feed, claimable_feed = fetch_canonical_feeds(
            request, api_base_url, args.network
        )
        token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
        issues = fetch_github_issues(request, repository, token)

    plans = build_plans(issues, full_feed, claimable_feed, repository)
    drift = [plan for plan in plans if plan.add_labels or plan.remove_labels]
    execution_results: list[dict[str, Any]] = []
    if args.execute:
        token = (os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN") or "").strip()
        if not token:
            raise LabelReconciliationError("GITHUB_TOKEN or GH_TOKEN is required for --execute")
        execution_results = execute_plans(plans, repository, token, request)

    report = {
        "schema_version": 1,
        "mode": "execute" if args.execute else "dry-run",
        "repository": repository,
        "api_base_url": api_base_url,
        "network": args.network,
        "canonical_record_count": len(full_feed),
        "claimable_record_count": len(claimable_feed),
        "managed_issue_count": len(plans),
        "drift_count": len(drift),
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
        print(f"GitHub bounty-label reconciliation blocked: {error}", file=sys.stderr)
        raise SystemExit(2) from error
