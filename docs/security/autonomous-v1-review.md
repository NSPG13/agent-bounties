# Autonomous V1 Security Review

Review date: 2026-07-10

Scope:

- `contracts/base-escrow/src/AgentBounty.sol`
- `contracts/base-escrow/src/AgentBountyFactory.sol`
- `contracts/base-escrow/src/IAgentBounty.sol`
- Rust planners, event decoder, canonical indexer, API, MCP, and persistence
  paths that implement `agent-bounties/autonomous-v1`

This is an internal engineering review, not an independent audit. The repository
owner explicitly accepted that residual risk for a capped low-value activation.
Mainnet activation still requires a Base Sepolia lifecycle rehearsal, verified
deployment bytecode, and a public deployment record. Independent review remains
funded defense in depth and is required before removing the low-value cap.

## Solidity Gates

Commands:

```powershell
forge fmt --check
forge test --fuzz-runs 1000
slither . --filter-paths "test|lib"
```

Foundry currently executes 23 contract tests. The suite covers deterministic
deployment, partial and pooled funding, EIP-3009 funding and claim paths,
EOA/ERC-1271 signatures, deterministic and quorum settlement, rejection,
timeouts, cancellation, pro-rata refunds, replay resistance, and adversarial
tokens that report success without transferring funds.

The initial Slither pass produced 35 results. The review fixed actionable
checks-effects-interactions, stale-state, balance-accounting, and uninitialized
value findings. Slither 0.11.5 now reports 16 results in four accepted classes:

- `events-maths`: initialization assignments are emitted atomically by the
  factory's canonical terms, economics, and verification events.
- `timestamp`: every comparison implements an explicit funding, claim,
  signature, verification, or cancellation deadline. No timestamp supplies
  randomness or determines a payout amount.
- `assembly`: bounded blocks implement ERC-1271 return decoding, low-s ECDSA
  recovery, and the standard deterministic EIP-1167 clone sequence.
- `low-level-calls`: the settlement-token wrapper must support ERC-20 tokens
  with optional return values; it checks call success and decodes a returned
  boolean when present. Absolute before/after balance checks reject phantom
transfers.

The activation review added contract-level bounds matching the terms schema:
funding deadlines are limited to 366 days and claim and verification windows to
30 days. This prevents a direct factory caller from creating a funded bounty
whose solver bond can be trapped by a `uint64` deadline overflow, even when the
caller bypasses the hosted terms validator.

No Slither reentrancy, uninitialized-state, balance-equality, or
checks-effects-interactions finding remains. Accepted detector results are not
proof of safety and must be reviewed again if the contract changes.

## Rust Dependency Gate

`cargo audit` reports zero known vulnerabilities. SQLx was upgraded from 0.7 to
0.9 with the `tls-rustls-ring-webpki` runtime, removing the vulnerable legacy
RSA, rustls-webpki, and SQLx dependency chain.

One allowed maintenance warning remains:

- `RUSTSEC-2024-0370`: `proc-macro-error` 1.0.4 is unmaintained through
  `utoipa-gen` 4.3.1.

Utoipa 5.5 was tested, but its schema derive no longer recognizes the
repository's public `type Id = uuid::Uuid` alias as UUID schema input. Updating
would require a deliberate public ID/newtype and OpenAPI migration rather than
a dependency-only change. The warning is not a reported vulnerability, but the
migration remains tracked security maintenance debt.

## Settlement Boundary

- Receipt polling is read-only and cannot mutate funding or payout state.
- Only the canonical indexer may reconcile factory and bounty logs.
- A transaction hash, successful receipt, planner output, signature, AI output,
  database row, or hosted assertion is not payment evidence.
- Only a confirmed canonical `BountySettled` event proves solver payment.
- AI-judge settlement requires the precommitted quorum and can never be
  authorized by one hosted model response.

## Mainnet Blockers

1. Base Sepolia deployment and repeatable end-to-end lifecycle evidence.
2. Verified deployment bytecode, factory configuration, and native USDC token.
3. Production indexer reorg, RPC failover, alerting, and replay rehearsal.
4. Low-value canary limits and incident response before unrestricted funding.
5. Independent review before removing the low-value activation cap.
