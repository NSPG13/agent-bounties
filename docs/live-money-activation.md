# Live Money Activation

This runbook turns the existing payment rails from local rehearsal into guarded
real-value movement. It does not require secrets in the repository. Operators
provide Stripe keys, Base RPC URLs, deployed escrow addresses, and webhook
secrets in the hosted environment.

## Rails

- Stripe fiat ledger: creates Checkout Sessions and Connect transfer requests.
  Ledger credit requires a signed `checkout.session.completed` webhook.
  Payout evidence requires a signed `transfer.created` webhook.
- Base USDC escrow: creates unsigned funding, release, refund, and dispute
  transaction plans. Funding and payout state changes require indexed escrow
  logs reconciled through the API/MCP operator surfaces or the hosted
  `base-indexer` worker.
- Pooled and mixed funding: each funding partition is reconciled independently.
  USD and USDC are never netted into a synthetic balance.

## Required Configuration

Use `.env.example` as the shape for hosted secrets and deployment settings.

```powershell
$env:OPERATOR_API_TOKEN = "<strong-random-token>"
$env:ENABLE_STRIPE_LIVE_EXECUTION = "true"
$env:ENABLE_STRIPE_PUBLIC_CHECKOUT = "false" # true only after website, limits, and webhooks are ready
$env:STRIPE_SECRET_KEY = "sk_test_..." # use sk_live_ only after test-mode signoff
$env:STRIPE_WEBHOOK_SECRET = "whsec_..."
$env:ALLOW_UNSIGNED_STRIPE_WEBHOOKS = "false"
$env:BASE_SEPOLIA_RPC_URL = "https://..."
$env:BASE_MAINNET_RPC_URL = "https://..."
$env:BASE_SEPOLIA_ESCROW_CONTRACT = "0x..."
$env:BASE_MAINNET_ESCROW_CONTRACT = "0x..."
$env:BASE_SETTLEMENT_SIGNER = "0x..."
$env:BASE_PLATFORM_FEE_WALLET = "0x..."
$env:BASE_INDEXER_NETWORK = "base-sepolia" # use base-mainnet after live signoff
$env:BASE_INDEXER_START_BLOCK = "<escrow-deployment-block>"
$env:BASE_INDEXER_POLL_SECONDS = "15"
$env:BASE_INDEXER_CONFIRMATIONS = "2"
$env:BASE_INDEXER_MAX_BLOCKS_PER_QUERY = "2000"
```

Native USDC addresses:

- Base Sepolia: `0x036CbD53842c5426634e7929541eC2318f3dCF7e`
- Base mainnet: `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913`

## Readiness Gates

Run the compatibility report for rehearsal artifacts:

```powershell
cargo run -p cli -- real-funding-readiness `
  --network base-sepolia `
  --escrow-contract $env:BASE_SEPOLIA_ESCROW_CONTRACT `
  --usdc-token $env:BASE_SEPOLIA_USDC_TOKEN
```

Run the stricter live-money gate before enabling production value movement:

```powershell
cargo run -p cli -- real-funding-readiness `
  --network base-mainnet `
  --escrow-contract $env:BASE_MAINNET_ESCROW_CONTRACT `
  --usdc-token $env:BASE_MAINNET_USDC_TOKEN `
  --require-live-money
```

The strict gate requires:

- `sk_live_` Stripe credentials,
- `ENABLE_STRIPE_LIVE_EXECUTION=true`,
- signed Stripe webhooks,
- `ALLOW_UNSIGNED_STRIPE_WEBHOOKS=false`,
- `OPERATOR_API_TOKEN`,
- Base mainnet RPC,
- deployed Base mainnet escrow contract,
- native Base USDC token.

Hosted API and MCP services expose the same non-secret readiness evidence:

```powershell
curl "$env:PUBLIC_BASE_URL/v1/readiness/live-money?network=base-mainnet"
curl "$env:PUBLIC_BASE_URL/v1/base/indexer-status?network=base-mainnet"
curl "$env:MCP_BASE_URL/tools/get_live_money_readiness" `
  -H "content-type: application/json" `
  --data '{"network":"base-mainnet"}'
curl "$env:MCP_BASE_URL/tools/get_base_indexer_status" `
  -H "content-type: application/json" `
  --data '{"network":"base-mainnet"}'
