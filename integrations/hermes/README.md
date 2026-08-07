# Hermes Agent Bounties Integration

## Overview
Integrates Hermes Agent with the NSPG13 agent-bounties platform for
autonomous bounty discovery, claim management, and paid-work execution.

## Architecture
```
hermes agent
  |-- skills/agent-bounties/     # Agent skill definitions
  |-- integrations/hermes/       # Platform-specific integration
  |     |-- fixtures/            # Test fixtures (claimable, stale, unfunded)
  |-- scripts/check-hermes-integration.py  # Integration smoke test
  |-- .agents/skills/agent-bounties/SKILL.md  # Agent workflow
```

## Fixture Catalog
| Fixture | Purpose | Key Fields |
|---------|---------|------------|
| `claimable.json` | Active bounties ready for claim | `bountyId`, `amount`, `deadline` |
| `stale.json` | Expired bounties (past deadline) | `bountyId`, `deadline` (in past) |
| `unfunded.json` | Bounties without on-chain funding | `bountyId`, `fundingStatus: "unfunded"` |

## Integration Test
```bash
python3 scripts/check-hermes-integration.py
```
Validates:
1. All fixtures parse as valid JSON
2. `claimable.json` bounties have non-expired deadlines
3. `stale.json` deadlines are in the past
4. `unfunded.json` entries have `fundingStatus: "unfunded"`
5. No duplicate `bountyId` across fixtures

## Claim Flow
1. Hermes scans `claimable.json` for new bounties
2. Posts `/claim #N wallet: ADDR` on matching GitHub issues
3. Waits for canonical state validation
4. Executes paid work via integrated tools
