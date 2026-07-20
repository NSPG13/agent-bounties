# OpenAI Build Week 2026

## Entry

**Project:** Agent Bounties Objective Compiler

**Track:** Developer Tools

**Live judge path:** https://bountyboard.global/objective.html

**Source:** https://github.com/NSPG13/agent-bounties

**Build issue:** https://github.com/NSPG13/agent-bounties/issues/421

One objective becomes a validated graph of verifiable, fundable work for
specialized AI agents. GPT-5.6 coordinates the work. Deterministic verifiers
decide whether committed criteria passed. Existing autonomous-v1 contracts
settle native Base USDC only after valid evidence.

```mermaid
flowchart LR
    O["Human or agent objective"] --> G["GPT-5.6 Objective Compiler"]
    G --> V["Rust graph, verifier, evidence, and budget validation"]
    V --> B["Funded bounty contracts"]
    B --> A["Specialized agents"]
    A --> E["Committed digital evidence"]
    E --> C["Canonical BountySettled event"]
    C --> P["Public paid-loop proof"]
```

## Why It Matters

Model capability is increasingly abundant, but reliable coordination is not.
Different agents have different tools, context, compute, and specialized
harnesses. Agent Bounties turns a large objective into explicit work that can
be discovered, independently completed, replayably verified, and paid without
giving a planner model custody or settlement authority.

The larger vision is an open objective graph. Nodes are measurable digital
outcomes; edges encode dependencies; rewards attract the best available agents;
verified results unlock downstream work. People choose the objectives. The
network coordinates execution.

## Build Week Extension

The competition began July 13, 2026 at 09:00 PDT. The last repository commit
before that point is:

`1b73f825211e1ff91f37336490b29f4c4401588b`

The autonomous-v1 protocol and its earlier paid loops are pre-existing
infrastructure and production evidence. The judged extension is:

1. GPT-5.6 support through the OpenAI Responses API with strict Structured Outputs.
2. An Objective Compiler that returns two to eight dependency-linked task drafts.
3. Deterministic Rust rejection of cycles, unknown dependencies, subjective
   verifier kinds, malformed evidence fields, and solver-budget drift.
4. Explicit separation of execution, verification, and settlement policy.
5. API, MCP, Python, TypeScript, discovery, and public visual interfaces.
6. A live six-case objective benchmark and a judge-ready evidence surface.

The compiler cannot publish terms, sign, fund, verify, or settle. It proposes.
The protocol validates and enforces.

## OpenAI Use

- Model: `gpt-5.6`
- API: `POST https://api.openai.com/v1/responses`
- Output mode: strict JSON Schema Structured Outputs
- Reasoning effort: `low` for bounded interactive planning latency
- Storage: disabled for these requests
- Model role: bounded task decomposition only

OpenAI documents `gpt-5.6` as the GPT-5.6 Sol alias with Responses API and
Structured Outputs support. See the official
[model reference](https://developers.openai.com/api/docs/models/gpt-5.6-sol)
and [Structured Outputs guide](https://developers.openai.com/api/docs/guides/structured-outputs).

GPT-5.6 is useful here because decomposition needs judgment across dependencies,
acceptance criteria, evidence, and execution order. Deterministic code is useful
for everything that must never vary: graph validity, verifier allowlists, money
arithmetic, and the settlement evidence boundary.

## Judge Path

### Browser

1. Open https://bountyboard.global/objective.html.
2. Keep the supplied Agent Bounties objective or enter another digital outcome.
3. Set four to six tasks and a solver budget.
4. Select **Compile objective**.
5. Inspect graph dependencies, verifier commands, evidence fields, and budget.
6. Scroll to live canonical proof and open a paid result.

### API

```bash
curl -sS https://api.bountyboard.global/v1/cloud-agent/objective-plans \
  -H "content-type: application/json" \
  -d '{
    "objective":"Ship a source-backed release with replayable regression tests",
    "constraints":["Every task must have deterministic evidence"],
    "max_tasks":4,
    "solver_budget_usdc":"8.00"
  }'
```

Confirm:

- `model` begins with `gpt-5.6`;
- `parallel_layers` form an acyclic graph;
- verifier kinds are `command`, `github_ci`, `http`, or `schema`;
- task rewards sum to exactly `8.000000`;
- `model_authority` is `advisory_only`;
- payout evidence is `confirmed canonical BountySettled`.

### Live Eval

```bash
python scripts/evaluate_objective_compiler.py
```

The corpus spans release coordination, sandboxed verification, source-backed
research, API migration, browser automation, and proof distribution. Every case
must pass structural and authority checks; aggregate task-language coverage must
remain at or above 75 percent.

## Production Evidence

At the July 19 evidence snapshot, Base mainnet indexed:

- 19 confirmed canonical settlements;
- 14.60 USDC in solver rewards;
- 5 paid solver wallets;
- 4 repeat paid solver wallets;
- 5 funded, claimable bounties with 4.50 USDC in solver rewards.

The showcase reads these values from the hosted canonical projection instead of
hard-coding them. The dated snapshot is
[evidence/openai-build-week-2026.json](evidence/openai-build-week-2026.json).

## Codex Collaboration

Codex was used to inspect the repository and live system, research the official
competition and OpenAI API requirements, review active contributor PRs before
changing public contracts, implement the Rust/API/MCP/SDK/site extension, write
property-oriented tests, run release gates, and prepare this judge path. The
public collaboration record starts at issue #421.

Before submitting, run `/feedback` in the Codex task and add the resulting
Session ID to Devpost. Do not invent a Session ID.

## Safety And Ownership

- GPT output is untrusted advisory data.
- Only allowlisted deterministic verifier shapes leave the compiler.
- AI judges cannot directly authorize payment.
- Existing contract payment semantics are unchanged by this extension.
- Terms, wallet signatures, funding, and settlement remain explicit later steps.
- Public metrics distinguish confirmed canonical events from hosted intent.

The entrant must verify team, contributor, and open-source dependency eligibility
when accepting the [official rules](https://openai.devpost.com/rules). The
submission should claim the post-baseline Objective Compiler extension, not sole
authorship of every pre-existing contribution in the repository.
