# Open Competition V1 Release Runbook

Open Competition V1 is an additive deterministic protocol. This runbook never
deploys over an existing factory, migrates historical bounty rows, changes an
active bounty, or rewrites contribution, claim, or payout evidence.

## Release Boundary

- Branch from current `origin/main` and freeze one reviewed source commit.
- Keep creation and new commitments disabled by default.
- Keep reveal, expiry, cancellation refund, and bond withdrawal recovery
  available for any competition that already exists.
- Treat factory origin as provenance only. Public inventory requires an exact
  approved verifier catalog match.
- Restart rehearsal after any contract bytecode change.

## Build And Freeze

Run the contract suite with 1,000 fuzz runs and build exact artifacts:

```text
cd contracts/base-escrow
forge test --fuzz-runs 1000
forge build
```

Record a recent Base Sepolia block number/hash, the admin pending nonce, and
admin ETH and native-USDC balances. Then build the unsigned deployment bundle:

```text
python scripts/build_open_competition_v1_bundle.py \
  --deployer-nonce <pending-nonce> \
  --source-commit <frozen-commit> \
  --preflight-block-number <block> \
  --preflight-block-hash <hash> \
  --output target/open-competition-v1/base-sepolia-deployment-bundle.json
```

The builder pins chain `84532`, admin
`0x884834e884d6e93462655a2820140ad03e6747bc`, native Base Sepolia USDC,
LeadingZeroWorkVerifier difficulty `16`, compiler settings, creation bytecode,
expected runtimes, and predicted addresses. The bundle contains no private key
or signature.

Serve `scripts/open-competition-v1-signer.html` locally. It discovers injected
wallets through EIP-6963 and defaults to the announced MetaMask provider. The
operator selects a provider, after which the console accepts only the frozen
bundle, exact admin account, and Base Sepolia chain. It verifies the pending
nonce and predicted addresses, displays each exact transaction, and records
receipts. Never paste a seed phrase or private key into the page.

## Sepolia Rehearsal

Use distinct ephemeral wallets for creator, competitors, and relayer. Do not
commit their keys. Record two complete scenarios:

1. Authorized creation, failed entrant, passing entrant, settlement, and
   losing-bond withdrawal.
2. Unrevealed commitment expiry, competition cancellation, and contributor
   refund.

Also record copied-reveal rejection, same-block reveal rejection,
authorization substitution rejection, capacity handling, and verifier-revert
recovery. The manifest must include deployment and scenario transaction
hashes, blocks, events, runtime hashes, balance deltas, compiler settings, and
the source commit.

Validate the evidence against the versioned schema:

```text
python scripts/audit_open_competition_v1_rehearsal.py \
  --bundle target/open-competition-v1/base-sepolia-deployment-bundle.json \
  --rehearsal target/open-competition-v1/base-sepolia-rehearsal.json \
  --output target/open-competition-v1/base-sepolia-rehearsal-audit.json
```

Only a passing audit may advance the release to
`sepolia_rehearsed_not_ready_to_earn`.

## Mainnet Canary And Activation

Before signing, require static analysis, full Foundry/invariant/fuzz checks,
independent contract review, exact mainnet-fork replay, bundle/manifest audit,
monitoring, sufficient wallet balances, and explicit signing-time approval.

The hidden canary is capped at a 1.00 USDC solver reward, 0.10 USDC verifier
reward/bond, four entries, a 24-hour competition window, and a one-hour reveal
window. The creator cannot compete; use a separate bounded solver wallet.

Advance to `active_ready_to_earn` only after safe-block `BountySettled`, exact
USDC reconciliation, healthy indexer heartbeat, and verified API, MCP, CLI,
Python SDK, and TypeScript SDK behavior. If a problem appears, disable hosted
creation and commitments. Do not describe an immutable deployment as rolled
back, and do not block existing recovery actions.
