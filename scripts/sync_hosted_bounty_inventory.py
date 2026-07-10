#!/usr/bin/env python3
"""Plan or execute idempotent GitHub issue to hosted bounty reconstruction.

Dry-run is the default. Execution writes bounty metadata only; it cannot fund,
claim, accept, release escrow, or settle payment.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Mapping


USER_AGENT = "agent-bounties-hosted-inventory-sync/1"
REQUIRED_LLMS_MARKERS = (
    "/v1/github/issue-api-sync",
    "more and higher-value funded bounties",
)
BOUNDARIES = (
    "This tool creates or updates hosted bounty metadata only.",
    "It does not fund a bounty or make an unfunded bounty claimable.",
    "It does not reserve a claim, accept work, authorize payout, release escrow, or prove settlement.",
)


class InventorySyncError(RuntimeError):
    pass


@dataclass(frozen=True)
class HttpResult:
    status: int
    body: Any
    headers: Mapping[str, str]


HttpRequest = Callable[[str, str, Any | None, Mapping[str, str] | None], HttpResult]


def normalize_api_base_url(value: str) -> str:
    parsed = urllib.parse.urlsplit(value.strip())
    if parsed.scheme not in {"http", "https"} or not parsed.netloc:
        raise InventorySyncError("api base URL must be an absolute http(s) URL")
    if parsed.query or parsed.fragment or parsed.username or parsed.password:
        raise InventorySyncError("api base URL cannot contain credentials, query, or fragment")
    host = (parsed.hostname or "").lower()
    if parsed.scheme != "https" and host not in {"localhost", "127.0.0.1", "::1"}:
        raise InventorySyncError("non-local hosted execution requires https")
    path = parsed.path.rstrip("/")
    return urllib.parse.urlunsplit((parsed.scheme, parsed.netloc, path, "", ""))


def default_http_request(
    method: str,
    url: str,
    body: Any | None,
    headers: Mapping[str, str] | None,
) -> HttpResult:
    request_headers = {
        "Accept": "application/json, text/plain;q=0.9",
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
                status=response.status,
                body=decode_response_body(raw, response.headers.get("Content-Type", "")),
                headers=dict(response.headers.items()),
            )
    except urllib.error.HTTPError as error:
        raw = error.read().decode("utf-8", errors="replace")
        return HttpResult(
            status=error.code,
            body=decode_response_body(raw, error.headers.get("Content-Type", "")),
            headers=dict(error.headers.items()),
        )
    except urllib.error.URLError as error:
        raise InventorySyncError(f"request failed for {url}: {error.reason}") from error


def decode_response_body(raw: str, content_type: str) -> Any:
    if "json" in content_type.lower() or raw.lstrip().startswith(("{", "[")):
        try:
            return json.loads(raw)
        except json.JSONDecodeError:
            pass
    return raw


def github_headers(token: str | None) -> dict[str, str]:
    headers = {
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    }
    if token:
        headers["Authorization"] = f"Bearer {token}"
    return headers


def fetch_github_issue(
    repository: str,
    issue_number: int,
    request: HttpRequest,
    token: str | None,
) -> dict[str, Any]:
    owner, separator, repo = repository.partition("/")
    if not separator or not owner or not repo:
        raise InventorySyncError(f"invalid repository: {repository!r}")
    url = f"https://api.github.com/repos/{owner}/{repo}/issues/{issue_number}"
    result = request("GET", url, None, github_headers(token))
    if result.status != 200 or not isinstance(result.body, dict):
        raise InventorySyncError(
            f"GitHub issue #{issue_number} lookup returned HTTP {result.status}"
        )
    return result.body


def load_issue_fixture(path: Path) -> list[dict[str, Any]]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    issues = payload.get("issues") if isinstance(payload, dict) else payload
    if not isinstance(issues, list) or not all(isinstance(item, dict) for item in issues):
        raise InventorySyncError("issue fixture must be a JSON list or {\"issues\": [...]} object")
    return issues


def issue_label_names(issue: Mapping[str, Any]) -> set[str]:
    labels = issue.get("labels") or []
    names: set[str] = set()
    for label in labels:
        if isinstance(label, str):
            names.add(label.lower())
        elif isinstance(label, dict) and label.get("name"):
            names.add(str(label["name"]).lower())
    return names


def validate_source_issue(issue: Mapping[str, Any], repository: str) -> None:
    number = issue.get("number")
    if not isinstance(number, int) or number <= 0:
        raise InventorySyncError("source issue is missing a positive integer number")
    if str(issue.get("state", "")).lower() != "open":
        raise InventorySyncError(f"source issue #{number} is not open")
    if issue.get("pull_request") is not None:
        raise InventorySyncError(f"source issue #{number} is a pull request")
    if "bounty" not in issue_label_names(issue):
        raise InventorySyncError(f"source issue #{number} is missing the bounty label")
    expected_url = f"https://github.com/{repository}/issues/{number}"
    if issue.get("html_url") != expected_url:
        raise InventorySyncError(
            f"source issue #{number} URL must be the canonical repository issue URL"
        )
    if not str(issue.get("title") or "").strip() or not str(issue.get("body") or "").strip():
        raise InventorySyncError(f"source issue #{number} needs a title and body")


def sync_request_body(
    issue: Mapping[str, Any],
    repository: str,
    api_base_url: str,
    existing_bounty_ids: list[str] | None = None,
    hosted_api_error: str | None = None,
) -> dict[str, Any]:
    return {
        "repository": repository,
        "issue_url": issue["html_url"],
        "title": issue["title"],
        "body": issue["body"],
        "api_base_url": api_base_url,
        "existing_bounty_ids": existing_bounty_ids or [],
        "hosted_api_error": hosted_api_error,
    }


def require_plan_shape(plan: Any, target_api_base_url: str) -> dict[str, Any]:
    if not isinstance(plan, dict):
        raise InventorySyncError("hosted planner did not return a JSON object")
    if not plan.get("ready"):
        return plan
    bounty_id = plan.get("bounty_id")
    calls = plan.get("calls")
    if not isinstance(bounty_id, str) or not isinstance(calls, list) or len(calls) != 1:
        raise InventorySyncError("ready planner response lacks one stable bounty call")
    call = calls[0]
    expected_url = f"{target_api_base_url}/v1/github/issue-api-sync"
    if not isinstance(call, dict) or call.get("method") != "POST" or call.get("url") != expected_url:
        raise InventorySyncError("planner returned an unexpected hosted write target")
    if call.get("settlement_authority") is not False:
        raise InventorySyncError("inventory sync plan must explicitly lack settlement authority")
    if not str(plan.get("idempotency_key") or "").startswith("github-issue-sync:"):
        raise InventorySyncError("planner returned an invalid issue-sync idempotency key")
    return plan


def call_planner(
    request: HttpRequest,
    planner_base_url: str,
    target_api_base_url: str,
    payload: Mapping[str, Any],
) -> dict[str, Any]:
    result = request(
        "POST",
        f"{planner_base_url}/v1/github/issue-api-sync-plan",
        dict(payload),
        None,
    )
    if result.status != 200:
        raise InventorySyncError(f"hosted sync planner returned HTTP {result.status}")
    return require_plan_shape(result.body, target_api_base_url)


def server_preflight(request: HttpRequest, api_base_url: str) -> dict[str, Any]:
    health = request("GET", f"{api_base_url}/health", None, None)
    health_text = health.body if isinstance(health.body, str) else ""
    llms = request("GET", f"{api_base_url}/llms.txt", None, None)
    llms_text = llms.body if isinstance(llms.body, str) else ""
    marker_results = {marker: marker in llms_text for marker in REQUIRED_LLMS_MARKERS}
    return {
        "health_status": health.status,
        "health_ok": health.status == 200 and health_text.strip() == "ok",
        "llms_status": llms.status,
        "llms_markers": marker_results,
        "ready_for_execute": (
            health.status == 200
            and health_text.strip() == "ok"
            and llms.status == 200
            and all(marker_results.values())
        ),
    }


def plan_issue(
    issue: Mapping[str, Any],
    repository: str,
    target_api_base_url: str,
    request: HttpRequest,
    planner_base_url: str | None = None,
) -> dict[str, Any]:
    planner_base_url = planner_base_url or target_api_base_url
    validate_source_issue(issue, repository)
    initial_payload = sync_request_body(issue, repository, target_api_base_url)
    initial_plan = call_planner(
        request, planner_base_url, target_api_base_url, initial_payload
    )
    bounty_id = initial_plan.get("bounty_id")
    if not initial_plan.get("ready") or not isinstance(bounty_id, str):
        return report_entry(issue, initial_plan, None, initial_payload)

    status_url = f"{target_api_base_url}/v1/bounties/{bounty_id}"
    status = request("GET", status_url, None, None)
    existing_ids: list[str] = []
    hosted_error = None
    status_state = "absent"
    if status.status == 200:
        if not isinstance(status.body, dict):
            hosted_error = "status endpoint returned non-JSON body"
            status_state = "error"
        elif str((status.body.get("bounty") or {}).get("id")) != bounty_id:
            hosted_error = "status endpoint returned a different bounty id"
            status_state = "error"
        else:
            existing_ids = [bounty_id]
            status_state = "existing"
    elif status.status != 404:
        hosted_error = f"GET /v1/bounties/{bounty_id} returned {status.status}"
        status_state = "error"

    final_payload = sync_request_body(
        issue,
        repository,
        target_api_base_url,
        existing_bounty_ids=existing_ids,
        hosted_api_error=hosted_error,
    )
    final_plan = call_planner(
        request, planner_base_url, target_api_base_url, final_payload
    )
    status_report = {
        "url": status_url,
        "http_status": status.status,
        "state": status_state,
    }
    return report_entry(issue, final_plan, status_report, final_payload)


def report_entry(
    issue: Mapping[str, Any],
    plan: Mapping[str, Any],
    status_probe: Mapping[str, Any] | None,
    execution_payload: Mapping[str, Any],
) -> dict[str, Any]:
    call = (plan.get("calls") or [None])[0]
    return {
        "issue_number": issue["number"],
        "issue_url": issue["html_url"],
        "title": issue["title"],
        "source_updated_at": issue.get("updated_at"),
        "source_body_sha256": hashlib.sha256(str(issue["body"]).encode("utf-8")).hexdigest(),
        "ready": bool(plan.get("ready")),
        "operation": plan.get("operation"),
        "bounty_id": plan.get("bounty_id"),
        "idempotency_key": plan.get("idempotency_key"),
        "status_url": plan.get("status_url"),
        "public_bounty_url": plan.get("public_bounty_url"),
        "funding_page_url": plan.get("funding_page_url"),
        "status_probe": dict(status_probe) if status_probe else None,
        "settlement_authority": call.get("settlement_authority") if isinstance(call, dict) else None,
        "error": plan.get("error"),
        "execution": {"attempted": False, "status": "not-requested"},
        "_execution_payload": dict(execution_payload),
    }


def public_report(report: Mapping[str, Any]) -> dict[str, Any]:
    sanitized = json.loads(json.dumps(report))
    for entry in sanitized.get("entries", []):
        entry.pop("_execution_payload", None)
    return sanitized


def execute_entries(
    report: dict[str, Any],
    api_base_url: str,
    operator_token: str,
    request: HttpRequest,
) -> bool:
    all_ok = True
    headers = {"Authorization": f"Bearer {operator_token}"}
    for entry in report["entries"]:
        execution = entry["execution"]
        if not entry.get("ready"):
            execution.update(attempted=False, status="blocked-by-plan")
            all_ok = False
            break
        execution["attempted"] = True
        try:
            result = request(
                "POST",
                f"{api_base_url}/v1/github/issue-api-sync",
                entry["_execution_payload"],
                headers,
            )
        except InventorySyncError as error:
            execution["status"] = "write-request-failed"
            execution["error"] = str(error)
            all_ok = False
            break
        execution["http_status"] = result.status
        if result.status != 200 or not isinstance(result.body, dict):
            execution["status"] = "write-failed"
            all_ok = False
            break
        if str(result.body.get("id")) != entry.get("bounty_id"):
            execution["status"] = "write-returned-wrong-bounty"
            all_ok = False
            break
        try:
            verification = request("GET", str(entry["status_url"]), None, None)
        except InventorySyncError as error:
            execution["status"] = "post-write-verification-request-failed"
            execution["error"] = str(error)
            all_ok = False
            break
        execution["verification_http_status"] = verification.status
        verified_bounty = (
            verification.body.get("bounty")
            if verification.status == 200 and isinstance(verification.body, dict)
            else None
        )
        if not isinstance(verified_bounty, dict) or str(verified_bounty.get("id")) != entry.get(
            "bounty_id"
        ):
            execution["status"] = "post-write-verification-failed"
            all_ok = False
            break
        execution["status"] = "metadata-synced"
        execution["bounty_status"] = verified_bounty.get("status")
    return all_ok


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", default="NSPG13/agent-bounties")
    parser.add_argument("--issue", action="append", type=int, default=[])
    parser.add_argument("--fixture", type=Path)
    parser.add_argument(
        "--api-base-url",
        default=os.environ.get("AGENT_BOUNTIES_API_BASE_URL"),
    )
    parser.add_argument(
        "--planner-base-url",
        default=os.environ.get("AGENT_BOUNTIES_PLANNER_BASE_URL"),
        help="Optional trusted planner service. Plans still target --api-base-url.",
    )
    parser.add_argument("--output", type=Path)
    parser.add_argument("--execute", action="store_true")
    parser.add_argument(
        "--confirm-api-base-url",
        help="Required with --execute and must exactly match --api-base-url.",
    )
    parser.add_argument(
        "--confirm-issue-count",
        type=int,
        help="Required with --execute and must equal the number of live GitHub issues selected.",
    )
    parser.add_argument(
        "--operator-token-env",
        default="AGENT_BOUNTIES_OPERATOR_TOKEN",
        help="Environment variable containing the operator token; never pass the token as an argument.",
    )
    return parser.parse_args(argv)


def write_report(report: Mapping[str, Any], output: Path | None) -> None:
    rendered = json.dumps(public_report(report), indent=2, sort_keys=True) + "\n"
    sys.stdout.write(rendered)
    if output:
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(rendered, encoding="utf-8")


def main(argv: list[str] | None = None, request: HttpRequest = default_http_request) -> int:
    args = parse_args(argv)
    if not args.api_base_url:
        raise InventorySyncError(
            "set --api-base-url or AGENT_BOUNTIES_API_BASE_URL"
        )
    api_base_url = normalize_api_base_url(args.api_base_url)
    planner_base_url = normalize_api_base_url(args.planner_base_url or api_base_url)
    if args.execute:
        if args.fixture:
            raise InventorySyncError("--execute cannot use an issue fixture")
        if planner_base_url != api_base_url:
            raise InventorySyncError(
                "--execute requires the planner and target API base URLs to match"
            )
        confirmed_url = normalize_api_base_url(args.confirm_api_base_url or "")
        if confirmed_url != api_base_url:
            raise InventorySyncError(
                "--confirm-api-base-url must exactly match the normalized API base URL"
            )
    if args.fixture:
        issues = load_issue_fixture(args.fixture)
        if args.issue:
            selected = set(args.issue)
            issues = [issue for issue in issues if issue.get("number") in selected]
    else:
        if not args.issue:
            raise InventorySyncError("pass at least one --issue or use --fixture")
        github_token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
        issues = [
            fetch_github_issue(args.repository, number, request, github_token)
            for number in args.issue
        ]
    if not issues:
        raise InventorySyncError("no source issues selected")
    if args.execute and args.confirm_issue_count != len(issues):
        raise InventorySyncError(
            "--confirm-issue-count must equal the number of live GitHub issues selected"
        )

    preflight = server_preflight(request, api_base_url)
    entries = [
        plan_issue(
            issue,
            args.repository,
            api_base_url,
            request,
            planner_base_url=planner_base_url,
        )
        for issue in issues
    ]
    report: dict[str, Any] = {
        "schema_version": 1,
        "mode": "execute" if args.execute else "dry-run",
        "repository": args.repository,
        "api_base_url": api_base_url,
        "planner_base_url": planner_base_url,
        "server_preflight": preflight,
        "source_issue_count": len(issues),
        "all_plans_ready": all(entry["ready"] for entry in entries),
        "entries": entries,
        "boundaries": list(BOUNDARIES),
    }

    exit_code = 0
    if args.execute:
        token = os.environ.get(args.operator_token_env, "").strip()
        if not token:
            raise InventorySyncError(
                f"set operator token in {args.operator_token_env} before --execute"
            )
        if not preflight["ready_for_execute"]:
            raise InventorySyncError("hosted server preflight is not ready for execution")
        if not report["all_plans_ready"]:
            raise InventorySyncError("one or more issue sync plans are blocked")
        if not execute_entries(report, api_base_url, token, request):
            exit_code = 2

    write_report(report, args.output)
    return exit_code


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except InventorySyncError as error:
        print(f"hosted inventory sync blocked: {error}", file=sys.stderr)
        raise SystemExit(2) from error
