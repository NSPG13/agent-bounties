# Standing Meta V3 Profit Invariant

Tracking issue: [#527](https://github.com/NSPG13/agent-bounties/issues/527)

## Why V3 exists

Standing-meta-v2 requires a child target greater than or equal to the parent solver reward. If the parent solver funds the child, gross profit is never positive:

```text
parent reward - required child funding <= 0
```

A refundable bond reduction does not repair that incentive. V3 makes positive gross profit an immutable settlement condition while retaining a meaningful paid child opportunity.

## Immutable economic floors

`CanonicalIndependentChildVerifierV3` commits:

- minimum child target: **1.00 native Base USDC**;
- minimum parent gross margin: **1.00 native Base USDC**;
- minimum parent solver reward implied by those constraints: **2.00 USDC**.

A child qualifies only when:

```text
child target >= 1.00 USDC
child target <= parent solver reward - 1.00 USDC
```

The initial replacement parents should use:

| Component | Amount |
| --- | ---: |
| Parent solver reward | 2.00 USDC |
| Parent verifier reward | 0.01 USDC |
| Parent refundable bond | 0.01 USDC |
| Parent total funding | 2.01 USDC |
| Child total target | 1.00 USDC |
| Guaranteed gross parent margin | 1.00 USDC |

The parent bond still equals its verifier reward. Acceptance returns the bond; verifier timeout returns it; no-submission timeout forfeits it under autonomous-v1.

## Retained anti-gaming conditions

V3 still requires:

1. canonical parent and child contracts from the pinned factory;
2. child creation by the active parent solver;
3. complete child terms published on-chain before the parent claim;
4. parent and child wallets registered before the parent claim with different immutable participant IDs;
5. the exact threshold-two sandboxed-regression verifier set;
6. a fully funded and canonically settled child;
7. nonzero child solver, submission hash, evidence hash, and terms hash;
8. exact parent acceptance criteria and V3 verifier module.

A tiny or fake child cannot satisfy the 1 USDC floor. An expensive child that eliminates the promised margin cannot settle the parent.

## What profit means

The guaranteed amount is **gross protocol profit**:

```text
parent solver reward - child total target
```

It does not include taxes, exchange costs, optional external services, or unsponsored gas. Eligible public gas paths should remain sponsored. External co-funding may increase the parent solver's realized return, but settlement eligibility does not infer who economically supplied each child contribution.

## Deployment boundary

V2 contracts and their acceptance hash are immutable. V3 requires a new module deployment and new parent contracts. Before activation require:

- focused unit and adversarial tests;
- 1,000-run fuzz coverage where applicable;
- Base Sepolia rehearsal;
- Base-mainnet fork rehearsal against the pinned factory and native USDC;
- runtime bytecode and immutable getter attestation;
- reviewed API, MCP, bounded-wallet, discovery, and verifier-runner compatibility;
- exact canonical creation and funding evidence for replacements.

Source code, a deployment plan, terms, signature, or transaction hash is not a live bounty or payout. `BountyBecameClaimable` proves claimability; only `BountySettled` proves payment.
