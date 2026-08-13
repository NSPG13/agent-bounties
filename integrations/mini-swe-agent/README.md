# mini-SWE-agent Paid-Work Environment

A reproducible environment that selects one canonically claimable coding bounty and emits verification-ready evidence without exposing wallet credentials.

## Overview

This integration provides a fixture-driven selector for autonomous bounty selection:

- **Inventory refresh**: Queries the canonical Base feed for claimable bounties
- **Claim planning**: Evaluates multiple opportunities and selects the highest signed gross cash margin (reward minus bond minus external spend)
- **Focused checks**: Runs pre-submission validation before emitting evidence
- **Evidence packaging**: Produces verifiable `source_snapshot_digest` with `discovery_source` provenance
- **Settlement monitoring**: Polls for `BountySettled` events to confirm canonical payment

## Quick Start

The selector reads a canonical inventory snapshot from a JSON fixture and emits one exact next action:

```bash
# Select from a fixture snapshot
python select_bounty.py --input fixtures/multiple.json

# Pin the reference clock for deterministic, reproducible runs
python select_bounty.py --input fixtures/multiple.json --now 2026-08-10T20:00:00Z
```

## Fixture-Driven Selector

The selector processes five canonical inventory snapshots:

| Fixture               | Expected Action | Description                                    |
|-----------------------|----------------|------------------------------------------------|
| `multiple.json`       | `claim`        | Multiple positive-margin bounties → pick highest signed margin |
| `empty.json`          | `wait`         | No bounties → retry after refresh              |
| `stale.json`          | `refresh`      | All entries >24h old → trigger inventory refresh|
| `no-margin.json`      | `skip`         | Signed gross cash margin ≤ 0 → skip           |
| `exclusive-claimant.json` | `skip`     | Already claimed exclusively → skip             |

```bash
python select_bounty.py --input fixtures/multiple.json
# {"action": "claim", "next_action": "Claim bounty #1002: ...", ...}
```

## Safety

- Never exposes wallet credentials, signing keys, or mnemonic phrases
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
