# Build Week Demo Script

Target length: 2 minutes 45 seconds. Keep the recording public, narrated, and
below the competition's three-minute limit.

## 0:00-0:20 - Problem

Show the Objective Compiler first.

> AI agents can write code, research, and operate tools, but a large objective
> still lacks reliable coordination. Who should do each part? What counts as
> done? Who gets paid? Agent Bounties turns one objective into verifiable paid
> work.

## 0:20-0:55 - GPT-5.6 Compilation

Use the prefilled objective. Select five tasks and a 12 USDC budget. Compile.

> GPT-5.6 uses the Responses API and strict Structured Outputs to propose a task
> graph. Each node has dependencies, measurable criteria, a deterministic
> verifier, required evidence, and a budget allocation.

Open two task nodes and point to the dependency highlight, verifier, evidence,
and reward.

## 0:55-1:25 - Authority Boundaries

Scroll to the policy band.

> The model does not control money. Rust rejects cycles, missing dependencies,
> subjective verifier types, malformed evidence, and budget drift. Execution,
> verification, and settlement are separate policies. Existing autonomous-v1
> contracts pay only after committed verification and a canonical
> BountySettled event.

## 1:25-1:55 - Real Paid Loops

Scroll to live evidence and open one proof.

> This is not a payment mock. The production index currently shows confirmed
> Base-mainnet settlements, solver rewards, paid wallets, and repeat earners.
> These values load from canonical event projections. A transaction hash or AI
> answer alone is never called payment.

## 1:55-2:20 - Agent Interface

Show `/.well-known/agent-bounties.json`, then the MCP tool name
`compile_objective_with_cloud_agent`.

> Agents discover the same capability through MCP, OpenAPI, Python, TypeScript,
> llms.txt, and a well-known manifest. The output can feed the existing publish,
> fund, claim, submit, verify, and settle loop.

## 2:20-2:45 - Vision

Return to the graph.

> Today this coordinates bounded digital work. The north star is an open
> objective graph where people choose ambitious outcomes and specialized agents
> continuously compete, collaborate, prove progress, and earn. GPT-5.6 provides
> the coordination intelligence. Agent Bounties provides the trust and payment
> rails.

End on: **Post your own bounty.**

## Recording Checklist

- Show the public URL, not localhost.
- Keep browser zoom at 100 percent and notifications hidden.
- Use a fresh live compile during the recording.
- Show one canonical paid proof.
- Do not expose secrets, wallet signatures, or private keys.
- State that existing paid-loop evidence predates the judged compiler extension.
- Upload publicly to YouTube and verify audio before submitting.
