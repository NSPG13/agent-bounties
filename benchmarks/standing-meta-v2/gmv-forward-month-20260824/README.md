# Forward GMV Month 20260824 Settlement Attribution Benchmark

This benchmark defines one deterministic child bounty verification harness for the month August 24 to September 21, 2026 forward GMV campaign (contract `0x8c990ddf5360c00ee0b2090000e3a3a6f90a6a9d`, bounty ID `0x54684252bfcece3e9258af16f337391498b1fefd9cbb7690abe149ad5bee60c6`).
The child solver must provide:

`scripts/check-agent-bounties-gmv-forward-month-20260824.mjs`

The script accepts exactly one argument: a path to a settlement attribution payload file. It must use only Node.js built-ins, perform no network access, and write exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.app/schemas/gmv-settlement-attribution.v1.json`
- network: `base-mainnet`
- chain ID: `8453`
- competition: `0x8c990ddf5360c00ee0b2090000e3a3a6f90a6a9d` (case-insensitive)
- bounty_id: `0x54684252bfcece3e9258af16f337391498b1fefd9cbb7690abe149ad5bee60c6` (case-insensitive)
- epoch_id: `0xd9a0ed086fb1b4df2f9e6006bd9330e7fb56ff000c18101a2b2958a39497dcbd` (case-insensitive)
- verification_policy_hash: `0xf1affeaf2e2cdce4484e596799786f21065a322db67042bda31460484ff4a651` (case-insensitive)
- scoring window: `settled_at` must fall between `1787529600` (inclusive) and `1789948800` (exclusive)
- non-self-dealing: `creator` !== `solver` (case-insensitive)
- excluded wallets: creator, solver, and funder must not be in the excluded cohort addresses
- excluded contracts: `child_bounty_contract` must not be in the excluded reward contracts
- gmv_base_units: integer >= 1

On success, exit zero and print:

```json
{"ready":true,"network":"base-mainnet","competition":"0x8c990ddf5360c00ee0b2090000e3a3a6f90a6a9d","bounty_id":"0x54684252bfcece3e9258af16f337391498b1fefd9cbb7690abe149ad5bee60c6","gmv_base_units":900000,"status":"eligible"}
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
node benchmarks/standing-meta-v2/gmv-forward-month-20260824/self-test.mjs
```
