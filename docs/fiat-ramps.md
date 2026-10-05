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
- `STRIPE_CRYPTO_ONRAMP_CLIENT_IP_HEADER` (default `x-forwarded-for`) and
  `STRIPE_CRYPTO_ONRAMP_TRUSTED_PROXY_HOPS` (default 1). Proxies append to
  `X-Forwarded-For` and a client can prepend anything, so the client IP is the
  entry that many trusted proxies from the right. A count that is too low
  returns a proxy's IP, so every client shares one bucket and live sessions
  are refused. A count that is too high reaches entries the client wrote. On
  Render, set the header to `true-client-ip` and keep the count at 1 (see
  [Client IP on Render](#client-ip-on-render)). Live sessions require a public
  customer IP;
- `STRIPE_CRYPTO_ONRAMP_MIN_USDC` and `_MAX_USDC` (defaults 1.00 and
  2,500.00);
- rate limits per minute: `STRIPE_CRYPTO_ONRAMP_SESSIONS_PER_MINUTE` per
  client IP (or IPv6 /64, default 5), `STRIPE_CRYPTO_ONRAMP_WALLET_SESSIONS_PER_MINUTE`
  per wallet (default 3) and `STRIPE_CRYPTO_ONRAMP_GLOBAL_SESSIONS_PER_MINUTE`
  overall (default 30). The global cap bounds Stripe API use however the
  client IP is derived;
- `STRIPE_CRYPTO_ONRAMP_NETWORK` and `STRIPE_CRYPTO_ONRAMP_WALLET_KEY` (both
  default `base`).

The Origin check stops cross-site browser requests but is not
authentication, since scripts can send any Origin. Every input is validated
before it reaches the rate limiter, the limiter's memory is bounded, each
session uses a server-generated idempotency id (a caller-chosen one could
replay another caller's `client_secret`), and Stripe's error bodies stay
server-side.

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

Set `MOONPAY_TRUSTED_PROXY_HOPS` (1 to 8) once the proxy chain is confirmed.
Both MoonPay routes then read the client IP that many trusted proxies from the
right of `MOONPAY_CLIENT_IP_HEADER`, and nothing else. Unset keeps the legacy
behavior: the first entry of `MOONPAY_CLIENT_IP_HEADER`, falling back to other
headers when it is absent. On Render, set `MOONPAY_CLIENT_IP_HEADER` to
`true-client-ip`, which makes even the legacy behavior safe.

## Client IP on Render

Measured on 2026-10-05 with an echo service on the same Render edge as
`api.agentbounties.app` and `mcp.agentbounties.app`. Both hostnames point
straight at Render; only the `agentbounties.app` site goes through a separate
Cloudflare proxy.

| Header | What reaches the service | Client can forge it? |
| --- | --- | --- |
| `X-Forwarded-For` | `<client-supplied…>, <client>, <Cloudflare edge>, <Render internal 10.x>` | Yes, every entry left of the client |
| `True-Client-IP`, `CF-Connecting-IP` | `<client>` | No: Render's edge overwrites a forged value |
| `X-Real-IP` | removed | — |

So on Render:
- use `true-client-ip`, with a trusted hop count of 1 where one is set;
- if you use `x-forwarded-for` instead, set the hop count to 3.

`true-client-ip` stays correct if Render changes its internal hops. Behind a
proxy that does not set and overwrite `True-Client-IP`, a client controls it,
so measure before using it anywhere else.

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
