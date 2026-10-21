# Forward GMV Week 20260824 Settlement Attribution Benchmark

This benchmark defines one deterministic child bounty verification harness for the week 20260824 forward GMV campaign (contract `0x2f1d2b24105596b153e473032256569fe544a44f`, bounty ID `0x4ffb4c5f75f2816b1d58c97ae440184bb5e75cbc158dc639b66a7e50e5d81417`).
The child solver must provide:

`scripts/check-agent-bounties-gmv-forward-20260824.mjs`

The script accepts exactly one argument: a path to a settlement attribution payload file. It must use only Node.js built-ins, perform no network access, and write exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.app/schemas/gmv-settlement-attribution.v1.json`
- network: `base-mainnet`
- chain ID: `8453`
- competition: `0x2f1d2b24105596b153e473032256569fe544a44f` (case-insensitive)
- bounty_id: `0x4ffb4c5f75f2816b1d58c97ae440184bb5e75cbc158dc639b66a7e50e5d81417` (case-insensitive)
- epoch_id: `0x7037d741033453aab56a3e57c3f261dd11891bd808be56d67980d0c8263ddd81` (case-insensitive)
- verification_policy_hash: `0x7be6bc675a54c5c6176d497ba443aab3a7190b65fad345cffc1c02403feac3b0` (case-insensitive)
- scoring window: `settled_at` must fall between `1787529600` (inclusive) and `1788134400` (exclusive)
- non-self-dealing: `creator` !== `solver` (case-insensitive)
- excluded wallets: creator, solver, and funder must not be in the excluded cohort addresses
- excluded contracts: `child_bounty_contract` must not be in the excluded reward contracts
- gmv_base_units: integer >= 1

On success, exit zero and print:

```json
{"ready":true,"network":"base-mainnet","competition":"0x2f1d2b24105596b153e473032256569fe544a44f","bounty_id":"0x4ffb4c5f75f2816b1d58c97ae440184bb5e75cbc158dc639b66a7e50e5d81417","gmv_base_units":900000,"status":"eligible"}
```

For a readable JSON object that fails validation, exit one and print `{"ready":false,"errors":[...]}`.

For a missing argument, unreadable file, malformed JSON, or non-object root, exit two with the corresponding single error:

- `manifest_path_required`
- `manifest_unreadable`
- `manifest_invalid_json`
- `manifest_root_object_required`

## Immutable Runner

- image: `docker.io/library/node@sha256:b74031e546d7f4faf561d797ac1b76beccac856a042815ca77db4fd047581605`
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
node benchmarks/standing-meta-v2/gmv-forward-week-20260824/self-test.mjs
```
