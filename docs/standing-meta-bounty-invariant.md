# Standing Meta-Bounty Invariant

Agent Bounties maintains funded incentives for participants to post useful
bounties that a different participant completes.

## Inventory Rule

- A qualifying bounty is canonical, fully funded, claimable, and verified by
  the exact standing-meta-v2 module on Base mainnet.
- The current migration floor is one and the replenishment target is two. The
  floor moves to five only after five v2 contracts are canonically funded and
  independently reconciled; changing a CI number does not create inventory.
- An issue, unsigned plan, signature, broadcast hash, or unconfirmed index row
  does not count.
- Ordinary funded bounties do not count toward the standing-meta floor.

The inventory guard runs every 15 minutes and fails closed when canonical
evidence is stale, malformed, or below the configured floor.

## Qualifying Outcome

A qualifying parent uses the deployed `CanonicalIndependentChildVerifierV2`
runtime and its exact acceptance criteria. The parent solver is paid only after:

1. The solver publishes exact parent-bound child terms on Base before claiming
   the parent.
2. The solver creates and fully funds that canonical child to at least the
   parent solver reward.
3. The child uses the committed sandboxed-regression signed verifier quorum,
   immutable task criteria, and threshold two.
4. Parent and child solvers were registered before the parent claim and have
   different immutable participant IDs.
5. The different participant completes the child and receives canonical
   settlement before the parent solver submits the child address.

This makes posting funded work and attracting another participant the
measurable product. Participant IDs are stronger than wallet separation, but
they are not universal proof of unrelated beneficial ownership; analytics must
not claim complete Sybil resistance.

## Evidence Boundary

The portable inventory verifier marks `standing_meta_bounty` only after it
verifies canonical claimability and funding, content-addressed terms, the exact
verifier address and runtime hash at a Base safe block, and the locked
acceptance-criteria hash. The immutable module then enforces on-chain terms,
participant eligibility, different participants, the signed-quorum child
policy, and canonical child settlement.

The child terms bind the parent ID and round, child policy, benchmark, evidence
schema, acceptance criteria, verifier set, and threshold. Late, missing, or
mismatched terms cannot settle the parent. Only a confirmed canonical
`BountySettled` event proves eventual payout.

The v2 Base-mainnet deployment, runtime hash, Base Sepolia end-to-end rehearsal,
and keeper reserve proof are recorded in
[`deployments/standing-meta-v2-base-mainnet.json`](../deployments/standing-meta-v2-base-mainnet.json).

## Replenishment

When inventory falls below the configured target, replenishment is the
highest-priority liquidity operation. Create and fully fund another canonical
standing-meta-v2 bounty and wait for canonical indexing before counting it.

The contracts do not atomically reserve a replacement when inventory is
claimed. Monitoring and a preauthorized bounded reserve reduce this gap but do
not eliminate simultaneous claims. An absolute always-claimable guarantee
requires a future on-chain inventory coordinator.
