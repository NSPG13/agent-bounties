# Child Bounty: MCP Inventory Tool

**Parent Bounty:** #860 — Seed a paid MCP child bounty  
**Reward:** 2 USDC  
**Status:** Open — awaiting solver

## Goal
Create an MCP tool that queries the live bounty feed and returns formatted results.

## Requirements
- Add an MCP tool to the `plugins/agent-bounties/` directory
- Tool name: `list_claimable_bounties`
- Queries `https://api.agentbounties.app/v1/base/autonomous-bounties/feed?network=base-mainnet&claimable_only=true`
- Returns JSON with: issue number, title, labels, reward amounts, claim status
- Handles API errors gracefully (timeout, invalid response)
- Includes input schema for optional filters (category, min_reward)

## Acceptance Criteria
1. Tool returns valid JSON array of claimable bounties
2. Each item includes: `issue_number`, `title`, `labels`, `reward_usdc`, `is_claimable`
3. API errors produce a descriptive error message (not a crash)
4. Tool works in the MCP sandbox mode

## Skills
- Rust (for crates/mcp-server integration)
- MCP protocol
- REST API consumption

## Funding
2 USDC escrowed in parent bounty #860 contract.
