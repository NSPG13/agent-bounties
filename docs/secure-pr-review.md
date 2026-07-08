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
- Close or reject a PR only after leaving a public repair path. The repair path
  should say whether the contributor should update the same PR, open a smaller
  replacement PR, or continue against a collaboration branch.
- Changes under `.github/workflows/`, `scripts/`, `contracts/`, `migrations/`,
  `crates/`, dependency manifests, or lockfiles require manual review.
- Automation can post review comments or request changes, but it must not merge,
  approve payment, or approve a bounty payout.
- Successful CI is not enough to authorize bounty settlement for PR artifacts.
  Automatic GitHub CI verification requires merged PR metadata, a non-author
  merger, and at least one `APPROVED` review from a non-author reviewer.
  Self-merged, unmerged, or unreviewed PR evidence must stay in review.

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

## Reviewer Feedback Standard

Every public PR response must be useful to a future contributor, not just the
current author. Use this structure for approvals, change requests, declined PRs,
and collaboration-branch acceptance:

- **Decision**: `main-candidate`, `request-changes`,
  `collaboration-branch-candidate`, `manual-security-review`, or
  `declined/superseded`.
- **What passed**: name the useful contribution, trusted checks, or safe files.
  If nothing passed, say that directly and keep the explanation factual.
- **What blocks main**: list the first concrete blocker, stale contract, risky
  path, missing test, failing check, or product-scope mismatch.
- **Repair path**: give the exact local command, file, test, or smaller PR split
  that would make the next submission easier to accept.
- **Branch path**: say whether the work should stay on the same PR, move to a
  collaboration branch, or be restarted as a new focused PR.
- **Payment boundary**: state that code review, branch preservation, and CI
  approval do not authorize bounty acceptance, settlement, payout, or payment.

Do not leave a contributor with only "not approved" or "closed as stale". If the
work is useful but not safe for `main`, prefer preserving the idea on a
collaboration branch when the branch rules allow it. If the work is unsafe or
out of scope, explain the smallest safe version that could be reconsidered.

### Collaboration Branch Rules

- Prefer collaboration branches for docs/spec/template work that is valuable but
  fails a contract check, or for larger feature drafts that need multiple PRs.
- Use collaboration branches to keep useful contribution energy visible without
  relaxing the `main` gate. They are for continued iteration, not for running
  privileged automation on unreviewed code.
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
- If closing the original PR after creating a collaboration branch, link the
  branch in the close comment and state what follow-up PR should target.

To post a GitHub review result:

```powershell
scripts\review-external-pr.ps1 -Pr 6 -PostReview
```

```bash
bash scripts/review-external-pr.sh --pr 6 --post-review
```

To preserve useful work for continued iteration without merging it into
`main`, opt in to creating or reusing a collaboration branch:

```powershell
scripts\review-external-pr.ps1 -Pr 6 -CreateCollaborationBranch -PostReview
```

```bash
bash scripts/review-external-pr.sh --pr 6 --create-collaboration-branch --post-review
```

Use an explicit branch name when a maintainer has already announced one:

```powershell
scripts\review-external-pr.ps1 -Pr 6 -CreateCollaborationBranch -CollaborationBranch collab/pr-6-agent-quickstart
```

```bash
bash scripts/review-external-pr.sh --pr 6 --create-collaboration-branch --collaboration-branch collab/pr-6-agent-quickstart
```

The branch creation flag is intentionally narrow. It refuses PRs that touch
risky paths, will reuse an existing `collab/pr-<number>-...` branch when exactly
one exists, and will not overwrite an existing branch that points at a different
commit. A maintainer can still make a manual branch after deeper review, but the
automation should not turn untrusted code into an upstream execution surface.

## Constructive Review Format

Every review outcome should leave the contributor with a repair path:

- **Approve for main**: state what passed, list the checks that were trusted,
  and remind readers that code review does not approve bounty payout or payment
  settlement.
- **Request changes**: state the blocker, give the exact local command to run,
  point at the first failing file or contract mismatch, and explain what would
  make the PR main-ready.
- **Accept for collaboration branch**: state that the work is useful but not
  main-ready, name the branch, invite follow-up PRs against that branch, and
  say clearly that the branch is not a merge approval, bounty acceptance, or
  payout approval.
- **Manual security review**: state which risky paths triggered the lane and ask
  for a smaller split if that would help review.
- **Decline or close**: state why the PR is being closed, name any superseding
  PR/issue/branch, and describe the smallest revised PR that would be welcome.

Suggested "not main-ready yet" comment:

```text
Thanks for the contribution. Decision: request-changes. I cannot approve this
for main yet, but the repair path is concrete.

What passed:
- <docs-only / useful topic / no risky paths, when true>

What blocks main:
- <first failing check or risky path>

Please run:
`cargo run -p cli -- docs-contract-check`

Recommended lane:
<main-candidate | collaboration-branch-candidate | manual-security-review>

Collaboration branch:
<branch name or why one is not safe yet>

This review does not approve bounty acceptance, merge, payout, or payment
settlement.
```

Suggested "accepted for collaboration branch" comment:

```text
Thanks for the contribution. Decision: collaboration-branch-candidate.

What passed:
- <useful docs/spec/template/feature direction>

What blocks main:
- <contract mismatch / incomplete acceptance criteria / larger split still in progress>

Collaboration branch:
- <collab/pr-number-topic>
- Please target follow-up PRs at that branch until it has a clean path back to main.

Repair path:
- <exact command or files to update before a maintainer-owned PR back to main>

This branch preserves iteration space. It is not merge approval, bounty
acceptance, payout approval, or payment settlement.
```

## Docs Contract Check

`docs-contract-check` fails when docs reference:

- REST paths served from the MCP port,
- API routes not present in the trusted Axum router,
- MCP tool names not present in the trusted MCP descriptor source,
- stale discovery endpoint aliases instead of the manifest `endpoints` object,
- key curl JSON examples whose fields do not match request contracts.

This gate is also part of `scripts/check.ps1` and `scripts/check.sh`.
