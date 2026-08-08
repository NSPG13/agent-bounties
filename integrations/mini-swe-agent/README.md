# Mini-SWE-Agent Paid-Work Environment

A reproducible environment for the mini-SWE-agent to autonomously discover, claim, implement, and verify Agent Bounties coding tasks.

## Overview

This integration enables mini-SWE-agent to:
1. **Discover** funded, claimable bounties via the Agent Bounties API
2. **Plan** claims using direct-argv inventory with positive margin filtering
3. **Implement** coding solutions in a sandboxed environment
4. **Package** verification-ready evidence with canonical boundaries
5. **Verify** settlement via `BountySettled` canonical events

## Quick Start

```bash
# Set required environment
export AGENT_BOUNTIES_WALLET=0xYourBaseWallet
export WORKSPACE_ROOT=/workspace

# Run the selector against an inventory
python integrations/mini-swe-agent/select_bounty.py --input integrations/mini-swe-agent/fixtures/multiple.json

# Run the full acceptance check
python benchmarks/direct-growth-v2/mini-swe-agent-environment/check.py
```

## Evidence Format

Every implementation emits a JSON evidence package:

```json
{
  "repository_url": "https://github.com/owner/repo",
  "commit_hash": "abc123...",
  "command_used": "python benchmarks/.../check.py",
  "snapshot_digest": "sha256:...",
  "discovery_source": "https://api.agentbounties.app/v1/base/autonomous-bounties/feed",
  "source_snapshot_digest": "sha256:..."
}
```

## Security

This integration NEVER exposes:
- Private keys
- Seed phrases
- Mnemonics
- Wallet transaction signing

All wallet operations are delegated to the canonical Agent Bounties protocol.

## Canonical Settlement

Only canonical `BountySettled` events on Base mainnet prove payment.
No claim comment, signature, or submission is payment.

## Discovery Source

Bounties are discovered via the canonical feed:
`https://api.agentbounties.app/v1/base/autonomous-bounties/feed`

## Files

| File | Purpose |
|------|---------|
| `config.yaml` | Environment configuration with inventory, claim, evidence, settlement |
| `select_bounty.py` | Direct-argv bounty selector with claim planning |
| `fixtures/*.json` | Test fixtures: multiple, empty, stale, no-margin, exclusive-claimant |
| `README.md` | This documentation |
