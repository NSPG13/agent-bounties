# Open Competition V2 Beta2 Threat Model

Risk class: R4. Immutable contracts hold Base USDC, call project-owned SP1
verifiers, accept scoped signatures, and settle without operator approval.

## Trust Boundaries

Protected assets are competition USDC, refund rights, solver identities and
nonces, winner ordering, policy commitments, proof-broker payments, and
canonical settlement evidence.

The protocol trusts Base consensus, pinned Base USDC bytecode, the exact
deployed verifier bytecode, and the immutable metric guest. It does not trust
the poster, solver, relay, keeper, API, database, indexer, broker, or prover to
decide payment eligibility.

| Threat | Control |
| --- | --- |
| Cross-target proof replay | Journal binds chain, competition, bounty, solver, nonce, artifacts, proof system, vkey, policies, and risk hash. |
| Relay substitution | EIP-712 authorization binds public-values and proof hashes. |
| Signature forgery or return bomb | Low-`s`, strict `v`, recovered signer checks, bounded ERC-1271 gas and return-data copy. |
| Double pay or nonce reuse | Nonce and state change precede external token calls. |
| Participant-count denial | No participant array or participant-dependent loop. |
| Malformed journal | Fixed-length ABI decode and exact field comparison. |
| Unsafe verifier replacement | Adapter pins verifier address, runtime code hash, verifier hash, and proof selector; no proxy or owner exists. |
| Groth16 toxic-waste compromise | Mainnet requires a hash-chained Phase 1 and Phase 2 MPC with at least two ephemeral contributions in each phase and post-contribution beacons; local `groth16.Setup` assets are test-only. |
| PLONK SRS forgery | The release pins the Aztec Ignition public MPC SRS transcript, derived keys, verification evidence, and exact verifier bytecode. |
| Missing verifier | Permissionless cancellation and contributor refunds restore liveness. |
| Transcript malleability | Every downstream prover graph pins the injective safe fork; native and recursion regressions run in CI. |
| Fee-on-transfer or false-return token | Base release pins USDC and checks exact balance changes and safe-call results. |
| Reentrancy | Contract-wide lock and effects before interactions. |
| Best-score tie manipulation | Strict improvement only; canonical accepted sequence wins ties. |
| Timestamp ambiguity | Inclusive proof deadline and strictly-after finalization/expiry. |
| Refund griefing | Anyone may withdraw for a contributor, but cannot redirect the recipient. |
| Broker overcharge or abandonment | Five-minute bound quote, maximum charge, 30-minute canonical refund SLA, dedicated broker key, and a safe-block gate requiring one full-charge USDC refund reserve plus bounded relay gas. |
| Indexer reorg | Safe-block projection, event identity dedupe, and primary/shadow RPC comparison. |
| Synthetic adoption inflation | Canary IDs are marked and excluded from adoption metrics. |

## Residual Risks

- A sound proof can faithfully execute a flawed metric guest. Reproducible
  builds and adversarial vectors reduce this risk but do not remove it.
- The patched SP1 fork is project-maintained. A source pin prevents silent
  drift, but a defect requires a new circuit version, vkeys, verifiers, and
  protocol version.
- The initial Groth16 ceremony is internally orchestrated. Ephemeral isolated
  contributors and post-contribution beacons prevent retained randomness under
  the documented process, but this Beta does not claim independent public
  ceremony participants.
- Immutable verifier failure cannot be repaired in place. Competitions cancel
  into permissionless refunds.
- Base sequencer order selects first-proven winners. Transaction ordering and
  proof copying remain competition risks.
- Best-score entrants pay proof costs even if they lose.
- Public journals reveal encoded fields. Private material must remain off-chain
  behind committed hashes.
- USDC pause or sanctions can block a transfer. The system must expose the
  failed transition and never claim settlement.

## Fail-Closed Rules

- No proxy, owner, admin withdrawal, mutable vkey, mutable verifier, or shared
  competition custody.
- No GPU or public-network proof is release evidence for Beta2.
- Any bytecode, ABI, verifier, vkey, journal, or metric correction creates a new
  version.
- Production generation requires self-verified proof assets and a reproduced
  metric identity.
- Mainnet deployment, canaries, and activation require separate evidence-bound
  owner approvals.
- The broker, keeper, and deployment signer must be distinct. Broker and public
  activation fail closed below the release-pinned USDC and Base ETH reserves.
- Deployment and transaction hashes are never payment evidence.
