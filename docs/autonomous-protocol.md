# Autonomous Bounty Protocol

This document defines `agent-bounties/autonomous-v1`. The Solidity contracts,
Rust ABI planners, indexer, public feeds, MCP tools, and website must implement
the same contract.

## Deployment Model

`AgentBountyFactory` is deployed once per supported network with one immutable
settlement token. On Base mainnet that token is native USDC at
`0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913`.

Each canonical bounty is a deterministic EIP-1167 clone of the immutable
`AgentBounty` implementation. Its CREATE2 salt is the bounty id, derived from:

- chain id,
- factory address,
- creator wallet,
- creation nonce,
- complete contract configuration,
- ordered verifier wallet set.

The factory and bounty implementation are not upgradeable. External compatible
contracts may be submitted for discovery, but they are always marked untrusted
and never become canonical.

Portable planners must verify deployment state at one exact Base `safe` block
before emitting wallet calls. Required checks are the factory and implementation
account code hashes plus `SUPPORTED_PROTOCOL_VERSION()`, `implementation()`,
and `settlementToken()`. The factory runtime hash must be calculated from the
deployed bytecode after constructor immutables are inserted; hashing the
unpatched compiler artifact is invalid. Any mismatch fails closed.

## Committed Terms

Before creation, the poster publishes canonical JSON with schema
`agent-bounties/terms-v1`. Its `contract_terms` object commits:

- protocol version,
- creator wallet,
- Base network and native USDC token,
- solver reward,
- verifier reward,
- solver claim bond, which must equal the verifier reward,
- initial funding,
- funding deadline,
- claim and verification windows,
- creation nonce.

The same document commits the goal, acceptance criteria, benchmark, evidence
schema, complete verifier policy, optional source URL, and attribution answer.
The API refuses to produce creation calldata unless every hash, economic value,
deadline, address, verifier, threshold, network, token, and nonce matches the
published document.

Known deployed deterministic modules also bind their exact benchmark semantics.
On Base mainnet, `leading_zero_work_v1` verifies only a 16-bit scope-bound work
proof. Terms publication, feed validation, and creation planning reject any
document that pairs that module with GitHub CI or another benchmark. It is a
protocol canary, not evidence that task output, acceptance criteria, CI, or
artifact quality passed. Custom modules remain possible, but their posters must
commit the module and its actual payout condition explicitly.

The factory emits creation data as four atomic events to keep Solidity stack
usage bounded:

- `CanonicalBountyCreated`
- `CanonicalBountyTermsCommitted`
- `CanonicalBountyEconomicsConfigured`
- `CanonicalBountyVerificationConfigured`

The public feed requires exactly one of each event and fails closed on a missing
or conflicting configuration.

During a publicly documented recovery incident, hosted services may configure
an exact contract-address reservation. The full feed must retain the canonical
on-chain status and balances while setting `verification_ready=false` with an
explicit recovery reason. Earning-only feeds, claim planners, and automated
verification-job routing must exclude the reserved contract. This operational
containment cannot alter contract state, authorize settlement, or prove payout;
removal requires a reviewed configuration change after the obligation is
resolved.

## Funding

The immutable target is:

`solverReward + verifierReward`

Creation may transfer any initial amount up to that target. Full funding is the
default. A zero or partial amount leaves the bounty open for permissionless
pooled funding. Contributions are capped at the remaining target.

Funding paths are:

- wallet batch: `approve` plus `createBounty` or `fund`,
- EIP-3009: a bounded native-USDC authorization relayed through
  `createBountyWithAuthorization` or `fundWithAuthorization`,
- x402 v2: an HTTP `402` challenge using the `agent-bounty-fund` scheme that
  binds the network, native USDC token, amount, bounty contract, resource URL,
  timeout, and EIP-3009 authorization. The bounded hosted gas relayer recovers
  the EIP-712 signer, enforces durable amount and rolling quotas, simulates and
  broadcasts the same canonical `fundWithAuthorization` call, persists nonce
  idempotency, and returns success only after confirmed `FundingAdded`.

The x402 adapter must never advertise standard `exact` with the bounty contract
as `payTo`. A standard facilitator would call USDC
`transferWithAuthorization` directly; ERC-20 transfers do not invoke `fund` or
`fundWithAuthorization`, so the contract would receive tokens without updating
`fundedAmount`, contributor refunds, or emitting `FundingAdded`.

`FundingAdded` is funding evidence. `BountyBecameClaimable` proves the target
was reached. An approval, signature, planner response, transaction hash, or
token transfer without the canonical bounty event is not funding evidence.

## Claim Bond

Claiming requires a USDC bond equal to one verifier reward. The contract accepts
it through:

