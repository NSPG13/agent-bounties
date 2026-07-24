#!/usr/bin/env python3
"""Fail-closed, cross-platform intake for untrusted external pull requests."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
from typing import Any, Sequence


ROOT = pathlib.Path(__file__).resolve().parents[1]
DOCS_EXACT = {"README.md", "AGENTS.md", "llms.txt", ".github/PULL_REQUEST_TEMPLATE.md"}
DOCS_PREFIXES = ("docs/", "examples/", ".github/ISSUE_TEMPLATE/")
RISKY_PREFIXES = (".github/workflows/", "scripts/", "contracts/", "migrations/", "crates/")


class UsageParser(argparse.ArgumentParser):
    def error(self, message: str) -> None:
        self.print_usage(sys.stderr)
        self.exit(64, f"{self.prog}: error: {message}\n")


def run(
    command: Sequence[object],
    *,
    cwd: pathlib.Path = ROOT,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        [str(value) for value in command],
        cwd=cwd,
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if check and result.returncode:
        detail = (result.stderr or result.stdout).strip()
        raise RuntimeError(f"{command[0]} failed with exit {result.returncode}: {detail}")
    return result


def is_docs_path(path: str) -> bool:
    return path in DOCS_EXACT or path.startswith(DOCS_PREFIXES)


def is_risky_path(path: str) -> bool:
    return (
        path.startswith(RISKY_PREFIXES)
        or path in {"Cargo.toml", "Cargo.lock"}
        or path.endswith(("package.json", "package-lock.json"))
    )


def collaboration_branch_name(pr: int, topic: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", topic.lower()).strip("-")[:48].rstrip("-")
    return f"collab/pr-{pr}-{slug or 'contribution'}"


def markdown_list(items: Sequence[str], empty: str = "- None", limit: int = 12) -> str:
    values = [item for item in items if item][:limit]
    return "\n".join(f"- {item}" for item in values) if values else empty


def docs_contract_issues(output: str) -> list[str]:
    return [
        line.strip()
        for line in output.splitlines()
        if re.match(r"^[^\s:][^:]+:\d+:", line.strip())
    ][:20]


def feedback(docs_only: bool, risky: Sequence[str], docs_ok: bool, issues: Sequence[str]) -> list[str]:
    items = []
    if not docs_only:
        items.append(
            "Split docs-only changes from code or infrastructure changes, or wait for manual "
            "maintainer review of the non-doc paths."
        )
    if risky:
        items.append(
            "Risky paths need line-by-line maintainer review before CI or any upstream "
            "collaboration branch is approved."
        )
    if not docs_ok:
        items.append(
            "Run `cargo run -p cli -- docs-contract-check` locally and update examples to match "
            "the current API routes, MCP tools, discovery manifest shape, and request payloads."
        )
    if issues:
        items.append(
            "Start with the first docs-contract issue listed below, then rerun the checker until "
            "it reports `docs_contract_check=ok`."
        )
    return items or [
        "Perform semantic review before approving merge, and keep payment or bounty acceptance "
        "separate from code review."
    ]


def require_tools() -> None:
    for tool in ("gh", "git", "cargo"):
        if not shutil.which(tool):
            print(f"{tool} is required for external PR review", file=sys.stderr)
            raise SystemExit(127)


def gh_pr(pr: int, repo: str) -> dict[str, Any]:
    fields = "number,title,url,author,headRefName,headRefOid,baseRefOid,files"
    return json.loads(run(("gh", "pr", "view", pr, "--repo", repo, "--json", fields)).stdout)


def remove_worktree(path: pathlib.Path) -> None:
    run(("git", "worktree", "remove", "--force", path))


def check_docs_contract(pr: int, head_ref: str, base_oid: str) -> tuple[bool, str]:
    target_dir = pathlib.Path(os.environ.get("CARGO_TARGET_DIR", str(ROOT / "target"))).resolve()
    with tempfile.TemporaryDirectory(prefix=f"agent-bounties-pr-{pr}-") as directory:
        temporary = pathlib.Path(directory).resolve()
        worktree, base, contract = temporary / "worktree", temporary / "base", temporary / "contract"
        added: list[pathlib.Path] = []
        try:
            run(("git", "worktree", "add", "--detach", worktree, head_ref))
            added.append(worktree)
            run(("git", "cat-file", "-e", f"{base_oid}^{{commit}}"))
            run(("git", "worktree", "add", "--detach", base, base_oid))
            added.append(base)
            run(
                (
                    sys.executable,
                    ROOT / "scripts/stage_review_contract_root.py",
                    "--worktree",
                    base,
                    "--output",
                    contract,
                )
            )
            result = run(
                (
                    "cargo",
                    "run",
                    "--manifest-path",
                    base / "Cargo.toml",
                    "--target-dir",
                    target_dir,
                    "-p",
                    "cli",
                    "--",
                    "docs-contract-check",
                    "--root",
                    worktree,
                    "--contract-root",
                    contract,
                ),
                check=False,
            )
            output = "\n".join(part for part in (result.stdout, result.stderr) if part).strip()
            return result.returncode == 0, output
        finally:
            cleanup_error = None
            for path in reversed(added):
                try:
                    remove_worktree(path)
                except RuntimeError as error:
                    cleanup_error = cleanup_error or error
            if cleanup_error:
                raise cleanup_error


def existing_collaboration_branch(pr: int) -> str | None:
    result = run(("git", "ls-remote", "--heads", "origin", f"refs/heads/collab/pr-{pr}-*"))
    matches = [line.split()[1].removeprefix("refs/heads/") for line in result.stdout.splitlines()]
    return matches[0] if len(matches) == 1 else None


def validate_collaboration_branch(branch: str) -> None:
    if not branch.startswith("collab/"):
        raise ValueError(f"Collaboration branches must be named collab/<topic>: {branch}")
    run(("git", "check-ref-format", "--branch", branch))


def maybe_create_collaboration_branch(
    requested: bool,
    candidate: bool,
    branch: str | None,
    pr: int,
    topic: str,
    head_oid: str,
) -> tuple[str | None, str]:
    if not requested:
        return None, "not_requested"
    if not candidate:
        raise ValueError(
            f"Refusing to create an upstream collaboration branch for PR #{pr} because the "
            "changed files require manual security review."
        )
    branch = branch or existing_collaboration_branch(pr) or collaboration_branch_name(pr, topic)
    validate_collaboration_branch(branch)
    remote = f"refs/heads/{branch}"
    lines = run(("git", "ls-remote", "--heads", "origin", remote)).stdout.splitlines()
    existing_oid = lines[0].split()[0] if lines else None
    if existing_oid:
        return branch, "exists_at_pr_head" if existing_oid == head_oid else "exists_different_head"
    run(("git", "push", "origin", f"{head_oid}:{remote}"))
    return branch, "created"


def review_body(
    *,
    main_candidate: bool,
    lane: str,
    passed: Sequence[str],
    non_docs: Sequence[str],
    risky: Sequence[str],
    docs_ok: bool,
    issues: Sequence[str],
    fixes: Sequence[str],
    collaboration_candidate: bool,
    branch: str | None,
    branch_status: str,
    pr: int,
) -> str:
    if main_candidate:
        return f"""Automated external PR intake passed.

