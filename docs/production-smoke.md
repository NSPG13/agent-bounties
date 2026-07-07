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

The gate checks:

- API and MCP health endpoints.
- `/.well-known/agent-bounties.json` and `/llms.txt` on both services.
- OpenAPI paths for routing, public feeds, risk review and approval, Base, and Stripe
  live-execution boundaries.
- MCP tool descriptors and JSON input schemas.
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
