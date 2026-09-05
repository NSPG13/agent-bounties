# Api Reliability Benchmark

This benchmark defines one deterministic child bounty for standing-meta-v2,
bound to parent issue [#647](https://github.com/NSPG13/agent-bounties/issues/647)
([META] Earn 1 USDC margin with a API reliability bounty).

The child solver must add:

`scripts/check-agent-bounties-api-reliability.mjs`

The script accepts exactly one argument: a path to a API reliability manifest.
It must use only Node.js built-ins, perform no network access, and write
exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.org/schemas/api-reliability-manifest.v2.json`
- verify a routed API reliability manifest: HTTPS transport, expected statuses [200,201,204], exponential-backoff retry policy (5s timeout, 3 retries, 30s health check), and the four required reliability metrics.

On success, exit zero and print:

```json
{"ready": true, "protocol": "https", "expected_statuses": [200, 201, 204], "timeout": 5, "retry_strategy": "exponential_backoff", "max_retries": 3, "health_check_interval": 30, "required_metrics": ["latency_p95", "error_rate", "uptime_percentage", "throughput_rps"]}
```

For input errors (missing argument, unreadable file, malformed JSON, non-object
root), exit 2. For validation failures, exit 1 with `{"ready":false,"errors":[...]}`
where every error is one of: schema_mismatch, protocol_mismatch, endpoint_pattern_mismatch, status_code_unknown, timeout_mismatch, retry_strategy_mismatch, max_retries_mismatch, health_check_interval_mismatch, required_metric_missing:latency_p95, required_metric_missing:error_rate, required_metric_missing:uptime_percentage, required_metric_missing:throughput_rps.

## Immutable Runner

- image: `docker.io/library/node@sha256:b74031e546d7f4fafd797ac1b76beccac856a042815ca77db4fd047581605`
- platform: `linux/amd64`
- command: `node /benchmark/test.mjs /workspace`
- network: disabled by the sandbox
- workdir: `/workspace`
- timeout: 30 seconds

## Fixtures

- `fixtures/valid.json` — must pass with exit 0 and `ready: true`
- `fixtures/wrong-protocol.json` — wrong protocol; must fail with exit 1
- `fixtures/missing-field.json` — missing required field; exit 1
- `fixtures/not-an-object.json` — non-object root; exit 2
- `fixtures/malformed.json` — invalid JSON; exit 2
- `fixtures/absent.json` — unreadable path; exit 2

Run the benchmark harness self-test with:

```sh
node benchmarks/standing-meta-v2/api-reliability/self-test.mjs
```
