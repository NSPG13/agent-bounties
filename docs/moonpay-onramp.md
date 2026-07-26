# MoonPay Base-USDC wallet on-ramp

This integration adds a bounded Base-USDC wallet-top-up step without changing the autonomous bounty protocol or the hosted ChatGPT tool surface. It supports both an existing bounty and **pre-bounty wallet onboarding**, where a person creates a wallet and acquires USDC before any bounty contract exists.

## Evidence boundary

MoonPay handles **asset acquisition**. Agent Bounties handles **bounty actions and canonical settlement**.

1. MoonPay can deliver Base USDC to the wallet the user creates or connects.
2. Returning from MoonPay, receiving a MoonPay transaction identifier, or observing a larger wallet balance does **not** post, fund, claim, submit, verify, or settle a bounty.
3. The user must separately review and approve the exact original Agent Bounties action.
4. Only the matching indexed canonical protocol event changes bounty state.
5. Only a confirmed canonical `BountySettled` event proves solver payment.

The destination is the user's wallet, never a bounty contract. A plain ERC-20 transfer to a contract would not necessarily call the protocol's intended function, enforce its cap, associate the value with the right bounty, or emit the expected event.

## User flow

1. The user chooses an action that needs a wallet, such as posting or funding a bounty.
2. They create a non-custodial embedded wallet through email, SMS, or social login, or connect an existing EIP-1193 wallet.
3. Agent Bounties checks that wallet's Base USDC balance.
4. If the balance is insufficient, the user chooses **Buy Base USDC with MoonPay**.
5. The on-ramp carries the exact wallet, intended USDC amount, bounded return URL, optional ChatGPT action-intent identifier, and optional bounty contract.
6. Agent Bounties requests a short-lived, server-signed MoonPay checkout URL bound to the browser's public IP in live mode.
7. MoonPay performs its own eligibility, KYC, fraud, payment, and purchase-limit checks and delivers Base USDC to the reviewed wallet.
8. The user returns to Agent Bounties and refreshes the Base USDC balance.
9. The user separately reviews and approves the exact original bounty action.
10. Agent Bounties waits for the matching canonical protocol event before reporting that action complete.

No ETH purchase is required for the embedded-wallet path. Direct embedded-wallet calls use the configured CDP paymaster, hosted Agent Bounties routes retain their existing gas sponsorship, and external wallets retain their configured sponsor or normal Base gas behavior.

The fiat amount is only a starting value. MoonPay remains authoritative for its final quote, fees, payment methods, purchase limits, eligibility, and received crypto amount.

## Direct consumer fallback

The on-ramp page also exposes a bounded manual fallback through MoonPay's public consumer page:

- `https://www.moonpay.com/buy/usdc`

This keeps Base-USDC wallet top-up available when Agent Bounties' MoonPay partner credentials are not active or the signed checkout service is unavailable. It is deliberately less seamless than the partner checkout:

1. Agent Bounties does not append the wallet, network, amount, API key, signature, bounty contract, or action intent to the public URL.
2. The user must explicitly copy the connected wallet address.
3. Inside MoonPay, the user must select `USDC_BASE`, verify the Base network, paste the exact address, and review the final amount and fees.
4. The user is told to stop if MoonPay shows another network or wallet address.
5. After delivery, the user returns to Agent Bounties, refreshes Base USDC, and separately authorizes the original bounty action.

The fallback is not equivalent to the signed integration. It cannot cryptographically bind the reviewed wallet or context to MoonPay's checkout. It remains a resilience path, not proof that the MoonPay partner integration is activated.

## Architecture

- Wallet adapter registry: `site/wallet-adapters.js`
- Coinbase embedded-wallet adapter source: `tools/coinbase-wallet-adapter/`
- Browser page: `site/onramp.html`
- Signed-checkout controller: `site/moonpay-onramp.js`
- Direct consumer fallback: `site/moonpay-direct-fallback.js`
- Existing-bounty funding handoff: `site/moonpay-link.js`
- Pre-bounty composer handoff: `site/objective-onramp-link.js`
- Server route: `/v1/onramps/moonpay/checkout` on the hosted MCP origin
- Server implementation: `crates/mcp-server/src/moonpay.rs`
- Static and evidence-boundary gate: `scripts/check-moonpay-onramp.py`

The browser never receives `MOONPAY_SECRET_KEY`. It sends the reviewed wallet, Base-USDC starting amount, return URL, optional hosted action intent, and optional bounty contract to the first-party server. The server validates the request origin and return URL, rate-limits the device, binds live URLs to a hash of the public client IP, signs the final encoded query with HMAC-SHA256, appends `signature` last, and returns a `no-store` response.

The checkout response always reports:

```json
{
  "protocol_action_completed": false,
  "canonical_event": null,
  "bounty_funded": false,
  "canonical_funding_event": null
}
```

The legacy funding fields remain for backward compatibility; the action-neutral fields make the same boundary accurate for pre-bounty onboarding, posting, claims, and other future wallet-gated actions.

