# Canonical Autonomous Bounty Terms

These JSON documents are public terms preimages for Base mainnet
autonomous-v1 bounties. `manifest.json` records the first four activation
canaries and their expected
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
python scripts\rehearse_autonomous_activation.py `
  --fork-block-number 48496661
```

The checked-in bundle predicts the factory and implementation from the
deployer nonce, then reduces four funded creations to one exact 4 USDC approval
and four factory calls. The rehearsal adds fork-only gas, impersonates the
creator only inside Anvil, and checks USDC conservation and claimable state. It
never signs or broadcasts a Base mainnet transaction.

Issue `#187` uses the separately deployed permissionless verifier recorded in
`deployments/leading-zero-work-verifier-base-mainnet.json`. Its terms are not
funding evidence until a matching canonical bounty is created and indexed.

Issues `#217` through `#220` are the first canonical-child-v1 seed set. Their
terms and aggregate unsigned wallet calls are locked by
`canonical-child-seeds-manifest.json` and
`deployments/canonical-child-seeds-base-mainnet.json`. Reproduce the batch and
replay the verifier plus all four creations against current Base state with:

```powershell
cargo run -p cli -- autonomous-activation-bundle `
  --manifest bounties/autonomous-v1/canonical-child-seeds-manifest.json `
  --deployer 0x884834E884d6e93462655A2820140aD03E6747bC `
  --deployer-nonce 4 `
  --output deployments/canonical-child-seeds-base-mainnet.json
python scripts/rehearse_autonomous_activation.py `
  --bundle deployments/canonical-child-seeds-base-mainnet.json `
  --expect-existing-factory `
  --verifier-deployment deployments/canonical-child-verifier-base-mainnet-deployment.json `
  --fork-url https://your-base-mainnet-rpc
```

The seed issues remain activation-blocked until confirmed mainnet events are
recorded. The bundle and fork replay are not funding evidence.
