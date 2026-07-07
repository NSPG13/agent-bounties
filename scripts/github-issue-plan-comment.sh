#!/usr/bin/env bash
set -euo pipefail

if command -v python3 >/dev/null 2>&1; then
  python_cmd=(python3)
elif command -v python >/dev/null 2>&1; then
  python_cmd=(python)
elif command -v py >/dev/null 2>&1; then
  python_cmd=(py -3)
else
  echo "python3, python, or py is required" >&2
  exit 127
fi

if command -v cygpath >/dev/null 2>&1 && [[ -n "${USERPROFILE:-}" ]]; then
  export PATH="$(cygpath -u "$USERPROFILE")/.cargo/bin:$PATH"
fi
if [[ -d "/mnt/c/Users/${USER:-}/.cargo/bin" ]]; then
  export PATH="/mnt/c/Users/${USER}/.cargo/bin:$PATH"
fi
use_windows_cargo_paths=0
if ! command -v cargo >/dev/null 2>&1 && command -v cargo.exe >/dev/null 2>&1; then
  use_windows_cargo_paths=1
  cargo() { cargo.exe "$@"; }
elif command -v cargo >/dev/null 2>&1 && [[ "$(command -v cargo)" == *.exe ]]; then
  use_windows_cargo_paths=1
fi

to_cargo_path() {
  local path="$1"
  if [[ "$use_windows_cargo_paths" == "1" ]]; then
    if command -v cygpath >/dev/null 2>&1; then
      cygpath -w "$path"
      return
    fi
    if command -v wslpath >/dev/null 2>&1; then
      wslpath -w "$path"
      return
    fi
  fi
  printf '%s\n' "$path"
}

event_path="${GITHUB_EVENT_PATH:?GITHUB_EVENT_PATH is required}"
workspace="${GITHUB_WORKSPACE:-$(pwd)}"
tmp_dir="${RUNNER_TEMP:-$workspace/target/tmp}"
mkdir -p "$tmp_dir"

body_file="$tmp_dir/paid-bounty-issue-body.md"
meta_file="$tmp_dir/paid-bounty-issue-meta.json"
plan_file="$tmp_dir/paid-bounty-plan.json"
comment_file="$tmp_dir/paid-bounty-comment.md"

"${python_cmd[@]}" - "$event_path" "$body_file" "$meta_file" <<'PY'
import json
import os
import pathlib
import sys

event = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
issue = event.get("issue") or {}
repository = event.get("repository") or {}

pathlib.Path(sys.argv[2]).write_text(issue.get("body") or "", encoding="utf-8")
meta = {
    "repo": os.environ.get("GITHUB_REPOSITORY") or repository.get("full_name") or "",
    "number": issue.get("number"),
    "title": issue.get("title") or "",
    "url": issue.get("html_url") or "",
}
missing = [key for key, value in meta.items() if value in ("", None)]
if missing:
    raise SystemExit(f"issue event missing required metadata: {', '.join(missing)}")
pathlib.Path(sys.argv[3]).write_text(json.dumps(meta), encoding="utf-8")
PY

read_json_field() {
  local file="$1"
  local field="$2"
  "${python_cmd[@]}" - "$file" "$field" <<'PY'
import json
import pathlib
import sys

value = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
for part in sys.argv[2].split("."):
    value = value[part]
if isinstance(value, bool):
    print("true" if value else "false")
else:
    print(value)
PY
}

repo="$(read_json_field "$meta_file" repo)"
issue_number="$(read_json_field "$meta_file" number)"
issue_title="$(read_json_field "$meta_file" title)"
issue_url="$(read_json_field "$meta_file" url)"

body_file_for_cargo="$(to_cargo_path "$body_file")"

cargo run -p cli -- github-plan \
  --repository "$repo" \
  --issue-url "$issue_url" \
  --title "$issue_title" \
  --body-file "$body_file_for_cargo" > "$plan_file"

conclusion="$(read_json_field "$plan_file" check.conclusion)"
summary="$(read_json_field "$plan_file" check.summary)"
details="$(read_json_field "$plan_file" check.text)"
if [[ "$conclusion" == "Success" ]]; then
  ready=true
else
  ready=false
fi

{
  echo "<!-- agent-bounties-plan -->"
  echo "### Agent bounty validation: $conclusion"
  echo
  echo "$summary"
  echo
  if [[ "$ready" == "true" ]]; then
    echo "This issue can be routed into a funded bounty."
  else
    echo "This issue needs edits before it can be routed into a funded bounty."
  fi
  echo
  echo "<details><summary>Planner output</summary>"
  echo
  echo '```'
  echo "$details"
  echo '```'
  echo
  echo "</details>"
} > "$comment_file"

if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  {
    echo "## Agent bounty validation"
    echo
    cat "$comment_file"
    echo
  } >> "$GITHUB_STEP_SUMMARY"
fi

if [[ "${DRY_RUN:-}" == "1" ]]; then
  cat "$plan_file"
  echo
  cat "$comment_file"
  exit 0
fi

if ! command -v gh >/dev/null 2>&1; then
  echo "gh is required to publish the paid-bounty validation comment" >&2
  exit 127
fi

existing_comment_id="$(
  gh api "repos/$repo/issues/$issue_number/comments" \
    --jq '.[] | select(.body | contains("<!-- agent-bounties-plan -->")) | .id' \
    | head -n 1
)"

if [[ -n "$existing_comment_id" ]]; then
  gh api \
    --method PATCH \
    "repos/$repo/issues/comments/$existing_comment_id" \
    --field body="$(cat "$comment_file")" >/dev/null
else
  gh issue comment "$issue_number" --repo "$repo" --body-file "$comment_file" >/dev/null
fi
