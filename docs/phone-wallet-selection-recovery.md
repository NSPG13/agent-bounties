# Phone-wallet selection recovery

A restored phone session could connect successfully while its balance RPC timed
out. The posting window then retained its earlier “connect to sign” state, making
both the direct phone option and the linked-wallet chooser appear unresponsive.
Repeated linked-wallet selection also appended duplicate “Use this wallet” buttons.

This is a bounded R1 UI/read-only recovery fix for the reported posting incident.
Phone balances use the existing public Base endpoint, with the wallet network and
RPC network both checked before accepting a single-block USDC/ETH snapshot.
Connection, loading and unavailable states render separately. Failed or stale
reads cannot retain spendable balances or enable posting. Signing requests,
consent, rewards, transaction journals and canonical payment evidence are unchanged.

## Contributor notice and overlap

The open PR queue was inspected on 2026-09-17; the relevant active maintainer PR
is #1451, “Guide wallet funding through saved quotes and recoverable handoffs”.
Its metadata reports mergeable and its release description records completed
checks with independent review still pending. This repair stays on current main
and does not merge or replace that broader top-up work. Its authors should retain
both the guided funding handoff and these wallet-state guards when rebasing the
shared composer, HTML version and funding-readiness test file. No collaboration
branch or API migration is needed for this narrow fix. Other listed PRs concern
bounty artifacts, discovery, email, or homepage work, with no known direct impact.
The urgent incident-response exception in contributor-first-maintenance applies;
this note accompanies the focused repair rather than delaying a blocked user for
unrelated contributor work.

## Verification

- 135 focused Node checks pass across funding readiness, phone transport, wallet
  selection, posting recovery and WebMCP.
- `python3 scripts/check-site.py` passes with the bundled Node runtime on PATH.
- An isolated real-browser fixture uses the shipped phone adapter with a restored
  synthetic session whose wallet balance RPC fails. Both direct phone selection
  and “Use this wallet” → “Use a phone wallet” show connected state, then the exact
  2.01 USDC shortfall, with one connection action and posting disabled.
- The browser fixture makes no external API, wallet, signing or payment requests;
  no console errors appeared in the exercised flow. It is not mainnet funding or
  a fresh physical-phone interoperability canary.
- Regression cases cover wrong wallet/RPC chains, invalid chain responses,
  stalled RPC, duplicate selection, balance failures, and late results after a
  newer check or wallet change.

Rollback: revert the focused repair commit and the paired asset versions.
Before live release, require the repository's normal checks; after release,
verify the saved posting operation and phone session without sending a signature
or transaction. A funded/paid claim still requires canonical Base evidence.

Feedback: report which phone wallet and browser reached this flow, how you found
Agent Bounties, and which step remained unclear. Never include pairing material,
credentials or private wallet information in a report.
