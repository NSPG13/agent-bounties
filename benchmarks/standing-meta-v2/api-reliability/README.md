# Api Reliability Benchmark

This benchmark defines one deterministic child bounty for standing-meta-v2.
The child solver must add:

`scripts/check-agent-bounties-api-reliability.mjs`

The script accepts exactly one argument: a path to a api reliability manifest.
It must use only Node.js built-ins, perform no network access, and
write exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.org/schemas/api-reliability-manifest.v2.json`
- schema: `https://agentbounties.org/schemas/api-reliability-manifest.v2.json`
- network: `base-mainnet`
- chain_id: `8453`
- asset: `USDC`
- token: `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913`
- deployment_status: `active`
- api_base: `https://api.agentbounties.app`
- health_endpoint: `/api/health`
- expected_status: `200`
- max_latency_ms: `3000`
- required_endpoints: `['/api/bounties', '/api/claim', '/api/settlement', '/api/agent/status']`

On success, exit zero and print:

```json
{"ready": true, "schema": "https://agentbounties.org/schemas/api-reliability-manifest.v2.json", "network": "base-mainnet", "chain_id": 8453, "asset": "USDC", "token": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", "deployment_status": "active", "api_base": "https://api.agentbounties.app", "health_endpoint": "/api/health", "expected_status": 200, "max_latency_ms": 3000, "required_endpoints": ["/api/bounties", "/api/claim", "/api/settlement", "/api/agent/status"]}
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
node benchmarks/standing-meta-v2/api-reliability/self-test.mjs
```
