# Guided wallet funding

The flagged flow restores the selected wallet and approved posting operation,
quotes a purchase into that wallet, then returns to the same funding review.
Purchasing crypto, observing a wallet balance, authorizing creation, broadcasting
a transaction and confirming a funded, claimable bounty remain separate states.

## Release status and limits

Both flags default to **off**. Deployment is authorized; its exact live revisions
and fresh-browser results are recorded separately. No live purchase, relay
spending-limit change or Base transaction is authorized by this release.
Local PostgreSQL, browser fixtures, Anvil and the existing Solidity factory are
the rehearsal environments. Live provider accounts, issuer eligibility and a
Base fork/canary still need the release checks below.

**Gas policy is a real release gate.** The local factory rehearsal measured
634,251 gas for `createBountyWithAuthorization`; the existing 20% margin would
require 761,102, exceeding the default 700,000 limit. This measurement uses the
repository's mock USDC and is not a production estimate. The code preserves the
current limit and refuses an excessive estimate. Do not advertise live sponsored
creation until the exact operation passes simulation under an explicitly
approved policy. No gas budget is increased by enabling the feature flag.

Ordinary EOA creation is supported by the relay; the existing contract-wallet
batch and bounded-policy-wallet paths retain their own policies. Meta children
retain their parent and verifier flow. No new custody model, contract deployment,
standalone Transak integration or automatic financial consent is introduced.

## Person-facing flow

1. **Your wallet:** restore the operation, selected address, exact bounty budget
   and Base USDC shortfall. Account ownership does not grant signing permission.
2. **Add funds:** ask only for missing regional/payment eligibility information;
   recommend a compatible provider, prepare its quote and show payment total,
   received USDC, fees, expiry and excess wallet credit before checkout opens.
   Provider sign-in, identity verification and bank authentication remain with
   the person. A generic decline never identifies Mercado Pago as the cause.
3. **Fund bounty:** observe Base balances and automatically reopen the saved
   review. Keep the same operation and unchanged approvals. The person still
   reviews the financial request and confirms the bounded wallet authorization.

The primary action fits the first viewport at tested widths of 390, 532 and
1280 pixels. Optional numbered guides use sanitized illustrative interface
cards. They contain no real credentials, card details or pairing codes.
Desktop phone pairing uses the existing live WalletConnect interface; same-phone
pairing uses its native wallet handoff. Never screenshot a real pairing QR code.

## Provider behavior

* **Coinbase:** server-signed CDP requests discover live country, subdivision,
  payment method, currency and Base USDC availability. A hosted session receives
  the wallet, network, exact shortfall, operation return URL and attempt
  reference. An exact-crypto quote is preferred; a rejected minimum is retried
  using the provider's live fiat minimum. An underfunding or malformed quote is
  rejected. There is no assumption of Mexico or Mercado Pago compatibility.
* **MoonPay:** retain existing signed integration callers. The guided adapter
  additionally checks country/currency restrictions and obtains a live Base USDC
  quote, starting at the published fiat minimum and adjusting from its price.
  This release supports its card quote route; individual card acceptance remains
  the provider's decision. The existing HMAC mechanism signs the destination,
  amount, attempt reference and return URL. Sandbox keys cannot be described as
  funding a real Base wallet.
* **MetaMask/Transak:** expose the purchase continuation only when `eth_accounts`
  reveals a matching MetaMask connection. A portfolio address alone is
  insufficient. Without it, offer a saved handoff and phone pairing. MetaMask
  obtains its quote after authentication; this change does not invent unsupported
  amount/network deep-link parameters or promise a prefilled Transak quote.
  The person checks the preserved address, USDC and Base inside MetaMask.

Only allowlisted HTTPS provider hosts may be opened. The browser synchronously
creates a blank window during the person's trusted click, records `pending`,
then navigates it without an opener. A blocked window creates no checkout and
does not advance the attempt. A quote is returned only once; after a reload an
unopened quote can be replaced, while an opened/uncertain purchase must be
reconciled. A single-use Coinbase session URL is never persisted or replayed.

## Durable state and duplicate protection

Migration `0041_guided_wallet_funding.sql` adds three tables; existing tables,
MoonPay callers, signatures and contract interfaces remain compatible.

* `wallet_topup_attempts`: one unresolved attempt per account/operation and per
  account/wallet. An account advisory lock makes parallel provider requests
  return the same attempt. New quote preparation is capped at ten attempts per
  hour per account. Cookie authentication and an allowed Origin protect mutations.
* `posting_creation_relays`: binds operation, approved draft hash, exact creation
  fingerprint and bounded authorization. It permits one creation identity per
  operation and survives browser reloads and service restarts.
* `posting_relay_transactions`: retains signed bytes, exact hash, signer and
  nonce **before the first submission**. It is internal storage, never returned
  to analytics or public APIs. Restrict database access and backups accordingly.

