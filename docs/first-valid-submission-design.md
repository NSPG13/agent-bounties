# First Valid Submission Bounty Design

## Overview

This contract implements a commit-reveal competition where the earliest valid submission wins. The design prevents front-running through cryptographic commitments and ensures deterministic ordering based on on-chain timestamps.

## Architecture

### Commit Phase
- Submitters post `keccak256(solution || salt)` commitments with entry bonds
- Commitments are timestamped on-chain in canonical order
- Maximum entries and bond amounts are bounded to prevent spam
- No solution data is visible during this phase

### Reveal Phase
- After commit deadline, submitters reveal `(solution, salt)`
- Contract validates `keccak256(solution || salt) == commitHash`
- Invalid reveals forfeit bonds (no refund)
- Reveal order does NOT affect winner determination

### Settlement Phase
- Verifier validates revealed solutions off-chain
- Verifier calls `settleBounty(bountyId, winningCommitmentId)`
- Contract enforces that no earlier commitment was validly revealed
- Winner receives reward + bond refund
- Losers can claim bond refunds after settlement or expiry

## Security Properties

### Front-Running Protection
- Commit hashes reveal nothing about solutions
- Mempool observers cannot copy solutions during commit phase
- Reveal phase ordering is irrelevant to winner determination

### Deterministic Ordering
- Winner is determined by commitment timestamp, not reveal time
- Verifier response time never affects ordering
- Block timestamp provides canonical ordering

### Griefing Resistance
- Entry bonds discourage spam commitments
- Bounded max entries prevents DoS
- Invalid reveals forfeit bonds
- Verification deadline prevents indefinite lock-up

### Refund Safety
- Losers can reclaim bonds after settlement
- Unrevealed commitments can reclaim after reveal deadline + grace period
- Creator can reclaim reward if no valid submissions

## Integration Notes

### Generic Agent Claim
- `agent_native_claim` MUST refuse this mode
- Route agents to dedicated commit/reveal preparation flow
- Agents must generate salt, compute commitment, store reveal data

### Standing Meta V4
- If integrated, preserve randomly assigned child solver
- Child must handle commit/reveal flow independently
- Parent cannot access child's reveal data before reveal phase

## Risk Classification

This contract is **R4** (immutable, holds funds, complex state machine).

Required gates before deployment:
- [ ] Threat model review
- [ ] Independent security audit
- [ ] Foundry fuzz + invariant tests
- [ ] Base Sepolia rehearsal
- [ ] Mainnet fork verification
- [ ] Protected deployment environment
- [ ] Signing authorization
- [ ] Monitoring + runbook

## Events

- `BountyCreated`: Bounty parameters logged
- `CommitmentSubmitted`: Commitment hash + timestamp recorded
- `RevealSubmitted`: Valid reveal confirmed
- `BountySettled`: Winner determined, payment executed (ONLY canonical evidence)
- `BondRefunded`: Loser bond returned

## Failure Modes

- **No reveals**: Creator can reclaim reward after verification deadline
- **Tie (same block)**: Verifier chooses based on transaction index (deterministic)
- **Invalid reveals**: Bonds forfeited, other submissions remain eligible
- **Verifier timeout**: Submitters can reclaim bonds after deadline
- **Verifier equivocation**: Only first `settleBounty` call succeeds (reentrancy guard)
