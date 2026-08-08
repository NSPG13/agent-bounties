# API Reliability Checker Benchmark

This benchmark defines one deterministic child bounty for standing-meta-v2.
The child solver must add:

`scripts/check-agent-bounties-api-reliability.mjs`

The script accepts exactly one argument: a path to an API reliability manifest
JSON file. It must use only Node.js built-ins, perform no network access, and
write exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.org/schemas/api-reliability-manifest.v2.json`
- protocol: `https`
- endpoint base URL pattern: `^https://api\\.agentbounties\\.org/v[0-9]+/.*`
- expected status codes: `[200, 201, 204]`
- timeout (seconds): `5`
- retry strategy: `exponential_backoff`
- max retries: `3`
- health check interval (seconds): `30`
- required metrics, in order:
  `latency_p95`, `error_rate`, `uptime_percentage`, `throughput_rps`

On success, exit zero and print:

```json
{"ready":true,"protocol":"https","expected_statuses":[200,201,204],"timeout":5,"retry_strategy":"exponential_backoff","max_retries":3,"health_check_interval":30,"required_metrics":["latency_p95","error_rate","uptime_percentage","throughput_rps"]}
```

For input errors, exit 2. For validation failures, exit 1 with `{"ready":false,"errors":[...]}`.

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

## Child Coordination

- Parent issue: NSPG13/agent-bounties#647
- Child reward: 1.00 USDC
- Parent reward: 2.00 USDC
- Net margin: 1.00 USDC
- Verifier: sandboxed_regression_v1 threshold-two quorum