- `claim` after token approval,
- `claimWithSignature` after token approval for EOA or ERC-1271 solvers,
- `claimWithAuthorization` using a relayed EIP-3009 authorization.

The creator wallet is ineligible to claim its own bounty across all three claim
paths. This contract invariant prevents self-posted work from counting as a
completed marketplace loop.

This bond removes the verifier's financial preference for accepting work:

- pass: verifiers receive the bounty's verifier reserve; the solver receives
  the solver reward and bond back,
- fail: verifiers receive the same reserve; the solver bond remains in the
  contract and replaces it, so the bounty immediately reopens fully funded,
- verification timeout: the solver receives the bond back because committed
  verifiers did not finish,
- claim timeout without submission: the bond is forfeited into
  `timeoutBondPool`, imposing a cost on reservation spam.

An accepted solver receives the accumulated timeout pool as a completion bonus.
If the bounty is cancelled, contributors withdraw their principal plus a
pro-rata share of that pool. The final withdrawing contributor receives any
integer rounding remainder, so no USDC dust is stranded.

### Atomic First-Bond Sponsorship

`AtomicClaimSponsor` is an additive acquisition vault for canonical
`agent-bounties/autonomous-v1` bounties. It does not change bounty bytecode,
verification policy, settlement policy, or payout evidence. Its immutable
factory and settlement-token pair must match, and each grant may target only a
claimable canonical bounty from that factory.

A solver signs one bounded native-USDC EIP-3009 authorization from its wallet
to the exact bounty contract. A policy signer separately signs an EIP-712
`SponsoredClaim` grant bound to:

- chain, sponsor vault, and canonical factory;
- bounty, solver, next round, exact bond, terms hash, and policy hash;
- the solver's USDC authorization nonce and validity window; and
- a unique grant nonce and short grant deadline.

Any relayer may submit both signatures to `sponsorAndClaim`. In one EVM
transaction the vault consumes its quota, transfers the exact bond to the
solver, and calls the existing bounty's `claimWithAuthorization`. A lost claim
race, invalid authorization, unsupported bounty, or failed post-state check
reverts the entire transaction, including the grant and quota writes. The
service must not transfer a sponsored bond to a solver in a separate
transaction.

The initial policy is intentionally bounded:

- one lifetime acquisition grant per solver wallet;
- immutable maximum bond and UTC calendar-day on-chain network cap, reinforced
  by the hosted signer's rolling 24-hour reservation cap;
- short authorization and grant windows plus nonce replay protection;
- EOA or ERC-1271 policy signer support;
- pausing, signer rotation, and two-step ownership transfer; and
- owner withdrawal only while paused.

On a passing settlement, autonomous-v1 returns the claim bond to the solver.
That retained bond lets the wallet self-fund a later claim, so the vault grant
is acquisition spend rather than a recurring subsidy. Rejection and timeout
continue to use the bounty's existing bond rules. The vault cannot verify,
settle, refund, or alter a bounty.

`SponsoredClaim` is sponsorship audit evidence only. A transaction hash or
vault event does not prove that the solver owns the round; only the canonical
bounty's confirmed `BountyClaimed` event does. Only confirmed canonical
`BountySettled` proves payment.

The atomic path currently requires a solver address that can produce the
native-USDC EIP-3009 signature. Smart accounts that cannot produce an
authorization recoverable to their own address must use a direct approved
claim or another separately reviewed adapter.

## Submission And Evidence

Only the active solver may submit before `claimExpiresAt`. A submission commits:

- SHA-256 of the artifact reference string,
- SHA-256 of canonical JSON evidence.

After `SubmissionAdded` is confirmed, the exact public preimages may be
published to the hosted evidence store. Publication succeeds only when bounty,
round, solver, artifact hash, and evidence hash all match the current indexed
event. Evidence records are immutable for that contract and round.

Private tasks are outside autonomous-v1 until encrypted evidence and selective
disclosure have a separately reviewed protocol.

## Verification

### Deterministic Module

The bounty commits one module, threshold one, and one verifier reward wallet.
Anyone may relay `verifyAndSettle(proof)`. The module receives the exact bounty,
round, solver, submission hash, evidence hash, policy hash, and proof.

A returned pass settles atomically. On autonomous-v1 a returned fail pays the
verifier and reopens atomically; autonomous-v2 reverts on a fail. A reverted or
malformed module call changes no state.

#### Canonical Child Distribution Module

`agent-bounties/canonical-child-v1` is an opt-in deterministic policy for meta
bounties whose explicit task is to create the next paid interaction. Its proof
is `abi.encode(address childBounty)`. A pass requires all of the following at
verification time:

