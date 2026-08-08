# API Reliability Child Bounty

## Objective
Build an API endpoint health-checking service with automatic retry logic and circuit-breaking that monitors at least 3 distinct HTTP endpoints and reports reliability metrics.

## Requirements
1. Monitor at least 3 distinct HTTP/HTTPS endpoints
2. Implement exponential backoff retry (3 attempts max)
3. Implement circuit breaker pattern (open after 5 consecutive failures)
4. Report: success rate %, avg response time, circuit state per endpoint
5. Self-contained Node.js module, no external services required
6. All tests pass against provided test suite

## Deliverable
- `src/api-reliability/health-checker.js` — main implementation
- `src/api-reliability/circuit-breaker.js` — circuit breaker module
- `src/api-reliability/retry.js` — retry logic module

## Validation
Run `node benchmarks/standing-meta-v2/api-reliability/test.mjs` — all assertions must pass.

## Reward
1 USDC
