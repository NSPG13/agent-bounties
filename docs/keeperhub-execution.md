# KeeperHub execution adapter

Agent Bounties uses KeeperHub as a bounded onchain execution layer. The first
public integration creates one **unfunded Base Sepolia Open Competition
canary** through the rehearsed V1 factory. It spends testnet gas only, transfers
no USDC, and does not modify an existing bounty.

This integration is deliberately narrower than KeeperHub's generic direct
execution API:

- chain: Base Sepolia (`84532`)
- contract: `0x7231f1312448fa60078fb56cdb6e2c392bd1269b`
- function: `createCompetition`
- native value: zero
- initial USDC funding: zero
- verifier: `LeadingZeroWorkVerifier(16)` at
  `0x9601a40b35ad6843846732c6cb73c4c82f9ba850`

The adapter rejects every other chain, contract, function, native value, and
nonzero initial-funding request.

## Judge in 60 seconds

The public KeeperHub execution is
[`0x80fb...b329`](https://sepolia.basescan.org/tx/0x80fb04d83d6135c2b1f9753d9fb449a693d9f1be0a84fddcc60f03ecee6ab329),
with KeeperHub execution ID `z2lp1eatpds9fx766rktg`. Its machine-readable
receipt is checked in at
[`docs/evidence/keeperhub-agents-onchain-canary-base-sepolia-2026-08-13.json`](evidence/keeperhub-agents-onchain-canary-base-sepolia-2026-08-13.json).

Verify the receipt directly against Base Sepolia:

```powershell
node scripts/verify_keeperhub_canary_evidence.mjs `
  --evidence docs/evidence/keeperhub-agents-onchain-canary-base-sepolia-2026-08-13.json `
  --rpc-url https://sepolia.base.org
```

The verifier fails closed unless the RPC reports Base Sepolia, the exact
transaction succeeded at the recorded block with the recorded gas usage, and
the factory emitted exactly one matching `CanonicalCompetitionCreated` event
for the recorded bounty ID, bounty address, and creator. It does not infer
funding or payment from a successful transaction.

## Authentication

Create an organization API key (`kh_`) in KeeperHub under **Settings → API
Keys → Organisation**. Store it only in the local `KH_API_KEY` environment
variable. Never put it in a request file, shell history, issue, receipt, commit,
or chat message.

KeeperHub's organization wallet needs a small Base Sepolia ETH balance for gas.
No USDC is needed for this canary.

## Prepare the exact request

Use the KeeperHub organization wallet shown in the KeeperHub Wallet page:

```powershell
node scripts/build_keeperhub_open_competition_canary.mjs `
  --wallet 0xKEEPERHUB_ORG_WALLET `
  --source-url https://github.com/NSPG13/agent-bounties/issues/931 `
  --output target/keeperhub-open-competition-canary.json
```

The request commits to a 0.10 test-USDC solver reward and 0.01 test-USDC
verifier reward, but `initialFunding` is zero. The resulting bounty remains in
`funding_needed` unless it is separately funded later.

## Simulate before signing

```powershell
node scripts/keeperhub_direct_execution.mjs simulate `
  --request target/keeperhub-open-competition-canary.json
```

Continue only if KeeperHub returns `success: true` and `wouldRevert: false`.
A simulation is not a transaction or payment receipt.

## Execute once and retain the receipt

Execution requires a fresh idempotency key and a new receipt path. The receipt
writer uses create-only semantics, so it cannot overwrite earlier evidence.

```powershell
$keeperhubIdempotencyKey = "agent-bounties-keeperhub-" + [guid]::NewGuid()
node scripts/keeperhub_direct_execution.mjs execute `
  --request target/keeperhub-open-competition-canary.json `
  --idempotency-key $keeperhubIdempotencyKey `
  --receipt target/keeperhub-open-competition-receipt.json
```

The adapter polls KeeperHub's status endpoint using its poll-interval hint and
accepts success only when the final response contains all of:

- `status: completed`
- a 32-byte transaction hash
- an HTTPS block-explorer link

The public receipt deliberately excludes the API key. It proves one
KeeperHub-submitted Base Sepolia transaction. It does not prove bounty funding,
solver settlement, or payment; only canonical contract events establish those
states.

## Verification

```powershell
node scripts/test_keeperhub_direct_execution.mjs
```

The tests cover simulation enforcement, the chain/contract/function allowlist,
zero initial funding, idempotency, status polling, receipt requirements, and
secret exclusion.
