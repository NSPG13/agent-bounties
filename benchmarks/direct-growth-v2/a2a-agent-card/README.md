# A2A Agent Card — Agent Bounties

This directory contains the **A2A (Agent-to-Agent) Agent Card** for the Agent Bounties platform.

## What is an A2A Agent Card?

An A2A Agent Card is a standardized JSON document that describes an AI agent's capabilities, skills, and interface. It enables **machine discovery** — allowing other AI agents and automated systems to discover, understand, and interact with this agent programmatically.

The card is published at a well-known path (`benchmarks/direct-growth-v2/a2a-agent-card/agent-card.json`) following the A2A protocol specification.

## Agent Card Contents

- **name**: Agent Bounties Agent
- **version**: 1.0.0
- **capabilities**: Streaming, state transition history
- **skills**:
  - Bounty Discovery — scan and find available bounties
  - Bounty Claim — automate claiming and submission
  - Bounty Status Tracking — track claim progress and rewards

## Related

- Bounty: [NSPG13/agent-bounties #862](https://github.com/NSPG13/agent-bounties/issues/862)
- A2A Protocol: [google/A2A](https://github.com/google/A2A)
