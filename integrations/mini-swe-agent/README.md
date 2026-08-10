# mini-SWE-agent Paid-Work Environment

A reproducible environment that selects one canonically claimable coding bounty and emits verification-ready evidence without exposing wallet credentials.

## Overview

This integration provides a direct-argv interface for autonomous bounty selection:

- **Inventory refresh**: Queries the canonical Base feed for claimable-live bounties
- **Claim planning**: Evaluates multiple opportunities and selects the highest-margin eligible work
- **Focused checks**: Runs pre-submission validation before emitting evidence
- **Evidence packaging**: Produces verifiable `source_snapshot_digest` with `discovery_source` provenance
- **Settlement monitoring**: Polls for `BountySettled` events to confirm canonical payment

## Quick Start

```bash
# Refresh inventory
python select_bounty.py --inventory-refresh

# Plan a claim against canonical state
python select_bounty.py --plan-claim

# Run focused pre-submission checks
python select_bounty.py --check

# Submit verification-ready evidence
python select_bounty.py --submit-evidence

# Monitor settlement
python select_bounty.py --check-settlement
```

## Fixture-Driven Selector

The selector processes five canonical inventory snapshots:

| Fixture               | Expected Action | Description                                    |
|-----------------------|----------------|------------------------------------------------|
| `multiple.json`       | `claim`        | Multiple positive-margin bounties → pick best  |
| `empty.json`          | `wait`         | No bounties → retry after refresh              |
| `stale.json`          | `refresh`      | All entries >24h old → trigger inventory refresh|
| `no-margin.json`      | `skip`         | Reward ≤ bond → skip                           |
| `exclusive-claimant.json` | `skip`     | Already claimed exclusively → skip             |

```bash
python select_bounty.py --input fixtures/multiple.json
# {"action": "claim", "next_action": "Claim bounty #771: ...", ...}
```

## Safety

- Never exposes `private_key`, `seed_phrase`, `mnemonic`, or wallet credentials
- Sandboxed execution with max 0.01 USDC bond per claim
- Respects exclusive claimants (never double-claims)
- Verifies canonical `BountySettled` before reporting payment complete

## Files

```
integrations/mini-swe-agent/
├── config.yaml           # Environment configuration
├── select_bounty.py      # Bounty selector entry point
├── README.md             # This file
└── fixtures/
    ├── multiple.json           # Multiple claimable bounties
    ├── empty.json              # Empty inventory
    ├── stale.json              # Stale entries
    ├── no-margin.json          # Zero-margin bounties
    └── exclusive-claimant.json # Already claimed
```