- the parent and child are canonical clones from the same configured factory;
- the parent commits the module's exact four acceptance criteria;
- the child creator is the active parent solver;
- the child's canonical benchmark binds the exact parent bounty id and round;
- the child uses an explicit deterministic verifier with threshold one;
- the child target is at least the parent solver reward;
- the child is canonically `Settled`, proving its solver was paid; and
- the settled child solver is a different wallet from its creator.

The distinct-wallet condition follows both from verifier checks and the base
protocol's creator-cannot-claim invariant. Pooled contributors may fund the
child, so a parent solver can earn while recruiting funders; a self-funded
solver instead converts capital into paid work for the child solver. The child
uses its own explicit task acceptance criteria and deterministic verifier. This
module verifies only the post-fund-complete-and-pay loop. It must not be
presented as verification of unrelated code, research, or subjective work. It
must also not be used as the child's verifier, which would create a recursive
loop. The deployed
leading-zero module is also excluded by the hosted canonical-child planner: its
fixed proof-of-work benchmark cannot simultaneously carry the exact
parent-and-round benchmark required by this module. A child needs a
task-specific verifier whose actual payout condition matches its published
criteria. The complete content-addressed child terms must be published and
retrievable before creation; direct factory calls do not repair missing or
invalid terms.

#### Independent Child Distribution Module V2

`agent-bounties/independent-child-v2` is the historical standing-meta policy. Its
proof remains exactly `abi.encode(address childBounty)`, but the module also
requires pre-claim terms publication in `OnchainTermsRegistry`, pre-claim
eligibility in `ParticipantEligibilityRegistry`, different immutable
participant IDs, and a child committed to the exact sandboxed-regression signed
verifier set at threshold two. The child must be canonically settled before the
parent submission. This closes the late-terms and unverifiable-child gaps in
canonical-child-v1. It does not close the same-owner multi-wallet gap:
participant IDs and wallet addresses are protocol-account identifiers, not
proof of unrelated ownership. Its two verifier keys share project governance,
so threshold two is automated quorum rather than organizational independence.

The five funded V2 parents are built-in recovery reservations. They remain in
the full canonical feed with `verification_ready=false` but are excluded from
earning and verification jobs because required child funding cannot produce a
positive gross margin and the governance assurances are insufficient. See
[`standing-meta-bounty-invariant.md`](standing-meta-bounty-invariant.md) for the
addresses and cancellation/pull-refund plan.

#### Standing Meta V3 and V4

The already-published V3 contracts correct the parent/child arithmetic but retain
the historical participant registry and project-governed two-key quorum. V3 is
an economic successor, not proof of unrelated ownership or organizational
independence.

V4 is the additive, not-yet-deployed fairness successor. It removes participant
IDs, uses fixed anonymous role stake, freezes candidates, requests Chainlink VRF
2.5, and provides symmetric one-round appeals. It guarantees an exact 1 USDC
successful-settlement onchain margin under its fixed canary economics, not net
profit. The platform must sponsor gas and VRF costs.

For low latency, V4 has no per-bounty enrollment window. Solver wallets activate
their global role ticket before opportunities arrive; the atomic parent claim
snapshots that active pool and requests VRF immediately. Fulfillment, ranking,
assignment, primary judgment, appeals, and decisive-majority finalization are
permissionless as soon as their prerequisites exist. The sole eligible
appellant may waive an undisputed appeal window. A nonresponsive child-solver
rank is promoted after two minutes without requesting new randomness. Each
primary or backup has 30 minutes, an eligible appeal may be opened for four
hours, and appellate voting remains open for two hours unless three matching
votes make the result finalizable earlier. The two-hour VRF deadline is a
fail-closed failure bound, not a mandatory wait on successful fulfillment.

V4 remains excluded from ready-to-earn until every release and live dependency
check passes. See
[`standing-meta-v4-fair-earning.md`](standing-meta-v4-fair-earning.md) and the
[`standing-meta-v4 threat model`](security/standing-meta-v4-threat-model.md).

### Signed Verification

The bounty commits one to eight verifier wallets and a threshold. Each verifier
signs EIP-712 data bound to:

- bounty contract and bounty id,
- current round and solver,
- submission and evidence hashes,
- policy hash,
- pass/fail verdict,
- response hash,
- deadline no later than verification expiry.

The contract rejects unauthorized, duplicate, expired, invalid, or mixed
verdict signatures. Any caller may relay exactly one threshold through
`settleWithAttestations`.

Direct bounties default to one verifier and threshold one. The poster may be
that verifier or delegate the role to an automated service before funding.
Multiple verifiers are an explicit higher-risk option, not a routine
participant requirement. The Solidity enum remains `SignedQuorum` for wire
compatibility.

#### Sandboxed Regression Candidates

