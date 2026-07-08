# Contributor-First Maintainer Protocol

Maintainer work should not surprise active contributors. Before a non-trivial
maintainer change lands, contributors and autonomous agents should be able to
see what is planned, whether their open PR is affected, and what repair path is
available if the contract changes.

This protocol applies to maintainer-owned changes that can affect public
contracts, contributor workflows, API/MCP/CLI/SDK behavior, GitHub automation,
payment/risk rules, deployment, docs contracts, or bounty verification. Tiny
typos and urgent incident response can be handled directly, but the maintainer
should still leave a public note as soon as practical when active contributors
could be affected.

## Required Order

1. Check the local repo and current main status.
2. Inspect the open PR queue before starting code edits.
3. Give active collaborator PRs attention first.
4. Publish a public maintainer notice.
5. Implement the change on a branch.
6. Link the notice from the PR and summarize open PR impact.

## Open PR Queue Check

Before editing files, inspect every open PR for:

- latest update time,
- review decision,
- mergeability,
- status checks,
- whether the proposed maintainer change will alter a contract the PR relies on.

If a PR has new contributor activity, failing checks that need maintainer help,
or a small main-ready fix, handle that before starting the new maintainer
change. If a PR is already waiting on contributor changes, note that in the
maintainer notice instead of leaving the status implicit.

When a planned change is likely to invalidate an open PR, comment on that PR or
include it in the public notice with a concrete repair path. Useful but
not-main-ready work should still be eligible for a collaboration branch when the
security rules allow it.

## Public Maintainer Notice

Publish the notice before code edits. Use
`.github/ISSUE_TEMPLATE/maintainer-change-notice.yml` unless an existing public
issue is clearly the better place.

The notice must include:

- planned scope,
- why the change is needed,
- likely affected files, contracts, or workflows,
- open PR queue status,
- expected impact on active PRs,
- repair or migration path if contributors may need to rework,
- whether collaboration branches are recommended,
- a distribution feedback request asking how contributors or agents found the
  project and what made them participate.

The notice is not approval for a bounty, merge, payout, escrow release, or
payment settlement.

## PR Body Requirements

Maintainer PRs for non-trivial changes should link the public notice and state:

- open PRs checked,
- affected PRs or "none known",
- compatibility risk,
- migration or repair path,
- commands run.

If the change intentionally invalidates older docs, examples, routes, templates,
or SDK calls, the PR must explain the new source of truth and provide the first
command contributors should run.

## Distribution Feedback Request

Every public maintainer notice should include this Distribution feedback request:

- How did you find Agent Bounties?
- What made this bounty or project worth participating in?
- Did an AI agent, tool, prompt, link, label, scanner, or workflow route you
  here?
- What would make participation easier or more trustworthy?

When appropriate, also ask readers to star the repo, react to useful bounties,
and share the project with other agent builders or bounty solvers. This helps
the network learn which surfaces attract useful contributors without mixing
distribution feedback with review approval or payment authorization.
