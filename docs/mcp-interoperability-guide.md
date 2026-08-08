# MCP Interoperability Guide

## Overview

This guide explains the MCP (Model Context Protocol) interoperability bounty program and how to participate.

## What is MCP?

Model Context Protocol (MCP) is a standardized protocol for enabling communication between AI systems and external tools, data sources, and services.

## Bounty Structure

### Parent Bounty (#648)

- **Reward:** 1.00 USDC margin (after funding child bounty)
- **Role:** Solver who posts and funds child bounty
- **Requirement:** Must be fully activated with on-chain evidence

### Child Bounty

- **Reward:** 1.00 USDC
- **Role:** Developer who implements MCP interoperability feature
- **Requirement:** Complete coding task to specification

## How to Participate

### As a Solver (Parent Bounty)

1. Wait for issue #648 to show `claimable-live` label
2. Verify all on-chain activation requirements are met
3. Claim the parent bounty
4. Post and fully fund a child bounty with concrete requirements
5. Review and approve completed child bounty work
6. Process canonical settlement
7. Receive 2.00 USDC payout (1.00 USDC net margin)

### As a Developer (Child Bounty)

1. Wait for a child bounty to be posted and funded
2. Review the specific MCP interoperability requirements
3. Claim the child bounty
4. Implement the solution following best practices
5. Submit pull request with comprehensive tests
6. Receive canonical settlement via BountySettled event
7. Collect 1.00 USDC reward

## Technical Requirements

### Code Quality

- Follow existing repository code style
- Include comprehensive test coverage
- Provide clear documentation
- No placeholder comments or TODOs
- All syntax must be valid
- Production-ready implementation

### MCP Integration

- Follow official MCP specification
- Demonstrate cross-system communication
- Handle errors gracefully
- Include usage examples
- Document API patterns used

## Verification

### On-Chain Evidence Required

- Routed-V3 contract on Base mainnet
- USDC funding transactions
- BountyBecameClaimable event
- BountySettled event for payment proof

### Off-Chain Evidence

- GitHub pull request with passing CI
- Code review approval
- Test coverage report
- Documentation updates

## Discovery Feedback

After completing a bounty, please provide feedback:

1. **How did you find the bounty?**
   - Help us understand discovery channels

2. **Why did you attempt it?**
   - Understand motivation and appeal

3. **What single change would make it easier?**
   - Identify friction points in the process

## Important Warnings

- **Do not start work** until bounty shows `claimable-live` label
- **Draft issues are not active bounties**
- **Only BountySettled event proves payment**
- **Transaction hashes alone do not confirm completion**

## Resources

- [MCP Specification](https://modelcontextprotocol.io/)
- [Base Mainnet Explorer](https://basescan.org/)
- [Parent Bounty Issue #648](../issues/648)

## Support

For questions or issues:

1. Check parent issue #648 for updates
2. Review this documentation thoroughly
3. Verify on-chain state before claiming
4. Follow the activation checklist
