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
- Every approval, rejection, or "not yet" response must include constructive
  next steps. Tell the contributor what passed, what blocked main, the exact
  command to run locally, and whether the work is suitable for a collaboration
  branch.
- Changes under `.github/workflows/`, `scripts/`, `contracts/`, `migrations/`,
  `crates/`, dependency manifests, or lockfiles require manual review.
- Automation can post review comments or request changes, but it must not merge,
  approve payment, or approve a bounty payout.

## Review Lanes

Use three lanes instead of treating every PR as merge-or-reject:

- **Main candidate**: the PR passes the trusted intake gate, passes required CI,
  and a maintainer agrees the semantics are correct. It can be approved for
  merge to `main`.
- **Collaboration branch candidate**: the work is useful but not main-ready,
  usually because docs examples are stale, acceptance criteria are incomplete,
  or a feature slice needs more contributors. A maintainer may preserve it on a
  branch such as `collab/pr-6-agent-quickstart` so others can open follow-up PRs
  against that branch without slowing main.
- **Manual security review**: the PR changes workflows, scripts, crates,
  contracts, migrations, dependency manifests, or other risky paths. Do not
  create an upstream collaboration branch until a maintainer has reviewed the
  diff line by line and decided the branch will not become a privileged
  execution surface.

A collaboration branch is not a bounty acceptance, payment approval, or merge
approval. It is a staging lane for useful work that should remain visible while
main stays production-safe.

### Collaboration Branch Rules

- Prefer collaboration branches for docs/spec/template work that is valuable but
  fails a contract check, or for larger feature drafts that need multiple PRs.
- Do not put changes to `.github/workflows/`, release scripts, deployment
  automation, secrets handling, payment settlement, escrow contracts, or
  dependency locks on a collaboration branch unless the maintainer explicitly
  accepts the security risk.
- Name branches `collab/pr-<number>-<short-topic>` or
  `collab/<bounty-id>-<short-topic>`.
- Ask follow-up contributors to target the collaboration branch, not `main`,
  until the branch has a clean path back through the normal gates.
- When the branch becomes main-ready, open a maintainer-owned PR from the
  collaboration branch to `main` and run the full gates again.

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
