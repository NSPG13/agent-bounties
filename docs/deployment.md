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

Application deployment authority lives in the `Render Deploy Recovery` GitHub
Actions workflow. Native Render auto-deploy is off because the Git provider
event stream failed to deliver reviewed main commits reliably. After `CI`
succeeds on a push to `main`, the workflow:

1. checks out the exact successful-CI SHA;
2. verifies it is the latest successful CI revision reachable from `main`;
3. resolves all three Render services by exact name and verifies repository,
   branch, and service type;
4. disables any drifted native auto-deploy setting;
5. calls Render's deploy API with the exact commit for API, MCP, and worker;
6. waits for all three deploys to reach `live` and fails on terminal errors;
7. verifies exact revision and protocol headers from API and MCP `/health`;
8. stores a redacted 30-day deployment evidence artifact.

Configure one GitHub Actions secret named `RENDER_API_KEY`. Create it in the
Render Dashboard for the workspace that owns these three services, then store
it only under repository **Settings > Secrets and variables > Actions**. Never
put the key in Render variables, workflow inputs, logs, issues, or Git. A
missing key is a visible workflow failure, not a silent manual-deploy fallback.
The key can deploy application services, so rotate it after suspected exposure.
Creating, rotating, or revoking the credential is an explicit R3 access change;
using the already-provisioned credential for the bounded exact-SHA application
deploy is R2.

The controller can be rehearsed without credentials:

```powershell
python scripts\test_render_deploy_recovery.py -v
python scripts\check-render-blueprint.py
```

After the secret exists, `workflow_dispatch` can recover the latest successful
CI revision reachable from `main`. An older successful SHA skips when a newer
successful SHA exists, while a newer failed commit cannot suppress deployment
of the last known-good revision. The scheduled `Operational Control Loop` stays
read-only and continues to fail closed on revision skew; it does not possess
the Render key. If current `main` is newer and failing, pass the latest
successful 40-character SHA in the manual `revision` input.

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
BASE_RECOVERY_RESERVED_BOUNTY_CONTRACTS=<comma-separated public incident contracts>
BASE_MAINNET_RPC_URL=<managed HTTPS RPC>
BASE_INDEXER_RPC_URL=<managed HTTPS RPC>
BASE_INDEXER_RETRY_INITIAL_SECONDS=5
BASE_INDEXER_RETRY_MAX_SECONDS=120
BASE_INDEXER_EXIT_AFTER_FAILURES=8
ENABLE_BASE_TX_BROADCAST=false
```

Use the corresponding `BASE_SEPOLIA_*` values for testnet. The worker accepts
`BASE_INDEXER_FACTORY_CONTRACT` as an explicit override.

`BASE_RECOVERY_RESERVED_BOUNTY_CONTRACTS` is a public, temporary hosted-routing
control. Every address must have a public incident record. Malformed values stop
API and MCP startup; configured contracts remain visible in the full canonical
feed but cannot appear as earning-ready work or verifier jobs.

Secrets belong in Render environment groups, never in Git:

- `DATABASE_URL`,
- managed RPC credentials,
- optional `OPERATOR_API_TOKEN` for non-protocol administrative surfaces,
- future Stripe secrets and verified webhook secret.

The separate `RENDER_API_KEY` belongs only in GitHub Actions and is never
injected into an application container.

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
phrase or private key into chat, Git, shell history, browser storage, or
committed files. The current exact addresses, constructor inputs, bytecode, and
test-USDC seed are pinned in
[`deployments/base-sepolia-sponsor-activation.json`](../deployments/base-sepolia-sponsor-activation.json).

```powershell
$env:Path = "$PWD\.tools\foundry;$env:Path"
forge test --root contracts\base-escrow --fuzz-runs 1000
python -m http.server 8879 --bind 127.0.0.1
```

Open
`http://127.0.0.1:8879/tools/base-sepolia-sponsor-activation.html` in the
browser profile containing the deployer wallet. The locked console verifies
every action before requesting a wallet confirmation and supports safe resume
after each confirmed component. See
[`base-sepolia-runbook.md`](base-sepolia-runbook.md) for regeneration, native
USDC fork, post-deploy attestation, hosted configuration, and full-loop gates.

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
  --api-base-url https://api.bountyboard.global `
  --mcp-base-url https://mcp.bountyboard.global
```

Check:

- `/health`, `/llms.txt`, OpenAPI, and discovery manifest,
- protocol status and exact factory/implementation agreement,
- canonical feed and verification-job feed,
- terms and evidence persistence across restarts,
- worker heartbeat and confirmed cursor,
- no active legacy escrow endpoints or addresses,
- no secret material in responses or logs.

Run the bounded operational controller after production smoke:

```powershell
python scripts\self_heal.py observe `
  --policy ops\self-healing-policy.json `
  --api-url https://api.bountyboard.global `
  --mcp-url https://mcp.bountyboard.global `
  --expected-revision <deployed-git-sha> `
  --snapshot-out target\operations\snapshot.json `
  --plan-out target\operations\recovery-plan.json
```

API/MCP health failure is handled by Render's service supervisor. The indexer
retries typed RPC/SQL transport failures from its persisted monotonic cursor
with capped exponential backoff and exits after its bounded failure budget so
the worker supervisor can replace the process. Integrity and unclassified
failures halt ingestion after a redacted failed heartbeat. See
[`self-healing-operations.md`](self-healing-operations.md) for SLOs, containment,
and actions that automation is prohibited from taking.

## Fiat Services

Stripe and PayPal are not autonomous-v1 settlement rails. Keep live execution
disabled unless a separately reviewed fiat-to-USDC onramp is implemented with
verified webhooks, compliance controls, idempotency, and exact canonical bounty
funding reconciliation.
