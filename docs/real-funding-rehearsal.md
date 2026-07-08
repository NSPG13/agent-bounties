# Real Funding Rehearsal

This runbook exercises actual payment rails in safe development modes:

- Stripe test mode for fiat top-ups and Connect payout eligibility,
- Base Sepolia for USDC escrow funding and release,
- the platform ledger for pooled and mixed bounty accounting.

The invariant is the same in every mode: a request, Checkout Session,
transaction plan, signed transaction, broadcast, or transaction hash is not
funding and is not payout. Funding and distribution become platform state only
after deterministic evidence is reconciled:

- Stripe fiat funding: a verified `checkout.session.completed` webhook.
- Base USDC funding: an indexed `EscrowCreated` log.
- Base USDC payout: an indexed `EscrowReleased` log.
- Stripe fiat payout: a Connect eligibility snapshot or later payout evidence.

## Preconditions

Local setup:

```powershell
.\scripts\preflight.ps1 -Mode core
docker compose up -d postgres
$env:DATABASE_URL = "postgres://agent_bounties:agent_bounties@localhost:5432/agent_bounties"
cargo run -p api
```

Optional hosted safety controls:

```powershell
$env:OPERATOR_API_TOKEN = "<operator-token>"
$env:ENABLE_STRIPE_LIVE_EXECUTION = "true"
$env:STRIPE_SECRET_KEY = "sk_test_..."
$env:STRIPE_WEBHOOK_SECRET = "whsec_..."
$env:ENABLE_BASE_TX_BROADCAST = "true"
$env:BASE_SEPOLIA_RPC_URL = "https://..."
```

Base Sepolia also needs:

- a deployed `AgentBountyEscrow` contract,
- a test USDC token address for the same network,
- a payer wallet with test token balance,
- a settlement signer wallet that can call `release`, `refund`, and
  `markDisputed`.

Generate deployment and payout commands:

```powershell
cargo run -p cli -- base-sepolia-runbook `
  --settlement-signer <settlement-signer-address> `
  --escrow-contract <escrow-contract-address> `
  --usdc-token <base-sepolia-usdc-token-address>
```

## Fast Local Rehearsal

This command runs the complete mixed rail lifecycle with deterministic local
fixtures. It does not call Stripe or Base RPC, but it uses the same funding
intent, webhook reconciliation, escrow event reconciliation, proof, release, and
payout state-machine code used by hosted services.

```powershell
cargo run -p cli -- funding-rehearsal-demo
```

Expected evidence boundary in the JSON output:

- `stripe.funding_intent` starts as `AwaitingEvidence`.
- `stripe.checkout_request` is a test-mode Checkout Session request intent.
- `stripe.funding_reconciliation` applies only after the simulated paid
  webhook with `bounty_id` and `funding_intent_id` metadata.
- `base.funding_intent` starts as `AwaitingEvidence`.
- `base.funding_plan` contains unsigned Base Sepolia `approve` and
  `createEscrow` calls.
- `base.created_reconciliation` applies only after the simulated
  `EscrowCreated` log.
- `base.release_plan` is unsigned release calldata.
- `base.released_reconciliation` applies only after the simulated
  `EscrowReleased` log.

## Stripe Test Mode Funding

Use funding intents when a contributor wants to assign real fiat funding to a
bounty through Stripe test mode.

1. Open a pooled fiat or mixed bounty.

```powershell
curl -X POST http://127.0.0.1:8080/v1/bounties/pooled `
  -H "content-type: application/json" `
  --data '{"title":"Stripe test funded bounty","template_slug":"small-code-change","target_amount_minor":5000,"currency":"usd","funding_mode":"StripeFiatLedger","privacy":"Public","funding_targets":[]}'
```

2. Create a Stripe funding intent.

```powershell
New-Item -ItemType Directory -Force target | Out-Null
curl.exe -sS -X POST http://127.0.0.1:8080/v1/bounties/<bounty-id>/funding-intents `
  -H "content-type: application/json" `
  --data '{"bounty_id":"<bounty-id>","source_organization_id":"00000000-0000-0000-0000-000000000001","amount_minor":5000,"currency":"usd","rail":"StripeFiat","external_reference":"stripe-test-5000"}' `
  | Set-Content target\stripe-funding-intent.json
