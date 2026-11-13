# Wallet Ux Benchmark

This benchmark defines one deterministic child bounty for standing-meta-v2,
bound to parent issue [#336](https://github.com/NSPG13/agent-bounties/issues/336)
([META] Earn 1 USDC margin with a wallet UX bounty).

The child solver must add:

`scripts/check-agent-bounties-wallet-ux.mjs`

The script accepts exactly one argument: a path to a wallet UX manifest.
It must use only Node.js built-ins, perform no network access, and write
exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.org/schemas/wallet-ux-manifest.v2.json`
- verify a wallet UX manifest: injected EIP-1193 + WalletConnect connectors, bounded-wallet-relay-v1 binding, EIP-1193 signing flow, visible refundable bond preview, explicit error guidance, on base-mainnet.

On success, exit zero and print:

```json
{"ready": true, "connectors": ["injected-eip1193", "walletconnect"], "binding": "agent-bounties/bounded-wallet-relay-v1", "signing_flow": "EIP-1193", "bond_preview": true, "error_guidance": true, "network": "base-mainnet"}
```

For input errors (missing argument, unreadable file, malformed JSON, non-object
root), exit 2. For validation failures, exit 1 with `{"ready":false,"errors":[...]}`
where every error is one of: schema_mismatch, connector_missing:injected-eip1193, binding_mismatch, signing_flow_mismatch, bond_preview_missing, error_guidance_missing, network_mismatch.

## Immutable Runner

- image: `docker.io/library/node@sha256:b74031e546d7f4fafd797ac1b76beccac856a042815ca77db4fd047581605`
- platform: `linux/amd64`
- command: `node /benchmark/test.mjs /workspace`
- network: disabled by the sandbox
- workdir: `/workspace`
- timeout: 30 seconds

## Fixtures

- `fixtures/valid.json` — must pass with exit 0 and `ready: true`
- `fixtures/wrong-protocol.json` — wrong wallet binding; must fail with exit 1
- `fixtures/missing-field.json` — missing required field; exit 1
- `fixtures/not-an-object.json` — non-object root; exit 2
- `fixtures/malformed.json` — invalid JSON; exit 2
- `fixtures/absent.json` — unreadable path; exit 2

Run the benchmark harness self-test with:

```sh
node benchmarks/standing-meta-v2/wallet-ux/self-test.mjs
```
