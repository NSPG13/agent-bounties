# mini-SWE-agent Configuration Guide

## Quick Start
```bash
# Validate all fixtures
python3 integrations/mini-swe-agent/test_fixtures.py

# Run the bounty selection logic
python3 integrations/mini-swe-agent/select_bounty.py
```

## Configuration File: config.yaml
```yaml
# Agent identity
agent:
  id: "mini-swe-agent-001"
  wallet: "0x780B5ea2B039DAcC08C6334fF613def2c18a5Ee9"
  
# Bounty selection strategy
selection:
  strategy: "highest_value_first"  # or "earliest_deadline", "skill_match"
  max_concurrent: 1
  min_margin_usdc: 1.0
  
# Execution sandbox
sandbox:
  image: "python:3.11-slim"
  timeout_minutes: 30
  max_disk_mb: 512
  
# Claim rules
claim:
  wallet: "0x780B5ea2B039DAcC08C6334fF613def2c18a5Ee9"
  auto_claim: true
  max_retries: 3
  retry_delay_seconds: 60
```

## Fixture Reference
| Fixture | Scenario |
|---------|----------|
| `empty.json` | No bounties available |
| `exclusive-claimant.json` | Single-agent exclusive claim |
| `exclusive-multi.json` | Multi-agent race, first valid claim wins |
| `multiple.json` | Multiple bounties, selection strategy picks best |
| `no-margin.json` | Bounty below minimum margin threshold |
| `stale.json` | Bounty past deadline |
| `stale-claimant.json` | Agent claimed but didn't submit work |

## Troubleshooting
- **"No selectable bounties"**: Check `min_margin_usdc` threshold vs available bounties
- **"Claim rejected"**: Verify agent card exists and skills match
- **"Canonical state unavailable"**: Wait for protocol state transition (~1 hour)