Decision: main-candidate.

What passed:
{markdown_list(passed)}

What blocks main:
- Nothing from the automated external intake gate. A maintainer still needs to review semantics before merge.

Next steps:
- A maintainer should review the content and required checks before merging to `main`.
- This review does not approve bounty acceptance, payout, or payment settlement."""
    blockers = []
    if non_docs:
        blockers.append(f"Non-doc files changed:\n{markdown_list(non_docs)}")
    if risky:
        blockers.append(f"Risky files changed:\n{markdown_list(risky)}")
    if not docs_ok:
        blockers.append(
            "Docs contract check failed:\n"
            + markdown_list(issues, "- The checker failed without line-specific issues. Run the command below for full output.")
        )
    if collaboration_candidate:
        guidance = (
            f"This is suitable for a collaboration branch. Branch `{branch}` status: "
            f"`{branch_status}`. That branch does not imply bounty acceptance, merge approval, or payment approval."
            if branch
            else f"This looks suitable for `collab/pr-{pr}-<short-topic>` if a maintainer wants others to iterate without merging."
        )
    else:
        guidance = "Do not move this to an upstream collaboration branch automatically; risky or non-doc paths need manual security review first."
    return f"""Thanks for the contribution. Decision: request-changes for `main`.

Recommended lane: {lane}.

What passed:
{markdown_list(passed)}

