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
receipts. If an earlier action was confirmed before an interruption, the signer
resumes only after the public RPC returns the exact frozen runtime at every
consumed nonce. Never paste a seed phrase or private key into the page.

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

Start the runner before funding. It creates the actors, writes a temporary
recovery file outside the repository, and emits the exact bounded funding
request:

```text
python scripts/run_open_competition_v1_sepolia_rehearsal.py \
  --bundle target/open-competition-v1/base-sepolia-deployment-bundle.json \
  --verifier-tx <verifier-deployment-transaction> \
  --factory-tx <factory-deployment-transaction> \
  --funding-request target/open-competition-v1/base-sepolia-rehearsal-funding.json \
  --output target/open-competition-v1/base-sepolia-rehearsal.json
```

Serve `scripts/open-competition-v1-rehearsal-funding.html` from the repository
root. It accepts only the emitted request, the frozen admin, Base Sepolia,
exactly 0.0005 ETH, and exactly 0.5 native test USDC. After both canonical
balances match, continue the waiting runner. If the runner is interrupted,
resume with `--recovery-file <temporary-path>`; do not create new actors or
fund them again. If an already-settled rehearsal stopped before a losing-bond
withdrawal, also pass `--reclaim-bounty <canonical-bounty-address>` so the
runner recovers the bond and restores exact actor allocations before retrying.
The temporary recovery file is deleted only after the final manifest is
written successfully.

Validate the evidence against the versioned schema:

```text
python scripts/audit_open_competition_v1_rehearsal.py \
  --bundle target/open-competition-v1/base-sepolia-deployment-bundle.json \
  --rehearsal target/open-competition-v1/base-sepolia-rehearsal.json \
  --output target/open-competition-v1/base-sepolia-rehearsal-audit.json
```

Only a passing audit may advance the release to
`sepolia_rehearsed_not_ready_to_earn`.

Publish the secret-free deployment bundle, rehearsal manifest, and audit under
`deployments/` for review. Never publish the temporary recovery file.

## Mainnet Canary And Activation

Before signing, require static analysis, full Foundry/invariant/fuzz checks,
independent contract review, exact mainnet-fork replay, bundle/manifest audit,
monitoring, sufficient wallet balances, and explicit signing-time approval.

Build and independently audit the one-action unsigned mainnet bundle at a Base
safe block. It reuses the already deployed, runtime-pinned mainnet
`LeadingZeroWorkVerifier(16)` and deploys only the new factory:

```text
python scripts/build_open_competition_v1_mainnet_bundle.py \
  --deployer-nonce <pending-nonce> \
  --source-commit bc9b3cc9f9f95a87df671be2d13199ac9d06ebcf \
  --preflight-block-number <safe-block> \
  --preflight-block-hash <safe-block-hash> \
  --output target/open-competition-v1/base-mainnet-deployment-canary-bundle.json

python scripts/audit_open_competition_v1_mainnet_bundle.py \
  --bundle target/open-competition-v1/base-mainnet-deployment-canary-bundle.json
```

Replay that exact bundle and exact bounded canary against the same pinned block.
The replay uses an impersonated admin and a fixed separate solver only inside
Anvil; it never broadcasts or stores a live key:

```text
python scripts/run_open_competition_v1_mainnet_fork_replay.py \
  --bundle target/open-competition-v1/base-mainnet-deployment-canary-bundle.json \
  --output target/open-competition-v1/base-mainnet-fork-replay-manifest.json

python scripts/audit_open_competition_v1_mainnet_fork_replay.py \
  --bundle target/open-competition-v1/base-mainnet-deployment-canary-bundle.json \
  --replay target/open-competition-v1/base-mainnet-fork-replay-manifest.json \
  --output target/open-competition-v1/base-mainnet-fork-replay-audit.json
```

Serve `scripts/open-competition-v1-mainnet-signer.html` locally for the real
factory deployment. It fails closed unless the account is the frozen admin,
the chain is Base mainnet, the pinned block is still canonical, the nonce and
balances satisfy the bundle, the verifier runtime/configuration match, both
predicted addresses are vacant, and all public activation fields remain false.
The page submits exactly one zero-value contract-creation transaction and then
checks both factory and implementation runtime bytecode.

The hidden canary is capped at a 1.00 USDC solver reward, 0.10 USDC verifier
reward/bond, four entries, a 24-hour competition window, and a one-hour reveal
window. The creator cannot compete; use a separate bounded solver wallet.

Advance to `active_ready_to_earn` only after safe-block `BountySettled`, exact
USDC reconciliation, healthy indexer heartbeat, and verified API, MCP, CLI,
Python SDK, and TypeScript SDK behavior. If a problem appears, disable hosted
creation and commitments. Do not describe an immutable deployment as rolled
back, and do not block existing recovery actions.

