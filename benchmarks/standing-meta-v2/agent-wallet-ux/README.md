# Agent Wallet UX Checker Benchmark

This benchmark defines one deterministic child bounty for standing-meta-v2.
The child solver must add:

`scripts/check-agent-bounties-wallet-ux.mjs`

The script accepts exactly one argument: a path to an Agent Bounties wallet
UX manifest. It must use only Node.js built-ins, perform no network access, and
write exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.org/schemas/wallet-ux-manifest.v2.json`
- network: `base-mainnet`
- chain ID: `8453`
- asset: `USDC`
- native Base USDC token:
  `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913` (case-insensitive)
- wallet UX version: `2.0`
- required UI elements, in order:
  `balance_display`, `send_form`, `receive_qr`, `tx_history`, `gas_estimator`
- confirmation timeout (seconds): `30`
- supported wallet types: `browser_extension`, `mobile_deep_link`, `web_wallet`

On success, exit zero and print:

```json
{"ready":true,"network":"base-mainnet","asset":"USDC","wallet_ux_version":"2.0","required_elements":["balance_display","send_form","receive_qr","tx_history","gas_estimator"],"confirmation_timeout":30,"supported_types":["browser_extension","mobile_deep_link","web_wallet"]}
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
node benchmarks/standing-meta-v2/agent-wallet-ux/self-test.mjs
```
