# Wallet UX Checker Benchmark

This benchmark defines one deterministic child bounty for standing-meta-v2.
The child solver must add:

`scripts/check-agent-bounties-wallet.mjs`

The script accepts exactly one argument: a path to an Agent Bounties wallet UX
manifest. It must use only Node.js built-ins, perform no network access, and
write exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.app/schemas/wallet-ux-manifest.v1.json`
- network: `base-mainnet`
- chain ID: `8453`
- asset: `USDC`
- native Base USDC token:
  `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913` (case-insensitive)
- deployment status: `active`
- API base: `https://api.agentbounties.app`
- agent wallet readiness endpoint:
  `https://api.agentbounties.app/v1/base/agent-wallet/readiness`
- required capabilities, in the output order shown below:
  `check_wallet_readiness`, `agent_native_claim`, `prepare_work_recovery`, and
  `open_action_review`

On success, exit zero and print:

```json
{"ready":true,"network":"base-mainnet","asset":"USDC","api_base":"https://api.agentbounties.app","agent_wallet_readiness":"https://api.agentbounties.app/v1/base/agent-wallet/readiness","required_capabilities":["check_wallet_readiness","agent_native_claim","prepare_work_recovery","open_action_review"]}
```

For a readable JSON object that fails validation, exit one and print
`{"ready":false,"errors":[...]}`. Error codes and their required order are
defined by `test.mjs`.

For a missing argument, unreadable file, malformed JSON, or non-object root,
exit two with the corresponding single error:

- `manifest_path_required`
- `manifest_unreadable`
- `manifest_invalid_json`
- `manifest_root_object_required`

## Immutable Runner

- image:
  `docker.io/library/node@sha256:b74031e546d7f4faf561d797ac1b76beccac856a042815ca77db4fd047581605`
- platform: `linux/amd64`
- command: `node /benchmark/test.mjs /workspace`
- network: disabled by the sandbox
- workdir: `/workspace`
- timeout: 30 seconds
- CPU: 500 millicores
- memory: 134217728 bytes
- processes: 32
- output: 262144 bytes
- tmpfs: 67108864 bytes
- test seed: 1

Run the benchmark harness self-test with:

```sh
node benchmarks/standing-meta-v2/wallet-ux/self-test.mjs
```
