# Distribution attribution

This subsystem measures whether an approved discovery rail leads to an externally
funded, canonically settled autonomous bounty. Attribution is analytics-only. It
cannot hold a wallet key, authorize a signature, verify a solution, or move funds.

## Attributed MCP routes

Every approved route delegates to the same canonical MCP implementation:

`/r/{rail}/mcp`

Approved rail slugs are `bankr`, `openclaw`, `vscode`, `cursor`, `cline`, `github`,
`linear`, `claude-custom`, `chatgpt-dev`, `glama`, `mcp-so`, and `mcpservers`.
Unknown rails return 404. Attributed routes fail closed with 503 unless both a
durable database and `DISTRIBUTION_ATTRIBUTION_SIGNING_SECRET` are configured. The
same generated secret, containing at least 32 bytes, must be provided to the MCP
and API services.

On the first MCP POST, the service returns
`x-agent-bounties-acquisition-id`. The value matches
`^aba1_[0-9a-f]{64}\.[0-9a-f]{64}$`: a random opaque nonce and HMAC-SHA256
signature. Clients should echo the exact value on subsequent POSTs. Only its
SHA-256 hash is stored. Reuse through another attributed route preserves the
immutable first touch and records the later rail separately as an assist.

The response also returns `x-agent-bounties-attribution-rail` and
`x-agent-bounties-first-touch-rail`, plus
`x-agent-bounties-measurement-eligible`. The same evidence is included in MCP
result metadata under `agentbounties.app/acquisition`. MCP `initialize` and `tools/list`
are safe probes: they write analytics observations only and do not prepare a
draft, start wallet review, sign, fund, or invoke any mutation tool.

Safe probes send `x-agent-bounties-canary: dry-run-v1`; a mainnet lifecycle
canary sends `x-agent-bounties-canary: mainnet-v1`. Unknown values are rejected.
The first request receives a server-signed acquisition token bound immutably to
that classification. Every reuse must present the same canary kind; adding,
removing, or changing the classification returns a conflict without mutating the
original acquisition. This keeps the canary workflow secret-free while preventing
an existing measurement-eligible acquisition from being reclassified.
The explicit bounded classification is retained on the acquisition, becomes
irreversibly measurement-ineligible, and excludes that acquisition and its
assists, rail usage, handoffs, failures, and lifecycle outcomes from every
marketing funnel aggregate. It grants no authority. Mainnet canary wallets should
also be classified `synthetic_canary` as defense in depth.

## Draft-to-lifecycle join

An attributed `prepare_bounty_post` result reserves one retry-safe handoff and
adds `acquisition` and `handoff` to its first-party review URL. When the reviewed
terms are published or wallet review begins, the browser or client sends both of
these headers, or neither:

- `x-agent-bounties-acquisition-id`
- `x-agent-bounties-handoff-id`

Immediately before opening wallet discovery or its funding modal, the first-party browser calls
`POST /v1/distribution/handoffs/wallet-review` with the same headers and no body.
The idempotent endpoint stores the first review-boundary timestamp. An attributed
flow fails closed if this acknowledgement cannot be persisted. Wallet UI and
transaction signing remain entirely on the first-party browser surface; the
analytics endpoint receives no wallet, terms, prompt, or task payload.
After wallet connection supplies the creator address, and before any funding
signature, `POST /v1/base/autonomous-bounties/terms` verifies the acquisition
signature and durably binds the handoff to the immutable terms hash and creator
wallet. Replays of the same binding are safe; attempts to change the terms or
creator conflict.

## Reports and evidence boundaries

`GET /v1/operator/distribution/report` requires the existing operator token and
returns one cumulative, event-driven funnel per approved rail: acquisitions,
assists, MCP requests and failures, prepared handoffs, bound terms, externally
funded bounties and posters, claims, submissions, canonical settlements, funding,
and settled GMV. Wallet-review counts come from the durable review-boundary
acknowledgement. Handoff failures are unique acquisition/request-digest/failure
code signals; replays increment an observation count without duplicating the
failure total, and no prompt or task body is stored. `attribution_coverage_ready`
means only that coverage is at least 95%; it is not approval to activate or scale
paid distribution.

The current report scope is only `agent-bounties/autonomous-v1`, not legacy bounty
records or either Open Competition protocol. Funding requires confirmed canonical
`FundingAdded` plus `BountyBecameClaimable`, and the sum of non-excluded confirmed
funding must meet the positive canonical target amount. An excluded subsidy plus
a token external contribution is therefore not an external funded conversion.
Claims, submissions, and settlements require their respective confirmed canonical
events. A settlement with verifier evidence additionally requires a hash-matched
published submission-evidence record. `verified_useful_settlements` returns
`null`: settlement and verifier evidence do not prove completion of an originating
GitHub or Linear issue, and origin-completion evidence is not yet persisted in
this shared model.

Wallets classified as `maintainer`, `operator`, `test`, `synthetic_canary`,
`sponsored`, `circular_funding`, `related_party`, or
`operator_funded_development` are excluded according to
`DISTRIBUTION_EXCLUDED_WALLET_CLASSES`. The API refuses to start unless the
configured set contains every required class; omitting the variable selects all
classes. The legacy input aliases `team`, `circular`, and `related` normalize to
their canonical classes.
Operators configure individual classifications with
`PUT /v1/operator/distribution/wallet-exclusions`. These classifications affect
analytics only.

`GET /v1/distribution/summary` is public and contains aggregates only. Per-rail
outcomes are withheld until that rail has at least three unique external funded
poster wallets; overall figures are likewise withheld below the minimum sample.
Raw acquisition identifiers and wallets are never returned publicly.

## Canary

After deploying both services and the migration, run:

```bash
python scripts/check-distribution-rail-mcp.py \
  --endpoint https://mcp.agentbounties.app \
  --repetitions 3 --canary-kind dry-run-v1 --json
```

The matrix performs at least three repetitions across all 12 routes with
`initialize` and `tools/list`, emits machine-readable evidence, verifies the
signed retry-stable acquisition header and canonical `prepare_bounty_post` catalog
entry, and does not invoke a draft, wallet, or payment action. A paid rail should
not activate until its own route passes this probe and the wider exclusion,
verification, incident, and approval gates are satisfied.
