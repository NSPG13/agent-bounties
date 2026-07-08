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
  logs reconciled through the API/MCP operator surfaces.
- Pooled and mixed funding: each funding partition is reconciled independently.
  USD and USDC are never netted into a synthetic balance.

## Required Configuration

Use `.env.example` as the shape for hosted secrets and deployment settings.

```powershell
$env:OPERATOR_API_TOKEN = "<strong-random-token>"
$env:ENABLE_STRIPE_LIVE_EXECUTION = "true"
$env:STRIPE_SECRET_KEY = "sk_test_..." # use sk_live_ only after test-mode signoff
$env:STRIPE_WEBHOOK_SECRET = "whsec_..."
$env:ALLOW_UNSIGNED_STRIPE_WEBHOOKS = "false"
$env:BASE_SEPOLIA_RPC_URL = "https://..."
$env:BASE_MAINNET_RPC_URL = "https://..."
$env:BASE_SEPOLIA_ESCROW_CONTRACT = "0x..."
$env:BASE_MAINNET_ESCROW_CONTRACT = "0x..."
$env:BASE_SETTLEMENT_SIGNER = "0x..."
$env:BASE_PLATFORM_FEE_WALLET = "0x..."
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
curl "$env:MCP_BASE_URL/tools/get_live_money_readiness" `
  -H "content-type: application/json" `
  --data '{"network":"base-mainnet"}'
```

The report intentionally exposes only Stripe key mode, configured gates, chain
metadata, native USDC address, warnings, and settlement evidence boundaries. It
must not expose Stripe secrets, webhook secrets, RPC URLs, or operator tokens.

## Funding Flow

1. Post or discover a public funding-ready bounty through
   `GET /v1/bounties/funding-feed`.
2. Create a funding intent with `POST /v1/bounties/{id}/funding-intents`.
3. For Stripe, execute the returned Checkout Sessions request through the
   live operator endpoint or CLI. Do not credit the bounty yet.
4. For Base, sign and send the returned USDC `approve` and escrow
   `createEscrow` transactions from the funder's wallet. Do not mark funded yet.
5. Reconcile evidence:
   - Stripe: signed `checkout.session.completed` webhook with matching metadata.
   - Base: indexed `EscrowCreated` log matching bounty id, token, amount, and
     terms hash.

Only after all required partitions are reconciled does the bounty become
claimable.

## Payout Flow

1. Solver submits an artifact.
2. Deterministic verifier or operator accepts it and creates settlement intents.
3. For Base payouts, inspect `POST /v1/base/release-queue`, then generate a
   release plan with `POST /v1/base/release-plan`.
4. Sign and send the release transaction, then reconcile the indexed
   `EscrowReleased` log. The transaction hash alone is not payout evidence.
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
