#!/usr/bin/env python3
"""Report repository code size without changing repository or GitHub state."""

from __future__ import annotations

import argparse
import collections
import datetime as dt
import fnmatch
import hashlib
import io
import itertools
import json
import pathlib
import re
import subprocess
import tarfile
from typing import Iterable, Mapping, Sequence


REPORT_SCHEMA = "agent-bounties/code-size-report-v1"
DEFAULT_POLICY = pathlib.Path("ops/code-size-policy.json")
WORKTREE_REF = "WORKTREE"


class ReportError(RuntimeError):
    pass


def git(repo: pathlib.Path, *args: str, text: bool = True) -> str | bytes:
    try:
        return subprocess.run(
            ["git", *args], cwd=repo, check=True, capture_output=True, text=text
        ).stdout
    except (OSError, subprocess.CalledProcessError) as error:
        detail = getattr(error, "stderr", "") or str(error)
        if isinstance(detail, bytes):
            detail = detail.decode("utf-8", errors="replace")
        raise ReportError(f"git {' '.join(args)} failed: {detail.strip()}") from error


def repository_root(start: pathlib.Path) -> pathlib.Path:
    return pathlib.Path(str(git(start, "rev-parse", "--show-toplevel")).strip()).resolve()


def load_policy(path: pathlib.Path) -> tuple[dict[str, object], str]:
    raw = path.read_bytes()
    try:
        policy = json.loads(raw)
    except json.JSONDecodeError as error:
        raise ReportError(f"invalid code-size policy: {error}") from error
    if policy.get("schema_version") != "agent-bounties/code-size-policy-v1":
        raise ReportError("unsupported code-size policy schema")
    if policy.get("metric") != "nonblank_physical_lines":
        raise ReportError("code-size policy metric must be nonblank_physical_lines")
    return policy, hashlib.sha256(raw).hexdigest()


def normalize_path(path: str) -> str:
    path = path.replace("\\", "/")
    while path.startswith("./"):
        path = path[2:]
    return path.lstrip("/")


def is_protected(path: str, policy: Mapping[str, object]) -> bool:
    path = normalize_path(path)
    return any(path.startswith(str(prefix)) for prefix in policy["protected_prefixes"]) or any(
        fnmatch.fnmatchcase(path, str(pattern))
        for pattern in policy.get("protected_patterns", [])
    )


def classify_path(path: str, policy: Mapping[str, object]) -> str | None:
    path = normalize_path(path)
    if is_protected(path, policy):
        return None
    suffix = pathlib.PurePosixPath(path).suffix.lower()
    for raw_group in policy["groups"]:
        group = dict(raw_group)
        if not any(path.startswith(str(prefix)) for prefix in group["prefixes"]):
            continue
        if group.get("path_contains") and str(group["path_contains"]) not in path:
            continue
        if suffix in group["extensions"]:
            return str(group["name"])
    for raw_group in policy["pattern_groups"]:
        group = dict(raw_group)
        if any(fnmatch.fnmatchcase(path, str(pattern)) for pattern in group["patterns"]):
            return str(group["name"])
    return None


def decode_source(path: str, raw: bytes) -> str:
    try:
        return raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise ReportError(f"eligible source is not UTF-8: {path}") from error


def archive_sources(
    repo: pathlib.Path, ref: str, policy: Mapping[str, object]
) -> tuple[str, dict[str, dict[str, object]]]:
    if ref == WORKTREE_REF:
        commit = str(git(repo, "rev-parse", "HEAD")).strip()
        listed = bytes(
            git(repo, "ls-files", "--cached", "--others", "--exclude-standard", "-z", text=False)
        ).split(b"\0")
        raw_files = {}
        for encoded in listed:
            if not encoded:
                continue
            path = encoded.decode("utf-8")
            file_path = repo / pathlib.PurePosixPath(path)
            if file_path.is_file():
                raw_files[normalize_path(path)] = file_path.read_bytes()
    else:
        commit = str(git(repo, "rev-parse", f"{ref}^{{commit}}")).strip()
        archive = bytes(git(repo, "archive", "--format=tar", commit, text=False))
        raw_files = {}
        with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as tar:
            for member in tar.getmembers():
                if member.isfile():
                    handle = tar.extractfile(member)
                    if handle is not None:
                        raw_files[normalize_path(member.name)] = handle.read()

    files: dict[str, dict[str, object]] = {}
    for path, raw in raw_files.items():
        group = classify_path(path, policy)
        if group is None:
            continue
        text = decode_source(path, raw)
        lines = text.splitlines()
        files[path] = {
            "group": group,
            "physical_lines": len(lines),
            "nonblank_lines": sum(bool(line.strip()) for line in lines),
            "text": text,
        }
    return commit, files


