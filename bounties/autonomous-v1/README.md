# Canonical Autonomous Bounty Terms

These JSON documents are the public preimages committed by the first four
Base mainnet autonomous-v1 bounties. `manifest.json` records their expected
Keccak-256 commitments, creator, verifier set, economics, and creation nonces.

The files are immutable after a matching canonical bounty is funded. Changing
one byte changes the terms hash and must use a new creation nonce and contract.
Repository publication is not funding evidence. Only confirmed
`CanonicalBountyCreated`, `FundingAdded`, and `BountyBecameClaimable` events
from the configured factory prove that one of these documents is live and
funded.

## Reproduce The Activation Candidate

Build the contracts, generate the unsigned bundle, and replay it against a
fresh Base mainnet fork:

```powershell
cd contracts\base-escrow
forge build
cd ..\..
cargo run -p cli -- autonomous-activation-bundle `
  --deployer 0x884834E884d6e93462655A2820140aD03E6747bC `
  --deployer-nonce 4 `
  --output deployments/base-mainnet-activation.json
python scripts\rehearse_autonomous_activation.py
```

The checked-in bundle predicts the factory and implementation from the
deployer nonce, then reduces four funded creations to one exact 4 USDC approval
and four factory calls. The rehearsal adds fork-only gas, impersonates the
creator only inside Anvil, and checks USDC conservation and claimable state. It
never signs or broadcasts a Base mainnet transaction.