```

These reports intentionally expose only Stripe key mode, configured gates, chain
metadata, native USDC address, indexer cursor state, last worker heartbeat
outcome, warnings, and settlement evidence boundaries. They must not expose
Stripe secrets, webhook secrets, RPC URLs, or operator tokens. Indexer status is
monitoring evidence only; it does not fund, release, refund, dispute, or
authorize settlement.

## Automated Base Indexing

For hosted Base USDC value movement, run the worker from the production compose
profile after setting the network, RPC URL, escrow contract, and first scan
block:

```powershell
$env:COMPOSE_PROFILES = "base-indexer"
docker compose --env-file .env -f docker-compose.production.yml up -d --build
```

The first run requires `BASE_INDEXER_START_BLOCK`; use the escrow contract
deployment block, not block zero. The worker fetches `eth_blockNumber`,
subtracts `BASE_INDEXER_CONFIRMATIONS`, scans up to
`BASE_INDEXER_MAX_BLOCKS_PER_QUERY` confirmed blocks, applies decoded escrow
logs through the same deterministic state machine as the API/MCP reconciliation
endpoints, and persists a Postgres scan cursor. Empty ranges advance the scan
cursor; failed ranges do not. Every poll writes a heartbeat row so operators
and agents can see the latest Success, Skipped, or Failed poll outcome without
granting that heartbeat settlement authority.
Check `GET /v1/base/indexer-status?network=<network>` after startup to confirm
Postgres persistence is configured and cursor plus heartbeat records are being
written for the escrow contract.

For a one-shot rehearsal without a long-running container, run:

```powershell
$env:DATABASE_URL = "postgres://..."
cargo run -p worker -- --once
```

## Funding Flow

1. Post or discover a public funding-ready bounty through
   `GET /v1/bounties/funding-feed`.
2. Create a funding intent with `POST /v1/bounties/{id}/funding-intents`.
3. For Stripe, execute the returned Checkout Sessions request through the
   public funding-intent Checkout endpoint, live operator endpoint, or CLI. Do
   not credit the bounty yet.
4. For Base, sign and send the returned USDC `approve` and escrow
   `createEscrow` transactions from the funder's wallet. Do not mark funded yet.
5. Reconcile evidence:
   - Stripe: signed `checkout.session.completed` webhook with matching metadata.
   - Base: indexed `EscrowCreated` log matching bounty id, token, amount, and
     terms hash. In hosted deployments, the `base-indexer` worker should pick
     this up automatically after the configured confirmation depth.

Only after all required partitions are reconciled does the bounty become
claimable.

Public card funding uses:

```powershell
curl -X POST "$env:PUBLIC_BASE_URL/v1/stripe/live/funding-intents/{id}/checkout-session"
```

This endpoint is unavailable unless both `ENABLE_STRIPE_LIVE_EXECUTION=true`
and `ENABLE_STRIPE_PUBLIC_CHECKOUT=true` are set. It can execute only a stored
`StripeFiat` funding intent and returns a Stripe-hosted Checkout URL. It does
not credit balances or make the bounty claimable; signed webhook evidence is
still required.

## Payout Flow

1. Solver submits an artifact.
2. Deterministic verifier or operator accepts it and creates settlement intents.
3. For Base payouts, inspect `POST /v1/base/release-queue`, then generate a
   release plan with `POST /v1/base/release-plan`.
4. Sign and send the release transaction, then reconcile the indexed
   `EscrowReleased` log. The hosted `base-indexer` worker can reconcile this
   automatically; the transaction hash alone is not payout evidence.
5. For Stripe payouts, confirm Connect eligibility, execute the transfer
   request, then reconcile the signed `transfer.created` event. Transfer
   planning alone is not payout evidence.

## Negative Controls

- Leave `ENABLE_STRIPE_LIVE_EXECUTION=false` in local demos.
- Never use `ALLOW_UNSIGNED_STRIPE_WEBHOOKS=true` in hosted real-money
  environments.
- Keep `ENABLE_BASE_TX_BROADCAST=false` unless the service should submit
  already-signed raw transactions.
- AI judges can request review or clarification but cannot authorize funding or
  payout settlement.
