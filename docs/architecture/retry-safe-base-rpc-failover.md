# ADR: Retry-Safe Base RPC Failover Transport

## Status
Proposed — part of DIRECT bounty #869 implementation.

## Context
The maintainer and inventory automation workflows depend on Base L2 RPC
endpoints that experience transient rate limits (HTTP 429), server errors
(HTTP 5xx), and network timeouts. Without retry logic, a single transient
failure can cascade into stale inventory snapshots and missed bounty state
transitions.

## Decision
Implement a retry-safe RPC failover transport layer with the following
properties:

1. **Ordered endpoint list**: Accept a priority-ordered list of HTTPS Base
   RPC endpoints. The first responsive endpoint is used until it fails.

2. **Chain-ID validation**: Before using any endpoint, validate that
   `eth_chainId` returns `0x2105` (Base mainnet, chain ID 8453). Endpoints
   on wrong chains are silently skipped.

3. **Deterministic backoff**: Retry only bounded transport errors (connection
   refused, DNS failure, TLS handshake timeout), HTTP 429 (rate limit), and
   HTTP 5xx (server errors). Use exponential backoff with jitter: 1s, 2s, 4s,
   8s, max 15s between retries. Maximum 3 retries per endpoint before moving
   to the next.

4. **No retry on execution errors**: Confirmed JSON-RPC execution errors
   (non-zero error codes in the response) are NEVER retried. The caller
   receives the error immediately.

5. **Credential safety**: Logs include endpoint hostname (not full URL),
   HTTP status code, and retry count. API keys and full URLs are never
   written to logs.

## Consequences
- **Positive**: Inventory and routing scripts survive Base RPC rate limits
  without manual intervention.
- **Positive**: Chain-ID validation prevents accidental testnet queries
  against mainnet state contracts.
- **Negative**: Additional latency during failover (up to ~35s worst case
  with 3 endpoints each failing 3 times with 15s max backoff).
- **Negative**: Requires at least 2 Base RPC endpoints for meaningful
  failover; single-endpoint deployments still fail on rate limits.

## Implementation
See `crates/chain-base/src/lib.rs` for the Rust implementation and
`scripts/_shared/rpc.py` for the Python maintainer-script implementation.
