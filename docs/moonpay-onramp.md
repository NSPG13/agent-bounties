> Current status (2026-09-17): generic MoonPay purchase links are disabled.
> They can select a MoonPay account wallet instead of the user's bounty wallet.
> The primary card alternative is the user's wallet app. **Use money in MoonPay**
> explains how to send existing Base USDC to the exact saved funding address.
> The live signing endpoint currently returns sandbox checkout; this is not
> production activation. Do not enable MoonPay card checkout until the release
> checklist below has been completed with a live, wallet-bound checkout.

# MoonPay wallet on-ramp

> Public UI status: active after deployment at
> `https://agentbounties.app/onramp.html`. The page offers the signed MoonPay
> path only after live destination checks; the public-consumer fallback is blocked.

This integration adds a bounded MoonPay wallet-top-up step without changing the autonomous bounty protocol. The ChatGPT app exposes only a first-party handoff planner; provider checkout and every wallet or purchase step remain outside ChatGPT.

## Evidence boundary

MoonPay handles **asset acquisition**. Agent Bounties handles **bounty allocation and settlement**.

1. MoonPay can deliver Base USDC or Base ETH to the wallet the user connects.
2. Returning from MoonPay, receiving a MoonPay transaction identifier, or observing a larger wallet balance does **not** fund a bounty.
3. The user must separately review and approve the existing canonical contribution flow.
4. Only the matching indexed canonical `FundingAdded` event changes the bounty's funded state.
5. Only a confirmed canonical `BountySettled` event proves solver payment.

The destination is the user's wallet, never the bounty contract. A plain ERC-20 transfer to a bounty contract would not necessarily call the protocol's contribution function, enforce its cap, associate the amount with the correct bounty, or emit `FundingAdded`.

## Signed partner checkout flow

1. Open a canonical bounty and choose **Help fund**.
2. Select **Buy Base USDC or gas with MoonPay**.
3. Connect the same Base wallet that will later authorize the contribution.
4. Review the planned bounty amount and current Base USDC and Base ETH balances.
5. Choose:
   - **Base USDC** for the bounty contribution; or
   - **Base ETH** when the wallet needs transaction gas and does not sponsor it.
6. Agent Bounties requests a short-lived, server-signed MoonPay checkout URL bound to the browser's public IP in live mode.
7. MoonPay collects payment information, performs its own eligibility, KYC, fraud, and payment checks, and delivers the selected asset to the connected wallet.
8. Return to Agent Bounties, refresh balances, and separately approve the exact canonical contribution.

The fiat amount is only a starting value. MoonPay remains authoritative for its final quote, fees, supported payment methods, purchase limits, eligibility, and received crypto amount.

### Shortfall and recovery

The posting handoff can pass `wallet`, `amount`, and `operation` with a
same-origin `return` URL. The selected public address is checked automatically
on Base; this does not connect a signing session. The read-only
`AgentBountiesFundingReadiness.readBalances` helper verifies chain 8453 and
native Base USDC, reads both balances at one block, and times out after 12
seconds. Returning to the tab refreshes the snapshot. A nonzero ETH balance
does not prove the exact creation gas is affordable.

The page calculates the USDC shortfall in integer base units. It does not
assume a dollar exchange rate, add an invented percentage fee, or enforce a
universal $20 provider minimum. It shows the target received USDC amount and
lets the person choose the amount in public checkout; MoonPay confirms its applicable purchase
minimum, total fees, and actual received amount before payment. No top-up is
needed when the observed USDC balance already covers the contribution. Final
gas and the bounty transaction remain a separate review on the posting page.

