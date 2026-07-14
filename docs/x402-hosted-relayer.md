# Hosted x402 Relayer

The hosted relayer removes the gas and transaction-broadcast steps from Base
USDC bounty funding. The funder's wallet still signs the exact EIP-3009 amount,
bounty, chain, nonce, and expiry. The canonical bounty contract pulls USDC
directly from that wallet.

## Enable

Set these only on the API service:

```text
ENABLE_X402_HOSTED_RELAY=true
X402_RELAYER_PRIVATE_KEY=<dedicated gas-only Base key>
X402_RELAYER_MIN_USDC_BASE_UNITS=100000
X402_RELAYER_MAX_USDC_BASE_UNITS=5000000
X402_RELAYER_MAX_GAS=300000
X402_RELAYER_MAX_FEE_PER_GAS_WEI=10000000000
X402_RELAYER_MAX_DAILY_ATTEMPTS=100
X402_RELAYER_MAX_DAILY_ATTEMPTS_PER_CONTRIBUTOR=10
X402_RELAYER_CONFIRMATIONS=2
X402_RELAYER_WAIT_SECONDS=20
X402_RELAYER_RPC_TIMEOUT_SECONDS=15
X402_RELAYER_LEASE_SECONDS=45
```

Fund the relayer address with only enough Base ETH for bounded gas. Do not send
USDC to it and do not reuse a creator, solver, verifier, keeper, or treasury
key. Keep `ENABLE_BASE_TX_BROADCAST=false`. The API verifies the EIP-712 signer
before persistence, rejects hosted contributions below 0.10 USDC, and
atomically enforces rolling 24-hour network and contributor quotas.

## Runtime Contract

1. An unsigned request receives `402` and `PAYMENT-REQUIRED`.
2. The funder verifies and signs the exact EIP-3009 challenge under its wallet
   policy, then retries with `PAYMENT-SIGNATURE`.
3. The API rejects noncanonical bounties, mismatched relayers, invalid signers,
   under-minimum or over-cap amounts, exhausted rolling quotas, wrong selectors,
   nonzero ETH value, conflicting nonce replays, failed simulation, excessive
   gas, excessive fees, and wrong-chain RPCs.
4. PostgreSQL serializes relayer sends and makes retries idempotent.
5. The API broadcasts, waits for the receipt and configured confirmations, and
   decodes the exact contributor, amount, contract, and transaction from
   `FundingAdded`.
6. Confirmed funding returns `200` and `PAYMENT-RESPONSE`. Pending work returns
   `202` and `/v1/x402/base/relays/{relay_id}`. No other state is settlement.

Monitor the relayer's ETH balance, retryable failures, reverted transactions,
RPC availability, lease age, and confirmation latency. Rotate the relayer by
replacing the API secret and funding the new public address; outstanding signed
challenges bind the old address in their resource URL and should expire rather
than be rewritten.
