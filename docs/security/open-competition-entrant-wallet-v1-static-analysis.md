# Open Competition Entrant Wallet V1 Static Analysis

Slither `0.11.5` analyzed the complete `contracts/base-escrow` Foundry project
with tests, scripts, and dependency findings filtered from the release review.
The machine-readable result is generated at
`target/open-competition-entrant-wallet-slither.json` and is not committed.

The new entrant wallet and factory produced 13 findings. No untriaged finding
is accepted for release:

| Detector | Count | Classification |
| --- | ---: | --- |
| `reentrancy-balance` | 2 | False positive for the two factory funding paths. Both external entry points hold the factory's storage reentrancy lock across deployment, pre-balance, native-USDC transfer, and post-balance. The deployment manifest pins native Base USDC, which has no receiver callback. Tests reject a non-transferring token. |
| `incorrect-equality` | 2 | Intentional exact balance-delta invariant. Fee-on-transfer, rebasing, mint-on-transfer, and malformed tokens are unsupported and must fail rather than underfund or overfund a policy-bound wallet. |
| `timestamp` | 4 | Intended policy validity, signature-expiry, and spend-period boundaries. Deadlines are short and checked again in action-time simulation; they do not provide randomness or select a winner. |
| `assembly` | 3 | Minimal-proxy CREATE2, bounded-gas ERC-1271 static call, and low-s ECDSA decoding. Exact runtime hashes and signature adversarial tests cover these paths. |
| `low-level-calls` | 2 | Owner-only ETH recovery and exact native-USDC allowance reset/set with return-value validation. The delegate and keeper cannot invoke withdrawals. |

The apparent factory reentrancy findings do not make arbitrary tokens safe.
Hosted deployment is permitted only when the exact competition factory binds
the manifest's native-USDC address and action-time balance-delta checks pass.
Any deployment with another token requires a new threat
model, static-analysis triage, and tests.

Static analysis is one R4 input, not release approval. Base Sepolia rehearsal,
exact mainnet-fork replay, independent review, bytecode/manifest audit, and
safe-block receipt reconciliation remain required before hosted relay or gas
sponsorship can be enabled.

## Public-activation rerun

Before the public-activation candidate was promoted, Slither `0.11.5` was run
again over every `OpenCompetition*` contract with tests, scripts, and
dependencies excluded from detector output. The machine-readable result is
generated at `target/open-competition-public-activation-slither.json` and is
not committed. The run reported 25 raw results and zero untriaged high- or
medium-impact findings.

The two raw high-impact `reentrancy-balance` results and two medium-impact
`incorrect-equality` results are the same entrant-factory funding paths
triaged above. The factory's storage lock remains held across deployment,
native-USDC transfer, and exact post-transfer balance validation. Exact
equality is the fail-closed token invariant, not an assumption that arbitrary
ERC-20 behavior is supported.

The remaining raw medium-impact result is Slither's `uninitialized-local`
warning for `bonus` in `OpenCompetitionBountyV1.withdrawRefund`. Solidity
initializes local value types to zero. The function assigns a nonzero bonus
only when `refundBonusRemaining > 0`, so zero is the intended value when the
bonus pool is exhausted. Contract tests cover contributor refunds and escrow
conservation. Changing this source-only spelling would alter frozen bytecode
without changing runtime behavior and would invalidate the completed
rehearsal, fork replay, and deployed-runtime review.

The low and informational results are the expected deadline comparisons,
initialization event arithmetic, minimal-proxy and signature assembly, and
owner-only ETH recovery / exact native-USDC approval calls. They do not add a
new verifier, token, upgrade path, or settlement authority.
