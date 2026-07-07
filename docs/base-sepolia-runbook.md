# Base Sepolia Runbook

Base Sepolia is the first open testnet rail for escrow and payout rehearsal.
This runbook keeps signing outside the app: the platform generates deterministic
transaction intent and operator commands, while Foundry signs and broadcasts.

## Environment

Use a low-value test wallet. Never place private keys in source files.

```powershell
$env:BASE_SEPOLIA_RPC_URL = "https://..."
$env:BASE_DEPLOYER_PRIVATE_KEY = "0x..."
$env:BASE_PAYER_PRIVATE_KEY = "0x..."
$env:BASE_SETTLEMENT_SIGNER_PRIVATE_KEY = "0x..."
```

The runbook command requires the current settlement signer, escrow contract, and
USDC token addresses. For a fresh deploy, use the deploy command first, then
rerun the command with the deployed escrow address.

```powershell
cargo run -p cli -- base-sepolia-runbook `
  --settlement-signer 0x5555555555555555555555555555555555555555 `
  --escrow-contract 0x1111111111111111111111111111111111111111 `
  --usdc-token 0x3333333333333333333333333333333333333333
```

The output includes:

- `forge create` commands for `AgentBountyEscrow`,
- `cast send --data ...` commands for USDC approval and `createEscrow`,
- `cast send --data ...` command for settlement-signer release,
- refund and dispute transaction planning is available through
  `base-refund-plan`, `base-dispute-plan`, `POST /v1/base/refund-plan`,
  `POST /v1/base/dispute-plan`, MCP `plan_base_refund`, and MCP
  `plan_base_dispute`,
- `eth_getLogs` request planning is available through `base-log-query`,
- configured RPC fetch and reconciliation is available through
  `base-fetch-logs`, `POST /v1/base/fetch-rpc-logs`, or MCP
  `fetch_base_rpc_logs`,
- signed release transaction broadcast is available through
  `base-broadcast-signed-transaction`, `POST /v1/base/broadcast-signed-transaction`,
  or MCP `broadcast_base_signed_transaction` when operator-enabled,
- receipt polling and optional log reconciliation is available through
  `base-transaction-receipt`, `POST /v1/base/transaction-receipt`, or MCP
  `get_base_transaction_receipt`,
- the expected signer role for each transaction.

## Operator Flow

1. Run `forge test` from `contracts/base-escrow`.
2. Deploy `AgentBountyEscrow` with the settlement signer address.
3. Fund a bounty on the platform and keep its terms hash.
4. Execute the generated USDC `approve` transaction from the payer wallet.
5. Execute the generated `createEscrow` transaction from the payer wallet.
6. Build an escrow log query with:

   ```powershell
   cargo run -p cli -- base-log-query `
     --escrow-contract 0x1111111111111111111111111111111111111111 `
     --from-block <deployment-or-bounty-block>
   ```

7. Reconcile the funding log. If the API/MCP service has
   `BASE_SEPOLIA_RPC_URL` configured, call `POST /v1/base/fetch-rpc-logs` or MCP
   `fetch_base_rpc_logs` with the same contract and block range. For an
   operator-local fetch, run `cargo run -p cli -- base-fetch-logs ...`. The
   manual fallback is still to execute the returned `eth_getLogs` request
   against `$env:BASE_SEPOLIA_RPC_URL` and submit the provider response to
   `POST /v1/base/rpc-logs`.
8. After deterministic verification accepts the work, get the release queue from
   `POST /v1/base/release-queue`.
   If the work needs to be refunded or marked disputed instead, generate an
   unsigned settlement-signer transaction with `POST /v1/base/refund-plan` or
   `POST /v1/base/dispute-plan`. The local equivalents are:

   ```powershell
   cargo run -p cli -- base-refund-plan `
     --escrow-contract 0x1111111111111111111111111111111111111111 `
     --onchain-escrow-id <escrow-id> `
     --reason-hash 0x<32-byte-reason-hash>

   cargo run -p cli -- base-dispute-plan `
     --escrow-contract 0x1111111111111111111111111111111111111111 `
     --onchain-escrow-id <escrow-id> `
     --dispute-hash 0x<32-byte-dispute-hash>
   ```
9. Execute the generated `release` transaction from the settlement signer
   wallet, or the generated `refund`/`markDisputed` transaction for a non-happy
   path. Operators can use Foundry `cast send` directly, or sign elsewhere and
   submit the signed raw transaction through `base-broadcast-signed-transaction`,
   `POST /v1/base/broadcast-signed-transaction`, or MCP
   `broadcast_base_signed_transaction`. Hosted broadcast requires
   `ENABLE_BASE_TX_BROADCAST=true`.
10. Poll the release transaction receipt. If using the API/MCP receipt endpoint,
    pass `reconcile_logs=true` so emitted escrow logs run through the indexer.
    The CLI prints normalized logs that can be submitted to
    `POST /v1/base/evm-logs`.
11. Fetch/reconcile logs again from the last indexed block if receipt polling was
    not used. The response must include the emitted `EscrowReleased` log, whether
    ingested through `/v1/base/fetch-rpc-logs`, MCP `fetch_base_rpc_logs`, or the
    manual `/v1/base/rpc-logs` provider-response path.

The platform should move a Base bounty to `Paid`, `Refunded`, or `Disputed`
only after the matching escrow log is indexed. The transaction itself is not
treated as proof of settlement.
