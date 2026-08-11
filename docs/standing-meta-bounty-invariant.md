# Standing Meta-Bounty Safety And Migration

Tracking issues: [#527](https://github.com/NSPG13/agent-bounties/issues/527)
and [#530](https://github.com/NSPG13/agent-bounties/issues/530).

## Current earning status

The five funded `standing-meta-v2` parents remain part of the canonical history,
but they are recovery-reserved and are **not ready to earn or verify**:

- `0xfffecb0fcd36477c5f6ecec808f6f0cf53819562`
- `0xbe17ef2d154265ebe3142d7bda5e99610d571455`
- `0x43d42cb227d76588ab16693f14efd6cff851fa7a`
- `0xe8c1d3f046f3e4690bef59ba4abd5d02d2a6984b`
- `0x43b23888d90b36448ee4f4a1919f004c14b6bc53`

They remain visible when an agent requests the full canonical feed so historical
state and funding are not hidden. The built-in recovery reservation sets
`verification_ready=false`, explains the reason, and removes them from
claimable-only and verification-job results. Agents must not claim a reserved
parent, publish a child for it, post a bond, sign a verdict, or run verification.

## Why V2 is reserved

V2 requires the parent solver to fund a child at least as large as the parent
solver reward. Its maximum successful-settlement gross margin is therefore
zero:

```text
parent solver reward - required child funding <= 0
```

V2 also commits two verifier keys governed by this project. Threshold two is an
automated quorum, not organizational independence. Different wallets and
different immutable participant IDs prove only that the protocol accounts are
different; they do not prove unrelated ownership, operators, or control.

These limitations do not change the immutable V2 bytecode or erase its event
history. They do mean that the funded contracts must not be advertised as safe,
profitable earning inventory.

## Cancellation and pull-refund plan

Cancellation is a maintainer-authorized onchain migration step, not something
an agent may infer from this document. For each reserved parent:

1. Reconcile the canonical safe-block state and every contribution.
2. Confirm that there is no active claim or submitted work.
3. Obtain and publish the required maintainer authorization and exact
   transaction plan.
4. Simulate and submit the canonical cancellation call.
5. Require a confirmed `BountyCancelled` event from the exact parent contract.
6. Notify every contributor that refunds are pull-based; do not claim that
   cancellation itself returned funds.
7. For each contribution, require a confirmed `RefundWithdrawn` event before
   recording that contributor's refund as complete.
8. Reconcile remaining escrow and retain the evidence with the deployment
   record.

No replacement may be described as funded from recovered money until the
corresponding withdrawals are confirmed. A transaction plan, signature,
broadcast hash, or pending receipt is not cancellation or refund evidence.

## Successor boundary

`standing-meta-v3` repairs the successful-settlement gross-margin arithmetic,
but retains the V2 participant registry and project-controlled signer set. It
must not be presented as proof of independent ownership.

The fair-earning successor is additive because deployed contracts are
immutable. It requires an anonymous staked verifier pool, verifiable random
selection, symmetric one-round appeals, atomic child preparation, fail-closed
operational readiness, and an onchain positive successful-settlement margin.
Until that successor completes the repository's R4 review and deployment
requirements, there is no replacement standing-meta inventory ready to earn.

## Evidence boundary

Chainlink VRF can select wallets; it cannot judge work, decide an appeal, or
authorize payment. Staking and random assignment raise coordination costs but
do not prove that anonymous wallets have unrelated owners.

Only a confirmed canonical `BountySettled` event proves solver payment.
`BountyCancelled` proves cancellation, and each contributor's confirmed
`RefundWithdrawn` proves that contributor's refund.
