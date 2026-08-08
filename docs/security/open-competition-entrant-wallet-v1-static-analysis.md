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