def summarize_snapshot(
    ref: str, commit: str, files: Mapping[str, Mapping[str, object]]
) -> dict[str, object]:
    groups: dict[str, dict[str, int]] = collections.defaultdict(
        lambda: {"files": 0, "nonblank_lines": 0, "physical_lines": 0}
    )
    for record in files.values():
        group = groups[str(record["group"])]
        group["files"] += 1
        group["nonblank_lines"] += int(record["nonblank_lines"])
        group["physical_lines"] += int(record["physical_lines"])
    return {
        "ref": ref,
        "commit": commit,
        "files": len(files),
        "nonblank_lines": sum(int(record["nonblank_lines"]) for record in files.values()),
        "physical_lines": sum(int(record["physical_lines"]) for record in files.values()),
        "groups": dict(sorted(groups.items())),
    }


def file_deltas(
    base: Mapping[str, Mapping[str, object]], head: Mapping[str, Mapping[str, object]]
) -> list[dict[str, object]]:
    deltas = []
    for path in sorted(set(base) | set(head)):
        before = int(base.get(path, {}).get("nonblank_lines", 0))
        after = int(head.get(path, {}).get("nonblank_lines", 0))
        if before != after:
            deltas.append(
                {
                    "path": path,
                    "group": str((head.get(path) or base[path])["group"]),
                    "base": before,
                    "head": after,
                    "delta": after - before,
                }
            )
    return sorted(deltas, key=lambda item: (-abs(int(item["delta"])), str(item["path"])))


def normalized_source_lines(text: str) -> list[tuple[int, str]]:
    return [
        (number, re.sub(r"\s+", " ", line.strip()))
        for number, line in enumerate(text.splitlines(), 1)
        if line.strip()
    ]


def duplicate_blocks(
    files: Mapping[str, Mapping[str, object]], minimum_lines: int, limit: int = 40
) -> list[dict[str, object]]:
    normalized = {
        path: normalized_source_lines(str(record["text"])) for path, record in files.items()
    }
    windows: dict[tuple[str, ...], list[tuple[str, int]]] = collections.defaultdict(list)
    for path, lines in normalized.items():
        values = [value for _, value in lines]
        for start in range(len(values) - minimum_lines + 1):
            block = tuple(values[start : start + minimum_lines])
            if sum(map(len, block)) >= 120:
                windows[block].append((path, start))

    diagonals: dict[tuple[str, str, int], set[tuple[int, int]]] = collections.defaultdict(set)
    for occurrences in windows.values():
        if len(occurrences) < 2 or len(occurrences) > 20:
            continue
        for left, right in itertools.combinations(occurrences, 2):
            if left[0] == right[0]:
                if abs(left[1] - right[1]) < minimum_lines:
                    continue
                if left[1] > right[1]:
                    left, right = right, left
            elif left[0] > right[0]:
                left, right = right, left
            diagonals[(left[0], right[0], left[1] - right[1])].add((left[1], right[1]))

    blocks = []
    for (left_path, right_path, _), starts in diagonals.items():
        ordered = sorted(starts)
        run_start = previous = ordered[0]
        for current in ordered[1:] + [(10**12, 10**12)]:
            if current == (previous[0] + 1, previous[1] + 1):
                previous = current
                continue
            length = previous[0] - run_start[0] + minimum_lines
            left_lines = normalized[left_path]
            right_lines = normalized[right_path]
            blocks.append(
                {
                    "lines": length,
                    "left": {
                        "path": left_path,
                        "start": left_lines[run_start[0]][0],
                        "end": left_lines[previous[0] + minimum_lines - 1][0],
                    },
                    "right": {
                        "path": right_path,
                        "start": right_lines[run_start[1]][0],
                        "end": right_lines[previous[1] + minimum_lines - 1][0],
                    },
                }
            )
            run_start = previous = current
    unique = {
        (
            block["left"]["path"],
            block["left"]["start"],
            block["right"]["path"],
            block["right"]["start"],
        ): block
        for block in blocks
    }
    return sorted(
        unique.values(),
        key=lambda block: (-int(block["lines"]), str(block["left"]["path"])),
    )[:limit]


