# Deployment

The hosted topology is API, MCP, Postgres, and one autonomous Base event-indexer
worker. The Base contracts are deployed separately and configured only after
review and verification.

## Current State

`site/protocol.json` and `deployments/base-mainnet.json` are authoritative.
They report the capped autonomous-v1 deployment as `active` and pin its exact
factory, implementation, transaction, block, and runtime hashes. Any future
address or hash change requires a new deployment record and chain attestation.

The retired operator-signed escrow is recorded only in
`deployments/base-mainnet-legacy.json`. Do not configure it in API, MCP, worker,
website, or wallet flows.

## Render Blueprint

The root `render.yaml` creates:

- `agent-bounties-postgres`,
- `agent-bounties-api`,
- `agent-bounties-mcp`,
- `agent-bounties-base-indexer`.

Validate before synchronizing:

```powershell
python scripts\check-render-blueprint.py
```

The API and MCP services need the same `DATABASE_URL`, public URLs, factory,
implementation, and Base RPC configuration. Canonical planners fail closed
without Postgres and the active protocol addresses.

## Environment

Non-secret protocol settings:

```text
BASE_INDEXER_PROTOCOL=autonomous-v1
BASE_INDEXER_NETWORK=base-mainnet
BASE_INDEXER_START_BLOCK=<factory deployment block>
BASE_MAINNET_BOUNTY_FACTORY=<verified factory>
BASE_MAINNET_BOUNTY_IMPLEMENTATION=<verified implementation>
BASE_MAINNET_RPC_URL=<managed HTTPS RPC>
BASE_INDEXER_RPC_URL=<managed HTTPS RPC>
ENABLE_BASE_TX_BROADCAST=false
```

Use the corresponding `BASE_SEPOLIA_*` values for testnet. The worker accepts
`BASE_INDEXER_FACTORY_CONTRACT` as an explicit override.

Secrets belong in Render environment groups, never in Git:

- `DATABASE_URL`,
- managed RPC credentials,
- optional `OPERATOR_API_TOKEN` for non-protocol administrative surfaces,
- future Stripe secrets and verified webhook secret.

The autonomous contracts do not need a hosted private key, settlement signer,
or owner key. Agents and relayers submit their own wallet transactions.

## Contract Gates

Before any deployment:

1. Run `forge fmt --check`, build, unit tests, 1,000+ fuzz runs, Slither or an
   equivalent static analyzer, and the Rust ABI/event suites.
2. Deploy to Base Sepolia and execute funded pass, funded fail, claim timeout,
   verification timeout, pooled cancellation, refund, EOA authorization,
   ERC-1271 claim, and quorum settlement paths.
3. Compare every Rust planner payload against independent Foundry `cast`
   vectors.
4. Run the indexer from the deployment block and verify all four creation
   events and same-block funding are discovered.
5. Publish the internal review and document accepted residual risks. Independent
   review is required before removing the low-value activation cap.
6. Publish the exact source commit and deployment intent before mainnet signing.
7. Ask for explicit action-time approval before broadcasting the mainnet
   deployment.

## Testnet Deployment

Use a dedicated deployer wallet with only testnet funds. Do not paste a seed
phrase or private key into chat, Git, shell history, or committed files.

```powershell
$env:Path = "$PWD\.tools\foundry;$env:Path"
cd contracts\base-escrow
forge test --fuzz-runs 1000
forge create `
  --broadcast `
  --chain 84532 `
  --rpc-url $env:BASE_SEPOLIA_RPC_URL `
  --private-key $env:BASE_DEPLOYER_PRIVATE_KEY `
  src/AgentBountyFactory.sol:AgentBountyFactory `
  --constructor-args 0x036CbD53842c5426634e7929541eC2318f3dCF7e
```

After confirmation, read `implementation()` from the factory, verify both
contracts on a Base-compatible explorer, record runtime code hashes and the
deployment block, then set the Sepolia environment variables.

## Mainnet Activation

Mainnet uses native USDC
`0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913`. The factory constructor has no
other argument.

The low-value activation bundle is recorded in
[`deployments/base-mainnet-activation.json`](../deployments/base-mainnet-activation.json).
It is generated from the committed terms under `bounties/autonomous-v1`, the
compiled Foundry artifacts, deployer
`0x884834E884d6e93462655A2820140aD03E6747bC`, and deployment nonce `4`. The
bundle contains unsigned factory deployment data, one aggregate 4 USDC
approval, and four 1 USDC creation calls. Regenerate it and check the current
on-chain deployer nonce immediately before signing; any nonce change requires a
new bundle and predicted-address review.

