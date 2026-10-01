# Algorand x402 opportunity reports

A separate, stateless paid report service for the Global x402 Challenge.
It filters the existing public bounty feed by keywords and maximum claim bond,
ranks matching work, and returns deadlines, review requirements, acceptance
criteria, funding references, and a source digest. Raw source data remains free.

`GET /v1/opportunity-report?q=parser&maxBond=1000000&limit=10`

One report costs **0.01 USDC** on Algorand Mainnet, asset **31566704**.
Bounty escrow/rewards still use Base; buying a report is not funding or claiming
a bounty. Feed observations are not an independent proof of canonical state.

## Run

Node 24. `npm ci --ignore-scripts`, then `npm test`.
Set `ALGORAND_PAY_TO` to a valid public wallet, `ALGORAND_NETWORK=mainnet`, and
`PUBLIC_ORIGIN=https://your-service` (Render's `RENDER_EXTERNAL_URL` is the
default). Run `npm start`. No merchant or payer key belongs in the service.
`/health`, `/.well-known/x402.json`, and `/llms.txt` are free.

Payments use the official x402 SDK and GoPlausible `/verify` and `/settle`.
The 402 response declares Bazaar schema, service metadata, and
`accepts[].extra.tag=x402-global-challenge`. The exact legacy Algorand network
identifier is intentional: it matches GoPlausible's live `/supported` response.
SDK 2.28 accepts and normalizes that identifier. Dependencies are pinned.

## Wallet and activation

`npm run wallet:create` creates merchant and one-time canary-payer accounts.
Secret backups are exclusive-created with mode 0600 beneath
`~/.config/agent-bounties/algorand-challenge/` (directory mode 0700).
Keep those backups private and back them up safely. The server receives only
the merchant's public address. The extra payer is for one operator test, not
artificial customer volume.

Each fresh account needs 0.2 ALGO minimum balance to hold one ASA, plus its
opt-in fee. Funding each with **0.21 ALGO** leaves room for the 0.001 ALGO
opt-in. Send ALGO first; USDC transfers fail until opt-in is confirmed.

Preview each transaction with:

```
node scripts/opt-in.mjs merchant mainnet
node scripts/opt-in.mjs canary-payer mainnet
```

After explicit approval, append `--execute-once` to perform the corresponding
zero-amount self-transfer, capped at 0.001 ALGO. The exclusive journal prevents
blind rebroadcast after ambiguous outcomes. Inspect the recorded transaction
before any manual recovery. Then fund the canary payer with **0.01 Algorand
USDC (ASA 31566704)** and execute one paid request. Do not send Base USDC to an
Algorand address. GoPlausible advertises sponsored x402 transaction fees.

Reconcile all three pieces: paid HTTP response, successful facilitator receipt,
and confirmed Algorand ASA transfer to the merchant, followed by catalog and
challenge-filtered leaderboard visibility. A 402, a test double, a configured
tag, or a wallet balance alone is not completion.

```
node scripts/check-readiness.mjs https://your-service output.json
```

This read-only checker checks all catalog pages, exact receiver and network,
USDC opt-in and leaderboard attribution. It deliberately does not assert paid
delivery. No recurring self-payments are created.

## Deploy and rollback

Use a separate free Node Render web service, build command
`npm ci --prefix tools/algorand-x402 --ignore-scripts`, start command
`npm start --prefix tools/algorand-x402`, `NODE_VERSION=24.19.0`, and the public
receiver/network environment variables above. Disable automatic deploys and
pin the reviewed branch. The service does not use any production database,
signer, existing Render environment group, or private runtime configuration.
Suspend this service to roll back. No migration or escrow rollback is needed.

## Threat model and validation

Only the SDK settlement success releases the response. Invalid authorizations
and failed settlement return no report. Upstream errors fail before settlement;
an upstream outage is never silently turned into a successful empty report.
Source URLs are fixed, input is bounded, redirects are rejected, and source
responses are capped at 8 MB. Public metadata pins the resource origin.
Keys stay local. No privileged wallet or funding authority is available to the
web process. Settlement timeout is an unknown payment outcome; inspect the
chain before retrying. A caller can fetch the public source for free.

Focused tests exercise the real x402 middleware with a fake facilitator:
unpaid/invalid payment, failed settle, source failure, successful response
gating, metadata propagation, route bypasses, and unsafe inventory exclusion.
These tests are not a live Testnet or Mainnet settlement rehearsal.

New service is confined to this directory. The 102 open public PRs were
inspected on 2026-10-01; none introduced Algorand/GoPlausible/Bazaar changes.
This additive service does not alter the source feed or existing payment APIs.

References:
- https://algorand.co/blog/the-x402-global-challenge-is-live-how-to-build-submit-your-entry
- https://algorand.co/blog/is-your-x402-endpoint-showing-up-in-the-facilitator-leaderboard-how-to-troubleshoot-if-not
- https://github.com/algorandfoundation/x402-demo
- https://dev.algorand.co/concepts/assets/asset-operations/

The reminder states September 30, 2026 as submission deadline without a cutoff
time. Registration is not final submission; deploying this service does not
prove timely submission or contest acceptance.
