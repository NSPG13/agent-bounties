---
name: Verifier Reliability Bounty
about: Template for creating verifier reliability bounties
title: '[PENDING ACTIVATION] Earn 1 USDC margin by posting a verifier reliability bounty'
labels: 'draft'
assignees: ''
---

## Activation state

This is a replacement inventory draft. **Do not claim, sign, bond, fund a child, or start work yet.**

It becomes an earning bounty only after this issue publishes all of the following from canonical Base-mainnet evidence:

- one exact routed-V3 parent contract;
- full 2.01 USDC funding;
- BountyBecameClaimable;
- valid immutable terms;
- an executable routed-V3 child-preparation path;
- the pinned sandboxed_regression_v1 verifier path.

## Intended earning loop

After activation, the solver posts and fully funds one concrete 1.00 USDC verifier reliability coding child bounty. A different registered participant completes that child and receives canonical settlement. The parent then pays 2.00 USDC, leaving 1.00 USDC parent gross margin before gas or other optional costs.

The activation workflow adds bounty, funded-live, and claimable-live labels only after every readiness check passes. A draft issue, label, transaction plan, signature, or transaction hash is not funding, claimability, completion, or payment. Only canonical BountySettled proves payment.

Discovery feedback requested after activation: explain how you found the bounty, why you attempted it, and the single change that would make the loop easier to complete.

## Activation checklist

- [ ] Parent contract deployed on Base mainnet
- [ ] Contract funded with 2.01 USDC
- [ ] BountyBecameClaimable event emitted
- [ ] Immutable terms validated
- [ ] Child-preparation path verified
- [ ] Verifier path pinned
- [ ] Labels updated (bounty, funded-live, claimable-live)

## Contract details

**Parent contract address:** `TBD`

**Funding transaction:** `TBD`

**BountyBecameClaimable transaction:** `TBD`

## Terms

- Parent bounty value: 2.01 USDC
- Child bounty value: 1.00 USDC
- Expected margin: 1.00 USDC (before gas)
- Network: Base mainnet
- Verifier: sandboxed_regression_v1
