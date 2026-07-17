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

The `#217` through `#220` seed set was activated on Base mainnet. These files
remain immutable historical terms after claim, cancellation, settlement, or
refund and must not be rewritten. Live standing-meta inventory is determined
from canonical events, not this document.

Issues `#333` through `#337` are the first standing-meta-v2 inventory set.
Their exact terms, commitments, deterministic bounty IDs, and predicted
contracts are locked by `standing-meta-v2-manifest.json`. Each contract starts
with 1 USDC only after its own canonical creation and funding receipt. The
manifest, terms publication, or predicted address is not funding evidence.

Issues `#244`, `#248`, `#249`, and `#250` are an experiment testing whether a
2 USDC solver reward improves claim and completion behavior; this amount is not
a protocol default. Their exact terms and aggregate 7.89 USDC initial-funding
calls are locked by `direct-canaries-manifest.json` and
`deployments/direct-canaries-base-mainnet.json`. Reproduce the unsigned batch
with:

```powershell
cargo run -p cli -- autonomous-activation-bundle `
  --manifest bounties/autonomous-v1/direct-canaries-manifest.json `
  --deployer 0x884834E884d6e93462655A2820140aD03E6747bC `
  --deployer-nonce 4 `
  --output deployments/direct-canaries-base-mainnet.json
```

Issues `#244`, `#248`, and `#249` each start fully funded at 2.01 USDC. Issue
`#250` starts at 1.86 of its 2.01 USDC target and remains unclaimable until any
wallet contributes the remaining 0.15 USDC. This is a pooled-funding canary,
not a reduced solver reward.

Browser, API, and relayer references in these canaries are optional
instrumentation. Payment eligibility is only the committed 16-bit
`LeadingZeroWorkVerifier` result. The manifest, terms publication, unsigned
bundle, wallet signature, or transaction hash is not funding evidence; wait
for confirmed canonical creation, `FundingAdded`, and
`BountyBecameClaimable` events.
