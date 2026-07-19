# Hosted Cloud Agent Operations

Agent Bounties can turn an unstructured digital-work objective into measurable
draft bounty terms without running a model on a contributor's computer.

## Runtime Boundary

- `POST /v1/cloud-agent/bounty-drafts` runs in the hosted API service.
- `draft_bounty_with_cloud_agent` in MCP proxies that API; the MCP service does
  not receive the model credential.
- `GET /v1/cloud-agent/readiness` reports the provider, model, public access,
  limits, missing configuration, and the fact that there is no local fallback.
- The website calls the same API from **Draft measurable terms** on
  `post.html`.

Cloud output is untrusted draft data. It cannot publish terms, hold a key,
request a wallet signature, fund a contract, verify a submission, settle a
bounty, or prove payment. The creator must review the draft, select an
executable verifier, and publish and fund through the canonical protocol.

## Provider Configuration

The adapter supports OpenAI Responses, OpenAI-compatible chat completions, and
Anthropic Messages. Production uses GPT-5.6 through the Responses API with a
strict JSON Schema for objective graphs.
Only `agent-bounties-api` receives `CLOUD_AGENT_API_KEY`. The Blueprint declares
it directly on that service with `sync: false`; Render does not support
`sync: false` inside environment groups. The exact-SHA deployment controller
reconciles all nonsecret settings and, when the repository Actions secret is
present, copies the model credential directly to the API service without
including its value in deployment evidence.

Required production settings:

```text
CLOUD_AGENT_ENABLED=true
CLOUD_AGENT_PUBLIC_DRAFTS=true
CLOUD_AGENT_PROVIDER=openai
CLOUD_AGENT_PROTOCOL=openai_responses
CLOUD_AGENT_ENDPOINT=https://api.openai.com/v1/responses
CLOUD_AGENT_MODEL=gpt-5.6
CLOUD_AGENT_API_KEY=<Render secret>
```

Store the same credential as the repository Actions secret
`CLOUD_AGENT_API_KEY`, then dispatch **Render Deploy Recovery**. Existing Render
Blueprints ignore newly added `sync: false` placeholders, so the deployment
controller is the repeatable configuration path. Rotating the Actions secret
and dispatching again rotates the API-service value and forces a redeploy. The
controller then fails closed unless readiness reports hosted execution,
draft-only authority, no local fallback, and `available: true`.

The default public quota is 100 fresh drafts or objective plans per UTC day per
API process. This bounds spend while leaving enough capacity for the six-case
Build Week evaluation and multiple independent judges.
Idempotent retries return the cached draft and do not consume another model
call. Inputs, outputs, timeout, arrays, URLs, and idempotency keys are bounded.

## Failure Policy

- Missing credentials or model configuration: readiness is unavailable and
  drafting returns `503`; manual exact-term entry remains available.
- Invalid objective or invalid model JSON: `400`; no terms are published.
- Daily quota exhausted: `429`; no local process or local model is invoked.
- Provider outage: `503`; no wallet or protocol transition occurs.

No cloud-model response is allowed to authorize settlement. Deterministic
verifier modules or a precommitted verifier quorum remain the only verification
authorities, and only a confirmed canonical `BountySettled` event proves
payment.

## Cloud-Only Availability

Render runs the API, MCP service, Postgres, and Base indexer. GitHub Actions
runs the scheduled inventory, verifier, relay, and deployment-control loops.
GitHub Pages serves `bountyboard.global`. Turning off a maintainer workstation
must not affect any of those paths.

Verify after each production deployment:

```bash
curl https://api.bountyboard.global/health
curl https://api.bountyboard.global/v1/cloud-agent/readiness
curl "https://api.bountyboard.global/v1/base/autonomous-bounties/inventory-summary?network=base-mainnet&claimable_only=true"
curl https://mcp.bountyboard.global/tools
```