`AgentBountiesFundingReadiness.estimateFees` accepts only bounded exact calls
and uses current Base execution gas plus the GasPriceOracle's
`getL1FeeUpperBound(uint256)` and `getOperatorFee(uint256)`. The L1 estimate
includes a conservative unsigned type-2 envelope allowance with an empty
access list. Wallet batching, authorization lists, or later rate changes can
change the charge. Missing oracle data or a failed sequential approval
simulation leaves the total unknown, never zero. `maximumWei` is always null:
the estimate is not a guaranteed wallet spending cap or proof of sponsorship.
See [Base network fees](https://docs.base.org/specifications/transactions/network-fees)
and the [official GasPriceOracle implementation](https://github.com/ethereum-optimism/optimism/blob/develop/packages/contracts-bedrock/src/L2/GasPriceOracle.sol).

Before opening a provider purchase, the browser stores only bounded recovery
metadata for the wallet and asset: operation ID, start time, status, and
provider reference when returned. It never stores a signed checkout URL or
credentials. An open or uncertain request blocks another purchase across
reloads and same-browser tabs. A direct checkout uses one named tab and clears
its opener before external navigation. The resume button focuses that tab;
if it is closed, the user returns to the original provider order or
confirmation email. A failed receipt upload after payment must be resolved
with the provider on that order, without paying again.

The user can clear the recovery guard only after explicitly confirming that
the provider shows the earlier purchase completed or cancelled and no payment
pending. This confirmation is recovery context, never payment evidence. The
page refreshes balances and resets purchase consent. Browser storage is a
duplicate-purchase guard, not a provider status API or cross-device order
record. Provider API/order reconciliation remains required to prove a
particular purchase completed; confirmed bounty events remain required for
bounty funding.

## Preserve wallet choice

The generic consumer links do not accept a trusted wallet handoff. Appending
unsigned wallet query parameters is not a supported substitute for server
signing. The page no longer opens those links or labels them a working funding
route. The controller blocks them before opening a tab or recording a purchase,
even if stale UI or an old caller invokes `openDirectCheckout("moonpay")`.

A direct checkout must preserve the exact destination address, the chosen Base
asset, and the bounty context in a server-signed URL. The browser rejects sandbox,
a mismatched address or asset, a non-Base currency code, duplicate destination
parameters, and an unsigned or non-approved host. Changing wallet must require
an explicit user choice and a newly reviewed checkout. The user's quote, purchase,
legal and wallet confirmations stay at the provider; ownership is not signing
consent. New-bounty checkout must preserve the posting operation without demanding
a pre-existing bounty contract or a duplicate amount form. That new-bounty
partner route and live provider activation remain prerequisites, not capabilities
claimed by this containment release.

## Use existing money in MoonPay

MoonPay's documented phone app can hold, receive and send assets on Base. It
requires native ETH on Base for USDC sending fees. This does not establish
support for our wallet connection, personal-message proof, typed-data verdict or
contract calls; direct MoonPay signing is **unverified**, not advertised as a
connectable wallet. Never ask the user to export or share recovery material.

The guided page offers **Use money in MoonPay**, including during an unresolved
older purchase. It shows one action: send the current USDC shortfall on Base to
the full saved wallet address. If USDC is already sufficient, it asks only for
Base ETH for the bounty wallet's fee. Fees and any transfer remain subject to the
user's wallet confirmation. This view does not clear, settle, or repeat a
provider order. It does not infer who controls a supplied public address.

Balance polling continues every five seconds. A confirmed wallet balance that
covers the bounty budget unlocks the same saved bounty review regardless of an
earlier purchase amount; the uncertain order record remains intact. A separate
canonical funding transaction and its confirmed events are still required.

Official sources checked 2026-09-17:
- [MoonPay wallet capabilities and Base support](https://support.moonpay.com/en/articles/383215-managing-your-wallets)
- [Sending and receiving assets](https://support.moonpay.com/en/articles/385117-how-do-i-send-and-receive-crypto-assets)
- [Wallet-prefill signing requirements](https://moonpay.readme.io/docs/quickstart)

## Architecture

- Browser page and controllers: `site/onramp.html`, `site/moonpay-onramp.js`, and `site/moonpay-direct-fallback.js`
- ChatGPT handoff planner and in-chat funding control: `crates/mcp-server/src/chatgpt_app.rs`
- Server route: the MoonPay checkout endpoint on the configured MCP origin
- Server implementation: `crates/mcp-server/src/moonpay.rs`
- Production endpoint gate: `scripts/check-moonpay-production.py`

The browser never receives `MOONPAY_SECRET_KEY`. It sends the reviewed wallet, asset, fiat amount, return URL, optional hosted action intent, and bounty contract to the first-party server. The server validates the request origin and return URL, rate-limits the device, binds live URLs to a hash of the public client IP, signs the final encoded query with HMAC-SHA256, appends `signature` last, and returns a `no-store` response.

`prepare_moonpay_onramp` accepts only a canonical Base bounty contract, a bounded planned USDC amount, and an optional hosted-intent UUID. It returns `https://agentbounties.app/onramp.html` with `checkout_created: false`, `purchase_completed: false`, and `bounty_funded: false`. It never returns MoonPay's provider checkout URL or accepts a wallet address, email, card field, or identity document. The on-ramp can therefore be removed, replaced, or supplemented later without changing the canonical contribution planner.

## Required environment variables

Set these on the hosted MCP service to activate the prefilled, server-signed partner checkout. The generic consumer fallback remains disabled regardless of configuration.

| Variable | Required for partner checkout | Example / purpose |
| --- | --- | --- |
| `MOONPAY_PUBLISHABLE_KEY` | Yes | `pk_test_...` in sandbox or `pk_live_...` in production |
| `MOONPAY_SECRET_KEY` | Yes | `sk_test_...` in sandbox or `sk_live_...` in production; server only |
| `MOONPAY_ENVIRONMENT` | Yes | `sandbox` or `live` |
| `MOONPAY_ALLOWED_ORIGINS` | Recommended | Comma-separated exact origins; production default is `https://agentbounties.app` |
| `MOONPAY_CLIENT_IP_HEADER` | Recommended | Reverse-proxy header containing the customer's public IP; Render default is `x-forwarded-for` |
| `MOONPAY_USDC_BASE_CURRENCY_CODE` | Optional | Dashboard-enabled live code; default `usdc_base` |
| `MOONPAY_ETH_BASE_CURRENCY_CODE` | Optional | Dashboard-enabled live code; default `eth_base` |
| `MOONPAY_SANDBOX_USDC_CURRENCY_CODE` | Optional | Default `usdc` |
| `MOONPAY_SANDBOX_ETH_CURRENCY_CODE` | Optional | Default `eth` |
| `MOONPAY_MIN_FIAT_AMOUNT` | Optional | Local lower safety bound; default `1.00` |
| `MOONPAY_MAX_FIAT_AMOUNT` | Optional | Local upper safety bound; default `10000.00` |
| `MOONPAY_CHECKOUTS_PER_MINUTE` | Optional | Per-device signing limit; default `10` |

MoonPay's dashboard must also approve the website origin and enable the exact Base USDC and Base ETH currency codes used by the account. Do not assume another account's code casing or enabled asset set; override the defaults with the codes returned for this partner account. The server
must also return that value as `destination_currency_code`, bound to the response's
reviewed asset and Base network. The client checks an exact match in the signed URL.
Legacy responses without this field accept only `usdc_base` / `eth_base`; the
currently deployed legacy endpoint cannot enable account-specific codes through
an environment override alone.

## Sandbox and live activation

MoonPay sandbox uses simulated payments and test assets. It validates:

- origin and return URL handling;
- wallet prefilling;
- URL encoding and server-side signing;
- checkout navigation and return handling;
- the separation between a MoonPay purchase and canonical bounty funding.

MoonPay sandbox does **not** top up Base mainnet. MoonPay's test ERC-20 flow uses supported test networks, so a production Base top-up requires approved live credentials and live asset codes.

Activation sequence:

1. Create or obtain the MoonPay partner account.
2. Add `https://agentbounties.app` as the approved production origin.
3. Confirm the account's codes for USDC on Base and ETH on Base.
4. Configure sandbox keys first and deploy.
5. Run the repository checks and complete a sandbox checkout-return rehearsal.
6. Configure live keys, set `MOONPAY_ENVIRONMENT=live`, and verify the reverse proxy supplies the true public client IP.
7. Perform one bounded live purchase to a controlled wallet.
8. Confirm the asset arrived on Base.
9. Return to the same bounty and complete the separate canonical contribution.
10. Confirm the matching indexed `FundingAdded` event before describing the bounty as funded.

Until this sequence is complete, MoonPay card checkout remains unavailable. Offer the wallet-app purchase or existing-money transfer guide; do not reopen generic MoonPay checkout. The production smoke reports route health separately from live checkout availability, and `--require-checkout` rejects sandbox-only setup.

## Verification

Run:

```bash
cargo test -p mcp-server moonpay
cargo test -p mcp-server moonpay -- --nocapture
python scripts/check-site.py
python scripts/check-public-handoffs.py
node --test scripts/test-funding-readiness.js
```

The Rust tests include MoonPay's published URL-signing test vector, verify that live URLs are IP-bound and signed with `signature` appended last, verify that the secret never appears in the checkout URL, and assert that every checkout plan reports `bounty_funded: false` with no canonical event.

The static gate verifies that the generic MoonPay control is disabled, has no navigation URL, and does not imitate a signed or wallet-prefilled URL. It reports healthy containment separately from live checkout availability and keeps the canonical funding boundary.

## Deliberate limitations

- No card, bank, PayPal, identity, or KYC data is collected by Agent Bounties.
- No MoonPay checkout is treated as escrow or protocol funding.
- No MoonPay redirect parameter is treated as authoritative transaction evidence.
- No checkout URL is persisted in browser storage.
- No email or other personal identifier is sent to MoonPay from Agent Bounties.
- No affiliate fee is added in this version. This keeps the first implementation focused on reducing entry friction rather than creating an incentive to encourage unnecessary purchases.
- The direct consumer fallback is blocked. It cannot preserve the user's wallet and Base network.
- The in-memory rate limit is appropriate for the current single-service deployment. A horizontally scaled deployment should replace it with a shared rate limiter before increasing traffic.
