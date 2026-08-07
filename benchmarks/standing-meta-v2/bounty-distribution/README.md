# Bounty Distribution Checker Benchmark

This benchmark defines one deterministic child bounty for standing-meta-v2.
The child solver must add:

`scripts/check-agent-bounties-distribution.mjs`

The script accepts exactly one argument: a path to an Agent Bounties distribution
manifest. It must use only Node.js built-ins, perform no network access, and
write exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.org/schemas/bounty-distribution-manifest.v2.json`
- network: `base-mainnet`
- chain ID: `8453`
- native USDC token: `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913`
- distribution settlement endpoint: `https://api.agentbounties.app/v1/base/distribution/settle`
- distribution status endpoint: `https://api.agentbounties.app/v1/base/distribution/status`
- required distribution phases: `funding_locked`, `terms_published`, `claimable`, `settled`
- minimum child bounty target: `1.00 USDC`

On success, exit zero and print:

```json
{"ready":true,"network":"base-mainnet","usdc_token":"0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913","settle_endpoint":"https://api.agentbounties.app/v1/base/distribution/settle","status_endpoint":"https://api.agentbounties.app/v1/base/distribution/status","required_phases":["funding_locked","terms_published","claimable","settled"],"min_target":"1.00 USDC"}
```

## Error Handling

Exit 2 for input problems (missing argument, unreadable file, invalid JSON).
Exit 1 for validation failures (wrong schema, missing phases, protocol mismatch).
All errors must output `{"ready":false,"errors":["error_code"]}`.

## Parent Bounty

This child bounty is bound to parent #651 (bounty distribution META).
Completion pays 1 USDC to the child solver, producing 1 USDC gross margin for the parent.
