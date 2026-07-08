# Secure External PR Review

External PRs are untrusted input. Do not approve a gated Actions run until the
changed files and static contracts have been reviewed.

## Intake Command

Run the trusted checker from a clean maintainer checkout:

```powershell
scripts\review-external-pr.ps1 -Pr 6
```

```bash
bash scripts/review-external-pr.sh --pr 6
```

The wrapper fetches the PR into a temporary worktree, classifies changed files,
and runs:

```bash
cargo run -p cli -- docs-contract-check --root <pr-worktree> --contract-root <trusted-checkout>
```

The checker validates docs against the trusted API and MCP contracts without
executing code from the PR.

## Decision Rules

- `safe_for_maintainer_ci=true` means the PR is docs-only, avoids risky paths,
  and passes the docs contract check. A maintainer may still inspect semantics
  before approving CI.
- `safe_for_maintainer_ci=false` means do not approve CI yet.
- Changes under `.github/workflows/`, `scripts/`, `contracts/`, `migrations/`,
  `crates/`, dependency manifests, or lockfiles require manual review.
- Automation can post review comments or request changes, but it must not merge,
  approve payment, or approve a bounty payout.

To post a GitHub review result:

```powershell
scripts\review-external-pr.ps1 -Pr 6 -PostReview
```

```bash
bash scripts/review-external-pr.sh --pr 6 --post-review
```

## Docs Contract Check

`docs-contract-check` fails when docs reference:

- REST paths served from the MCP port,
- API routes not present in the trusted Axum router,
- MCP tool names not present in the trusted MCP descriptor source,
- stale discovery endpoint aliases instead of the manifest `endpoints` object,
- key curl JSON examples whose fields do not match request contracts.

This gate is also part of `scripts/check.ps1` and `scripts/check.sh`.
