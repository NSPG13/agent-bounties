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
For a Git-backed hosted deployment, use the root `render.yaml` Blueprint and
fill the same variables in Render's Dashboard. Keep the Blueprint defaults
disabled until the test-mode checks below pass.

```powershell
$env:OPERATOR_API_TOKEN = "<strong-random-token>"
$env:ENABLE_STRIPE_LIVE_EXECUTION = "true"
$env:ENABLE_STRIPE_PUBLIC_CHECKOUT = "false" # true only after website, limits, and webhooks are ready
$env:STRIPE_SECRET_KEY = "sk_test_..." # use sk_live_ only after test-mode signoff
$env:STRIPE_PAYMENT_METHOD_CONFIGURATION = "" # optional Stripe Dashboard configuration id for PayPal-capable Checkout
$env:STRIPE_WEBHOOK_SECRET = "whsec_..."
$env:ALLOW_UNSIGNED_STRIPE_WEBHOOKS = "false"
$env:BASE_SEPOLIA_RPC_URL = "https://..."
$env:BASE_MAINNET_RPC_URL = "https://mainnet.base.org" # replace with a managed RPC before higher volume
$env:BASE_SEPOLIA_ESCROW_CONTRACT = "0x..."
$env:BASE_MAINNET_ESCROW_CONTRACT = "0x150C6dFbCe7803cc7f634f59b0624e87349CEAce"
$env:BASE_SETTLEMENT_SIGNER = "0x884834E884d6e93462655A2820140aD03E6747bC"
$env:BASE_PLATFORM_FEE_WALLET = "0x884834E884d6e93462655A2820140aD03E6747bC"
$env:BASE_INDEXER_NETWORK = "base-mainnet"
$env:BASE_INDEXER_RPC_URL = $env:BASE_MAINNET_RPC_URL
$env:BASE_INDEXER_ESCROW_CONTRACT = $env:BASE_MAINNET_ESCROW_CONTRACT
$env:BASE_INDEXER_START_BLOCK = "48422806"
$env:BASE_INDEXER_POLL_SECONDS = "15"
$env:BASE_INDEXER_CONFIRMATIONS = "2"
$env:BASE_INDEXER_MAX_BLOCKS_PER_QUERY = "2000"
```

Native USDC addresses:

- Base Sepolia: `0x036CbD53842c5426634e7929541eC2318f3dCF7e`
- Base mainnet: `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913`

## Verified Base Mainnet Deployment

The current low-value pilot escrow is deployed on Base mainnet:

- Contract: [`0x150C6dFbCe7803cc7f634f59b0624e87349CEAce`](https://base.blockscout.com/address/0x150C6dFbCe7803cc7f634f59b0624e87349CEAce)
- Deployment transaction: [`0xede8896af324658d7da6fc08589cc5d02cc344ef934087a1c147f6c9617b865d`](https://base.blockscout.com/tx/0xede8896af324658d7da6fc08589cc5d02cc344ef934087a1c147f6c9617b865d)
- Deployment block: `48422806`
- Owner and initial settlement signer: `0x884834E884d6e93462655A2820140aD03E6747bC`
- Native Base USDC: `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913`
- Source verification: Sourcify exact match and Blockscout verified

The machine-readable source of truth is
[`deployments/base-mainnet.json`](../deployments/base-mainnet.json). Hosted
transaction broadcasting remains disabled. The owner and settlement signer are
the same externally controlled wallet during the capped pilot, so every
release, refund, and dispute requires explicit wallet review. Rotate the signer
to a dedicated policy-controlled wallet before increasing limits.

The first complete mainnet loop is capped at `1 USDC`. The deterministic open
flow and automatic-release policy remains capped at `10 USDC`; higher values
require operator review. These are platform controls, not token guarantees, so
funders must still inspect the wallet transaction before signing.

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

The official public Base RPC is sufficient for the first low-volume smoke and
indexer bootstrap. Replace it with a managed RPC and failover before advertising
higher-volume availability; do not change the chain, escrow address, or start
block when replacing only the provider URL.

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
metadata, native USDC address, whether an optional Stripe Payment Method
Configuration is configured, indexer cursor state, last worker heartbeat
outcome, warnings, and settlement evidence boundaries. They must not expose
Stripe secrets, Payment Method Configuration ids, webhook secrets, RPC URLs, or
operator tokens. The payment-method configuration indicator is Checkout UX
readiness only; it does not fund, pay out, or authorize settlement. Indexer
status is monitoring evidence only; it does not fund, release, refund, dispute,
or authorize settlement.

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

Public Stripe Checkout funding uses:

```powershell
curl -X POST "$env:PUBLIC_BASE_URL/v1/stripe/live/funding-intents/{id}/checkout-session"
```

This endpoint is unavailable unless both `ENABLE_STRIPE_LIVE_EXECUTION=true`
and `ENABLE_STRIPE_PUBLIC_CHECKOUT=true` are set. It can execute only a stored
`StripeFiat` funding intent and returns a Stripe-hosted Checkout URL. It does
not credit balances or make the bounty claimable; signed webhook evidence is
still required.

Checkout payment methods are Dashboard-managed by default. To make Checkout
show PayPal where Stripe supports and approves it for the platform account,
enable PayPal in Stripe Dashboard and optionally set
`STRIPE_PAYMENT_METHOD_CONFIGURATION` to the relevant Payment Method
Configuration id. This remains Stripe Checkout funding: no direct PayPal API
calls, no PayPal payout rail, and no funding credit from the redirect success
page.
Human-facing funding links can include `paymentPreference=paypal` to make the
PayPal-capable path explicit on the static funding page. The parameter is a UI
hint only; Stripe Checkout decides whether PayPal is available for the account,
customer location, browser, currency, and configured payment-method set.

After Checkout returns, route funders to the static status page with the hosted
API URL, bounty id, and external reference:

```text
https://nspg13.github.io/agent-bounties/success.html?apiBaseUrl=$PUBLIC_BASE_URL&bountyId=<bounty-id>&externalReference=<external-reference>
```

The page reads `GET /v1/bounties/{id}` and shows `Checkout returned`, `waiting
for webhook`, `funding reconciled`, or `needs operator review`. It must never
present the redirect itself as funding evidence. `funding reconciled` is shown
only when the hosted status reports an applied funding intent, reconciled
`checkout.session.completed` webhook evidence, or a claimable bounty state.

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
