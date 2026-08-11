# Open Competition V2 Beta1 Threat Model

Risk class: R4. The contracts are immutable, hold Base USDC, call an external
proof gateway, accept delegated signatures, and settle without operator
approval.

## Assets And Trust Boundaries

Protected assets are competition USDC, contributor refund rights, solver
identity and nonces, winner ordering, artifact and policy commitments, proof
broker payments, and canonical settlement evidence.

Trust boundaries:

1. A solver or relay sends untrusted journal and proof bytes to a competition.
2. The competition calls a pinned SP1 adapter and canonical gateway.
3. USDC moves between contributors, isolated competitions, solvers, keepers,
   and refund recipients.
4. Base logs cross into the safe-block indexer and database projections.
5. The hosted broker accepts x402 payments and requests off-chain proving.

The protocol trusts Base consensus, the exact deployed USDC bytecode, the
pinned SP1 gateway route, and the correctness of the immutable guest program.
It does not trust the poster, solver, relay, keeper, hosted API, indexer,
database, or broker to decide payment eligibility.

## Primary Abuse Cases And Controls

| Threat | Control | Required test/evidence |
| --- | --- | --- |
| Replay a valid proof on another target | Journal binds chain, contract, bounty, solver, nonce, artifacts, proof system, vkey, policies, and risk hash | Cross-domain replay matrix |
| Relay substitutes a proof or artifact | EIP-712 authorization binds public-values and proof hashes | Substitution and expiry tests |
| ECDSA malleability, forged contract-wallet signature, or return bomb | Low-`s`, strict `v`, recovered signer check; bounded ERC-1271 gas and 32-byte return-data copy | EOA, ERC-1271, and oversized-return vectors |
| Double payment or nonce reuse | Nonce is consumed before transfers; settlement state changes before external token calls | Replay, reentrancy, duplicate transaction tests |
| Participant-count gas denial | No participant arrays; only nonce map, sequence, and current leader | 1/100/10,000 entry gas comparison |
| Malicious or malformed journal | Exact fixed-schema ABI decode and field-by-field comparison | Truncation, trailing data, type, and scope fuzzing |
| Malicious custom vkey | UI marks it unreviewed; on-chain behavior is explicit and immutable | Catalog classification and warning tests |
| Frozen, rerouted, or unavailable gateway | Adapter pins the v6.1 route selector and verifier, rejects route drift and cross-system proof prefixes; unresolved work enters permissionless refunds | Frozen-route, mismatched-route, prefix, and deadline tests |
| Fee-on-transfer or false-return token | Base deployments pin USDC; funding checks exact balance increase; safe-call wrappers reject false/malformed calls | Token behavior tests |
| Reentrant token or ERC-1271 wallet | Contract-wide reentrancy lock; effects precede interactions; ERC-1271 gas bound | Malicious callback tests |
| Best-score tie manipulation | Only strict improvement replaces leader; canonical accepted sequence wins ties | Same-score ordering tests |
| Timestamp boundary ambiguity | Inclusive proof deadline, strictly-after finalization and expiry | Exact boundary tests |
| Refund griefing | Anyone can withdraw for a contributor; recipient cannot be redirected; O(1) remaining-pool math | Third-party withdrawal and rounding tests |
| Factory custody or allowance theft | Factory never receives USDC and never calls `transferFrom`; competition is the spender | Balance/allowance assertions |
| Broker overcharge or abandonment | Release-reviewed profiles only, five-minute exact quote, max charge, provider absorbs overrun, bounded proof/relay authorization, deterministic EIP-3009 refund identity, and canonical refund evidence | Quote/payment/refund race and crash-recovery suite |
| Indexer reorg creates false payment | Safe-block confirmation, event identity dedupe, replay-safe projection; only canonical `CompetitionSettledV2` is payment | Reorg and replay fixtures |
| Synthetic metrics inflate adoption | Canary IDs are explicitly tagged and excluded | Analytics contract tests |

## Residual Risks

- **Unresolved high severity:** SP1 6.3.1 includes
  `p3-challenger@0.4.3-succinct`, which remains covered by
  `GHSA-vj64-rjf3-w3v7` for Fiat-Shamir transcript malleability. The package is
  quarantined to the V2 metric build, and CI requires the exact version,
  checksum, lockfile locations, SP1 commit, unresolved-high gate, and disabled
  mainnet flag. V2 must not graduate or activate mainnet until a compatible
  patched SP1 prover and canonical verifier route are independently reviewed.
- A sound SP1 proof can faithfully execute a flawed metric guest. Program
  review and reproducibility reduce this risk but do not remove it.
- A canonical SP1 gateway route can be frozen or contain an undiscovered bug.
  Immutability prevents a verifier swap; liveness falls back to refunds.
- Base sequencer ordering selects first-proven winners. Solvers should use
  private relay paths when proof copying or transaction ordering is material.
- Best-score entrants pay proving costs even when they lose. Quotes must state
  this before payment.
- Public journals reveal their encoded fields. Private inputs and artifacts
  must be committed by hash and kept outside the journal.
- USDC can be paused or addresses can be sanctioned. The contract cannot make
  a blocked token transfer succeed; monitoring must surface the exact failed
  transition without claiming settlement.

## Fail-Closed Release Rules

- No proxy, owner, admin withdrawal, mutable vkey, mutable gateway, or shared
  bounty custody is permitted in Beta1.
- Mainnet constructor parameters must equal the pinned Base addresses.
- A bytecode, ABI, gateway, program vkey, journal-schema, or metric correction
  creates a new version.
- Build artifacts include source commit, compiler image, compiler settings,
  SP1 version, source/ELF/vkey hashes, runtime bytecode hashes, constructor
  calldata, risk hash, and golden vectors.
- Mainnet signing is blocked until the repository R4 gate, Base Sepolia
  rehearsal, mainnet-fork replay, and independent hash-bound review pass.
- `GHSA-vj64-rjf3-w3v7` must be removed from the dependency-review allowance,
  and the quarantine gate must be replaced by fixed-version evidence, before
  `critical_and_high_findings_resolved` can become true.
- Deployment and transaction hashes are not payment evidence.
