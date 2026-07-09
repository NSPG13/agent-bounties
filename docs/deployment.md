# Deployment

The production package is one Dockerfile with build arguments for the service
binary. API, MCP, and worker containers use the same image recipe and differ by
`APP_PACKAGE`, `APP_BINARY`, bind address, and service-specific configuration.
Rust and Cargo 1.88 or newer are required for local builds because the locked
dependency graph includes crates with that minimum supported Rust version.

## Render Blueprint

The root [render.yaml](../render.yaml) is the lowest-friction hosted deployment
path for connecting the public website and Stripe funding page to a durable API.
It defines:

- `agent-bounties-api`: public API web service.
- `agent-bounties-mcp`: public MCP-compatible tool service.
- `agent-bounties-base-indexer`: background worker for Base escrow log polling.
- `agent-bounties-postgres`: shared Postgres database for API, MCP, and worker
  persistence.

The API and MCP binaries now fall back to Render's `PORT` environment variable
when `API_BIND_ADDR` or `MCP_BIND_ADDR` is unset, so hosted web services bind to
`0.0.0.0:$PORT` while local defaults remain `127.0.0.1:8080` and
`127.0.0.1:8090`.

Before applying the Blueprint, inspect and adjust these non-secret URL defaults
if you rename services or attach custom domains:

- `PUBLIC_BASE_URL=https://agent-bounties-api.onrender.com`
- `MCP_BASE_URL=https://agent-bounties-mcp.onrender.com`

The Blueprint intentionally starts with live-value mutation disabled:

- `ENABLE_STRIPE_LIVE_EXECUTION=false`
- `ENABLE_STRIPE_PUBLIC_CHECKOUT=false`
- `ENABLE_BASE_TX_BROADCAST=false`
- `ALLOW_UNSIGNED_STRIPE_WEBHOOKS=false`

Render generates `OPERATOR_API_TOKEN`. Stripe secrets, Base RPC URLs, escrow
contracts, settlement signer, platform fee wallet, and indexer start block are
Dashboard-provided values. `STRIPE_PAYMENT_METHOD_CONFIGURATION` is an optional
non-secret Stripe Dashboard id for targeting a Checkout payment-method set such
as a PayPal-enabled configuration. Do not enable public Checkout funding until
Stripe webhooks are configured and
`GET /v1/readiness/live-money?network=base-mainnet` reports the expected
non-secret readiness gates.

Validate the checked-in Blueprint contract without requiring the Render CLI:

```powershell
python scripts\check-render-blueprint.py
```

On Unix-like shells:

```bash
python3 scripts/check-render-blueprint.py
```

### Hosted API returns 404 (repair path)

If `https://agent-bounties-api.onrender.com/health` (or your `PUBLIC_BASE_URL`)
returns **404** for `/health`, `/v1/readiness/live-money`, and
`/v1/bounties/funding-feed`, run:

```powershell
python scripts\diagnose_hosted_api.py
python scripts\diagnose_hosted_api.py --base-url https://agent-bounties-api.onrender.com --json-out target/hosted-api-diagnosis.json
```

```bash
python3 scripts/diagnose_hosted_api.py
python3 scripts/diagnose_hosted_api.py --base-url https://agent-bounties-api.onrender.com --json-out target/hosted-api-diagnosis.json
```

Typical causes when **all** paths are 404 and DNS still resolves:

1. The Render Blueprint was never applied (no live `agent-bounties-api` service).
2. Docs still advertise `agent-bounties-api.onrender.com` but Render assigned a
   different hostname after rename/recreate.
3. The web service is running the wrong binary (`worker` instead of `api`) so
   HTTP routes are missing.
4. The service exists but failed health checks / is suspended (sometimes
   surfaces as 404/502 depending on platform edge).

Repair sequence:

1. Open
   `https://dashboard.render.com/blueprint/new?repo=https://github.com/NSPG13/agent-bounties`
   and apply [render.yaml](../render.yaml) on `main`.
2. Confirm web service **`agent-bounties-api`**: Docker runtime,
   `healthCheckPath: /health`, env `APP_PACKAGE=api`, `APP_BINARY=api`.
3. Confirm the process listens on `0.0.0.0:$PORT` (API already falls back to
   Render `PORT` when `API_BIND_ADDR` is unset).
4. Wait until deploy is **Live**, then re-run `diagnose_hosted_api.py` until
   `/health` is HTTP 200.
5. If the hostname differs, update `PUBLIC_BASE_URL` / `MCP_BASE_URL` on the
   service and in operator docs/vars.
6. Only after production smoke passes, set `PRODUCTION_API_BASE_URL` and
   `AGENT_BOUNTIES_API_BASE_URL` so funding pages do not advertise a dead API.

**Payment boundary:** a healthy `/health` response does **not** create funding,
credit balances, enable Checkout, or authorize payout. Stripe/Base rails still
require secrets + readiness gates in
[live-money-activation.md](live-money-activation.md).

To deploy from the Dashboard after the PR is merged:

1. Open
   `https://dashboard.render.com/blueprint/new?repo=https://github.com/NSPG13/agent-bounties`.
2. Connect GitHub if prompted and select the merged `main` branch.
3. Review the API, MCP, worker, and Postgres resources.
4. Fill the `sync: false` values for Stripe/Base only when you are ready to run
   the corresponding rail.
5. Apply the Blueprint and wait for all services to deploy.
6. Update `PUBLIC_BASE_URL` and `MCP_BASE_URL` if Render assigned different
   hostnames or you attached custom domains.
