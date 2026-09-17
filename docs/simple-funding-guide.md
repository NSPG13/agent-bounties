# Simple wallet and funding guide

Maintainer notice: https://github.com/NSPG13/agent-bounties/issues/1456

This R1 frontend change follows the wallet recovery fix in #1455. People saw wallet ownership, signing, balances, top-ups, legal consent and transaction diagnostics at once. The replacement shows one necessary step with a large heading and short instructions:

1. Connect: pick a saved account wallet, browser wallet or phone wallet. Restored phone sessions still require a visible choice; a new QR requires an explicit action.
2. Add money: show the exact missing USDC, or ETH when only the network fee is missing. The same-tab top-up preserves the operation and return link. A four-screen wallet tutorial explains opening the app, selecting the asset on Base, checking the address/amount and returning to check delivery. Card purchases use separate amount/review/consent screens and the existing provider route.
3. Review: show reward/reserve split, total, zero platform fee and additional network fee, then the unchanged legal acceptance and wallet confirmations. An uncertain recorded request shows its saved status rather than another payment action.

Optional wallet facts, technical diagnostics and recovery details remain available in collapsed sections. Connection and balance checks show waiting states. Failed reads leave a retry action. Blocked checkout popups display an error; an existing purchase takes priority over every new purchase screen, including after reload.

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