def changed_protected_paths(
    repo: pathlib.Path, base_ref: str, head_ref: str, policy: Mapping[str, object]
) -> list[dict[str, str]]:
    target = base_ref if head_ref == WORKTREE_REF else f"{base_ref}..{head_ref}"
    output = str(git(repo, "diff", "--name-status", target))
    changes = []
    for line in output.splitlines():
        fields = line.split("\t")
        if len(fields) < 2:
            continue
        status, paths = fields[0], fields[1:]
        changes.extend(
            {"status": status, "path": normalize_path(path)}
            for path in paths
            if is_protected(path, policy)
        )
    if head_ref == WORKTREE_REF:
        untracked = str(git(repo, "ls-files", "--others", "--exclude-standard"))
        changes.extend(
            {"status": "??", "path": normalize_path(path)}
            for path in untracked.splitlines()
            if is_protected(path, policy)
        )
    return changes


def churn(
    repo: pathlib.Path,
    base_ref: str,
    head_ref: str,
    policy: Mapping[str, object],
    limit: int = 30,
) -> list[dict[str, object]]:
    args = (
        ("diff", "--numstat", base_ref)
        if head_ref == WORKTREE_REF
        else ("log", "--format=", "--numstat", f"{base_ref}..{head_ref}")
    )
    totals: dict[str, list[int]] = collections.defaultdict(lambda: [0, 0])
    for line in str(git(repo, *args)).splitlines():
        fields = line.split("\t")
        if len(fields) != 3 or not fields[0].isdigit() or not fields[1].isdigit():
            continue
        path = normalize_path(fields[2])
        if classify_path(path, policy) is not None:
            totals[path][0] += int(fields[0])
            totals[path][1] += int(fields[1])
    rows = [
        {"path": path, "added": value[0], "deleted": value[1], "churn": sum(value)}
        for path, value in totals.items()
    ]
    return sorted(rows, key=lambda row: (-int(row["churn"]), str(row["path"])))[:limit]


def trend_snapshots(
    repo: pathlib.Path,
    head_ref: str,
    head_total: int,
    policy: Mapping[str, object],
    days: Iterable[int],
) -> list[dict[str, object]]:
    resolved_head = "HEAD" if head_ref == WORKTREE_REF else head_ref
    root = str(git(repo, "rev-list", "--max-parents=0", resolved_head)).splitlines()[-1]
    trends = []
    for day_count in days:
        ref = str(
            git(repo, "rev-list", "-1", f"--before={day_count} days ago", resolved_head)
        ).strip()
        ref = ref or root
        commit, files = archive_sources(repo, ref, policy)
        total = int(summarize_snapshot(ref, commit, files)["nonblank_lines"])
        trends.append(
            {"days": day_count, "commit": commit, "nonblank_lines": total, "delta": head_total - total}
        )
    return trends


def build_report(
    repo: pathlib.Path,
    policy: Mapping[str, object],
    policy_hash: str,
    head_ref: str,
    base_ref: str | None,
    trend_days: Sequence[int],
) -> dict[str, object]:
    head_commit, head_files = archive_sources(repo, head_ref, policy)
    head = summarize_snapshot(head_ref, head_commit, head_files)
    base = None
    deltas: list[dict[str, object]] = []
    report_churn: list[dict[str, object]] = []
    protected_changes: list[dict[str, str]] = []
    if base_ref:
        base_commit, base_files = archive_sources(repo, base_ref, policy)
        base = summarize_snapshot(base_ref, base_commit, base_files)
        deltas = file_deltas(base_files, head_files)
        report_churn = churn(repo, base_ref, head_ref, policy)
        protected_changes = changed_protected_paths(repo, base_ref, head_ref, policy)
    return {
        "schema_version": REPORT_SCHEMA,
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "policy": {
            "schema_version": policy["schema_version"],
            "sha256": policy_hash,
            "metric": policy["metric"],
        },
        "head": head,
        "base": base,
        "delta": None if base is None else int(head["nonblank_lines"]) - int(base["nonblank_lines"]),
        "file_deltas": deltas,
        "trends": trend_snapshots(repo, head_ref, int(head["nonblank_lines"]), policy, trend_days),
        "churn_hotspots": report_churn,
        "duplicate_blocks": duplicate_blocks(head_files, int(policy["minimum_duplicate_lines"])),
        "protected_changes": protected_changes,
    }