7. Run the read-only production smoke against the deployed URLs.
8. After production smoke passes, set repository variables
   `PRODUCTION_API_BASE_URL` and `PRODUCTION_MCP_BASE_URL` so the scheduled
   `Production Smoke` workflow can keep checking the hosted API/MCP surfaces.
9. Only after the same API URL passes production smoke, set
   `AGENT_BOUNTIES_API_BASE_URL` so GitHub funding-comment handoffs can prefill
   the public Stripe Checkout funding page. A dead or unverified API URL should
   not be advertised to funders.

The Blueprint uses low paid service/database plans because the Base indexer is a
background worker and real-money webhooks should not depend on sleeping
instances. For no-money experiments, use local Docker Compose or the local
service smoke instead.

## Container Images

Build the deployable API, MCP, and Base indexer worker images locally:

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
docker build --build-arg APP_PACKAGE=worker --build-arg APP_BINARY=worker -t agent-bounties-worker:local .
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
running, use the compose smoke. It validates and builds the optional
`base-indexer` profile, binds API/MCP to high local ports, runs the read-only
production smoke, and tears the stack down:

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
- optional Base RPC, escrow address, native USDC token, settlement signer, and
  platform-fee wallet variables
- optional Stripe live execution, public funder Checkout, API base URL, secret
  key, webhook secret, optional Payment Method Configuration id, and
  unsigned-webhook simulation variables
- optional `OPERATOR_API_TOKEN` for hosted operator-only mutation surfaces
- optional `base-indexer` profile variables for automated Base USDC escrow log
  polling

The API and MCP containers receive the same live-money environment contract so
`GET /v1/readiness/live-money` and MCP `get_live_money_readiness` agree about
Stripe webhook readiness, the non-secret Stripe payment-method configuration
indicator, Base escrow addresses, native USDC tokens, and operator mutation
protection. These readiness responses expose only whether
`STRIPE_PAYMENT_METHOD_CONFIGURATION` is configured, not the Stripe object id.

To run the Base USDC indexer alongside API and MCP, set the indexer variables
and opt into its compose profile:

```powershell
$env:COMPOSE_PROFILES = "base-indexer"
docker compose --env-file .env -f docker-compose.production.yml up -d --build
```

On Unix-like shells:

```bash
COMPOSE_PROFILES=base-indexer docker compose --env-file .env -f docker-compose.production.yml up -d --build
```

The worker uses `DATABASE_URL`, `BASE_INDEXER_NETWORK`, RPC/escrow contract
configuration, and `BASE_INDEXER_START_BLOCK` on first run. After it scans a
range, it persists a Postgres cursor keyed by network and escrow contract, so
later polls continue from the last confirmed scanned block even when no escrow
events were found. Each poll also persists a heartbeat with the last Success,
Skipped, or Failed outcome, block range, fetched log count, skipped reason, and
error message when present. Money state still changes only when decoded escrow
logs are persisted. Check `GET /v1/base/indexer-status?network=<network>` or
MCP `get_base_indexer_status` after startup to confirm the cursor and heartbeat
are being written; the status response is monitoring evidence, not settlement
authorization.

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

The same gate is available as the `Production Smoke` GitHub Actions workflow.
Use manual dispatch for one-off URL checks, or set repository variables
`PRODUCTION_API_BASE_URL` and `PRODUCTION_MCP_BASE_URL` for scheduled/push
checks. The workflow skips when URLs are absent and fails when configured hosted
endpoints are unhealthy. Treat a passing run as the prerequisite for configuring
`AGENT_BOUNTIES_API_BASE_URL` in the GitHub repository.

## Payment Controls

Keep live payment mutation disabled until operator controls and secrets are
ready:

- `ENABLE_BASE_TX_BROADCAST=false` keeps Base transaction broadcasting disabled.
- `ENABLE_STRIPE_LIVE_EXECUTION=false` keeps Stripe Checkout and Connect live
  creation disabled.
- `ENABLE_STRIPE_PUBLIC_CHECKOUT=false` keeps public funder Checkout disabled
  even when operator-only Stripe live execution is configured.
- `STRIPE_PAYMENT_METHOD_CONFIGURATION` can point Checkout Session creation at
  a Stripe Dashboard payment-method set. Use it for PayPal-capable Checkout
  only after Stripe supports and approves PayPal for the platform account.
- `OPERATOR_API_TOKEN` can require `Authorization: Bearer <token>` or
  `x-operator-token: <token>` on hosted risk review, settlement reconciliation,
  Base broadcast, receipt reconciliation, and live Stripe execution calls. Leave
  it unset for local open-source demos.
- Base receipt polling and log reconciliation still require configured RPC URLs
  and do not mark funds paid without indexed escrow logs.
- The optional `base-indexer` worker automates Base escrow log polling, but it
  does not sign or broadcast transactions and does not bypass deterministic log
  reconciliation.
- Stripe ledger credits require `STRIPE_WEBHOOK_SECRET` and verified Checkout
  webhooks. Keep `ALLOW_UNSIGNED_STRIPE_WEBHOOKS=false` outside local
  mock-provider simulations.

Do not put live private keys in compose files or committed env templates.

For a step-by-step low-value Base USDC beta process, including operator roles,
environment variables, funding, release, refund, dispute, and read-only
post-deploy checks, see
[Hosted Low-Value Base USDC Beta Runbook](hosted-base-usdc-beta-runbook.md).