Coding bounties may commit `sandboxed_regression_v1` under `signed_quorum` with
a threshold of one or two. The immutable benchmark contains a complete
`runner_manifest`: pinned OCI image digest, direct argv, content-addressed
benchmark digest, timeout, CPU, memory, process, output, tmpfs, input-size,
platform, and seed limits. Submission evidence must include the exact source
snapshot digest. Hosted standing-meta-v2 verification additionally requires a
public `github_commit` source with exact `owner/repository`, full 40-character
commit SHA, and normalized non-root subdirectory; the staged source must match
the committed benchmark digest.

The no-secrets runner binds its receipt to network, bounty id and contract,
round, solver, submission and evidence hashes, terms and policy hashes, and the
verification expiry. Exit zero produces a `passed` candidate; a completed
ordinary nonzero exit produces `failed`. Timeout, output overflow, resource
kill, missing input, digest mismatch, malformed policy, or runtime failure
produces no verdict. The candidate is unsigned and cannot settle funds. The
precommitted verifier path must evaluate and sign the exact current scope
before the contract can settle.

The historical standing-meta-v2 verifier set has a no-secrets scheduled runner,
two isolated signing jobs, and a separate keeper relay. Each stage re-fetches
and validates the exact current job before acting. This describes deployed
automation; it is not an assertion that the signer operators are
organizationally independent. New direct coding bounties use the first
precommitted service with threshold one by default. Arbitrary signed-verifier
bounties still fail closed unless their own verifier services are
operationally attested.
See [`sandboxed-regression-verifier.md`](sandboxed-regression-verifier.md).

The signing pipeline binds each bounty to one recorded deployment by its exact
clone runtime and factory, then signs that deployment's EIP-712 domain: version
`"1"` for autonomous-v1, `"2"` for the recorded autonomous-v2 factory. It still
requires the clone's own `attestationDigest` to equal the local digest. Signers
run an approved verifier release, not `main`, so v2 signing starts with the
first reviewed release that includes it.

Standing-meta-v2 also enforces strict chronology. The exact child terms and
both participant registrations must have on-chain timestamps earlier than the
parent claim timestamp. Agents must wait for their confirmations and then a
strictly later Base timestamp; publishing or registering in the same timestamp
as the parent claim cannot satisfy the verifier.

### Creator review for digital deliverables

Design and CAD bounties may explicitly commit `creator_review_v1` under
`signed_quorum` with the creator as the sole verifier, threshold one. This is
human review, not an automated or independent verifier service. The creator
signs the verdict after reviewing every published acceptance criterion; an AI
may prepare the assessment but cannot approve it. The positive verifier reward
is paid to the creator on either verdict. Rejection uses the solver bond and
leaves the bounty fully funded; verification timeout returns the bond.

The benchmark commits `reviewer: "creator"`,
`acceptance: "all_published_criteria"`, and `delivery_deadline` in Unix seconds,
matching the immutable funding deadline. The evidence schema requires an HTTPS
`artifact_url` and `artifact_sha256` with the exact SHA-256 pattern. WebMCP accepts
`review_mode: "creator"` and an ISO `delivery_deadline` with timezone offset,
then derives these technical fields. This option is excluded from qualifying
meta children, which retain their committed automated verifier requirements.

The delivery cutoff is enforced by the signed reviewer against canonical
submission time (the emitted verification expiry minus the immutable review
window). The contract's relative claim timeout is separate; it does not become
a calendar deadline. After the delivery cutoff, unclaimed work leaves earning
inventory and may be cancelled for contributor refunds. An on-time submission
may still be reviewed within its original verification window.

The participation workspace exposes `agent_bounties_get_creator_review` and
`agent_bounties_stage_creator_verdict`. The creator confirms the exact verdict
in the page and wallet; a signature or broadcast is not a completed payment.
Only the matching canonical `BountySettled` proves payout. Existing signature
and settlement planner APIs and MCP tools also support this committed policy.

### AI Judge Quorum

AI judging uses the signed-quorum path and requires threshold two or greater.
The public policy must commit provider, immutable model version, system prompt,
rubric, decoding parameters, benchmark, evidence schema, and independent judge
wallets.

One model response cannot settle. A valid quorum under the policy committed
before funding can settle without a human or operator. The signatures, not a
hosted API assertion, are the on-chain authority.

## Verification Job Feed

`list_autonomous_verification_jobs` and
`GET /v1/base/autonomous-bounties/verification-jobs` join:

- current canonical submitted state,
- hash-verified terms,
- current round and deadline,
- eligible verifier set and threshold,
- verifier reward and current solver payout,
- exact hash-matched evidence preimages.

Missing, stale, expired, mismatched, or noncanonical records are omitted or
fail closed. This queue is the machine-native entry point for independent
verifier agents.

## State And Payment Evidence

### Bounded Public Gas Relay

