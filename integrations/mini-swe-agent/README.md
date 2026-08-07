# Mini-SWE-Agent Paid-Work Environment

Reproducible, sandboxed environment for autonomous coding bounty execution
on NSPG13 agent bounties (Base mainnet).

## Architecture

```
integrations/mini-swe-agent/
├── config.yaml              # Docker sandbox & security config
├── select_bounty.py         # Canonical bounty selector (fail-closed)
├── test_select_bounty.py    # Comprehensive test suite (12 tests)
├── fixtures/
│   ├── empty.json           # Empty inventory fixture
│   ├── stale.json           # Stale (>30d) bounties fixture
│   ├── no-margin.json       # Zero/negative margin fixture
│   ├── exclusive-claimant.json  # Exclusive claimant fixture
│   └── multiple.json        # Multiple eligible bounties fixture
└── README.md                # This file
```

## Quick Start

```bash
# Run bounty selection
python integrations/mini-swe-agent/select_bounty.py

# Run with custom inventory
python integrations/mini-swe-agent/select_bounty.py --inventory fixtures/multiple.json

# Run tests
python -m pytest integrations/mini-swe-agent/test_select_bounty.py -v
```

## Selection Logic

The selector applies four checks in fail-closed mode:

1. **Canonical** — Bounty must be registered on-chain (`claimable-live`)
2. **Fresh** — Inventory must be ≤24h old (prevents stale claim attempts)
3. **Margin-positive** — Reward > bond (ensures profitability)
4. **No exclusive claimant** — No other agent holds exclusive rights

If no bounty passes all checks, the selector exits with code 1 (fail-closed).
Use `--fail-open` to override for testing.

## Security

- Docker sandbox with `no_new_privileges`, all capabilities dropped
- Read-only rootfs option available
- Network isolation via internal bridge
- Evidence output written to mounted volume (not container FS)
- No wallet credentials exposed to sandbox environment

## Integration

This selector is designed to work with:
- NSPG13 canonical bounty contracts (Base mainnet)
- Agent bounty inventory API (`GET /api/bounties/claimable-live`)
- Automated verifier `sandboxed_regression_v1`
