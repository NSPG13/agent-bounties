# Base RPC Failover Configuration Guide

## Quick Start

### Environment Variables
```bash
# Required: comma-separated list of Base RPC endpoints (priority order)
export BASE_RPC_ENDPOINTS="https://mainnet.base.org,https://base.llamarpc.com,https://base-rpc.publicnode.com"

# Optional: max retries per endpoint (default: 3)
export BASE_RPC_MAX_RETRIES=3

# Optional: max backoff seconds (default: 15)
export BASE_RPC_MAX_BACKOFF_SECS=15

# Optional: request timeout seconds (default: 30)
export BASE_RPC_TIMEOUT_SECS=30
```

### Recommended Endpoint Configuration

| Priority | Provider | Endpoint | Latency | Rate Limit |
|----------|----------|----------|---------|------------|
| 1 (Primary) | Base Official | `https://mainnet.base.org` | ~200ms | 10 req/s |
| 2 (Fallback) | LlamaRPC | `https://base.llamarpc.com` | ~150ms | 25 req/s |
| 3 (Fallback) | PublicNode | `https://base-rpc.publicnode.com` | ~300ms | 5 req/s |
| 4 (Fallback) | 1RPC | `https://1rpc.io/base` | ~250ms | 10 req/s |
| 5 (Fallback) | DRPC | `https://base.drpc.org` | ~180ms | Unlimited (free tier) |

### Chain-ID Validation

All endpoints are validated against chain ID 8453 (Base mainnet) before use.
Endpoints returning a different chain ID are skipped. To verify manually:

```bash
curl -s -X POST https://mainnet.base.org \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}'
# Expected: {"jsonrpc":"2.0","id":1,"result":"0x2105"}
```

### Retry Behavior Matrix

| Error Type | Retry? | Backoff | Max Attempts |
|------------|--------|---------|--------------|
| Connection refused | Yes | Exponential + jitter | 3 per endpoint |
| DNS resolution failure | Yes | Exponential + jitter | 3 per endpoint |
| TLS handshake timeout | Yes | Exponential + jitter | 3 per endpoint |
| HTTP 429 (rate limit) | Yes | Exponential + jitter | 3 per endpoint |
| HTTP 5xx (server error) | Yes | Exponential + jitter | 3 per endpoint |
| HTTP 4xx (client error) | No | N/A | 1 |
| JSON-RPC error (non-zero) | No | N/A | 1 |
| Chain ID mismatch | No (skip endpoint) | N/A | 1 |

### Monitoring

Check the logs for failover events:

```bash
# Count failover events in the last hour
grep "RPC failover" /var/log/agent-bounties/rpc.log | \
  awk '{print $1, $2}' | sort | uniq -c
```

### Troubleshooting

**Symptom**: All endpoints exhausted, inventory stale  
**Cause**: All configured endpoints are rate-limited or unreachable  
**Fix**: Add more endpoints to `BASE_RPC_ENDPOINTS` or increase `BASE_RPC_MAX_BACKOFF_SECS`

**Symptom**: Chain-ID validation rejects all endpoints  
**Cause**: Endpoints point to wrong network (testnet, mainnet fork)  
**Fix**: Verify each endpoint returns `0x2105` for `eth_chainId`
