"""Common, side-effect-explicit helpers for GitHub Actions scripts."""

from __future__ import annotations

import json
import pathlib
import re
import shutil
import subprocess
from collections.abc import Callable, Iterable, Mapping
from typing import TypeVar


Error = TypeVar("Error", bound=Exception)


def repo_root(script_file: str) -> pathlib.Path:
    return pathlib.Path(script_file).resolve().parents[1]


def find_executable(names: str | Iterable[str]) -> str | None:
    candidates = [names, f"{names}.exe"] if isinstance(names, str) else names
    return next((path for name in candidates if (path := shutil.which(name))), None)


def _windows_path(path: pathlib.Path) -> str:
    text = str(path)
    if not text.startswith("/"):
        return text
    for converter in ("cygpath", "wslpath"):
        if executable := shutil.which(converter):
            try:
                return subprocess.check_output(
                    [executable, "-w", text], text=True, stderr=subprocess.DEVNULL
                ).strip()
            except (OSError, subprocess.CalledProcessError):
                pass
    if match := re.match(r"^/mnt/([a-zA-Z])/(.*)$", text):
        drive, rest = match.groups()
        windows_rest = rest.replace("/", "\\")
        return f"{drive.upper()}:\\{windows_rest}"
    return text


def cargo_body_path(path: pathlib.Path, cargo_path: str) -> str:
    return _windows_path(path) if cargo_path.lower().endswith(".exe") else str(path)


def json_field(
    value: object, field: str, error_type: type[Error], message: str
) -> object:
    current = value
    for part in field.split("."):
        if not isinstance(current, dict) or part not in current:
            raise error_type(message.format(field=field))
        current = current[part]
    return current


def read_event(
    env: Mapping[str, str], error_type: type[Error] | None = None
) -> dict[str, object]:
    event_path = env.get("GITHUB_EVENT_PATH")
    if not event_path:
        if error_type is None:
            return {}
        raise error_type("GITHUB_EVENT_PATH is required")
    value = json.loads(pathlib.Path(event_path).read_text(encoding="utf-8"))
    return value if isinstance(value, dict) else {}


def append_step_summary(env: Mapping[str, str], heading: str, body: str) -> None:
    if summary_path := env.get("GITHUB_STEP_SUMMARY"):
        with pathlib.Path(summary_path).open("a", encoding="utf-8") as handle:
            handle.write(f"## {heading}\n\n{body}\n")


def load_issue_comments(
    env: Mapping[str, str],
    repo: object,
    issue_number: object,
    fixture_variable: str,
    missing_tool_message: str,
    error_type: type[Error],
) -> list[Mapping[str, object]]:
    if fixture := env.get(fixture_variable):
        value = json.loads(pathlib.Path(fixture).read_text(encoding="utf-8"))
        return value if isinstance(value, list) else []
    if env.get("DRY_RUN") == "1":
        return []
    gh = find_executable(["gh", "gh.exe"])
    if not gh:
        raise error_type(missing_tool_message)
    value = json.loads(
        subprocess.check_output(
            [gh, "api", f"repos/{repo}/issues/{issue_number}/comments"],
            env=dict(env),
            text=True,
        )
    )
    return value if isinstance(value, list) else []


def publish_issue_comment(
    env: Mapping[str, str],
    repo: object,
    issue_number: object,
    marker: str,
    body: str,
    file_name: str,
    missing_tool_message: str,
    error_type: type[Error],
    existing_comments: list[Mapping[str, object]] | None = None,
    matches: Callable[[str], bool] | None = None,
) -> None:
    gh = find_executable(["gh", "gh.exe"])
    if not gh:
        raise error_type(missing_tool_message)
    if existing_comments is None:
        value = json.loads(
            subprocess.check_output(
                [gh, "api", f"repos/{repo}/issues/{issue_number}/comments"],
                env=dict(env),
                text=True,
            )
        )
        existing_comments = value if isinstance(value, list) else []
    predicate = matches or (lambda text: marker in text)
    existing_id = next(
        (
            comment.get("id")
            for comment in existing_comments
            if predicate(str(comment.get("body") or ""))
        ),
        None,
    )
    if existing_id:
        command = [
            gh,
            "api",
            "--method",
            "PATCH",
            f"repos/{repo}/issues/comments/{existing_id}",
            "--field",
            f"body={body}",
        ]
    else:
        comment_file = pathlib.Path(env.get("RUNNER_TEMP") or ".") / file_name
        comment_file.write_text(body, encoding="utf-8")
        command = [
            gh,
            "issue",
            "comment",
            str(issue_number),
            "--repo",
            str(repo),
            "--body-file",
            str(comment_file),
        ]
    subprocess.run(
        command, env=dict(env), check=True, stdout=subprocess.DEVNULL
    )
