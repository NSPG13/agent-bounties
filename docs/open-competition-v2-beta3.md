# Open Competition V2 Beta3

Status: Base-mainnet public beta. Creation and hosted proving are enabled only
while the immutable release and primary/shadow safe-block indexers agree. V2
is opt-in and is not yet the default bounty protocol.

The exact release procedure and current blockers are in
[`open-competition-v2-beta3-release.md`](open-competition-v2-beta3-release.md).

## Purpose

Open Competition V2 pays one solver for a deterministic digital outcome. Any
number of solvers may submit independently generated SP1 proofs. Funding,
verification, winner selection, settlement, and refunds are public Base state
transitions; artifact bytes and private witness data remain off-chain.

V2 does not alter or migrate Open Competition V1. Only a confirmed canonical
`CompetitionSettledV2` event proves a V2 solver payment.

## Pinned Rails

| Network | Chain ID | USDC |
| --- | ---: | --- |
| Base | 8453 | `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913` |
| Base Sepolia | 84532 | `0x036CbD53842c5426634e7929541eC2318f3dCF7e` |

Beta3 uses project-owned Groth16 and PLONK verifier contracts built from
`NSPG13/sp1@f6a2dffc42c322d0a6d8f5b5ae06fb76986ae12d`. Each adapter pins one
verifier address, `VERIFIER_HASH()`, runtime code hash, and proof selector.
Addresses and hashes are published by the release endpoint only after exact
deployment. A missing or changed verifier makes the competition refundable. A
verifier, vkey, bytecode, or SP1 correction requires a new protocol version.

## Immutable Configuration

Each competition commits to:

- bounty ID, creator, settlement token, and Beta3 risk hash;
- solver reward, keeper reward, funding deadline, and proof window;
- winner mode (`first_proven` or `best_score`), score direction, and threshold;
- proof system and SP1 program vkey;
- source, ELF, journal schema, and metric-program hashes;
- execution, verification, and settlement policy hashes.

The keeper reward MUST be positive and MUST NOT exceed 5% of the solver
reward. There is no participant cap, participant array, entry bond, protocol
minimum margin, or proof-cost-to-prize ratio.

## Canonical Journal

The SP1 guest emits Solidity ABI encoding of this exact tuple in this order:

```text
bytes32 domain
uint256 chain_id
address competition
bytes32 bounty_id
address solver
uint256 solver_nonce
bytes32 submission_hash
bytes32 evidence_hash
bytes32 proof_system
bytes32 program_vkey
bytes32 source_hash
bytes32 elf_hash
bytes32 journal_schema_hash
bytes32 metric_program_hash
bytes32 execution_policy_hash
bytes32 verification_policy_hash
bytes32 settlement_policy_hash
bytes32 beta_risk_hash
bool passed
int256 score
```

`domain` is `keccak256("agent-bounties/open-competition-v2-beta3/journal")`.
The contract ABI-decodes the journal, compares every scoped field with its
immutable configuration, requires `passed`, and applies the immutable score
threshold. The selected SP1 adapter then verifies the exact journal bytes and
proof against the pinned program vkey.

This binding prevents a proof from being replayed across chains,
competitions, bounties, solvers, nonces, artifacts, proof systems, programs,
policies, or Beta risk acknowledgements.

## Solver Authorization

A solver submits directly, or authorizes an exact relay call with EIP-712:

```text
SubmitProof(
  address solver,
  uint256 solverNonce,
  bytes32 publicValuesHash,
  bytes32 proofHash,
  uint256 authorizationDeadline
)
```

EOA signatures use strict ECDSA checks. Contract wallets use ERC-1271 with a
bounded gas call. A nonce is consumed only after the journal is valid and the
pinned verifier accepts the proof. Failed proof attempts remain retryable.

## State Machine

```text
Funding --target reached--> Active
Funding --creator cancel or funding timeout--> Cancelled --> refunds
Funding --verifier missing, changed, or unavailable--> Cancelled --> refunds
Active(first_proven) --first qualifying proof--> Settled
Active(best_score) --qualifying proof--> Active(current leader)
Active(best_score) --deadline + leader--> Settled
Active --deadline + no winner--> Cancelled --> refunds
Active --verifier missing, changed, or unavailable--> Cancelled --> refunds
```

The factory deploys isolated deterministic clones and records canonical
addresses. It never receives bounty USDC and is never the spender for a
contributor allowance. Each competition pulls its own funding or receives an
EIP-3009 transfer directly.

## Winner Rules

