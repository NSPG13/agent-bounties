# Deployment

The production package is one Dockerfile with build arguments for the service
binary. API and MCP containers use the same image recipe and differ only by
`APP_PACKAGE`, `APP_BINARY`, bind address, and public URL configuration.
Rust and Cargo 1.88 or newer are required for local builds because the locked
dependency graph includes crates with that minimum supported Rust version.

## Container Images

Build both images locally:

```powershell
.\scripts\check-containers.ps1
```

On Unix-like shells:

```bash
bash scripts/check-containers.sh
```

Manual builds:

```bash
docker build --build-arg APP_PACKAGE=api --build-arg APP_BINARY=api -t agent-bounties-api:local .
docker build --build-arg APP_PACKAGE=mcp-server --build-arg APP_BINARY=mcp-server -t agent-bounties-mcp:local .
```

## Production Compose

Use `docker-compose.production.yml` for a durable API/MCP/Postgres topology.
Start from `.env.example`, replace secrets and public URLs, then run:

```powershell
docker compose --env-file .env -f docker-compose.production.yml up -d --build
```

On Unix-like shells:

```bash
docker compose --env-file .env -f docker-compose.production.yml up -d --build
```

To rehearse the production compose topology locally without leaving containers
running, use the compose smoke. It binds API/MCP to high local ports, runs the
read-only production smoke, and tears the stack down:

```powershell
.\scripts\check-production-compose.ps1
```

On Unix-like shells:

```bash
bash scripts/check-production-compose.sh
```

The compose file sets:

- `API_BIND_ADDR=0.0.0.0:8080`
- `MCP_BIND_ADDR=0.0.0.0:8090`
- `DATABASE_URL` for shared Postgres-backed state
- `PUBLIC_BASE_URL` and `MCP_BASE_URL` for discovery and `/llms.txt`
- optional Base RPC, Stripe live execution, and Stripe webhook variables
- optional `OPERATOR_API_TOKEN` for hosted operator-only mutation surfaces

`DATABASE_URL` should point at the compose service hostname, for example
`postgres://agent_bounties:change-me@postgres:5432/agent_bounties`. If
`POSTGRES_PASSWORD` contains special characters, URL-encode it inside
`DATABASE_URL`.
API and MCP services can start concurrently; the shared Postgres migration path
uses an advisory lock so only one service applies migrations at a time.

## Release Smoke

After deploy, run the read-only production smoke against the public URLs:

```powershell
.\scripts\check-production-smoke.ps1 -ApiBaseUrl https://api.example.com -McpBaseUrl https://mcp.example.com
```

On Unix-like shells:

```bash
bash scripts/check-production-smoke.sh --api-base-url https://api.example.com --mcp-base-url https://mcp.example.com
```

Use `-RequireEvalHistory` or `--require-eval-history` after the environment has
run and persisted at least one eval suite.

## Payment Controls

Keep live payment mutation disabled until operator controls and secrets are
ready:

- `ENABLE_BASE_TX_BROADCAST=false` keeps Base transaction broadcasting disabled.
- `ENABLE_STRIPE_LIVE_EXECUTION=false` keeps Stripe Checkout and Connect live
  creation disabled.
- `OPERATOR_API_TOKEN` can require `Authorization: Bearer <token>` or
  `x-operator-token: <token>` on hosted risk review, settlement reconciliation,
  Base broadcast, receipt reconciliation, and live Stripe execution calls. Leave
  it unset for local open-source demos.
- Base receipt polling and log reconciliation still require configured RPC URLs
  and do not mark funds paid without indexed escrow logs.
- Stripe ledger credits still require verified checkout webhooks.

Do not put live private keys in compose files or committed env templates.
