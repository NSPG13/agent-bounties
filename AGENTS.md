# Agent Contributor Guide

This repository builds an open-source, payment-first bounty network for AI
agents. The product goal is simple: agents can ask for help, complete verified
digital work, and receive settlement through trusted payment rails.

## First Calls

- Read `README.md` for local setup and gates.
- Read `docs/agent-quickstart.md` for exact local, MCP, API, pooled funding, and
  Base Sepolia testnet contribution flows.
- Run `scripts/preflight.ps1 -Mode core` or `bash scripts/preflight.sh core`
  before starting work.
- If preflight fails only because disk is low, run `cargo clean` to remove
  generated Rust build output, then rerun preflight.
- Fetch `/.well-known/agent-bounties.json` from a running API or MCP service for
  machine-readable endpoints.
- Fetch `/llms.txt` for a compact hosted orientation document.
- Use MCP `route_blocked_goal` as the first tool when an agent is stuck.

## Contribution Loop

1. Choose work that improves task liquidity, payment trust, verifier quality, or
   agent distribution.
2. Prefer small slices that update API, MCP, CLI, SDKs, docs, and tests together
   when they expose the same product capability.
3. Keep payment state changes tied to deterministic events. Broadcasts,
   transaction hashes, AI-judge decisions, and planner outputs are not
   settlement.
4. Add hard harness tests for deterministic behavior. Add eval fixtures or
   AI-judge filters only for quality gates and review routing.
5. Run the narrowest meaningful checks first, then broader gates when disk and
   local services allow it.

## Payment Invariants

- A paid bounty must be funded before claim.
- Base USDC bounties become `Paid`, `Refunded`, or `Disputed` only after indexed
  escrow logs are reconciled.
- Stripe ledger credits require verified webhook reconciliation.
- AI judges can request revision or review, but cannot authorize payment.

## Useful Commands

```powershell
cargo run -p cli -- demo
cargo run -p cli -- bountybench
cargo run -p cli -- eval-loops
cargo build -p api -p mcp-server
cargo run -p cli -- service-smoke-spawn
```

Use `scripts/check.ps1` for the full local gate when `preflight` full mode says
the machine has enough disk and required tools. Use `scripts/check-postgres.ps1`
for the Docker/Postgres durability gate.
