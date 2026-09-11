# Shared RPC Transport

Retry-safe Base RPC failover transport for maintainer and inventory automation.

## Features

- Ordered HTTPS Base endpoint list with chain ID 8453 validation
- Bounded retry on transport errors, HTTP 429, and HTTP 5xx
- No retry on confirmed JSON-RPC execution errors
- Credential redaction in logs

## Usage

```python
from scripts._shared.rpc import rpc_failover, select_working_base_rpc

endpoint = select_working_base_rpc()
result = rpc_failover(endpoint, method="eth_chainId", params=[])
```
