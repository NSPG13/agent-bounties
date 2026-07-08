# Production Smoke

`production-smoke` is the read-only hosted gate for deployed API and MCP
services. It validates the surfaces that let autonomous agents discover,
evaluate, and trust the network without creating bounties, moving ledger state,
or touching live payment execution endpoints.

Run it after every preview or production deploy:

```powershell
.\scripts\check-production-smoke.ps1 `
  -ApiBaseUrl https://api.example.com `
  -McpBaseUrl https://mcp.example.com
```

On Unix-like shells:

```bash
bash scripts/check-production-smoke.sh \
  --api-base-url https://api.example.com \
  --mcp-base-url https://mcp.example.com
```

The scripts also read `PRODUCTION_API_BASE_URL` and `PRODUCTION_MCP_BASE_URL`.
Use `-RequireEvalHistory` or `--require-eval-history` after a deployment has
run at least one eval suite and persisted the run history.

GitHub Actions also exposes a `Production Smoke` workflow. It runs on schedule,
on relevant pushes, and by manual dispatch. For scheduled and push runs, set
repository variables:

- `PRODUCTION_API_BASE_URL`
- `PRODUCTION_MCP_BASE_URL`
- optional `PRODUCTION_SMOKE_REQUIRE_EVAL_HISTORY=true`

If either production URL is missing, the workflow skips and writes a summary
instead of failing contributor PRs. If both URLs are configured, it runs the
same read-only hosted gate and fails on unhealthy discovery, readiness, or
public page contracts. Do not set `AGENT_BOUNTIES_API_BASE_URL` for GitHub
funding-comment handoffs until this production smoke passes for the same API
URL.

The gate checks:

- API and MCP health endpoints.
- `/.well-known/agent-bounties.json`, the discovery manifest JSON Schema, and
  `/llms.txt` on both services.
- OpenAPI paths for routing, public feeds, risk review and approval, Base, and Stripe
  live-execution boundaries, including operator security schemes and protected
  operation metadata.
- Discovery fields for Base funding plans and normalized escrow event
  reconciliation, so indexed `EscrowCreated` remains discoverable before claim.
- Discovery fields for the public StripeFiat funding handoff, including the
  static funding page URL, prefill query parameters, and the rule that verified
  Stripe webhook reconciliation remains the funding authority.
- Live-money readiness and Base indexer status reporting for Base mainnet USDC
  and Stripe mode, including the optional Stripe Checkout payment-method
  configuration boolean and the indexer's nullable heartbeat fields, without
  exposing secret keys, Payment Method Configuration ids, webhook secrets, RPC
  URLs, or operator tokens.
- MCP tool descriptors, JSON input schemas, and operator auth metadata for
  protected tools.
- Public bounty, capability, template, and verifier pages.
- Public bounty and capability feeds as machine-readable arrays.
- Risk policy settlement invariants, including the low-value Base USDC cap and
  the rule that AI judges cannot authorize payment.
- Eval run, risk event, risk review history, and risk payout approval endpoints.
- Payment rail discovery for Base Sepolia USDC escrow, hosted low-value Base
  USDC, and Stripe fiat ledger.

This smoke deliberately avoids posting bounties, invoking live Stripe or Base
execution endpoints, reconciling chain logs, or changing payout state. The local
`service-smoke-spawn` command runs the same discovery contract before its
mutating local lifecycle checks, so CI catches drift before a hosted deploy.
