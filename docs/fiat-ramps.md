# Non-Custodial Fiat Ramps

Individuals pay and earn in dollars while AgentBounties never holds their
funds ([ADR 0006](adr/0006-protocol-v2-platform-fee-and-non-custodial-fiat.md)).
The user's own wallet does all of these:
- receives USDC on Base from a licensed onramp;
- signs one exact EIP-3009 authorization that a relayer submits, so the user
  needs no ETH for gas;
- receives the solver payout from `BountySettled`;
- sells USDC to a licensed offramp to cash out.

The wallet is usually the Coinbase CDP embedded wallet
(`docs/coinbase-embedded-wallet.md`).

Ramp providers are the merchant of record for the purchase or sale. They run
KYC, fraud and chargeback handling. None of the steps below is bounty funding
or payout evidence: only canonical `FundingAdded` and `BountySettled` events
are.

## Buy USDC

| Provider | Route | Notes |
| --- | --- | --- |
| Stripe crypto onramp | `POST /v1/onramps/stripe/session` (API) | Mints a session with the wallet address locked, USDC only, on Base, for an approved origin. Returns the `client_secret` and publishable key for Stripe's embedded onramp element |
| MoonPay | `POST /v1/onramps/moonpay/checkout` (MCP server) | Existing signed, IP-bound checkout link (`docs/moonpay-onramp.md`) |

**Stripe session settings**

`ENABLE_STRIPE_CRYPTO_ONRAMP=true` turns the route on. It also needs:
- `STRIPE_SECRET_KEY` and `STRIPE_CRYPTO_ONRAMP_PUBLISHABLE_KEY`, in the same
  Stripe mode (test with test, live with live);
- `STRIPE_CRYPTO_ONRAMP_ALLOWED_ORIGINS` (default `https://agentbounties.app`);
- `STRIPE_CRYPTO_ONRAMP_CLIENT_IP_HEADER` (default `x-forwarded-for`). Live
  sessions require a public customer IP;
- `STRIPE_CRYPTO_ONRAMP_MIN_USDC` and `_MAX_USDC` (defaults 1.00 and
  2,500.00);
- `STRIPE_CRYPTO_ONRAMP_SESSIONS_PER_MINUTE` (default 5 per client);
- `STRIPE_CRYPTO_ONRAMP_NETWORK` and `STRIPE_CRYPTO_ONRAMP_WALLET_KEY` (both
  default `base`).

**Before activation**
- Stripe's onramp API is in public preview and requires an approved onramp
  application even for sandbox testing.
- Stripe supports it in the US (excluding Hawaii) and the EU. It announced
  Base USDC support for US customers.
- Confirm the Base network identifier and the wallet-address key in a sandbox
  session before enabling the route.

## Cash out USDC

`POST /v1/offramps/moonpay/sell` (MCP server) returns a signed MoonPay sell
link for the user's own wallet:
- `baseCurrencyCode` is the configured Base USDC code, `quoteCurrencyCode` is
  `usd`, and `refundWalletAddress` is the seller's wallet;
- it uses the same origin allowlist, IP binding, rate limit and amount bounds
  as MoonPay checkout.

MoonPay shows a deposit address. Only the user's wallet signs the USDC
transfer to it; AgentBounties never moves the funds.

## Evidence boundaries

| Step | What it proves |
| --- | --- |
| Onramp session or MoonPay link created | Nothing about money |
| Onramp `fulfillment_complete` or wallet balance | USDC reached the user's wallet; the bounty is still unfunded |
| Signed EIP-3009 authorization or relay hash | Intent only |
| Canonical `FundingAdded` | The bounty is funded |
| Canonical `BountySettled` | The solver was paid |
| MoonPay sell completion | MoonPay's payout, under MoonPay's terms |