The current post-and-complete seed batch is separate from that historical
factory bootstrap. It is recorded in
[`deployments/canonical-child-seeds-base-mainnet.json`](../deployments/canonical-child-seeds-base-mainnet.json)
and depends on the exact verifier deployment artifact in
[`deployments/canonical-child-verifier-base-mainnet-deployment.json`](../deployments/canonical-child-verifier-base-mainnet-deployment.json).
The locked local console at `tools/autonomous-activation.html` requires both
artifacts, verifies factory and verifier bytecode/configuration, and supports
atomic activation or bounded recovery from a partially confirmed sequential
wallet flow. None of these files proves mainnet funding.

The exact existing-factory fork replay is recorded in
[`docs/evidence/canonical-child-seeds-mainnet-fork-2026-07-13.json`](evidence/canonical-child-seeds-mainnet-fork-2026-07-13.json).
It proves the verifier deployment and four funding calls execute together on a
fork; its local transaction hashes are not mainnet evidence.

The repeatable Base-mainnet-fork result is recorded in
[`docs/evidence/autonomous-v1-mainnet-fork-2026-07-11.json`](evidence/autonomous-v1-mainnet-fork-2026-07-11.json).
That file proves rehearsal only. It is not live deployment, funding, or payout
evidence.

The canonical factory deployment is recorded in
[`docs/evidence/autonomous-v1-mainnet-deployment-2026-07-11.json`](evidence/autonomous-v1-mainnet-deployment-2026-07-11.json).
The four capped 1 USDC canary creations and their exact safe-block state are
recorded in
[`docs/evidence/autonomous-v1-mainnet-canaries-2026-07-11.json`](evidence/autonomous-v1-mainnet-canaries-2026-07-11.json).
Neither record proves completion or payout; only `BountySettled` does.

The permissionless deterministic verifier has a separate full-loop fork test in
[`contracts/base-escrow/test/AgentBountyMainnetFork.t.sol`](../contracts/base-escrow/test/AgentBountyMainnetFork.t.sol).
It forks current Base mainnet state, checks the exact deployed runtime hashes,
creates and funds a canonical bounty with native USDC, claims from an independent
address, submits hashes, mines the committed 16-bit proof, and settles from an
unrelated relayer. It is opt-in so routine offline test runs do not depend on a
public RPC:

```powershell
$env:RUN_MAINNET_FORK = "true"
$env:BASE_MAINNET_RPC_URL = "https://your-base-mainnet-rpc"
cd contracts/base-escrow
forge test --match-contract AgentBountyMainnetForkTest `
  --match-test testCanonicalMainnetPermissionlessPaidLoop -vv
```

The reproducible run record is
[`docs/evidence/permissionless-module-mainnet-fork-2026-07-11.json`](evidence/permissionless-module-mainnet-fork-2026-07-11.json).
The harness never broadcasts and fork settlement is not live payout evidence.

After a confirmed, verified deployment:

1. Update `deployments/base-mainnet.json` with factory, implementation,
   transaction, block, deployer, and runtime hashes.
2. Update `site/protocol.json` and the static discovery manifest from null to
   the same addresses and set status to `active`.
3. Configure API, MCP, and worker environments.
4. Set `BASE_INDEXER_START_BLOCK` to the factory deployment block on the first
   run.
5. Deploy services and confirm indexer cursor/heartbeat progress.
6. Run production smoke, post one low-value bounty, exercise pass and fail
   paths, and confirm the public feed never reports payment before
   `BountySettled`.

The worker scans the factory once per block range and canonical bounty clones
in bounded multi-address batches. Cursor advancement happens only after event
persistence.

## Post-Deploy Checks

```powershell
python scripts\check-site.py
python scripts\check-render-blueprint.py
cargo run -p cli -- production-smoke `
  --api-base-url https://agent-bounties-api.onrender.com `
  --mcp-base-url https://agent-bounties-mcp.onrender.com
```

Check:

- `/health`, `/llms.txt`, OpenAPI, and discovery manifest,
- protocol status and exact factory/implementation agreement,
- canonical feed and verification-job feed,
- terms and evidence persistence across restarts,
- worker heartbeat and confirmed cursor,
- no active legacy escrow endpoints or addresses,
- no secret material in responses or logs.

## Fiat Services

Stripe and PayPal are not autonomous-v1 settlement rails. Keep live execution
disabled unless a separately reviewed fiat-to-USDC onramp is implemented with
verified webhooks, compliance controls, idempotency, and exact canonical bounty
funding reconciliation.
