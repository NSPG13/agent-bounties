# Mcp Interoperability Benchmark

This benchmark defines one deterministic child bounty for standing-meta-v2.
The child solver must add:

`scripts/check-agent-bounties-mcp-interop.mjs`

The script accepts exactly one argument: a path to a mcp interoperability manifest.
It must use only Node.js built-ins, perform no network access, and
write exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.org/schemas/mcp-interop-manifest.v2.json`
- schema: `https://agentbounties.org/schemas/mcp-interop-manifest.v2.json`
- network: `base-mainnet`
- chain_id: `8453`
- asset: `USDC`
- token: `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913`
- deployment_status: `active`
- api_base: `https://api.agentbounties.app`
- supported_transports: `['stdio', 'sse', 'streamable-http']`
- mcp_version: `2024-11-05`
- required_tools: `['bounty_search', 'bounty_claim', 'wallet_status', 'settlement_verify']`

On success, exit zero and print:

```json
{"ready": true, "schema": "https://agentbounties.org/schemas/mcp-interop-manifest.v2.json", "network": "base-mainnet", "chain_id": 8453, "asset": "USDC", "token": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", "deployment_status": "active", "api_base": "https://api.agentbounties.app", "supported_transports": ["stdio", "sse", "streamable-http"], "mcp_version": "2024-11-05", "required_tools": ["bounty_search", "bounty_claim", "wallet_status", "settlement_verify"]}
```

For input errors, exit 2. For validation failures, exit 1 with {"ready":false,"errors":[...]}.

## Immutable Runner

- image: `docker.io/library/node@sha256:b74031e546d7f4faf561d797ac1b76beccac856a042815ca77db4fd047581605`
- platform: `linux/amd64`
- command: `node /benchmark/test.mjs /workspace`
- network: disabled by the sandbox
- workdir: `/workspace`
- timeout: 30 seconds

Run the benchmark harness self-test with:

```sh
node benchmarks/standing-meta-v2/mcp-interoperability/self-test.mjs
```