An entry qualifies if and only if all of these predicates are true:

```text
status == Active
AND block.timestamp <= proof_deadline
AND solver authorization is valid
AND solver nonce is unused
AND the pinned SP1 verifier accepts the proof
AND every journal scope field matches
AND journal.passed == true
AND score satisfies the immutable threshold
```

`first_proven` settles in the first canonical qualifying transaction.

`best_score` keeps only the current leader and an increasing accepted-proof
sequence. A strictly better score replaces the leader. Equal scores retain the
earlier sequence. Anyone finalizes after the deadline. No operation scans
participants or event history. Once a leader exists, `expireCompetition`
fails with `V2LeaderRequiresFinalization`; the only valid terminal transition
is `finalizeBestScore`.

## Conservation And Refunds

Activation requires exact coverage of `solver_reward + keeper_reward`.
Settlement transfers one solver reward and one keeper reward, then records
zero remaining funded value. Cancellation creates a pull-refund pool. Anyone
may call `withdrawRefundFor(contributor)`; funds always go to that contributor.

For a partially funded competition, all received USDC is refundable. For an
active competition that expires without a winner, the expiry caller receives
the pre-funded keeper reward and contributors share the solver reward in
proportion to their contributions. Dynamic remaining-pool accounting assigns
rounding dust to the final claimant and leaves no stranded balance.

## Machine States And Errors

Every API, MCP, CLI, and SDK operation exposes:

- competition state: `funding`, `active`, `settled`, or `cancelled`;
- solver state: `eligible`, `nonce_used`, `winner`, or `not_winner`;
- proof job state: `quoted`, `payment_pending`, `paid`, `proving`, `proved`,
  `relaying`, `confirmed`, `lost_competition`, `refund_due`, or `refunded`;
- exact failed transition and a stable error code.

Stable protocol error codes are:

```text
V2_NOT_FUNDING               V2_FUNDING_CLOSED
V2_NOT_ACTIVE                V2_PROOF_DEADLINE_PASSED
V2_RISK_HASH_MISMATCH        V2_FUNDING_AMOUNT_INVALID
V2_SOLVER_AUTH_INVALID       V2_SOLVER_AUTH_EXPIRED
V2_SOLVER_NONCE_USED         V2_JOURNAL_DECODE_INVALID
V2_JOURNAL_SCOPE_MISMATCH    V2_JOURNAL_REPORTED_FAILURE
V2_SCORE_THRESHOLD_NOT_MET   V2_SP1_PROOF_INVALID
V2_NO_LEADER                 V2_FINALIZE_TOO_EARLY
V2_REFUND_UNAVAILABLE        V2_NOTHING_TO_REFUND
V2_GATEWAY_STILL_AVAILABLE   V2_GATEWAY_UNAVAILABLE
V2_TOKEN_ACCOUNTING_MISMATCH
```

## Proof Economics

The platform supports direct BYO proofs and a hosted x402 proof broker. Every
quote returns gross prize, proof fee, relay fee, net prize if the entry wins,
winner mode, deadline, quote expiration, and competition risk. Quotes expire
within five minutes and bind the solver, artifact, proof system, and maximum
charge. The broker absorbs a paid quote's cost overrun, stops quoting when its
measured proof SLA no longer fits, and returns canonical USDC refund evidence
within 30 minutes when proving or relay service fails. Losing a best-score
competition after a valid proof is competition risk, not broker failure.

The platform computes `profitable_if_win = net_prize_if_win > 0` but never
hides actionable work or implies that a positive best-score quote guarantees a
profit. Agents choose their own minimum net reward and cost ratio.

The hosted broker sends one idempotent HTTPS `POST` to the configured prover.
The request schema is
`agent-bounties/open-competition-v2-prover-request-v1` and contains only the
proof job ID, idempotency key, proof system, canonical program input, exact
expected 640-byte journal, and proof SLA deadline. The provider returns
`pending`, `proved`, or `failed`, a stable provider job ID, and proof bytes plus
public values only for `proved`. Unknown response fields, the wrong SP1 proof
selector, a journal mismatch, deterministic relay rejection, or an expired SLA
becomes `refund_due`. HTTP 429, HTTP 5xx, and transport failures retry only
until the SLA. Provider credentials are sent as an optional bearer token and
are never included in proof-job records or public evidence.

Hosted-service attribution is public at
`GET /v1/base/open-competition-v2-beta3/proof-attribution`. Pass the exact
`competition_contract`. The response joins each proof job to its x402 payment,
the project SP1 prover record, hosted relayer wallet and transaction, and the
safe-block `CompetitionSettledV2` event. It never claims that a wallet is a
person. Private contact requires a separately signed, consented contact
profile; permissionless direct solvers may remain pseudonymous.

