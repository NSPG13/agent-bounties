#!/usr/bin/env bash
set -euo pipefail

repo="NSPG13/agent-bounties"
post_review=false
create_collaboration_branch=false
collaboration_branch=""
pr=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo)
      repo="$2"
      shift 2
      ;;
    --post-review)
      post_review=true
      shift
      ;;
    --create-collaboration-branch)
      create_collaboration_branch=true
      shift
      ;;
    --collaboration-branch)
      collaboration_branch="$2"
      shift 2
      ;;
    --pr)
      pr="$2"
      shift 2
      ;;
    *)
      if [[ -z "$pr" ]]; then
        pr="$1"
        shift
      else
        echo "unknown argument: $1" >&2
        exit 64
      fi
      ;;
  esac
done

if [[ -z "$pr" ]]; then
  echo "usage: scripts/review-external-pr.sh --pr <number> [--repo owner/name] [--post-review] [--create-collaboration-branch] [--collaboration-branch collab/name]" >&2
  exit 64
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

for tool in gh git cargo; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "$tool is required for external PR review" >&2
    exit 127
  fi
done

is_docs_path() {
  local path="$1"
  [[ "$path" == "README.md" ]] ||
    [[ "$path" == "AGENTS.md" ]] ||
    [[ "$path" == "llms.txt" ]] ||
    [[ "$path" == docs/* ]] ||
    [[ "$path" == examples/* ]] ||
    [[ "$path" == .github/ISSUE_TEMPLATE/* ]]
}

is_risky_path() {
  local path="$1"
  [[ "$path" == .github/workflows/* ]] ||
    [[ "$path" == scripts/* ]] ||
    [[ "$path" == contracts/* ]] ||
    [[ "$path" == migrations/* ]] ||
    [[ "$path" == crates/* ]] ||
    [[ "$path" == "Cargo.toml" ]] ||
    [[ "$path" == "Cargo.lock" ]] ||
    [[ "$path" == *package.json ]] ||
    [[ "$path" == *package-lock.json ]]
}

markdown_list() {
  local empty="$1"
  shift || true
  if [[ "$#" -eq 0 ]]; then
    printf '%s\n' "$empty"
    return
  fi
  local item
  for item in "$@"; do
    printf -- '- %s\n' "$item"
  done
}

json_array() {
  if [[ "$#" -eq 0 ]]; then
    return
  fi
  local first=true
  local item escaped
  for item in "$@"; do
    escaped="${item//\\/\\\\}"
    escaped="${escaped//\"/\\\"}"
    if [[ "$first" == true ]]; then
      first=false
    else
      printf ','
    fi
    printf '"%s"' "$escaped"
  done
}

slugify_branch_topic() {
  local topic="$1"
  local slug
  slug="$(
    printf '%s' "$topic" |
      tr '[:upper:]' '[:lower:]' |
      sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//; s/^(.{0,48}).*$/\1/; s/-+$//'
  )"
  if [[ -z "$slug" ]]; then
    slug="contribution"
  fi
  printf 'collab/pr-%s-%s' "$pr" "$slug"
}

existing_collaboration_branch() {
  mapfile -t matches < <(git ls-remote --heads origin "refs/heads/collab/pr-${pr}-*" | awk '{print $2}' | sed 's#^refs/heads/##')
  if [[ "${#matches[@]}" -eq 1 ]]; then
    printf '%s' "${matches[0]}"
  fi
}

assert_safe_collaboration_branch() {
  local branch="$1"
  if [[ -z "$branch" || "$branch" != collab/* ]]; then
    echo "collaboration branches must be named collab/<topic>: $branch" >&2
    exit 64
  fi
  git check-ref-format --branch "$branch" >/dev/null
}

mapfile -t changed_files < <(gh pr view "$pr" --repo "$repo" --json files --jq '.files[].path')
if [[ "${#changed_files[@]}" -eq 0 ]]; then
  echo "PR #$pr has no changed files" >&2
  exit 1
fi

risky_files=()
non_docs_files=()
for file in "${changed_files[@]}"; do
  if is_risky_path "$file"; then
    risky_files+=("$file")
  fi
  if ! is_docs_path "$file"; then
    non_docs_files+=("$file")
  fi
done

docs_only=true
if [[ "${#non_docs_files[@]}" -ne 0 ]]; then
  docs_only=false
fi

head_oid="$(gh pr view "$pr" --repo "$repo" --json headRefOid --jq '.headRefOid')"
head_ref_name="$(gh pr view "$pr" --repo "$repo" --json headRefName --jq '.headRefName')"
ref_name="refs/remotes/origin/pr-${pr}-review"
git fetch origin "+pull/${pr}/head:${ref_name}"
fetched_oid="$(git rev-parse "$ref_name")"
if [[ "$fetched_oid" != "$head_oid" ]]; then
  echo "fetched PR head $fetched_oid did not match GitHub head $head_oid; rerun review" >&2
  exit 1
fi

tmp_root="$(mktemp -d)"
worktree="$tmp_root/worktree"
cleanup() {
  if git worktree list --porcelain | grep -Fqx "worktree $worktree"; then
    git worktree remove --force "$worktree" >/dev/null 2>&1 || true
  fi
  rm -rf "$tmp_root"
}
trap cleanup EXIT

git worktree add --detach "$worktree" "$ref_name" >/dev/null

docs_contract_check="failed"
docs_output="$tmp_root/docs-contract-check.log"
if cargo run -p cli -- docs-contract-check --root "$worktree" --contract-root "$repo_root" >"$docs_output" 2>&1; then
  docs_contract_check="ok"
fi
cat "$docs_output"
mapfile -t docs_contract_issues < <(grep -E '^[^[:space:]:][^:]+:[0-9]+:' "$docs_output" | head -n 20 || true)

safe_for_maintainer_ci=false
if [[ "$docs_only" == true && "${#risky_files[@]}" -eq 0 && "$docs_contract_check" == "ok" ]]; then
  safe_for_maintainer_ci=true
fi
collaboration_branch_candidate=false
if [[ "$docs_only" == true && "${#risky_files[@]}" -eq 0 ]]; then
  collaboration_branch_candidate=true
fi
recommended_lane="manual-security-review"
if [[ "$safe_for_maintainer_ci" == true ]]; then
  recommended_lane="main-candidate"
elif [[ "$collaboration_branch_candidate" == true ]]; then
  recommended_lane="collaboration-branch-candidate"
fi

collaboration_branch_status="not_requested"
if [[ "$create_collaboration_branch" == true ]]; then
  if [[ "$collaboration_branch_candidate" != true ]]; then
    echo "refusing to create an upstream collaboration branch for PR #$pr because the changed files require manual security review" >&2
    exit 1
  fi
  if [[ -z "$collaboration_branch" ]]; then
    collaboration_branch="$(existing_collaboration_branch)"
  fi
  if [[ -z "$collaboration_branch" ]]; then
    collaboration_branch="$(slugify_branch_topic "$head_ref_name")"
  fi
  assert_safe_collaboration_branch "$collaboration_branch"
  existing_oid="$(git ls-remote --heads origin "refs/heads/${collaboration_branch}" | awk '{print $1}')"
  if [[ -n "$existing_oid" ]]; then
    if [[ "$existing_oid" == "$fetched_oid" ]]; then
      collaboration_branch_status="exists_at_pr_head"
    else
      collaboration_branch_status="exists_different_head"
    fi
  else
    git push origin "${fetched_oid}:refs/heads/${collaboration_branch}"
    collaboration_branch_status="created"
  fi
fi

feedback_items=()
if [[ "$docs_only" != true ]]; then
  feedback_items+=("Split docs-only changes from code or infrastructure changes, or wait for manual maintainer review of the non-doc paths.")
fi
if [[ "${#risky_files[@]}" -gt 0 ]]; then
  feedback_items+=("Risky paths need line-by-line maintainer review before CI or any upstream collaboration branch is approved.")
fi
if [[ "$docs_contract_check" != "ok" ]]; then
  feedback_items+=("Run cargo run -p cli -- docs-contract-check locally and update examples to match the current API routes, MCP tools, discovery manifest shape, and request payloads.")
fi
if [[ "${#docs_contract_issues[@]}" -gt 0 ]]; then
  feedback_items+=("Start with the first docs-contract issue listed below, then rerun the checker until it reports docs_contract_check=ok.")
fi
if [[ "${#feedback_items[@]}" -eq 0 ]]; then
  feedback_items+=("Perform semantic review before approving merge, and keep payment or bounty acceptance separate from code review.")
fi

printf '{\n'
printf '  "pr": %s,\n' "$pr"
printf '  "docs_only": %s,\n' "$docs_only"
printf '  "safe_for_maintainer_ci": %s,\n' "$safe_for_maintainer_ci"
printf '  "main_candidate": %s,\n' "$safe_for_maintainer_ci"
printf '  "collaboration_branch_candidate": %s,\n' "$collaboration_branch_candidate"
printf '  "collaboration_branch": "%s",\n' "$collaboration_branch"
printf '  "collaboration_branch_status": "%s",\n' "$collaboration_branch_status"
printf '  "recommended_lane": "%s",\n' "$recommended_lane"
printf '  "docs_contract_check": "%s",\n' "$docs_contract_check"
printf '  "risky_files": [%s],\n' "$(json_array "${risky_files[@]}")"
printf '  "non_docs_files": [%s]\n' "$(json_array "${non_docs_files[@]}")"
printf '}\n'

if [[ "$post_review" == true ]]; then
  if [[ "$safe_for_maintainer_ci" == true ]]; then
    body="$(
      cat <<'EOF'
Automated external PR intake passed.

What passed:
- The changed files are docs-only.
- No risky paths were changed.
- docs-contract-check passed against the trusted maintainer checkout.

Recommended lane: main-candidate.

Next steps:
- A maintainer should still review the semantics before merging.
- This review does not approve bounty acceptance, payout, or payment settlement.
EOF
    )"
    gh pr review "$pr" --repo "$repo" --comment --body "$body"
  else
    body_file="$tmp_root/review-body.md"
    {
      printf 'Thanks for the contribution. I cannot approve this for main yet, but the next repair steps are concrete.\n\n'
      printf 'Recommended lane: %s.\n\n' "$recommended_lane"
      printf 'Why it is blocked:\n'
      if [[ "${#non_docs_files[@]}" -gt 0 ]]; then
        printf '\nNon-doc files changed:\n'
        markdown_list "- None" "${non_docs_files[@]}"
      fi
      if [[ "${#risky_files[@]}" -gt 0 ]]; then
        printf '\nRisky files changed:\n'
        markdown_list "- None" "${risky_files[@]}"
      fi
      if [[ "$docs_contract_check" != "ok" ]]; then
        printf '\nDocs contract check failed:\n'
        markdown_list "- The checker failed without line-specific issues. Run the command below for full output." "${docs_contract_issues[@]}"
      fi
      printf '\nHow to fix:\n'
      markdown_list "- Rerun the trusted review command and address each blocker." "${feedback_items[@]}"
      printf '\nLocal command to run before pushing an update:\n\n'
      printf '```bash\ncargo run -p cli -- docs-contract-check\n```\n\n'
      printf 'Collaboration branch guidance:\n'
      if [[ "$collaboration_branch_candidate" == true ]]; then
        if [[ "$create_collaboration_branch" == true ]]; then
          printf 'This is suitable for a collaboration branch. Branch %s status: %s. That branch does not imply bounty acceptance, merge approval, or payment approval.\n' "$collaboration_branch" "$collaboration_branch_status"
        else
          printf 'This looks suitable for a collaboration branch such as collab/pr-%s-<short-topic> if a maintainer wants others to iterate on it without merging to main yet. That branch would not imply bounty acceptance or payment approval.\n' "$pr"
        fi
      else
        printf 'Do not move this to an upstream collaboration branch automatically. The risky or non-doc paths need manual maintainer security review first.\n'
      fi
    } >"$body_file"
    gh pr review "$pr" --repo "$repo" --request-changes --body "$(cat "$body_file")"
  fi
fi

if [[ "$safe_for_maintainer_ci" != true ]]; then
  exit 1
fi