Purchase transitions are `preparing → prepared → pending → completed/failed`;
unclear provider responses remain `unknown` or `pending`. Provider switching and
another bounty using the same wallet cannot bypass an unresolved purchase.
Legacy unresolved MoonPay markers are imported conservatively. A provider's
success remains pending if the required USDC has not appeared in that wallet.
The person can acknowledge that an order was resolved; there is no timed
auto-clear. If a flag is disabled mid-flow, guided recovery remains accessible.

The relay shares existing x402 quotas, signer lease, fee cap, balance checks and
simulation. Creation additionally requires the current durable legal receipt,
the exact approved reward split, checks, source, image, evidence/reference
snapshot, deadline and terms commitments. The factory planner binds the creator,
amount, chain, predicted bounty, creation/authorization nonce and expiry.

Before durable signing, a restart can resume the admitted authorization. Known
transient pre-broadcast failures receive up to five attempts, at least 60 seconds
apart; invalid simulation, expiry, chain mismatch or excessive gas stops it.
After signed bytes exist, recovery only reconciles the retained hash. A crash
between persistence and broadcast therefore deliberately leaves an uncertain
operation for investigation instead of allocating another nonce or sending again.
Such an unresolved transaction quarantines the shared signer nonce until the
canonical outcome is established. The status endpoint never broadcasts.

The browser may re-admit the **identical** signed operation after an authoritative
`not_started` response; server uniqueness and leases prevent a second creation.
It never obtains a new signature, changes gas payer or repeats a wallet
transaction automatically. Before authorization, unavailable sponsorship offers
waiting or an explicit user-paid review. After an admitted authorization fails
terminally, retain it for recovery; do not silently fork a fresh authorization.

## API and WebMCP contract

