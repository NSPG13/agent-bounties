# Website V2 wallet posting

New website USDC postings use the existing autonomous-v2 factory on Base. The
retired `protocol.json` remains a historical V1 descriptor; new posting loads
`protocol-v2.json`. This change does not deploy a contract or change the API.
`check-site.py` pins `protocol-v2.json` to
`deployments/autonomous-v2-base-mainnet.json` and fails if `protocol.json`
(autonomous-v1) is ever marked active for new creation again.

The creator approves the draft, reviews the reward plus the factory's 7.5% fee
on the solver reward (rounded up to one micro-USDC), and explicitly accepts that
the wallet pays Base ETH network fees. The website publishes the exact immutable
terms, checks the V2 quote and creation plan, then requests two wallet
transactions: an allowance for the exact full target and creation with full
funding. It waits for the first transaction before requesting the second.
A signature, allowance, receipt or returned hash alone cannot mark the bounty
funded; the existing canonical event and inventory reconciliation still applies.

`posting-v2-plan.js` independently encodes the requested creation, computes its
bounty ID and CREATE2 address, checks the fixed factory and implementation, and
compares every wallet call. A `ReceiveWithAuthorization` in the response is not
sent to the retired V1 sponsored relay. V2 sponsored creation remains unavailable.
The embedded email wallet (Coinbase) and other wallets limited to reviewed
signatures are not offered for this route yet; the person chooses a browser or
phone wallet that holds Base USDC and Base ETH, or the separate card route.
Held verifier and Workspace profiles remain held by their existing API checks:
the API currently refuses sandboxed-regression terms for new v2 bounties, so
this form funds creator-review bounties only.

## Interrupted payments

Only the creation call moves USDC. The posting journal records each call before
the wallet sees it, so the page can tell these cases apart:

- **Nothing reached the wallet, or the first allowance was cancelled.** The
  operation is cleared and the draft is editable again.
- **The allowance step was uncertain, reverted or confirmed, and the creation
  was never sent, was refused in the wallet (`4001`) or reverted.** No USDC has
  moved. The journal phase is `v2_creation_pending` and the page offers
  **Finish posting**. Finishing first reconciles canonical events (an earlier
  creation wins), must run in the tab that reserved the publication (so it
  reuses the exact terms and creation nonce), must produce the recorded bounty
  ID and address, reads the USDC allowance and requests the allowance again only
  if it does not already cover the target. A repeated creation cannot fund twice:
  the bounty address is deterministic (CREATE2) and the allowance is exact.
- **The wallet may have the creation** (a lost reply, a returned hash or a
  pending receipt). The operation stays locked and is only reconciled from
  canonical events; the page never asks for it again.

The old signed V1 continuation can reconcile canonical success, but cannot start
or replay retired V1 creation.

## Production dependency

Completion on the page needs the API's indexed events
(`/v1/base/autonomous-bounties/events`) and canonical inventory
(`/v1/opportunities`). On 2026-11-17 both reported the canonical read models
unavailable (HTTP 503 for events and feed; every protocol `unavailable` in
inventory coverage). Until those readers are healthy, a wallet posting would
create and fund the bounty on Base, but the page would keep showing that
funding evidence is pending. Nothing would be described as funded.

## Alternative: sponsored v2 creation relay (not built)

This change keeps the creator's wallet as the payer of Base gas, so it needs no
API deploy. A sponsored path would let people without Base ETH post with one
signature, as the retired V1 flow did. It needs an API change and an owner
deploy:

1. Add `POST /v1/base/autonomous-bounties/v2/creation-relay`, modelled on
   `crates/api/src/sponsored_creation.rs`, but built from
   `configured_autonomous_v2_planner` and
   `plan_v2_authorized_creation` (`createBountyWithAuthorization`). Keep the
   410 guard on the V1 relay unchanged.
2. Accept only `ReceiveWithAuthorization` payable to the v2 factory, whose nonce
   is the bounty ID (`plan_v2_creation` already returns this typed data). Unlike
   V1's `TransferWithAuthorization`, only the factory can execute it, and only
   for these exact terms.
3. Reuse the existing sponsor controls: `require_autonomous_v2_creation_terms`,
   simulation before broadcast, the daily gas budget, nonce reservation, and
   idempotent replay of an earlier broadcast for the same signature.
4. Advertise it in `/v1/base/gas-sponsorship` under a distinct key, so the
   website can use it only when it is reported available, and otherwise fall
   back to this direct-wallet route.
5. On the website, validate the v2 typed data (domain `USD Coin` version 2,
   `ReceiveWithAuthorization`, `to` = factory, nonce = bounty ID, value = fee-
   inclusive target), save the signature as the existing continuation does, and
   relay it.

Until then, the reviewed embedded wallet stays unsupported for USDC posting,
because people using it usually hold no Base ETH.

## Local checks (no production payments)

```sh
node --test scripts/test-posting-v2-plan.cjs scripts/test-website-v2-posting-fixture.cjs scripts/test-funding-readiness.js scripts/test-sponsored-posting.cjs scripts/test-posting-session.js scripts/test-terms-publication.cjs scripts/test-bounty-composer-retirement.cjs scripts/test-phone-wallet-network.cjs scripts/test-meta-child.js scripts/test-webmcp.js
cargo test -p chain-base --test website_v2_posting
node scripts/test-posting-v2-browser.cjs
python3 scripts/check-site.py
```

The ABI and predicted-address test uses the retained Foundry/Anvil fixture.
`test-website-v2-posting-fixture.cjs` regenerates the exact terms and creation
the page builds, at a fixed clock, and keeps
`crates/chain-base/tests/fixtures/website-v2-posting.json` current;
`website_v2_posting.rs` checks that fixture against the API's terms publication,
public-earning gate and v2 planner. `test-phone-wallet-network.cjs` drives the
real funding function through the pinned WalletConnect provider. Browser tests
isolate every external request and cover exact payment calls, changed calldata,
a rejected (then finished) or lost second request, a wrong network, no ETH,
account or chain changes during estimation, and another device changing the
saved draft revision while the quote is pending. These are local qualification,
not evidence of production deployment or a live customer payment.