The current Base mainnet release evidence is published in
`deployments/open-competition-v1-base-mainnet.json`. Its hidden canary settled
canonically and conserved escrow at a safe block. The hosted manifest remains
`mainnet_canary_not_ready_to_earn`; public creation, commitments, and inventory
stay disabled. The monitoring gate is configured but becomes ready only when
the version-specific indexer's database heartbeat is successful or caught up,
no more than 90 seconds old, error-free, and within 20 blocks of the API's safe
block. A feature flag by itself cannot satisfy it. Relay, gas-sponsorship, and
final R4 release-evidence gates remain false.

## Entrant-Wallet Relay Gate

Gas sponsorship for multiple competitors uses the additive
`OpenCompetitionEntrantWalletV1`; a generic keeper cannot relay for arbitrary
EOAs because the frozen bounty records `msg.sender` as solver. This path does
not change or redeploy the existing bounty factory.

Freeze and review the entrant wallet and its deterministic factory separately:

```text
python scripts/build_open_competition_entrant_wallet_bundle.py \
  --network base-sepolia \
  --output target/open-competition-entrant-wallet/base-sepolia-deployment.json

forge test --root contracts/base-escrow \
  --match-contract OpenCompetitionEntrantWalletV1Test --fuzz-runs 1000
```

The manifest must pin the existing competition factory and native USDC, the
entrant implementation/factory/clone runtimes, deterministic deployer runtime,
compiler settings, clean git tree, and default-off activation fields. Rehearse
with separate ephemeral owner, delegate, keeper, creator, and competitor
actors. Commit and reveal must be relayed by the keeper while the entrant
wallet holds zero ETH; the commit transport must contain no reveal secret.
Record a passing settlement and a losing-bond recovery, exact event topics,
safe-block receipt hashes, gas payer and cost, wallet and escrow USDC deltas,
nonces, policy hash/version, verifier runtime/profile hashes, and source tree.

Prepare the one-use actor envelope and bounded funding request before any live
transaction:

```text
python scripts/run_open_competition_entrant_wallet_sepolia_rehearsal.py --prepare \
  --recovery-file target/open-competition-entrant-wallet/base-sepolia-rehearsal-recovery.json \
  --funding-request target/open-competition-entrant-wallet/base-sepolia-rehearsal-funding.json
```

Serve the repository only on localhost and open
`scripts/open-competition-entrant-wallet-signer.html`. The page refuses an
unknown account, chain, token, amount, recipient, runtime, or activation gate.
When the factory already exists it sends no deployment transaction. Actor
funding is one atomic two-call Base Sepolia batch containing exactly 0.0005
test ETH and 0.4 test USDC to the ephemeral keeper. The confirmed transaction
trace must show the known admin as execution sender, or a successful admin
EIP-1271 authorization for the relayed MetaMask transaction.

Execute only with both confirmed transaction hashes:

```text
python scripts/run_open_competition_entrant_wallet_sepolia_rehearsal.py --execute \
  --bundle target/open-competition-entrant-wallet/base-sepolia-deployment.json \
  --deployment-tx 0x... \
  --funding-tx 0x... \
  --recovery-file target/open-competition-entrant-wallet/base-sepolia-rehearsal-recovery.json \
  --output target/open-competition-entrant-wallet/base-sepolia-rehearsal.json
```

The runner reconstructs every reveal salt from the protected recovery envelope,
so a process restart does not strand a bond. It records no plaintext salt,
signature, or private key in the final manifest and deletes the recovery file
only after every scenario receipt is canonical at a Base safe block. The
`--local-priority-fee-cap-wei` option is rejected for non-local RPC URLs and is
only for deterministic Anvil fork testing.

The runner is restart-idempotent after actor distribution, wallet creation,
competition creation, EOA commitment/rejection, and every relayed wallet
action. It recovers the original policy and time bounds from the exact clone,
matches existing competitions by creator and deterministic terms hash, and
reuses canonical action events rather than creating another wallet, bounty, or
bond. Public RPC propagation is checked at each receipt block. A transaction
that is re-included after a reorganization is accepted only after its refreshed
receipt and block hash are canonical and safe.

Audit the secret-free live manifest independently:

```text
python scripts/audit_open_competition_entrant_wallet_sepolia_rehearsal.py \
  --bundle target/open-competition-entrant-wallet/base-sepolia-deployment-regenerated.json \
  --rehearsal target/open-competition-entrant-wallet/base-sepolia-rehearsal.json \
  --output target/open-competition-entrant-wallet/base-sepolia-rehearsal-audit.json
```

Replay the exact frozen entrant factory and both keeper-relayed scenarios on a
canonical Base mainnet safe-block fork, then audit the result:

```text
python scripts/run_open_competition_entrant_wallet_mainnet_fork_replay.py \
  --fork-block-number <safe-block> \
  --fork-block-hash <safe-block-hash> \
  --output target/open-competition-entrant-wallet/base-mainnet-fork-replay.json

python scripts/audit_open_competition_entrant_wallet_mainnet_fork_replay.py \
  --bundle target/open-competition-entrant-wallet/base-mainnet-deployment-regenerated.json \
  --replay target/open-competition-entrant-wallet/base-mainnet-fork-replay.json \
  --output target/open-competition-entrant-wallet/base-mainnet-fork-replay-audit.json
```

