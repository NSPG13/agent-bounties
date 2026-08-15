#!/usr/bin/env python3
"""Publish canonically active Beta2 seed competitions as idempotent GitHub issues."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
from typing import Any
from urllib.error import HTTPError
from urllib.parse import urlencode
from urllib.request import Request, urlopen


class PublishError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise PublishError(message)


def request_json(method: str, url: str, token: str, body: dict[str, Any] | None = None) -> Any:
    payload = None if body is None else json.dumps(body, separators=(",", ":")).encode()
    request = Request(
        url,
        data=payload,
        method=method,
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "X-GitHub-Api-Version": "2022-11-28",
            **({"Content-Type": "application/json"} if payload is not None else {}),
        },
    )
    try:
        with urlopen(request, timeout=60) as response:
            return json.loads(response.read())
    except HTTPError as error:
        detail = error.read().decode("utf-8", errors="replace")
        raise PublishError(f"GitHub {method} {url} returned HTTP {error.code}: {detail}") from error


def existing_issues(repository: str, token: str) -> list[dict[str, Any]]:
    issues: list[dict[str, Any]] = []
    for page in range(1, 11):
        query = urlencode({"state": "all", "per_page": 100, "page": page})
        batch = request_json("GET", f"https://api.github.com/repos/{repository}/issues?{query}", token)
        require(isinstance(batch, list), "GitHub issue list response is invalid")
        issues.extend(issue for issue in batch if "pull_request" not in issue)
        if len(batch) < 100:
            break
    return issues


def publish(index_path: Path, repository: str, token: str) -> dict[str, Any]:
    require("/" in repository and not repository.startswith("/"), "repository must be owner/name")
    documents = json.loads(index_path.read_text(encoding="utf-8"))
    require(isinstance(documents, list) and len(documents) == 5, "issue index must contain exactly five competitions")
    issues = existing_issues(repository, token)
    results = []
    for document in documents:
        body_path = Path(document["body_path"])
        require(body_path.is_file(), f"missing issue body {body_path}")
        body = body_path.read_text(encoding="utf-8")
        marker = f"<!-- beta2-seed:{document['seed_id']}:{document['competition']} -->"
        require(marker in body, f"issue body does not bind {document['seed_id']} to its contract")
        matches = [issue for issue in issues if marker in (issue.get("body") or "")]
        require(len(matches) <= 1, f"multiple GitHub issues exist for {document['seed_id']}")
        if matches:
            issue = matches[0]
            require(issue.get("state") == "open", f"existing issue #{issue['number']} is not open")
            results.append({"seed_id": document["seed_id"], "number": issue["number"], "url": issue["html_url"], "created": False})
            continue
        issue = request_json(
            "POST",
            f"https://api.github.com/repos/{repository}/issues",
            token,
            {"title": document["title"], "body": body, "labels": document["labels"]},
        )
        results.append({"seed_id": document["seed_id"], "number": issue["number"], "url": issue["html_url"], "created": True})
        issues.append(issue)
    return {
        "schema_version": "agent-bounties/open-competition-v2-discovery-issues-v1",
        "passed": True,
        "repository": repository,
        "issues": results,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--index", type=Path, required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--token-env", default="GITHUB_TOKEN")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    token = os.environ.get(args.token_env, "")
    require(bool(token), f"{args.token_env} is required")
    result = publish(args.index, args.repository, token)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(json.dumps({"passed": True, "issues": len(result["issues"]), "output": str(args.output)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
