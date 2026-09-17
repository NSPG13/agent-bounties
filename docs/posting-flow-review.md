# Posting flow correction: review and release record

Branch: `codex/posting-flow-fix`, based on main `e33499fc`. Class: R3
(wallet authorization). This is a local implementation, not a production
release or evidence of live funding.

## Problem and acceptance

The creator's posting test exposed missing beginner explanations, no WebMCP
entry to sign-in, and an embedded wallet which could authenticate but could not
create bounties. A requested 200 USDC campaign paying 1 USDC per company reply
and 10 USDC per qualifying funded referral was also incorrectly adapted into
a single fixed bounty without explaining the limitation.

- Explain reviewer, review choices, protocol, reserve and network fee before
  staging. Preserve the requested economics and describe unsupported campaigns.
- Open account setup in the same tab, preserving the exact draft return.
- Let the Coinbase EOA create a validated Base bounty only after explicit
  funding-signature and transaction reviews. Show the amount, recipient, expiry
  and network fee. Linking an address grants no payment permission.
- Retain the original posting journal after an uncertain SDK result. Never
  declare funding or payment from an SDK reply.

There is no campaign payout engine, contract change, new relay, custody change,
production transaction, or deployment in this patch.

## Authority and data flow

`saved draft -> creator's page approval -> validated creation plan -> wallet
review -> trusted user click -> Coinbase SDK -> Base -> canonical reconciliation`

The assistant can prepare a draft and open sign-in. The creator controls login,
legal acceptance, publication, signatures and payments. Coinbase retains its
existing key custody. The app receives addresses and signatures for the existing
posting flow; no keys, seed phrases or credentials enter assistant tools.

The adapter accepts only recognized posting calls bound to the approved plan.
It checks exact amounts, recipient, wallet, Base chain and funding authorization
expiry. It delegates other supported funding signatures only after showing the
request. Arbitrary direct calls remain disabled. Public balance, fee and receipt
queries use an explicit read-only RPC allowlist because the locked CDP provider
does not forward them.

## Abuse, failure and recovery

- A synthetic click cannot approve a wallet request.
- Concurrent requests cannot share a confirmation. Frozen copies prevent
  changes to a transaction while it is being reviewed.
- Unknown fees, a changed wallet or expired authorization stop before sending.
- An SDK error after invocation is uncertain, even if its code resembles a
  cancellation. The durable posting journal remains authoritative for recovery;
  canonical creation/funding/inventory checks resume the same operation.
- The adapter also rejects repeated identical requests during the page session.
- Cancel before any SDK request sends nothing. Cancel after an earlier funding
  signature does not erase that authorization or create a fresh operation.
- The funding amount is bounded by the reviewed rewards. Gas is explicitly
  user-paid; the current Base fee estimate is not a guaranteed maximum.

## Contributor coordination and release gate

Open PRs were inspected before implementation. PR #1438 touches posting UI;
keep its CTA/layout work when integrating this additive help section. Wallet
UX child-bounty drafts #1411, #1356 and #880 are specifications, not competing
embedded-posting implementations. No contributor PR was modified or closed.

Maintainer notice draft (not posted): "Preparing beginner posting guidance,
same-tab WebMCP account entry, and explicitly reviewed Coinbase EOA creation.
No contract or campaign changes. Preserve #1438's posting layout work. Wallet
changes require R3 review and a staged canary before production."

Public notice, maintainer risk approval and a staged wallet canary remain
release requirements. They were not performed as part of this local fix.
Ship composer, adapter bundle, configuration and WebMCP guidance together.
Containment: revert the posting capability and composer change together to
restore relay-only operation. Preserve pending journals and reconcile existing
transactions; rollback must never resend them.

## Validation and continuation changes

The original safety-only report is superseded by the outcome scorecard in
[`posting-flow-verification.md`](posting-flow-verification.md). Baseline local
completion was 12/26 and recovery was 8/22. Each final journey must confirm
creation, funding, claimability and the exact public inventory item.

The added continuation retains the signed request locally and saves only its
immutable hash remotely. PostgreSQL revision checks reserve one unique
submission before CDP invocation; a reservation cannot be replaced or downgraded.
Known-unsent cancellation and top-up reuse the same authorization and operation.
Uncertain invocation remains reconciliation-only. Legal receipts survive reload
only for the same account, wallet, task and policy versions. Material changes
invalidate approval. See `posting-metrics-plan.md` for the threat model.

No human comprehension result, live Coinbase authentication, live-origin
configuration result or confirmed Base funding is claimed from fixtures.
Use `posting-live-canary.md` and `posting-novice-study.md` to clear those gates.
The notice above remains a draft; no public message or deployment occurred.

## Maintainer deployment decision — 2026-09-17

The user identifies as maintainer and explicitly instructed: “if it would be
easier to test live after deployment then do so”. This authorizes deploying
the tested application changes and running the bounded live Coinbase test
after deployment, replacing the prior staging-first requirement for this
change. The five-person pilot is waived. Human publication, legal acceptance
and wallet confirmations remain required; no payment is claimed in advance.

## Reviewed build fingerprint — PR #1449

The worker-build guard covers 165 inputs. Compared with main `c2261859`,
only `crates/api/src/site_auth.rs` and `crates/db/src/site_posting_drafts.rs`
change within that set. The API accepts a bounded continuation digest and
submission UUID, rejects signatures in persisted recovery, and constrains
submission phases. The database keeps the digest and reservation immutable
and prevents returning a reserved send to a retryable phase. The added tests
cover malformed metadata and concurrent/changed/replayed reservations.
No dependencies, worker execution, signing runtime or secret scopes change.

After formatting, the reviewed build digest is
`81d1fee1017801a61caec3a7a2538f71fbdc2d33fcb805d6667c5e0dcc57b809`,
replacing `7a55133c0025be81feac6354942bf655e5358572114c622e5a921bf549b18bb6`
in the eight production workflow checks. Signing-runtime remains
`6b0c5073097e9612f94e35089285729d0c0f5db580f75b351d19a97f8bffd2f1`.
Historical benchmark directories and committed bounty bindings are unchanged.
The guard, its scope and all before/after-build checks remain enforced.