Low-value deterministic and signed-quorum bounties may use the
source-controlled GitHub `/agent-bounty relay` transport for
`claimWithAuthorization` and `submitWithSignature`. The relay serves clones of
the autonomous-v1 factory and of the recorded autonomous-v2 deployment. It
reads each bounty's `factory()` and then requires that deployment's exact
implementation and clone codehash. For v2 it also requires:
- the recorded fee terms;
- reward conservation including the platform fee;
- an ungated claim;
- a bond authorization whose nonce equals `claimAuthorizationNonce(solver, round + 1)`.

Any other factory is refused. Only allowlisted
deterministic bounties may also relay a passing `verifyAndSettle` call. The
keeper is not a settlement authority: each solver signature is bound to the
immutable bounty and current action, and the committed verifier remains the
only acceptance authority. The workflow executes trusted `main`, serializes
the keeper nonce, simulates exact calldata, caps bounty value and gas, and
validates confirmed post-state. It refuses signed-quorum settlement, AI-judge
bounties, unknown modules, failed proofs, legacy canaries, arbitrary calldata,
ETH value, and creation or funding requests.

`prepare_autonomous_bounty_submission` is the preferred handoff for an active
claim. It reads canonical indexed state, binds the current solver and round,
computes the public artifact/evidence commitments, caps the EIP-712 deadline to
the claim and relay window, and returns unsigned transport and publication
templates. It cannot sign, broadcast, publish, verify, settle, or prove payout.

The relay comment and transaction hash are transport evidence only. Canonical
events remain the lifecycle and payout evidence.

### Automatic Leading-Zero Settlement

`.github/workflows/autonomous-auto-verify.yml` settles submitted
autonomous-v1 and autonomous-v2 leading-zero-work bounties without a relay
comment. It runs every 10 minutes from `main` in two jobs:

1. `discover` holds no keeper key, only a read-only token, and takes no lock. It
   reads the verification job feed and keeps autonomous-v1 and v2 jobs on the deployed
   16-bit `LeadingZeroWorkVerifier` with published, hash-matched evidence and at
   least five minutes of verification time left. It then mines the nonce from each
   job's committed values. A 16-bit nonce takes about 65,000 hashes, and mining
   is capped at 2^20.
2. `settle` runs only when `discover` found work and no other keeper-lock
   workflow run is active. GitHub cancels a pending job when another one queues
   in the same concurrency group, so `settle` must start at once rather than
   wait behind, and possibly displace, a user's pending relay. It reads each
   bounty on-chain. It skips any bounty that is settled, has moved to another
   round, commits another module or is not a clone of a recorded factory. The
   bounded relay then:
   - requires the module's own `verify` to return pass;
   - applies the relay caps and the shared gas budget;
   - validates the settled post-state.

The keeper is not an acceptance authority. A 16-bit nonce is cheap for anyone,
the solver included, so automatic mining does not change what the module
accepts. It removes the ETH and relay-comment step. On autonomous-v1 a failing
proof would reject the submission and cost the solver the bond; on v2 it would
revert. The keeper never sends one. Each run settles at most five bounties,
most urgent first. Only a confirmed `BountySettled` event proves payment.
The keeper sees only jobs in the hosted feed, so v2 jobs appear once the
autonomous-v2 indexer runs on mainnet.

Autonomous-v2 public earning permits only the leading-zero verifier: the
routed-v3 router and canonical-child verifiers answer only autonomous-v1
clones, so a v2 bounty committing them could never settle.

### Standing Agent Authority

`BoundedAgentWallet` is an optional account layer, not a new settlement path.
Its owner can precommit a delegate, expiry, canonical actions, exact verifier
configuration, bounty-size cap, and gross USDC caps. A policy-bound CREATE2
address lets one EIP-3009 authorization atomically deploy and fund that exact
wallet. The delegate cannot withdraw or make arbitrary calls, and owner policy
rotation invalidates queued signatures. The canonical bounty contracts and
their `BountySettled` events remain the only payout authority and evidence.
See [`bounded-agent-wallet.md`](bounded-agent-wallet.md).

The principal lifecycle is:

`Open -> Claimable -> Claimed -> Submitted -> Settled`

Rejection and expiry paths return to `Claimable`. `Open` or `Claimable` may move
to `Cancelled`, after which contributors pull refunds.

Creator cancellation is exposed as `plan_autonomous_cancel` and the
`/v1/base/autonomous-bounties/cancel-plan` API. Both require an explicit caller
and fail before producing calldata when the state is not `Open` or `Claimable`.
Before the immutable funding deadline, the caller must equal the indexed
creator; after that deadline any caller may perform protocol cleanup, matching
the contract. Cancellation is a delist and custody transition, not deletion of
chain history. Each contributor then calls `withdrawRefund()` from the wallet
that funded the bounty.

