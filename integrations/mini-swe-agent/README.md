# Mini-SWE-Agent Integration

## Overview
Integrates the mini-SWE-agent with the NSPG13 agent-bounties platform for
autonomous bounty discovery, claim management, and paid-work execution in
a reproducible sandboxed environment.

## Architecture
```
mini-swe-agent
  |-- integrations/mini-swe-agent/   # Platform-specific integration
  |     |-- fixtures/                # Test fixtures (claimable, stale, unfunded)
  |-- .mini-swe-agent/              # Agent configuration
  |     |-- hooks/                   # Work hooks (execute, verify)
```

## Fixture Catalog
| Fixture | Purpose | Key Fields |
|---------|---------|------------|
| `claimable.json` | Active bounties ready for claim | `bountyId`, `amount`, `deadline` |
| `stale.json` | Expired bounties (past deadline) | `bountyId`, `deadline` (in past) |
| `unfunded.json` | Bounties without on-chain funding | `bountyId`, `fundingStatus: "unfunded"` |

## Claim Flow
1. Mini-SWE-agent selects one canonically claimable coding bounty
2. Posts `/claim #N wallet: ADDR` on the matching GitHub issue
3. Executes work in a sandboxed environment without exposing wallet credentials
4. Emits verification-ready evidence (test results, diffs, execution traces)
5. Posts evidence as PR comment for maintainer review

## Security
- Wallet credentials are NEVER exposed to the agent environment
- All work execution is sandboxed
- Evidence is cryptographically verifiable
