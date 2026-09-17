# Phone-wallet selection recovery

## Follow-up: visible connection and same-tab top-up

The initial balance fix in #1453 did not cover two visible interaction failures.
`eth_requestAccounts` returned a restored session before opening the connection
dialog, and posting's top-up links targeted a named secondary window that the
in-app browser did not show. A working balance lookup did not prove either a
visible QR flow or successful top-up navigation.

Explicit phone selection now opens the connection dialog and lets the person use
the displayed wallet or explicitly disconnect and create a fresh pairing. Desktop
reconnection shows a QR; phones offer native app handoff. Read-only restoration
remains silent. Closing a selection preserves its existing session; failed
disconnection cannot create another pairing. No signature or payment is replayed.

Both posting top-up links save the exact operation before same-tab navigation.
The return link binds that operation and reopens its approved funding review.
Save failure keeps the person on the current page with an explanation. The token
visibility action is labeled “Show USDC in wallet” to distinguish it from buying.

[Maintainer notice #1454](https://github.com/NSPG13/agent-bounties/issues/1454)
records this follow-up under the existing administrator-authorized incident
deployment. The refreshed queue still identifies #1451 as the overlapping work;
it now reports a merge conflict and should retain the connection dialog and
same-tab operation return when rebasing. No data migration is required.

Regression coverage adds restored-session choice, explicit new pairing, native
phone handoff, cancellation, failed disconnect, blocked popups, and a real-browser
top-up round trip preserving the approval hash and frozen reference. Public
payment claims still require canonical evidence.

Rollback: revert the follow-up commit and its paired browser asset versions.

## Initial balance-read recovery

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
