#!/usr/bin/env bash
set -euo pipefail

repo="NSPG13/agent-bounties"
post_review=false
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
  echo "usage: scripts/review-external-pr.sh --pr <number> [--repo owner/name] [--post-review]" >&2
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

ref_name="refs/remotes/origin/pr-${pr}-review"
git fetch origin "pull/${pr}/head:${ref_name}"

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
if cargo run -p cli -- docs-contract-check --root "$worktree" --contract-root "$repo_root"; then
  docs_contract_check="ok"
fi

safe_for_maintainer_ci=false
if [[ "$docs_only" == true && "${#risky_files[@]}" -eq 0 && "$docs_contract_check" == "ok" ]]; then
  safe_for_maintainer_ci=true
fi

printf '{\n'
printf '  "pr": %s,\n' "$pr"
printf '  "docs_only": %s,\n' "$docs_only"
printf '  "safe_for_maintainer_ci": %s,\n' "$safe_for_maintainer_ci"
printf '  "docs_contract_check": "%s",\n' "$docs_contract_check"
printf '  "risky_files": [%s],\n' "$(printf '"%s",' "${risky_files[@]}" | sed 's/,$//')"
printf '  "non_docs_files": [%s]\n' "$(printf '"%s",' "${non_docs_files[@]}" | sed 's/,$//')"
printf '}\n'

if [[ "$post_review" == true ]]; then
  if [[ "$safe_for_maintainer_ci" == true ]]; then
    gh pr review "$pr" --repo "$repo" --comment --body \
      "Automated external PR intake passed static docs-only review and docs-contract-check. This does not approve merge or payment; a maintainer still needs to review semantics and decide whether to approve CI."
  else
    gh pr review "$pr" --repo "$repo" --request-changes --body \
      "Automated external PR intake failed. risky_files=${risky_files[*]} non_docs_files=${non_docs_files[*]} docs_contract_check=${docs_contract_check}. Maintainer review required; do not approve CI yet."
  fi
fi

if [[ "$safe_for_maintainer_ci" != true ]]; then
  exit 1
fi
