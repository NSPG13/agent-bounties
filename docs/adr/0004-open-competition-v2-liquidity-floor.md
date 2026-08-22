# ADR 0004: private competitive-liquidity floor with bounded replenishment

- Status: proposed, implementation starts disabled
- Date: 2026-08-21
- Change class: R4 (new value-bearing reserve contract and delegate policy)
- North star: rolling 28-day canonical marketplace GMV

## Decision

Operate a private Open Competition V2 liquidity floor of five and a target of
ten. The public product remains one marketplace: public pages, aggregate API
responses, MCP/CLI orientation, reports, and investor-facing material expose
only unified funded opportunities, available funding, and canonical GMV.

Every five minutes a read-only guard obtains safe-block evidence and compares
the internal qualifying V2 inventory with the policy. When inventory is below
ten, a deterministic planner may select enough reviewed candidates to restore
the complete deficit. It never partially fills a batch that cannot reach the
target. Missing, stale, future-dated, malformed, conflicting, or release-drifted
evidence blocks the plan.

Reviewed candidate specifications are public and content-addressable. The
50/30/20 user-evidence, GMV-impact, and confidence ranking is private and comes
from the isolated signer state service. Private comments and operational scores
must never be committed, logged, uploaded as workflow artifacts, or returned by
public APIs.

The GitHub workflow contains no spending key. A separately isolated delegate
submits calls, but never owns the reserve. The bounded on-chain wallet is owned
by the operator funding address and independently enforces exact 3.04-USDC
creations, a 30.40-USDC UTC-day cap, a 77.668098-USDC lifetime cap, the reviewed
factory, deterministic candidate commitments and nonces, exact allowance
consumption, revocation, and owner-only recovery. A planned or broadcast
execution blocks new planning until the delegate reconciles it to canonical
activation or rejection.

The owner can revoke the delegate and recover all uncommitted USDC without the
delegate's cooperation. Active escrow is not clawbackable while competitors
rely on its proof window; creator refunds become recoverable after canonical
cancellation, verifier unavailability, or expiry. Two-step ownership transfer
prevents a mistyped address from immediately receiving recovery control.

## Why

An empty demand side prevents solvers from transacting and suppresses GMV. A
ten-to-five buffer tolerates five close exits while externally posted and funded
demand grows. The operator-funded buffer is a bridge, not a target GMV source;
success is increasing non-operator funding and repeat posting.

## Consequences

- The public marketplace stays legible and mechanism-neutral.
- Replenishment is deterministic and fail-closed, but cannot run until the
  bounded reserve, isolated delegate, private ranking, and durable ledger are
  provisioned.
- A signer outage produces an internal incident and no spending.
- Reserve-funded inventory can preserve transaction availability, but cannot by
  itself prove acquisition, retention, or durable GMV growth.
- Candidate specifications expire and require another evidence-backed review.

## Rejected alternatives

- Publishing V2-specific inventory or GMV: creates investor confusion and
  exposes an internal mechanism distinction.
- Transferring reserve USDC to an ordinary delegate wallet: makes recovery
  depend on that delegate's key and grants arbitrary-transfer authority.
- Keeping a reusable private key in GitHub Actions: expands custody and secret
  exposure beyond the required authority.
- Counting broadcasts or transaction hashes: violates canonical evidence rules.
- Letting AI authorize a transfer or settlement: advisory ranking is not payment
  authority.
- Planning another batch around pending broadcasts: can overshoot the target
  after concurrent activation.

## Completion gate

This ADR may move to accepted only after focused tests pass, the reserve threat
model receives independent security review and explicit maintainer risk
approval, exact deployment bytecode is pinned, a Base Sepolia crash/concurrency
and recovery rehearsal succeeds, an internal canary canonically activates, and
rollback and revocation owners are named. Mainnet execution remains disabled
until then.