```

3. Execute the exact Checkout Session request in Stripe test mode.

```powershell
cargo run -p cli -- stripe-execute-request-intent `
  --intent-file target\stripe-funding-intent.json
```

Open the returned Checkout URL and pay with a Stripe test card. The Checkout
Session itself still does not credit the bounty. The command executes the
funding intent's own `StripeRequestIntent`, preserving `bounty_id`,
`funding_intent_id`, and `funding_intent_reference` metadata for webhook
reconciliation.

4. Reconcile the signed webhook.

Configure Stripe CLI or Dashboard webhooks to deliver
`checkout.session.completed` to:

```text
POST http://127.0.0.1:8080/v1/stripe/checkout-webhooks
```

The webhook must carry `metadata.bounty_id` and
`metadata.funding_intent_id`. After successful reconciliation, the platform
credits the source organization's Stripe balance and reserves that balance into
the bounty. Replaying the same Stripe event id must be ignored as a duplicate.

## Base Sepolia USDC Funding

Use Base funding intents when a contributor wants public, portable USDC escrow.

1. Open a Base or mixed bounty.

```powershell
curl -X POST http://127.0.0.1:8080/v1/bounties/pooled `
  -H "content-type: application/json" `
  --data '{"title":"Base Sepolia funded bounty","template_slug":"small-code-change","target_amount_minor":1000000,"currency":"usdc","funding_mode":"BaseUsdcEscrow","privacy":"Public","funding_targets":[]}'
```

2. Create a Base funding intent.

```powershell
curl -X POST http://127.0.0.1:8080/v1/bounties/<bounty-id>/funding-intents `
  -H "content-type: application/json" `
  --data '{"bounty_id":"<bounty-id>","amount_minor":1000000,"currency":"usdc","rail":"BaseUsdc","external_reference":"base-sepolia-1000000","base_escrow_contract":"<escrow-contract-address>","base_payer":"<payer-wallet>","base_token":"<base-sepolia-usdc-token-address>","base_network":"base-sepolia"}'
```

3. Sign and send the returned `approve` and `createEscrow` transactions from the
funding plan.

4. Reconcile the funding evidence.

```powershell
cargo run -p cli -- base-fetch-logs `
  --network base-sepolia `
  --escrow-contract <escrow-contract-address> `
  --from-block <deployment-or-funding-block>
```

Hosted operators can also use `POST /v1/base/fetch-rpc-logs` or reconcile a
transaction receipt with `reconcile_logs=true`. The bounty becomes claimable
only after the indexed `EscrowCreated` log matches bounty id, amount, token, and
terms hash.

## Mixed Stripe And Base Funding

Mixed bounties require explicit funding targets and settle each rail
separately.

```powershell
curl -X POST http://127.0.0.1:8080/v1/bounties/pooled `
  -H "content-type: application/json" `
  --data '{"title":"Mixed Stripe fiat and Base USDC bounty","template_slug":"payment-state-machine","target_amount_minor":5000,"currency":"usd","funding_mode":"MixedRails","privacy":"Public","funding_targets":[{"rail":"StripeFiat","amount_minor":5000,"currency":"usd"},{"rail":"BaseUsdc","amount_minor":1000000,"currency":"usdc"}]}'
```

Create one Stripe funding intent for the USD partition and one Base funding
intent for the USDC partition. The bounty remains unclaimable until both are
confirmed by their own evidence. The platform never nets USD and USDC into one
synthetic balance.

After deterministic verification:

- Stripe settlement creates blocked payout intents until Connect eligibility is
  reconciled.
- Base settlement creates pending release calldata until the settlement signer
  releases escrow and the indexed `EscrowReleased` log is reconciled.

## Distribution Feedback

Every funding, claiming, PR, or proof interaction should ask:

1. How did you find Agent Bounties?
2. What made this bounty or project worth participating in?
3. If an AI agent helped, what tool, prompt, link, label, or workflow led it
   here?

Those answers are distribution data only. They do not affect merge approval,
bounty acceptance, or payout authorization.
