# Bounty System Documentation

## Overview

This repository uses a two-tier bounty system for API reliability improvements:

- **Parent Bounties**: Funded with 2.01 USDC, these create the framework for child bounties
- **Child Bounties**: Funded with 1.00 USDC by the solver, these are the actual coding tasks

## Earning Flow

1. **Activation**: Parent bounty moves from `pending-activation` to `claimable-live`
2. **Claiming**: Solver posts and funds a 1.00 USDC child bounty
3. **Completion**: A different participant completes the child bounty
4. **Settlement**: Child receives 1.00 USDC, parent receives 2.00 USDC
5. **Margin**: Solver earns 1.00 USDC gross margin (2.00 - 1.00)

## Activation Requirements

A bounty is only active when ALL of the following are verified on Base-mainnet:

- ✓ Routed-V3 parent contract address
- ✓ Full 2.01 USDC funding transaction
- ✓ BountyBecameClaimable event emitted
- ✓ Valid immutable terms
- ✓ Executable routed-V3 child-preparation path
- ✓ Pinned sandboxed_regression_v1 verifier path

## Important Rules

### What Does NOT Count

- Draft issues or labels
- Transaction plans or signatures
- Transaction hashes without confirmation
- Off-chain agreements

### What DOES Count

- Canonical on-chain events on Base-mainnet
- Verified smart contract state
- BountySettled events for payment proof

## Child Bounty Guidelines

### Acceptable Scopes

- Error handling and retry mechanisms
- Rate limiting and backpressure systems
- Circuit breaker implementations
- Health check endpoints
- Graceful degradation patterns
- Timeout and deadline management

### Required Deliverables

1. **Code**: Production-ready, following repository style
2. **Tests**: Comprehensive coverage of new functionality
3. **Documentation**: Clear explanation of reliability improvements
4. **Metrics**: Observability integration where applicable

### Acceptance Criteria

- All existing tests pass
- New tests demonstrate reliability gains
- Code review approval
- No breaking changes
- No performance regressions

## Discovery Feedback

After completing a bounty, please provide:

1. How you discovered the bounty
2. Why you chose to attempt it
3. The single change that would make the loop easier

This feedback helps improve the bounty system for future participants.

## Smart Contract Integration

All bounties are managed through smart contracts on Base-mainnet. Key events:

- `BountyBecameClaimable`: Parent bounty is ready
- `ChildBountyCreated`: Solver has posted child bounty
- `BountySettled`: Payment has been executed

## Gas and Costs

The 1.00 USDC margin is gross profit before:

- Gas fees for contract interactions
- Optional tooling or infrastructure costs
- Time investment

Solvers should calculate net profit based on their specific costs.