What blocks main:
{chr(10).join(blockers)}

How to fix:
{markdown_list(fixes)}

Local command to run before pushing an update:

```bash
cargo run -p cli -- docs-contract-check
```

Collaboration branch guidance:
{guidance}

This review does not approve bounty acceptance, merge, payout, or payment settlement."""


def parse_args() -> argparse.Namespace:
    parser = UsageParser()
    parser.add_argument("positional_pr", nargs="?", type=int)
    parser.add_argument("--pr", type=int)
    parser.add_argument("--repo", default="NSPG13/agent-bounties")
    parser.add_argument("--post-review", action="store_true")
    parser.add_argument("--create-collaboration-branch", action="store_true")
    parser.add_argument("--collaboration-branch")
    args = parser.parse_args()
    args.pr = args.pr or args.positional_pr
    if not args.pr:
        parser.error("--pr <number> is required")
    return args


def main() -> int:
    args = parse_args()
    require_tools()
    pr_data = gh_pr(args.pr, args.repo)
    changed = [item["path"] for item in pr_data["files"]]
    if not changed:
        raise RuntimeError(f"PR #{args.pr} has no changed files")
    risky = [path for path in changed if is_risky_path(path)]
    non_docs = [path for path in changed if not is_docs_path(path)]
    docs_only = not non_docs
    ref = f"refs/remotes/origin/pr-{args.pr}-review"
    run(("git", "fetch", "origin", f"+pull/{args.pr}/head:{ref}"))
    fetched_oid = run(("git", "rev-parse", ref)).stdout.strip()
    if fetched_oid != pr_data["headRefOid"]:
        raise RuntimeError(
            f"Fetched PR head {fetched_oid} did not match GitHub head {pr_data['headRefOid']}; rerun review"
        )
    docs_ok, docs_output = check_docs_contract(args.pr, ref, pr_data["baseRefOid"])
    issues = docs_contract_issues(docs_output)
    collaboration_candidate = docs_only and not risky
    main_candidate = collaboration_candidate and docs_ok
    lane = "main-candidate" if main_candidate else (
        "collaboration-branch-candidate" if collaboration_candidate else "manual-security-review"
    )
    fixes = feedback(docs_only, risky, docs_ok, issues)
    branch, branch_status = maybe_create_collaboration_branch(
        args.create_collaboration_branch,
        collaboration_candidate,
        args.collaboration_branch,
        args.pr,
        pr_data["headRefName"],
        fetched_oid,
    )
    result = {
        "pr": pr_data["number"],
        "title": pr_data["title"],
        "url": pr_data["url"],
        "author": pr_data["author"]["login"],
        "contract_basis": "pull_request_base_commit",
        "contract_base_oid": pr_data["baseRefOid"],
        "docs_only": docs_only,
        "safe_for_maintainer_ci": main_candidate,
        "main_candidate": main_candidate,
        "collaboration_branch_candidate": collaboration_candidate,
        "collaboration_branch": branch,
        "collaboration_branch_status": branch_status,
        "recommended_lane": lane,
        "risky_files": risky,
        "non_docs_files": non_docs,
        "docs_contract_check": "ok" if docs_ok else "failed",
        "docs_contract_issues": issues,
        "constructive_feedback": fixes,
        "docs_contract_output": docs_output,
    }
    print(json.dumps(result, indent=2))
    if args.post_review:
        passed = []
        if docs_only:
            passed.append("The changed files are limited to documentation or contributor-facing metadata.")
        if not risky:
            passed.append("No risky paths were changed by the PR head reviewed here.")
        if docs_ok:
            passed.append("`docs-contract-check` passed with the trusted checker against bounded, staged PR route/tool sources.")
        passed = passed or ["The PR head was fetched and matched against GitHub; no merge-ready checks passed yet."]
        body = review_body(
            main_candidate=main_candidate,
            lane=lane,
            passed=passed,
            non_docs=non_docs,
            risky=risky,
            docs_ok=docs_ok,
            issues=issues,
            fixes=fixes,
            collaboration_candidate=collaboration_candidate,
            branch=branch,
            branch_status=branch_status,
            pr=args.pr,
        )
        mode = "--comment" if main_candidate else "--request-changes"
        run(("gh", "pr", "review", args.pr, "--repo", args.repo, mode, "--body", body))
    return 0 if main_candidate else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"external PR review failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
