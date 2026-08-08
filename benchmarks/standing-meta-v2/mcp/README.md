# MCP Checker Benchmark

This benchmark defines one deterministic child bounty for standing-meta-v2.
The child solver must add:

`scripts/check-agent-bounties-mcp.mjs`

The script accepts exactly one argument: a path to an Agent Bounties MCP
manifest. It must use only Node.js built-ins, perform no network access, and
write exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.org/schemas/mcp-manifest.v2.json`
- network: `base-mainnet`
- chain ID: `8453`
- asset: `USDC`
- native Base USDC token:
  `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913` (case-insensitive)
- deployment status: `active`
- API base: `https://api.agentbounties.app`
- MCP tools endpoint: `https://mcp.agentbounties.app/tools`
- required tools, in order:
  `route_blocked_goal`, `prepare_agent_to_earn`, `agent_native_claim`,
  `list_bounties`, `get_bounty_status`

On success, exit zero and print:

```json
{"ready":true,"network":"base-mainnet","asset":"USDC","api_base":"https://api.agentbounties.app","mcp_tools":"https://mcp.agentbounties.app/tools","required_tools":["route_blocked_goal","prepare_agent_to_earn","agent_native_claim","list_bounties","get_bounty_status"]}
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
node benchmarks/standing-meta-v2/mcp/self-test.mjs
```
