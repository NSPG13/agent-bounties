# Base Mainnet Escrow Evidence Report - 2026-07-09

Issue: https://github.com/NSPG13/agent-bounties/issues/127

Observation timestamp: `2026-07-09T23:38:03Z`

This report records independent read-only evidence for the Base mainnet
`AgentBountyEscrow` deployment. It uses the public Base RPC at
`https://mainnet.base.org`, the checked-in
`deployments/base-mainnet.json` coordinates, and public Sourcify and Blockscout
verification records.

This deployment verification does not prove hosted API health, bounty funding,
claimability, work acceptance, payout, escrow release, or settlement. A
transaction hash alone is not settlement evidence. Funding and payout state
require matching indexed escrow events reconciled by the platform.

## Coordinates

| Field | Value |
|---|---|
| Chain ID | `8453` |
| Escrow contract | `0x150C6dFbCe7803cc7f634f59b0624e87349CEAce` |
| Deployment transaction | `0xede8896af324658d7da6fc08589cc5d02cc344ef934087a1c147f6c9617b865d` |
| Deployment block | `48422806` |
| Native Base USDC | `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913` |
| Observation latest block | `48425473` |

## Read-Only Observations

| Check | Expected from `deployments/base-mainnet.json` | Observed | Result |
|---|---:|---:|---|
| `eth_chainId` | `8453` (`0x2105`) | `8453` (`0x2105`) | PASS |
| Deployment receipt status | successful receipt | `0x1` | PASS |
| Deployment receipt block | `48422806` | `48422806` | PASS |
| Deployment receipt contract | `0x150C6dFbCe7803cc7f634f59b0624e87349CEAce` | `0x150c6dfbce7803cc7f634f59b0624e87349ceace` | PASS |
| Runtime bytecode | non-empty | `3739` bytes | PASS |
| Runtime code hash | `0x8726789773ebdb4ea81642eb6f95b91965b93ce8341f356e0f8513188b72ffea` | `0x8726789773ebdb4ea81642eb6f95b91965b93ce8341f356e0f8513188b72ffea` | PASS |
| `owner()` | `0x884834E884d6e93462655A2820140aD03E6747bC` | `0x884834e884d6e93462655a2820140ad03e6747bc` | PASS |
| `settlementSigner()` | `0x884834E884d6e93462655A2820140aD03E6747bC` | `0x884834e884d6e93462655a2820140ad03e6747bc` | PASS |
| `paused()` | not paused | `false` | PASS |
| `nextEscrowId()` | no prior escrow creation expected for a fresh pilot | `1` | PASS |
| Native USDC contract | `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913` | non-empty bytecode, `1852` bytes; `decimals()` returned `6` | PASS |

The deployment receipt was fetched for
`0xede8896af324658d7da6fc08589cc5d02cc344ef934087a1c147f6c9617b865d`.
Its `from` address was `0x884834e884d6e93462655a2820140ad03e6747bc`,
`to` was `null`, and `contractAddress` was
`0x150c6dfbce7803cc7f634f59b0624e87349ceace`.

## Source Verification

| Source | Record | Observed status | Result |
|---|---|---|---|
| Sourcify | https://sourcify.dev/server/v2/contract/8453/0x150C6dFbCe7803cc7f634f59b0624e87349CEAce | HTTP `200`; `match` was `exact_match` | PASS |
| Blockscout | https://base.blockscout.com/address/0x150C6dFbCe7803cc7f634f59b0624e87349CEAce | HTTP `200`; API reported `is_verified: true`, `is_fully_verified: true`, `is_changed_bytecode: false`, compiler `0.8.26+commit.8a97fa7a`, EVM `cancun`, optimizer enabled | PASS |

Explorer availability is reference evidence only. The RPC checks above remain
the deterministic deployment observations.

## Reproducible Commands

These examples use only public read-only JSON-RPC calls. They do not require or
expose private keys, seed phrases, API tokens, signatures, wallet history, or
personal data.

```bash
RPC=https://mainnet.base.org
ESCROW=0x150C6dFbCe7803cc7f634f59b0624e87349CEAce
USDC=0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913
DEPLOY_TX=0xede8896af324658d7da6fc08589cc5d02cc344ef934087a1c147f6c9617b865d

curl -s "$RPC" \
  -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}'

curl -s "$RPC" \
  -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","id":2,"method":"eth_getTransactionReceipt","params":["0xede8896af324658d7da6fc08589cc5d02cc344ef934087a1c147f6c9617b865d"]}'

curl -s "$RPC" \
  -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","id":3,"method":"eth_getCode","params":["0x150C6dFbCe7803cc7f634f59b0624e87349CEAce","latest"]}'
```

The runtime code hash was computed by passing the `eth_getCode` result into
`web3_sha3`. The observed result was
`0x8726789773ebdb4ea81642eb6f95b91965b93ce8341f356e0f8513188b72ffea`.

The contract read selectors used for this report were:

| Function | Selector |
|---|---|
| `owner()` | `0x8da5cb5b` |
| `settlementSigner()` | `0xc46914d8` |
| `paused()` | `0x5c975abb` |
| `nextEscrowId()` | `0x89cb29dd` |
| `decimals()` on native USDC | `0x313ce567` |

Example `eth_call` requests:

```bash
curl -s "$RPC" \
  -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","id":4,"method":"eth_call","params":[{"to":"0x150C6dFbCe7803cc7f634f59b0624e87349CEAce","data":"0x8da5cb5b"},"latest"]}'

curl -s "$RPC" \
  -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","id":5,"method":"eth_call","params":[{"to":"0x150C6dFbCe7803cc7f634f59b0624e87349CEAce","data":"0xc46914d8"},"latest"]}'

curl -s "$RPC" \
  -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","id":6,"method":"eth_call","params":[{"to":"0x150C6dFbCe7803cc7f634f59b0624e87349CEAce","data":"0x5c975abb"},"latest"]}'

curl -s "$RPC" \
  -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","id":7,"method":"eth_call","params":[{"to":"0x150C6dFbCe7803cc7f634f59b0624e87349CEAce","data":"0x89cb29dd"},"latest"]}'

curl -s "$RPC" \
  -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","id":8,"method":"eth_getCode","params":["0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913","latest"]}'

curl -s "$RPC" \
  -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","id":9,"method":"eth_call","params":[{"to":"0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913","data":"0x313ce567"},"latest"]}'
```

Verification record checks:

```bash
curl -s \
  https://sourcify.dev/server/v2/contract/8453/0x150C6dFbCe7803cc7f634f59b0624e87349CEAce

curl -s \
  https://base.blockscout.com/api/v2/smart-contracts/0x150C6dFbCe7803cc7f634f59b0624e87349CEAce
```

## Hosted Funding Boundary

The hosted bounty record posted on issue #127 was also read at observation time:

- Hosted bounty ID: `31f83d55-f388-4cc8-b384-651403b71163`
- Status endpoint: `https://agent-bounties-api.onrender.com/v1/bounties/31f83d55-f388-4cc8-b384-651403b71163`
- Observed hosted status: `Unfunded`
- Observed `claimable`: `false`
- Observed confirmed Base amount: `0` USDC
- Observed escrow count: `0`

This hosted status means the deployment report is only deployment evidence. It
is not funding evidence, not claimability evidence, not work acceptance, not
payout evidence, and not settlement evidence.
