# Simple wallet and funding guide

Maintainer notice: https://github.com/NSPG13/agent-bounties/issues/1456

This R1 frontend change follows the wallet recovery fix in #1455. People saw wallet ownership, signing, balances, top-ups, legal consent and transaction diagnostics at once. The replacement shows one necessary step with a large heading and short instructions:

1. Connect: pick a saved account wallet, browser wallet or phone wallet. Restored phone sessions still require a visible choice; a new QR requires an explicit action.
2. Add money: show the exact missing USDC, or ETH when only the network fee is missing. The same-tab top-up preserves the operation and return link. Wallet instructions fit on one screen. Card choices show MoonPay and MetaMask directly; the person enters the amount and confirms payment at the provider.
3. Review: show reward/reserve split, total, zero platform fee and additional network fee, then the unchanged legal acceptance and wallet confirmations. An uncertain recorded request shows its saved status rather than another payment action.

Optional wallet facts, technical diagnostics and recovery details remain available in collapsed sections. Connection and balance checks show waiting states. Failed reads leave a retry action. Blocked checkout popups display an error; an existing purchase blocks new purchases, including after reload, while sufficient wallet funds allow returning to the saved review.

## Boundaries

This presentation does not change contracts, rates, caps, verification, canonical event reconciliation, gas estimation, reward rules, saved draft hashes or reference bindings. Legal summary copy is simplified only for the posting dialog; policy statements, policy hashes and durable acceptance remain unchanged. No guide transition signs, purchases, publishes or sends money. Money arriving in a wallet does not fund a bounty. The final fee remains subject to the exact transaction check and wallet review.

PR #1451 still owns saved provider quotes and its separate hosted-runtime interfaces. Rebase that branch onto this guide and attach its quote information to the purchase-review screen; retain its independent tests. This change neither merges nor replaces that work.

## Verification and rollback

- `node --test scripts/test-posting-auth.js scripts/test-posting-reference.js scripts/test-posting-session.js scripts/test-funding-readiness.js scripts/test-phone-wallet.js scripts/test-wallet-link.js scripts/test-marketplace-ui.js scripts/test-webmcp.js`: 166 passed.
- `node scripts/test-posting-layout.cjs`: seven viewports, keyboard/scroll, phone choices, unchanged approved draft/reference recovery, pending canonical recovery, and three guide viewports covering wallet tutorial, card review, blocked popup, pending purchase and exact return; no wallet writes.
- `POSTING_LAYOUT_GUIDE_ONLY=1 node scripts/test-posting-layout.cjs`: focused guide verification. Optional `POSTING_LAYOUT_SCREENSHOTS` captures only synthetic non-QR layout screens.
- `python3 scripts/check-site.py`: site, asset budgets and navigation pass.
- `bash scripts/preflight.sh core`: local environment lacks npm; CI must provide the full gate.

Rollback: revert this frontend commit and restore its prior asset versions. No migration or purchase retry is necessary. Never replay an uncertain transaction as part of rollback.

The embedded-wallet release fixtures also use the new wallet labels. Its browser gate (`test-readiness.mjs`, `test-posting-requests.mjs`, `test-account-link.mjs`) passes all 36 checks with the real chooser and a simulated external SDK. Optional local API/PostgreSQL flow fixtures use the same labels; their missing-terms case checks the disabled button and visible explanation without weakening the no-signature/no-submission assertions. Those optional integration suites were not rerun for the selector-only follow-up.

A second release harness follow-up stubs the presentation callback in the two VM tests that extract the funding function in isolation. All financial assertions remain unchanged. The broader posting, phone SDK/network, meta-child, competition, handoff and marketplace set now passes 256 checks locally, including the exact group that failed in hosted CI before the harness update.

## Shorter top-up follow-up (2026-09-17)

Maintainer notice: [#1464](https://github.com/NSPG13/agent-bounties/issues/1464).
The first top-up screen is the method chooser. A persistent full address and
connection badge distinguish a verified matching session from a saved public
address. Wallet changes do not silently redirect an existing destination.
MetaMask is visible beside MoonPay. The wallet-app instructions fit on one
screen; the card path goes from method choice to the provider without another
amount entry or an acknowledgement checkbox that merely gates navigation.
The person still chooses the amount, signs up/signs in, sees fees and minimums,
and approves a purchase at the provider. The advanced existing-bounty signed
checkout contract and its amount/consent fields are unchanged.

The incident was caused by the pending-order screen taking precedence over
wallet readiness. A starting fiat amount never determined wallet readiness.
The page now reads the fixed bounty USDC requirement against public Base
balances every five seconds while visible, and immediately on focus/return.
Overlapping reads are coalesced; failures invalidate readiness, and different
wallets cannot receive stale results. Provider processing time remains outside
our control. The UI reports wallet liquidity, not provider settlement.

Enough USDC plus a nonzero ETH balance (or the existing-bounty path) permits
returning to the saved review while an uncertain provider order remains saved
and blocks duplicate purchases. Exact gas sufficiency is still checked at
bounty review. Neither deposit detection nor a redirect marks a bounty funded.

On the top-up document, `agent_bounties_get_topup_status` refreshes read-only
balances/session and returns the shortfall, timestamp/block, pending order,
unchanged operation and review URL. `agent_bounties_prepare_topup` accepts
`method=wallet|card` and prepares the visible screen, never opening checkout or
approving anything. Both honor the WebMCP opt-out.

Regression: old starting amount 25, actual deposit 20 USDC, bounty requirement
2.01 USDC. Automatic polling shows readiness while retaining the uncertain
order. Also covered: partial deposit, failed RPC after a successful read,
reload, popup failure, missing ETH, MetaMask, saved-address vs live-connection
status, and no wallet writes or implicit account request.