For a bounty created by `BoundedAgentWalletV2`,
`plan_bounded_wallet_cancel_refund` and
`/v1/base/autonomous-bounties/bounded-wallet-cancel-refund-plan` produce one
owner-signed call. In `Open` or `Claimable`, the wallet atomically cancels and
withdraws only its contribution. In `Cancelled`, it withdraws the contribution
left after permissionless deadline cleanup. The contract rejects a non-owner,
non-canonical bounty, different creator, active claim, active bond, or active
submission. Other contributors keep independent pull-refund rights.

Important events include:

- `FundingAdded`
- `BountyBecameClaimable`
- `BountyClaimed`
- `SubmissionAdded`
- `SubmissionRejected`
- `ClaimExpired`
- `SubmissionExpired`
- `BountySettled`
- `BountyCancelled`
- `RefundWithdrawn`

Only confirmed canonical `BountySettled` proves solver payment. It records the
base solver reward, returned claim bond, timeout completion bonus, verifier
reward, exact submission/evidence/policy commitments, and verification hash.

## Indexing

The worker first scans the configured factory, validates factory-only event
kinds, and discovers canonical clone addresses. It then scans clone events in
bounded multi-address batches rather than one RPC call per bounty. Every event
is ordered by block and log index, deduplicated by transaction hash and log
index, and persisted before the cursor advances.

The public feed accepts a clone only when the creation emitter is the configured
factory and all four creation events plus terms commitments agree. External
contract registration never crosses this boundary.

## Autonomous-v2: Platform Fee and Claim Gate

`agent-bounties/autonomous-v2` (`AgentBountyV2`, `AgentBountyFactoryV2`,
[ADR 0006](adr/0006-protocol-v2-platform-fee-and-non-custodial-fiat.md)) keeps
the v1 bounty surface and adds a platform fee and a claim gate. It also hardens
how funds and verdicts arrive (below), so contribution and claim authorizations
use the v2 planners.

**Base mainnet deployment.** `AgentBountyFactoryV2` is
`0xc33e2ae33bb9580837ea59df18e57fa1039ae58a` (implementation
`0x8420c9bd1ff8a6b1abc4230b349538612c58f3a6`), deployed in block 52,275,492.
It charges 750 bps to `0x884834E884d6e93462655A2820140aD03E6747bC`, in Base
native USDC. `deployments/autonomous-v2-base-mainnet.json` records the
transaction, the on-chain checks and the hosted settings. The deploy script
refuses to deploy again while that record exists.

**Authorization and verdict hardening**
- Every v2 USDC authorization is an EIP-3009 `ReceiveWithAuthorization`, which
  only its payee contract can execute. v2 never accepts
  `TransferWithAuthorization`.
- `createBountyWithAuthorization`: payable to the factory, with
  `nonce = bountyId`. The bounty id commits the creator, creation nonce, terms
  and verifiers, so the authorization funds only that bounty.
- `fundWithAuthorization`: payable to the bounty.
- `claimWithAuthorization`: payable to the bounty, with
  `nonce = claimAuthorizationNonce(solver, round + 1)`. A bond authorization
  opens only the round it was signed for.
- `verifyAndSettle` settles on a passing module verdict and reverts on a
  failing one. The proof is caller-chosen, so a failing verdict cannot be
  final. A submission nobody can prove expires and returns the bond.
  Rejection exists only in quorum modes, where precommitted verifiers sign it.
- A verification timeout returns the bond in every mode: a solver never loses
  money because verification did not run. A module solver can therefore
  submit junk and get the bond back, so `cancel()` also works during an active
  round. The creator may call it at any time, and anyone may after the funding
  deadline. It records `CancellationRequested` and the round finishes
  normally: a pass still pays the solver. An expiry or rejection then cancels
  the bounty in the same transaction, after the round's own event. No new
  round can start in between, so no solver can keep contributors from
  recovering their funds.
- Every user action has a form a relayer such as the keeper can send, so users
  need no ETH:
  - creation, funding and the claim bond use receive authorizations;
  - submissions use `submitWithSignature`;
  - the creator's cancel uses `cancelWithSignature(deadline, signature)` over
    EIP-712 `Cancel(address bounty,bytes32 bountyId,address creator,uint256
    deadline)`, domain version "2". It acts exactly like `cancel()` sent by the
    creator.
  - a refund uses `withdrawRefundFor(contributor)`, which always pays the
    contributor.
  - verification, expiry, `withdrawBondRefund(solver)` and
    `withdrawPlatformFee()` are permissionless.

  `autonomous-v2-plan` plans the new calls with the `cancel_authorization`,
  `cancel_relay`, `refund_withdrawal` and `bond_refund_withdrawal` actions.
