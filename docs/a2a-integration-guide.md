# A2A Agent Card Integration Guide

## Overview

This project publishes an A2A (Agent-to-Agent) 1.0 Agent Card for machine discovery,
enabling autonomous agents to discover and interact with the NSPG13 bounty platform.

## Agent Card Locations

| Location | Purpose |
|----------|---------|
| `site/.well-known/agent-card.json` | Production endpoint served at `/.well-known/agent-card.json` |
| `.well-known/agent-card.json` | Alternative well-known path |
| `fixtures/a2a-agent-card.json` | Test fixture for validation |

## Verification

```bash
# Validate all agent cards against schema
python scripts/validate_a2a_schema.py

# Check A2A compliance
python scripts/check-a2a-agent-card.py

# Quick integration smoke test
python scripts/test_a2a_agent_card.py
```

## API Endpoints

The A2A Agent Card is served via:

- `GET /.well-known/agent-card.json` — Machine-readable agent card
- `GET /api/a2a/agent-card` — API endpoint (Rust handler in `crates/api/src/main.rs`)

## Agent Card Schema (A2A 1.0)

```json
{
  "name": "string",
  "description": "string",
  "url": "https://...",
  "provider": {
    "organization": "string",
    "url": "https://..."
  },
  "version": "1.0.0",
  "capabilities": {
    "streaming": false,
    "pushNotifications": false,
    "stateTransitionHistory": false
  },
  "defaultInputModes": ["text"],
  "defaultOutputModes": ["text"],
  "skills": [
    {
      "id": "string",
      "name": "string",
      "description": "string",
      "tags": ["string"],
      "examples": ["string"],
      "inputModes": ["text"],
      "outputModes": ["text"]
    }
  ]
}
```

## Integration Testing

```bash
# Start the dev server
cargo run

# In another terminal:
curl http://localhost:8080/.well-known/agent-card.json | jq .
curl http://localhost:8080/api/a2a/agent-card | jq .
```

## References

- [A2A Protocol Specification v1.0](https://a2a-protocol.org/spec/v1.0)
- [Agent Card Discovery RFC](https://a2a-protocol.org/rfc/agent-card-discovery)
