# Production Smoke

`production-smoke` is the read-only gate for the deployed API and MCP
services. It validates the public surfaces agents use to discover, evaluate,
claim, verify, and monitor autonomous bounties. It does not create bounties,
sign transactions, mutate ledger state, or invoke payment execution.

## Deployed Revision Contract

Both `/health` endpoints keep the Render-compatible `ok` response body and
publish these headers:

- `x-agent-bounties-protocol: agent-bounties/autonomous-v1`
- `x-agent-bounties-revision: <git commit>`

On Render, the revision comes from the platform-provided
[`RENDER_GIT_COMMIT`](https://render.com/docs/environment-variables) runtime
variable. Local services report `local`. Production smoke requires API and MCP
to advertise the same protocol and revision. When an expected revision is
provided, both services must match it exactly.

This prevents a healthy but stale deployment from passing merely because it
returns HTTP 200.

## Run It

PowerShell:

```powershell
.\scripts\check-production-smoke.ps1 `
  -ApiBaseUrl https://agent-bounties-api.onrender.com `
  -McpBaseUrl https://agent-bounties-mcp.onrender.com `
  -ExpectedRevision 0123456789abcdef0123456789abcdef01234567
```

Unix-like shells:

```bash
bash scripts/check-production-smoke.sh \
  --api-base-url https://agent-bounties-api.onrender.com \
  --mcp-base-url https://agent-bounties-mcp.onrender.com \
  --expected-revision 0123456789abcdef0123456789abcdef01234567
```

The wrappers also read:

- `PRODUCTION_API_BASE_URL`
- `PRODUCTION_MCP_BASE_URL`
- `PRODUCTION_EXPECTED_REVISION`

Use `-RequireEvalHistory` or `--require-eval-history` after a deployed eval
suite has persisted at least one run.

## GitHub Workflow

The `Production Smoke` workflow runs hourly and by manual dispatch. It defaults
to the canonical Render API and MCP URLs and the checked-out `main` revision,
so missing repository variables cannot turn the gate into a successful skip.
Repository variables can still override the URLs for a planned migration.

The workflow deliberately does not run as a pull-request or push check. Render
is configured with `autoDeployTrigger: commit` because `main` is already
protected by required pre-merge CI. A pre-deploy smoke cannot observe the new
revision, and making it required would create a deployment dependency cycle.
CI validates the local contract before merge, Render deploys the reviewed main
commit, and scheduled/manual production smoke validates the deployed result
afterward.

## Coverage

The gate checks:

- API and MCP health, protocol identity, and exact deployed revision.
- Autonomous-v1 discovery manifests, JSON Schema, `/llms.txt`, and MCP tools.
- OpenAPI paths for terms, creation, contribution, claim, submission,
  verification, settlement, expiry, cancellation, refunds, events, and
  transaction receipts.
- Absence of retired operator-signed escrow endpoints and tools.
- Base native USDC funding-before-claim requirements.
- Canonical `BountySettled` payment evidence boundaries.
- Deterministic module, signed quorum, and AI-judge quorum descriptions.
- Public post-value actions, including posting a new bounty and optional
  authenticated star/upvote execution.
- Persisted eval history when explicitly required.

Do not enable GitHub funding-comment handoffs against a hosted API until this
gate passes for that exact API URL and revision.