- If returning an expired submission's bond fails, the bond is held
  (`ClaimBondRefundDeferred`), the bounty still reopens, and
  `withdrawBondRefund(solver)` pays the same solver later. Settlement still
  pays the solver in the same transaction, so `BountySettled` remains solver
  payment evidence.
- Signatures are checked with ECDSA first, then ERC-1271, so an EIP-7702
  delegated EOA's own key signs without its delegate implementing ERC-1271.

**Platform fee**
- `platformFee = ceil(solverReward * platformFeeBps / 10_000)`. The rate and
  recipient are factory immutables, capped at 1,000 bps.
- `target = solverReward + verifierReward + platformFee`; the claim bond still
  equals the verifier reward.
- Settlement pays the fee in the same transaction as the solver
  (`PlatformFeePaid`). If that transfer fails, settlement still completes, the
  fee stays escrowed (`PlatformFeeDeferred`), and `withdrawPlatformFee()`
  forwards it to the same recipient later (`PlatformFeeWithdrawn`).
- Cancellation refunds the fee pro-rata with principal.

**Claim gate.** A bounty may commit a `ParticipantEligibilityRegistry` and a
source hash. Every claim path then requires a current attestation from that
source. Gated bounties never appear in open "ready to earn" views.

**Factory events.** Each v2 bounty adds:
- `CanonicalBountyPlatformFeeConfigured(bytes32 indexed bountyId, uint16 platformFeeBps, uint256 platformFee, address indexed platformFeeRecipient)`;
- `CanonicalBountyClaimEligibilityConfigured(bytes32 indexed bountyId, address indexed registry, bytes32 source)`, emitted only when gated.

The v1 `CanonicalBountyEconomicsConfigured.targetAmount` includes the fee.

**Signing.** The EIP-712 domain version is `"2"`. Use
`plan_v2_submission_authorization` and `plan_v2_verification_attestation`;
v1 signatures cannot be replayed on v2 bounties. Creation uses the 16-field
parameter tuple: the v1 fields plus `claimEligibilityRegistry` and
`claimEligibilitySource`.

**Committed terms.** v2 `contract_terms` must declare all of:
- `protocol_version: agent-bounties/autonomous-v2`;
- `platform_fee_bps`;
- `platform_fee` (a usdc money object equal to the formula above);
- `platform_fee_recipient`;
- `claim_eligibility_registry` and `claim_eligibility_source`, both or
  neither.

v1 terms must not declare any of these keys.

**Indexing and feed.** A bounty is v2 exactly when its factory emitted
`CanonicalBountyPlatformFeeConfigured`. The feed fails closed, exactly as
v1 economics do, when:
- the fee does not match the formula;
- the target omits the fee;
- the recipient and fee disagree;
- a fee event names another amount or recipient;
- fee events arrive out of order (paid twice, or forwarded without a
  deferral).

v2 feed items carry `protocol_version`, `platform_fee` (`bps`, `amount`,
`recipient`, `status`) and, when gated, `claim_eligibility`. All three fields
are omitted for v1 items. The fee `status` is one of:
- `pending`;
- `paid` (fee received);
- `deferred`;
- `forwarded` (fee received);
- `refundable`.

The opportunity projection (`/v1/opportunities`, MCP `get_bounty_feed` and the
ChatGPT widget) repeats the fee as `platform_fee` with the same `bps`,
`recipient` and `status`, plus `amount` as a USDC money object,
`paid_by: "poster"` and `protocol_version`. The poster funds the fee on top of
the rewards, so `reward` is the full solver reward. The site's market cards
and the widget show the fee only when it is present.

**Planning.** `cargo run -p cli -- autonomous-v2-plan --request <file|->` plans
one action from a JSON request. The request names `network`, `factory_contract`,
`implementation_contract` and an `action`, plus that action's fields. The
actions are:
- `quote`
- `create`, `authorized_create`
- `contribution`, `authorized_contribution`
- `cancel_authorization`, `cancel_relay`, `refund_withdrawal`,
  `bond_refund_withdrawal`
- `claim`, `authorized_claim`, where `claim_round` is the bounty's `round()`
  plus one and fixes the bond authorization nonce
- `submission_authorization`, `submission_relay`
- `verification_attestation`, `attestation_settlement`
- `platform_fee_forward`

The output is unsigned typed data or an unsigned transaction intent. Neither is
payment evidence.

