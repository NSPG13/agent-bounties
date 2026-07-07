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

Before running expensive checks, run the preflight:

```powershell
.\scripts\preflight.ps1 -Mode core
.\scripts\preflight.ps1 -Mode full
```

Real-money code paths require tests for idempotency, replay safety, and audit
links from request to proof to settlement.
