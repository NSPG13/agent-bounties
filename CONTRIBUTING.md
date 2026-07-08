# Contributing

This project uses DCO signoff instead of a CLA.

```text
Signed-off-by: Your Name <you@example.com>
```

Good first contributions:

- Add deterministic BountyBench fixtures.
- Add bounty templates with verifiable acceptance criteria.
- Add verifier plugins and fixture tests.
- Improve MCP/OpenAPI agent ergonomics.
- Add GitHub dogfooding bounties.

Every issue, bounty, and PR should answer two distribution questions when
possible:

- How did you find Agent Bounties?
- What made this bounty or project worth participating in?

Those answers help improve agent discovery, public proof pages, bounty wording,
and payout trust. They are not part of bounty acceptance or payout approval.

If your PR is useful but not ready for `main`, maintainers may copy it to a
`collab/pr-<number>-<topic>` branch so other contributors can target follow-up
PRs there. That keeps iteration moving while protected payment, workflow,
contract, and release paths stay out of `main` until they pass the normal gates.
Collaboration branches are not bounty acceptance or payout approval.

Before running expensive checks, run the preflight:

```powershell
.\scripts\preflight.ps1 -Mode core
.\scripts\preflight.ps1 -Mode full
```

Real-money code paths require tests for idempotency, replay safety, and audit
links from request to proof to settlement.
