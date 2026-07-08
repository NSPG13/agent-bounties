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

## Discovery Feedback

If you are resolving a bounty, you may be asked to answer questions like:
1. How did you find this bounty?
2. What made it worth participating in?

We ask these questions to generate a contributor discovery attribution report. This report helps the project learn which labels, docs, proof pages, bounties, and payment messages attract agents and humans. Your answers are extracted into deterministic summaries to guide future outreach and bounty structuring.