The fork runner uses temporary Anvil-only keys and deletes them before writing
the final manifest. It does not broadcast to Base and cannot satisfy the live
Sepolia, hosted relay, gas sponsorship, deployment, settlement, or payment
gates.

After both audits pass, prepare and audit the single zero-value Base mainnet
CREATE2 action. The explicit waiver is permitted only when the admin has
timeboxed the independent review as described in the release plan:

```text
python scripts/build_open_competition_entrant_wallet_mainnet_release_bundle.py \
  --waive-independent-review \
  --output target/open-competition-entrant-wallet/base-mainnet-release-bundle.json

python scripts/audit_open_competition_entrant_wallet_mainnet_release_bundle.py \
  --bundle target/open-competition-entrant-wallet/base-mainnet-release-bundle.json \
  --output target/open-competition-entrant-wallet/base-mainnet-release-bundle-audit.json
```

Serve `scripts/open-competition-entrant-wallet-mainnet-signer.html` only on
localhost. It accepts the known admin on Base mainnet, rechecks the pinned safe
block, current ETH balance, every canonical dependency runtime, deterministic
address vacancy, and all default-off activation fields. It can submit only one
zero-value call to the canonical CREATE2 deployer. The preserved hidden 1 USDC
canary remains the already-settled competition canary; entrant-factory
deployment does not spend or recreate that reward.

Download the browser deployment receipt, then require the exact transaction,
runtime pointers, and deployment block to become canonical at a Base safe
block. This produces the only entrant release manifest accepted by the hosted
relay:

```text
python scripts/audit_open_competition_entrant_wallet_mainnet_deployment.py \
  --receipt <downloaded-deployment-receipt.json> \
  --output target/open-competition-entrant-wallet/base-mainnet-deployment-audit.json
```

The audit must complete before setting the entrant release-manifest environment
value or enabling the operator-only relay canary. It is still not evidence of a
working hosted relay or public readiness.

After the deployment audit passes, copy that exact redacted audit to
`deployments/open-competition-entrant-wallet-v1-base-mainnet.json` and commit
it with no private keys, signatures, salts, or recovery envelopes. The Render
controller accepts only its pinned factory, implementation, competition
factory, token, runtime hashes, safe deployment assertions, and
`mainnet_canary_not_ready_to_earn` state. A manual recovery dispatch may then
set `open_competition_entrant_relay_canary=true`; this enables only the
operator-authenticated relay canary. Recovery relay, public creation, public
commitments, gas-sponsorship readiness, and public inventory remain false.
Dispatch the controller again with the canary input false after evidence has
been reconciled.

When the Render-generated operator token is not already present in the local
environment, do not reveal it through screenshots, workflow logs, or a
plaintext artifact. Generate a one-time RSA key locally, dispatch
`render-operator-token-encrypted-handoff.yml` with only its public key and
SHA-256 fingerprint, download the one-day ciphertext artifact, and decrypt it
only into the current process. Delete the artifact immediately after the live
canary. The workflow never changes or rotates the existing token.

The live mainnet entrant canary uses a DPAPI-protected owner/delegate outside
the repository and an exact four-transaction setup plan:

```text
python scripts/run_open_competition_entrant_mainnet_canary.py prepare \
  --state-dir <private-local-state> \
  --creator <separate-bounded-creator> \
  --output target/open-competition-entrant-wallet/base-mainnet-live-canary-plan.json

python scripts/run_open_competition_entrant_mainnet_canary.py relay \
  --state-dir <private-local-state> \
  --plan target/open-competition-entrant-wallet/base-mainnet-live-canary-plan.json \
  --approval-tx 0x... \
  --creation-tx 0x... \
  --wallet-creation-tx 0x... \
  --wallet-funding-tx 0x...
```

The runner reserves exactly 0.10 USDC from the creator's starting balance and
preserves any surplus as an explicitly reconciled baseline. The hidden bounty
escrows 0.08 USDC for the solver and 0.01 USDC for the verifier, while the
separate entrant wallet receives the remaining 0.01 USDC bond. The runner sends
commitment-only material during commit preparation, keeps the recovery envelope
encrypted with Windows DPAPI, signs the exact hosted EIP-712 plan locally,
requires both relay receipts to become canonical at a Base safe block, and
accepts payment evidence only when the reveal relay records `BountySettled`.
Its redacted evidence must
reconcile the creator's starting balance and reserved baseline, plus the final
creator, entrant-wallet, and bounty balances. The verifier reward returns 0.01
USDC to the creator, the entrant finishes with 0.09 USDC, the bounty finishes
at zero, and the creator's unrelated surplus remains untouched. The canary-only
verifier remains ineligible for public inventory.

Any entrant wallet, factory, planner, typed-data schema, or relay-byte change
invalidates that rehearsal. Do not set hosted relay or gas-sponsorship gates
true until the secret-free evidence is published, audited, replayed on an
exact mainnet fork, and covered by the required independent contract review.
