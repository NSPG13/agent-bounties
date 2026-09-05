# Glama Agent Bounties onboarding audit

- Started: 2026-09-05T04:00:00Z
- Completed: 2026-09-05T04:20:00Z
- First-touch rail: glama
- MCP protocol version: 2025-06-18
- Install guide: https://agentbounties.app/install/glama/
- Attributed MCP: https://mcp.agentbounties.app/r/glama/mcp

The redacted `initialize` and `tools/list` captures confirm that
`prepare_bounty_post` is discoverable. The acquisition identifier is redacted.
The agent received no private key, no seed phrase, no wallet signature, and no
payout authority. Human wallet review occurred only on agentbounties.app.

## Canonical lifecycle

- CanonicalBountyCreated: https://basescan.org/tx/0x1111111111111111111111111111111111111111111111111111111111111111
- FundingAdded: https://basescan.org/tx/0x2222222222222222222222222222222222222222222222222222222222222222
- Verifier evidence: https://example.com/glama-canary/verifier-evidence.json
- BountySettled: https://basescan.org/tx/0x3333333333333333333333333333333333333333333333333333333333333333

This fixture is synthetic self-test data. It is not a real bounty, transaction,
verification, settlement, payment, or marketing conversion.