def markdown_table(headers: Sequence[str], rows: Iterable[Sequence[object]]) -> list[str]:
    output = [
        "| " + " | ".join(headers) + " |",
        "| " + " | ".join("---" for _ in headers) + " |",
    ]
    output.extend("| " + " | ".join(str(value) for value in row) + " |" for row in rows)
    return output


def render_markdown(report: Mapping[str, object]) -> str:
    head = dict(report["head"])
    base = report.get("base")
    head_label = (
        head["commit"] if head["ref"] != WORKTREE_REF else f"WORKTREE ({head['commit']})"
    )
    lines = [
        "# Code-size report",
        "",
        f"Head `{head_label}` contains **{head['nonblank_lines']:,}** nonblank lines in **{head['files']}** eligible files.",
    ]
    if base:
        base = dict(base)
        lines.append(
            f"Compared with `{base['commit']}`, the delta is **{int(report['delta']):+,}** lines "
            f"({base['nonblank_lines']:,} -> {head['nonblank_lines']:,})."
        )
    lines.extend(["", "## Subsystems", ""])
    lines.extend(
        markdown_table(
            ["Group", "Files", "Nonblank", "Physical"],
            (
                (name, values["files"], f"{values['nonblank_lines']:,}", f"{values['physical_lines']:,}")
                for name, values in head["groups"].items()
            ),
        )
    )
    trends = list(report["trends"])
    if trends:
        lines.extend(["", "## Trends", ""])
        lines.extend(
            markdown_table(
                ["Window", "Commit", "Lines", "Delta"],
                (
                    (f"{item['days']} days", str(item["commit"])[:12], f"{item['nonblank_lines']:,}", f"{item['delta']:+,}")
                    for item in trends
                ),
            )
        )
    sections = (
        ("Largest file deltas", "file_deltas", ["Path", "Base", "Head", "Delta"]),
        ("Churn hotspots", "churn_hotspots", ["Path", "Added", "Deleted", "Churn"]),
    )
    for heading, key, headers in sections:
        items = list(report[key])[:20]
        if not items:
            continue
        lines.extend(["", f"## {heading}", ""])
        if key == "file_deltas":
            rows = ((item["path"], item["base"], item["head"], f"{item['delta']:+,}") for item in items)
        else:
            rows = ((item["path"], item["added"], item["deleted"], item["churn"]) for item in items)
        lines.extend(markdown_table(headers, rows))
    duplicates = list(report["duplicate_blocks"])
    if duplicates:
        lines.extend(["", "## Exact duplicate candidates", ""])
        lines.extend(
            markdown_table(
                ["Lines", "Left", "Right"],
                (
                    (
                        item["lines"],
                        f"{item['left']['path']}:{item['left']['start']}",
                        f"{item['right']['path']}:{item['right']['start']}",
                    )
                    for item in duplicates[:20]
                ),
            )
        )
    protected = list(report["protected_changes"])
    lines.extend(["", "## Protected paths", ""])
    if protected:
        lines.append(
            "Protected paths changed; these changes are excluded from size savings and require their normal review class."
        )
        lines.extend(f"- `{item['status']}` `{item['path']}`" for item in protected)
    else:
        lines.append("No protected paths changed in the comparison.")
    lines.extend(
        [
            "",
            "This report is informational. A smaller number never overrides behavior, compatibility, data-integrity, or payment-safety gates.",
            "",
        ]
    )
    return "\n".join(lines)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=pathlib.Path, default=pathlib.Path.cwd())
    parser.add_argument("--policy", type=pathlib.Path, default=DEFAULT_POLICY)
    parser.add_argument("--head-ref", default="HEAD", help="Git ref or WORKTREE")
    parser.add_argument("--base-ref")
    parser.add_argument("--trend-days", default="7,30")
    parser.add_argument("--json-out", type=pathlib.Path)
    parser.add_argument("--markdown-out", type=pathlib.Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    repo = repository_root(args.repo.resolve())
    policy_path = args.policy if args.policy.is_absolute() else repo / args.policy
    policy, policy_hash = load_policy(policy_path)
    trend_days = [int(value) for value in args.trend_days.split(",") if value.strip()]
    report = build_report(repo, policy, policy_hash, args.head_ref, args.base_ref, trend_days)
    markdown = render_markdown(report)
    if args.json_out:
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    if args.markdown_out:
        args.markdown_out.parent.mkdir(parents=True, exist_ok=True)
        args.markdown_out.write_text(markdown, encoding="utf-8")
    if not args.markdown_out:
        print(markdown, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
