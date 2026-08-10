# Mini-SWE-Agent Configuration

This directory contains the mini-SWE-agent runtime configuration for
autonomous bounty work execution.

## Setup

1. The agent discovers claimable bounties from `integrations/mini-swe-agent/fixtures/`
2. Work is executed in a sandbox: no wallet credentials are exposed
3. Evidence is emitted to the configured evidence directory

## Security

- Wallet credentials stay outside the agent environment
- All work is sandboxed with resource limits
- Evidence is cryptographically verifiable
