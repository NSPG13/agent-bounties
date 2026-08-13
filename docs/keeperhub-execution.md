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