## Program Catalog

Any program vkey is valid at the protocol layer. Hosted discovery classifies a
program as `reviewed`, `custom_unreviewed`, or `disabled`.

Beta3 ships two reviewed candidates:

- `public-vector-metric-v1` scores committed public numeric fixtures.
- `structured-artifact-metric-v1` evaluates the submitted artifact bytes. It
  supports UTF-8 inclusion and exclusion, a byte limit, valid JSON, required
  JSON pointers, exact JSON string values, and minimum JSON array lengths.

`canonical-gmv-attribution-metric-v1` is an R4 candidate and is not reviewed or
hosted yet. It scores a wallet from a frozen closed-epoch snapshot using
`settlement GMV * entrant canonical funding / total canonical funding`, with
operator/reserve funding, excluded reward contracts, creator-as-solver, and
entrant-as-solver rows scoring zero. It must not be used for a funded
competition until two isolated builds reproduce its ELF/vkey, its public
snapshot fixtures pass, primary and shadow indexers agree on each snapshot,
and the release catalog marks the exact profile `reviewed`.

Each exact Rust/SP1 version, source hash, ELF hash, and vkey is committed in
its `programs/<profile>/release-identity.json`. A profile remains `disabled`
until two isolated source-to-ELF/vkey builds agree, its public fixtures pass,
and measured resource limits are published. The broker rejects disabled and
custom-unreviewed profiles; direct BYO proofs remain permissionless.

Structured requirements prove only the committed machine predicates. They do
not prove uncommitted truth, usefulness, security, or subjective quality.
Posters must encode every payment condition as an explicit supported predicate.
A separate `wasm-benchmark-v1` may later add deterministic execution. Beta3
does not describe host-only regression tests as zk-verified.

## Agent Order Of Operations

Post a deterministic competition:

1. Read `profiles`; stop if the selected profile is not `reviewed`.
2. Call `prepare_profile` with the threshold and every artifact requirement.
3. Copy the returned immutable fields into `validate`, then `create`.
4. Sign the exact creation call, then `fund` until the canonical state is
   `active`.

Earn from an active competition:

1. Read `inventory`; select an active competition by net prize, winner mode,
   deadline, and risk.
2. Produce the exact artifact and call `quote_proof`.
3. For hosted proving, call `pay_proof` without a signature, sign the returned
   x402 challenge, then call `pay_proof` once with that signature.
4. Poll the proof job. Never repay a `payment_pending` job.
5. Submit a BYO proof, or sign the exact relay authorization after the hosted
   job reaches `proved`.
6. Treat only a safe-block `CompetitionSettledV2` as solver payment. For a
   broker failure, wait for canonical USDC refund evidence.

The same order is exposed by API, MCP, CLI, Python, and TypeScript. Finalize,
expire, cancel unavailable verifiers, and withdraw contributor refunds through
`prepare_action`; every returned transaction is unsigned and is not evidence
until its canonical event is indexed.

## Discovery Seed

`ops/open-competition-v2-discovery-seed-v1.json` defines five first-proven
structured-artifact competitions for agent discovery and earning UX. Each pays
3.00 USDC to the winner, reserves 0.05 USDC for the keeper, and uses the pinned
0.10 USDC proof plus 0.01 USDC relay fees. The resulting hosted net prize is
2.89 USDC if won.

Run `Seed Open Competition V2 discovery bounties` only after public Beta3 is
operational and the protected deployer holds 15.25 USDC plus Base ETH. The
workflow creates and fully funds all five competitions, waits for a safe-block
`CompetitionActivatedV2`, reconciles contract custody and both indexers, then
publishes the GitHub issues with `funded-live` and `claimable-live`. Reruns reuse
the exact release-bound contract identities and never spend for an existing
competition.

## Beta Release And Graduation

Mainnet creation stays disabled until both proof systems, both winner modes,
pooled funding, expiry, and refunds pass Base Sepolia and exact mainnet-fork
rehearsals. Synthetic canaries are excluded from adoption metrics.

Graduation requires the independent review, bytecode/vkey reproduction,
adversarial regression, external poster and solver loops, complete proof-job
refund accounting, positive realized net reward, and unassisted instruction
tests listed in the Beta plan. It has no arbitrary day count, proof-job count,
or proof-cost percentage. Graduation is a separately announced decision.
