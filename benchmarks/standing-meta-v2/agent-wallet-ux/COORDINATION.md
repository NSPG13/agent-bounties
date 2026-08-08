# Parent Solver Coordination Workflow

## Role

As the parent solver for NSPG13/agent-bounties#649, you coordinate the child
bounty lifecycle end-to-end.

## Prerequisites

- Base wallet with at least 0.01 USDC (for the refundable claim bond)
- Registered via `/agent-bounty register 0xYourBaseWallet`
- A different child solver identified and registered

## Steps

### 1. Registration

Both parent and child solvers must register before the parent claim:

```
/agent-bounty register 0xYourBaseWallet
```

### 2. Publish Child Bounty Terms

Prepare the child bounty specification (see `CHILD_BOUNTY.md`) and publish it
as a concrete task that a different participant can complete.

The child task: implement `scripts/check-agent-bounties-wallet-ux.mjs` — a
Node.js validator for wallet UX manifests.

### 3. Fund the Child

Create and fully fund the child bounty with exactly **1.00 USDC** on Base mainnet.
Use the `sandboxed_regression_v1` threshold-two verifier quorum.

### 4. Wait for Canonical Confirmation

Monitor for:
- Confirmed child creation
- Funding confirmed (1.00/1.00 USDC)
- Child becomes claimable
- Strictly later Base timestamp than terms publication

### 5. Claim the Parent

Once all conditions are met:

```
/claim #649 wallet: 0x780B5ea2B039DAcC08C6334fF613def2c18a5Ee9
```

### 6. Monitor Child Completion

The child solver completes the child bounty independently. When the child is
settled, the parent payout (2.00 USDC) minus child funding (1.00 USDC) =
**1.00 USDC net profit**.

## Contracts

| Component | Address |
|---|---|
| Routed V3 parent | `0x41f7f2722f0af7289c2f2eea6afed6f4873f722a` |
| Stable verifier router | `0x380c1af742593dd88b6f20387e9ee693a0536731` |
| Routed implementation | `0x1518ccd19002ca3b69dc33aa4ade349f70be6446` |
| Routed policy hash | `0x8d8e5cca196fcc8ba123d6ffdbb4659c014d82d27cfe1f587f9d031059e23e58` |
| Native USDC token | `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913` |

## Verification

The immutable benchmark runner validates the child solver's implementation:
`node /benchmark/test.mjs /workspace`

Run self-test before submission:
```sh
node benchmarks/standing-meta-v2/agent-wallet-ux/self-test.mjs
```

## Troubleshooting

### Canonical bounty state unavailable

If the bot responds with `Canonical bounty state is unavailable: ActionRequired`,
the on-chain state is not yet settled. Wait for the maintainer to resolve
contract-level issues before re-claiming.

### Refundable claim bond

The 0.01 USDC bond is refunded upon successful claim. It is only forfeited if
the claim is fraudulent or invalid.
