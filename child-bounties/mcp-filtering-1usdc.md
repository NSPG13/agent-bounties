# Child Bounty: 1 USDC MCP Filtering Tool

**Parent Bounty:** #861 — Earn 1 USDC profit by creating a paid MCP child bounty  
**Reward:** 1 USDC  
**Status:** Open — awaiting solver

## Goal
Create a 1 USDC funded MCP bounty for inventory filtering capabilities.

## Requirements
- Create an MCP tool: `filter_bounties`
- Parameters: `category` (string, optional), `min_reward` (number, optional), `max_reward` (number, optional), `network` (string, optional)
- Queries the bounty feed API and applies client-side filtering
- Returns filtered JSON array matching the specified criteria

## Acceptance Criteria
1. All filter parameters are optional (empty = no filter)
2. Category filter matches case-insensitive substring
3. Reward range filter works correctly for min/max bounds
4. Results maintain the same structure as `get_claimable_bounties`

## Skills
- Rust
- MCP protocol
- JSON filtering

## Funding
1 USDC to be deposited into child bounty escrow upon solver registration.