MoonPay remains outside `chatgpt_app.rs`. The hosted plugin continues to create expiring action intents and first-party authorization URLs. A user can leave ChatGPT, create or access a wallet, acquire Base USDC, return to the first-party action, and then refresh the same intent in ChatGPT. MoonPay never becomes a public ChatGPT transfer tool.

## Required environment variables

Set these on the hosted MCP service to activate the prefilled, server-signed partner checkout. The direct consumer fallback does not require these credentials.

| Variable | Required for partner checkout | Purpose |
| --- | --- | --- |
| `MOONPAY_PUBLISHABLE_KEY` | Yes | `pk_test_...` in sandbox or `pk_live_...` in production |
| `MOONPAY_SECRET_KEY` | Yes | `sk_test_...` or `sk_live_...`; server only |
| `MOONPAY_ENVIRONMENT` | Yes | `sandbox` or `live` |
| `MOONPAY_ALLOWED_ORIGINS` | Recommended | Comma-separated exact origins; production default is `https://agentbounties.app` |
| `MOONPAY_CLIENT_IP_HEADER` | Recommended | Reverse-proxy header containing the customer's public IP; Render currently uses `x-forwarded-for` |
| `MOONPAY_USDC_BASE_CURRENCY_CODE` | Optional | Dashboard-enabled live Base-USDC code; default `usdc_base` |
| `MOONPAY_SANDBOX_USDC_CURRENCY_CODE` | Optional | Sandbox code; default `usdc` |
| `MOONPAY_MIN_FIAT_AMOUNT` | Optional | Server-side lower bound; the public interface currently starts at `$20.00` |
| `MOONPAY_MAX_FIAT_AMOUNT` | Optional | Local upper safety bound; default `10000.00` |
| `MOONPAY_CHECKOUTS_PER_MINUTE` | Optional | Per-device signing limit; default `10` |

The backend retains legacy ETH currency-code configuration for backward compatibility, but the public embedded-wallet onboarding experience is Base-USDC-only because its supported transaction path uses sponsored gas.

MoonPay's dashboard must approve `https://agentbounties.app` and enable the exact Base-USDC currency code returned for the Agent Bounties partner account. Do not assume another account's asset code or casing.

## Sandbox and live activation

MoonPay sandbox uses simulated payments and test assets. It validates:

- origin and return-URL handling;
- wallet prefilling;
- optional pre-bounty context;
- URL encoding and server-side signing;
- checkout navigation and return handling;
- the separation between a MoonPay purchase and every canonical bounty action.

MoonPay sandbox does **not** top up Base mainnet. Production Base-USDC delivery requires approved live credentials and the live Base-USDC asset code.

Activation sequence:

1. Create or obtain the MoonPay partner account.
2. Add `https://agentbounties.app` as the approved production origin.
3. Confirm the account's exact code for USDC on Base.
4. Configure sandbox keys first and deploy.
5. Run repository checks and complete both an existing-bounty and a pre-bounty checkout-return rehearsal.
6. Configure live keys, set `MOONPAY_ENVIRONMENT=live`, and prove the reverse proxy supplies the true public client IP.
7. Perform one bounded live purchase to a controlled embedded wallet.
8. Confirm Base USDC arrived at the exact reviewed address.
9. Return to the original Agent Bounties action and approve it separately.
10. Confirm the matching canonical protocol event before describing that action as complete.

Until this sequence is complete, the direct consumer fallback remains available but must not be described as wallet-prefilled, signed, or partner-activated.

## Verification

Run:

```bash
cargo test -p mcp-server moonpay
python scripts/check-moonpay-onramp.py
python scripts/check-coinbase-embedded-wallet.py
python scripts/check-site.py
```

The Rust tests include MoonPay's published URL-signing vector, verify that live URLs are IP-bound and signed with `signature` appended last, verify that the secret never appears in the checkout URL, and assert that every checkout plan reports no completed protocol action and no canonical event. A dedicated test proves that a checkout can be prepared before a bounty contract exists.

The static gate verifies that the direct fallback uses only MoonPay's public USDC page, opens with `noopener noreferrer`, does not imitate a signed or wallet-prefilled URL, and keeps the same action-neutral evidence boundary.

## Deliberate limitations

- No card, bank, PayPal, identity, or KYC data is collected by Agent Bounties.
- No MoonPay checkout is treated as escrow or protocol funding.
- No MoonPay redirect parameter is treated as authoritative transaction evidence.
- No checkout URL is persisted in browser storage.
- No email, phone number, social identity, or one-time code is sent from Agent Bounties to MoonPay.
- No affiliate fee is added. This avoids creating an incentive to encourage unnecessary purchases.
- The direct consumer fallback cannot prefill or bind the user's wallet, Base network, or amount; the user must verify all three inside MoonPay.
- The public fallback begins at MoonPay's current practical minimum even when the user's bounty shortfall is smaller; excess USDC remains in the user's wallet.
- The in-memory checkout rate limit is appropriate for the current single-service deployment. A horizontally scaled deployment should use a shared limiter before traffic is increased.
