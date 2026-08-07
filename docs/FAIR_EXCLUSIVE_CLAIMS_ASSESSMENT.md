# Fair Exclusive Claims & Proportional Bonds Assessment

## 1. Protocol Architecture & Invariants
- **Slot Limits**: Enforces 1 active exclusive claim per solver wallet across canonical fair-claim bounties.
- **Progress Renewals**: Hour-scale initial reservation windows renewable strictly via public content-addressed progress evidence URIs.
- **Proportional Bonds**: Dynamic solver bond formula scaling with reward and maximum reservation duration, maintaining 100% rejection solvency.
- **Precommitted Appeals**: Non-deterministic verifier paths require symmetric precommitted appeal contracts before funding.

## 2. Impact & Verification
- **Compatibility**: Additive fair-claim registry preserving historical V1/V2/V3/V4 on-chain bounty bytecode.
- **Proof of Payment**: Only canonical `BountySettled` events prove solver payment.
