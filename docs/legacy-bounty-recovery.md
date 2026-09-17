# Recover a legacy bounty contribution

Open `https://agentbounties.app/recover-bounty.html?bountyContract=<address>`.
Connect the creator's original Base wallet, review the address and balance,
and choose **Cancel old bounty**. Confirm the exact zero-ETH contract call in
the wallet. After confirmed `BountyCancelled`, choose **Return my funds** and
confirm `withdrawRefund()`. Only confirmed `RefundWithdrawn` proves a refund.
Gas fees are additional. Cancellation alone does not return the contribution.

This screen supports creator-contributor recovery for canonical autonomous-v1
bounties on Base. Claimed, submitted, and settled bounties cannot be cancelled.
It does not alter deadlines, settle work, create replacements, sponsor gas,
sign in place of a person, or use operator credentials. Other pooled funders
retain their existing direct `withdrawRefund()` rights after cancellation.

The screen independently reads pinned Base RPC endpoints (PublicNode, with the Base public endpoint as a fallback), checks the pinned clone,
factory membership, settlement token, creator and balance at one safe block,
validates API planner calldata, then rechecks latest state and simulates the
exact call before opening the wallet. An API response cannot choose a different
recipient, value, sender, method, chain, or RPC destination.

One saved attempt and a browser lock prevent duplicate requests across tabs.
Reloading never resends. A lost response stays blocked for manual wallet-activity
reconciliation; do not clear browser storage to retry. Explicit wallet rejection
or a confirmed revert permits a new reviewed request. Receipt matching requires
the original transaction and bounty event, the refund's original contributor,
and a block still on the safe canonical chain. An RPC failure does not prove
success. Only read/simulation transport failures are retried against the other endpoint; wallet requests are never retried. Reads are serialized to avoid bursts. Polling stops after confirmation and only reads chain state.

After refund confirmation, return to the separately saved replacement draft.
Check its creator, reward, criteria, review window and new delivery deadline;
fund it only once. Keep participants informed of planned versus confirmed
renewal. These legacy contracts cannot gain a mutable deadline through a UI or
database edit.

Validation: `node --test scripts/test-bounty-recovery.cjs` runs as part of the
site gate. For the isolated browser fixture, install the existing Playwright
test dependency and run `RECOVERY_BROWSER_TEST=1 node --test
scripts/test-bounty-recovery.cjs`. Both flows use mocks and spend no funds.