The separately deployed hosted runtime exposes rollout flags through the
[funding capabilities endpoint](https://api.agentbounties.app/v1/wallet-funding/capabilities)
(GET). This website release does not add these routes to the public repository's
local API; local deployments need the matching hosted-runtime implementation.
Account routes use the existing secure site session cookie, not an API key. All operation routes live
under the site-auth posting-funding operation prefix and return
`Cache-Control: no-store`. The signed-in account must own the private posting draft.

| Method and suffix | Input and result |
| --- | --- |
| POST `readiness` | `wallet`, `required_usdc_units`; observed USDC/ETH, exact shortfall, specific creation sponsorship status and one next blocker |
| POST `topup-options` | Wallet/amount, confirmed `country`, optional `subdivision`; provider availability and current payment options |
| POST `topups` | Same context plus `provider`, `payment_currency`, `payment_method`, optional matching `metamask_address`; reserves the attempt and prepares a quote without opening checkout |
| GET `topups` | Existing attempt and reconciled provider state; never bounty funding evidence |
| POST `topups/action` | `attempt_id`, `action` = `open`, `cancel_unopened`, or `confirm_resolved`; the last requires the person's `provider_resolution_confirmed` acknowledgement |
| POST `creation-relay` | `draft_hash`, `legal_acceptance_id`, exact `create`, EIP-3009 `signature` (`v/r/s`); admits and simulates the exact person-authorized operation |
| GET `creation-relay` | Retained relay status/hash, simulation result, retryability and canonical funding event; read/reconcile only |

Amounts are decimal **base-unit strings**, never floating point USDC. Top-up
inputs reject unknown fields. Optional `analytics_disabled` preserves opt-out
across the provider redirect. `legacy_pending` conservatively imports an existing
unresolved purchase and never opens a replacement. Errors contain bounded codes,
not raw upstream errors, credentials, bank details or provider customer IDs.

`can_request_authorization: true` means preflight succeeded; it is not a promise
of sponsored completion. Readiness reports `awaiting_authorization` until exact
simulation. `eligible: true` must never be inferred from an enabled service flag.

On the posting page, discover `agent_bounties_get_posting_funding_readiness` and
`agent_bounties_open_topup`. On guided top-up, discover:

* `agent_bounties_get_topup_options`
* `agent_bounties_prepare_topup`
* `agent_bounties_get_topup_status`

Tools read or stage; they cannot open a provider payment, accept legal terms,
enter credentials, sign or fund. The AI prepares supported fields, explains the
returned shortfall/provider/gas result and resumes the same operation after the
person's action. Missing verification keeps a bounty unfundable. The existing
`agent_bounties_open_account_setup` remains the sign-in route.

## Operator configuration

* `ENABLE_GUIDED_WALLET_TOPUP=false` and
  `ENABLE_SPONSORED_POSTING_CREATION=false` by default; independent rollout.
* Existing account draft storage/session settings and a migrated PostgreSQL DB.
* Existing x402 relay enablement, signer, quotas, gas limits, fee caps and Base RPC;
  there are no silently enlarged limits or replacement signer credentials.
* For Coinbase, server-side `COINBASE_ONRAMP_KEY_ID` and
  `COINBASE_ONRAMP_KEY_SECRET` using a CDP ES256 key with Onramp permissions.
* Existing server-side `MOONPAY_PUBLISHABLE_KEY` and `MOONPAY_SECRET_KEY` for its
  signed integration; keys never enter the AI handoff.
* `TOPUP_TRUSTED_CLIENT_IP_HEADER` must name a header the trusted reverse proxy
  **strips from incoming requests and sets itself**. An arbitrary X-Forwarded-For
  is not trusted. Missing verified client IP blocks hosted checkout preparation.
* `WEBSITE_BASE_URL` must be the first-party HTTPS site. The return location is
  built server-side; callers cannot supply arbitrary provider redirect URLs.

No credentials are requested from the person or from the AI conversation.

## Validation and release gates

The backend is released separately on the existing contained production branch.
Backend source and checks below apply to that release; the public website release
contains the browser client and its tests.

Reproducible checks (use the repository's Node/Rust/Foundry environment):

```sh
node --test scripts/test-wallet-funding.js scripts/test-funding-readiness.js scripts/test-posting-session.js scripts/test-posting-communication.js scripts/test-webmcp.js
node scripts/test-guided-topup.cjs
node scripts/test-posting-layout.cjs
cargo test -p api
AGENT_BOUNTIES_TEST_DATABASE_URL=<local-test-db> cargo test -p db wallet_funding -- --ignored
AGENT_BOUNTIES_TEST_RPC_URL=<local-anvil> cargo test -p chain-base prepared_relay_does_not_broadcast -- --ignored
python3 scripts/check-site.py
python3 scripts/check-migration-history.py
python3 scripts/check-agent-discovery-contract.py
cd contracts/base-escrow
forge test --match-test testPredictedAddressAndRelayedUsdcAuthorizationCreateFundedBounty -vv
```

Browser fixtures exercise responsive first-view actions, quotes/excess, account
requirements, an unavailable country, a mismatched MetaMask wallet, blocked
navigation, expiry, generic purchase failure, reload, uncertain purchases,
rollout rollback, and existing keyboard/phone-pairing/account-recovery behavior.
Database tests exercise concurrent providers, account isolation, durable reload,
single checkout admission, signed-hash recovery and signer nonce quarantine.
Rust tests cover exact approved content/expiry binding, strict quote units,
allowlisted provider URLs and sanitized status/error parsing. Anvil proves that
preparation does not broadcast and one broadcast uses the retained hash.

These are fixtures and local contracts, not proof of a working live provider
account, actual card/bank acceptance, extension integration or Base reserve.
Before enabling production, complete each release gate:

1. Apply migration in staging and rehearse reload/restart/concurrent tabs with
   production-shaped synthetic provider responses; verify opt-out and no secrets
   in logs. The open PR queue was rechecked before this release; its maintainer notice
   accompanies the website changes.
2. Validate configured provider sessions in their supported sandbox, country and
   payment methods. Confirm actual desktop MetaMask, Coinbase account wallet,
   in-app browser without extensions, phone pairing and same-phone handoff.
3. Rehearse the real canonical factory/USDC on a Base fork, including zero creator
   ETH, invalid/expired signatures, insufficient reserve, fee/gas caps, lost RPC
   replies, restart and concurrent requests. Verify live relay reserve separately.
4. Resolve the measured creation-gas policy gate explicitly; keep sponsorship off
   while exact production-shaped simulation exceeds the unchanged cap.
5. Obtain separate authorization for one bounded live canary, review its quote,
   legal acceptance, rewards/fees/total and exact wallet authorization with the
   person. Do not reuse a failed order as permission for another payment.
6. Require canonical factory creation, matching `FundingAdded`,
   `BountyBecameClaimable`, and the exact public ready-to-earn entry. Only canonical
   Base USDC events prove payments. Roll back either flag independently; continue
   reconciliations and preserve attempts during rollback.

Screenshots in `evidence/guided-wallet-funding/` are sanitized synthetic examples;
the validation manifest records checks and boundaries. Current-step diagnostics
expose only bounded step/provider/outcome values. Existing opt-out-aware analytics
receive only their allowlisted event names, with no order/payment data.

## Provider references

Checked on 2026-09-17:

* [Coinbase country and currency discovery](https://docs.cdp.coinbase.com/onramp/coinbase-hosted-onramp/countries-%26-currencies)
* [Coinbase hosted session and quote API](https://docs.cdp.coinbase.com/api-reference/v2/rest-api/onramp/create-an-onramp-session)
* [Coinbase purchase status](https://docs.cdp.coinbase.com/api-reference/rest-api/onramp-offramp/get-onramp-transactions-by-id)
* [MoonPay live quotes](https://dev.moonpay.com/api-reference/widget/getbuyquote)
* [MoonPay countries](https://dev.moonpay.com/api-reference/widget/getcountries)
* [MoonPay external transaction status](https://dev.moonpay.com/api-reference/widget/getbuytransactionbyexternalid)
