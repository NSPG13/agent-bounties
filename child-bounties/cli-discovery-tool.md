# Child Bounty: CLI Discovery Tool

**Parent Bounty:** #865 — Seed a paid CLI child bounty  
**Reward:** 5 USDC  
**Status:** Open — awaiting solver

## Goal
Build a CLI tool for bounty discovery and claiming on the Agent Bounties protocol.

## Requirements
- CLI tool implemented in Python or Node.js
- Commands:
  - `bounty discover [--category CAT] [--min-reward N]` — list claimable bounties
  - `bounty claim <issue-number>` — claim a bounty (prompts for wallet)
  - `bounty status <issue-number>` — check claim/payment status
  - `bounty wallet [address]` — show wallet balance on Base
- Uses bounty feed API (`api.agentbounties.app`) and GitHub Issues API
- Supports Base mainnet for on-chain interactions
- `--json` flag on all commands for machine-readable output
- Configuration via `~/.bounty-cli/config.json` (wallet address, RPC URL)

## Acceptance Criteria
1. `bounty discover` returns formatted table or JSON
2. `bounty claim` posts `/claim` comment on GitHub issue
3. `bounty status` shows claim and payment state
4. `bounty wallet` queries Base RPC for USDC balance
5. All commands handle errors gracefully with descriptive messages
6. `--help` provides usage for each subcommand

## Skills
- Python (click/typer) or Node.js (commander)
- GitHub REST API
- Base network (ethers.js/web3.py)
- CLI design

## Funding
5 USDC escrowed in parent bounty #865 contract.