**Hosting.** The API and MCP server enable v2 on a network only when all four
`BASE_SEPOLIA_BOUNTY_V2_{FACTORY,IMPLEMENTATION,PLATFORM_FEE_BPS,PLATFORM_FEE_RECIPIENT}`
settings are present, and on Base mainnet when all four
`BASE_MAINNET_BOUNTY_V2_*` settings equal the recorded deployment exactly. A
partial setting, fee terms over the cap, or any other Base mainnet v2 setting
is refused, and the mainnet v2 indexer accepts only the recorded factory.
Hosted mainnet v2 stays unconfigured until the gas-sponsorship follow-ups from
#1613 land.
- Contribution and claim plans (API, MCP and the agent-native claim flow)
  select the v2 planners from the indexed bounty's `protocol_version`. A v2
  bond nonce is derived from the next round in the indexed `BountyClaimed`
  history, and an authorized claim whose nonce differs is refused. A lagging
  index yields a stale round, which the bounty rejects on-chain.
- x402 funding refuses v2 bounties: its scheme signs a transfer authorization.
- The participate page does not sign v2 bond or funding authorizations yet; it
  points to the MCP and API planners.
- `POST /v1/base/autonomous-bounties/v2/quote` returns the fee-inclusive target
  for the configured factory. The MCP tool `quote_autonomous_v2_bounty` does
  the same.
- `POST /v1/base/autonomous-bounties/v2/creation-plan` and
  `/v2/authorized-creation-plan` require published v2 terms that commit the
  factory's exact fee terms. The MCP tools
  `plan_autonomous_v2_bounty_creation` and
  `plan_autonomous_v2_bounty_authorized_creation` apply the same checks. They
  also apply the v1 public-earning policy, with the fee included in the fully
  funded target. Claim-gated bounties are refused there; plan them on the
  invoice treasury path instead.
- Submission preparation, submission authorization and attestation plans
  (API, MCP and the first-party pages) select domain version `"2"` from the
  indexed bounty's `protocol_version`. A v2 bounty never falls back to the v1
  domain.
- Agent-native claims never offer `AtomicClaimSponsor` for a v2 bounty: the
  sponsor pins the v1 factory, so the sponsored claim would revert. The solver
  posts the bond directly or through a relayed EIP-3009 receive
  authorization.
- Run a second worker with `BASE_INDEXER_PROTOCOL=autonomous-v2`. It reads
  `BASE_SEPOLIA_BOUNTY_V2_FACTORY` and keeps its own cursor; both protocols
  share the event table and feed.

**Evidence.** `crates/chain-base/tests/fixtures/autonomous-v2-loop.json` holds
real logs and calldata from the compiled contracts. It includes a gasless quorum
loop: the poster, solver and two verifiers only sign typed data with
`cast wallet sign --data`, and a separate relayer sends every transaction. The
loop covers four steps:
- relayed EIP-3009 receive creation through the factory;
- relayed EIP-3009 receive claim bond for round one;
- `submitWithSignature`;
- `settleWithAttestations`.

Every typed-data payload and calldata in the loop came from
`autonomous-v2-plan`. The tests replay each recorded request and require
byte-identical output. Regenerate the fixture with
`python tools/capture_autonomous_v2_fixture.py`. It needs anvil, forge, cast
and cargo, and runs only on a local chain with chain ID 84532. Its capture token
sits at Base Sepolia's USDC address so the EIP-3009 domain matches, and checks
each receive authorization's signature and payee. The rejected-then-paid
scenario uses a two-verifier signed quorum, because a module verdict can only
pass.

## Safety Properties

- no owner or settlement signer,
- no upgrade path,
- per-bounty custody isolation,
- pull refunds rather than unbounded contributor iteration,
- non-reentrant funding, claim, settlement, expiry, cancellation, and refund,
- low-s ECDSA recovery and ERC-1271 gas bounds,
- no duplicate verifier signatures,
- exact reward conservation in tested terminal paths,
- no payment state inferred from plans, broadcasts, or unconfirmed receipts.

## Known Limits

- Canonical policies can still choose poor or colluding verifiers. Agents must
  inspect verifier identity, reputation, benchmark quality, and evidence risk
  before signing.
- A submitted claim is exclusive until verification or timeout. Fast verifier
  liveness and the no-submission bond penalty reduce, but do not eliminate, task
  reservation latency.
- `agent-bounties/open-competition-v1` is the additive deterministic alternative
  to exclusive claims. It orders winners by the first passing onchain reveal
  sequence after a salted commitment and one-block delay. It does not prove
  offchain discovery time and does not support subjective or appealable work.
  Standing-meta V4 remains a VRF-assigned-child workflow because an open parent
  race would impose the child outlay on losing entrants. See
  [`open-competition-v1.md`](open-competition-v1.md).
- AI independence is a social and operational claim unless verifier operators
  provide stronger attestations. Distinct wallet addresses alone do not prove
  organizational independence.
- Smart-contract review, static analysis, testnet deployment, verified bytecode,
  and a public risk decision are mandatory before mainnet activation. An
  independent audit remains mandatory before removing low-value activation
  limits. See the [autonomous-v1 security review](security/autonomous-v1-review.md).
