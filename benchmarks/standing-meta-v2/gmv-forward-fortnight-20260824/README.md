# Forward GMV Fortnight 20260824 Settlement Attribution Benchmark

This benchmark defines one deterministic child bounty verification harness for the fortnight August 24 to September 7, 2026 forward GMV campaign (contract `0x6f635dfd07085aa48ec8b11767eeb48936969f5c`, bounty ID `0x650ab64c02cba5dd8d3d4f3efcc1bf4c2c8125d2fea0e13b69e8e326f1b8b81f`).
The child solver must provide:

`scripts/check-agent-bounties-gmv-forward-fortnight-20260824.mjs`

The script accepts exactly one argument: a path to a settlement attribution payload file. It must use only Node.js built-ins, perform no network access, and write exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.app/schemas/gmv-settlement-attribution.v1.json`
- network: `base-mainnet`
- chain ID: `8453`
- competition: `0x6f635dfd07085aa48ec8b11767eeb48936969f5c` (case-insensitive)
- bounty_id: `0x650ab64c02cba5dd8d3d4f3efcc1bf4c2c8125d2fea0e13b69e8e326f1b8b81f` (case-insensitive)
- epoch_id: `0xf4954bbd1a48ec059078bfbb84a24cd68d35e8837631a562704dbe83edde5a9d` (case-insensitive)
- verification_policy_hash: `0xcd88233c773ba4804318df8423e0313bbf733edd48cd0117d87100750fb36f76` (case-insensitive)
- scoring window: `settled_at` must fall between `1787529600` (inclusive) and `1788739200` (exclusive)
- non-self-dealing: `creator` !== `solver` (case-insensitive)
- excluded wallets: creator, solver, and funder must not be in the excluded cohort addresses
- excluded contracts: `child_bounty_contract` must not be in the excluded reward contracts
- gmv_base_units: integer >= 1

On success, exit zero and print:

```json
{"ready":true,"network":"base-mainnet","competition":"0x6f635dfd07085aa48ec8b11767eeb48936969f5c","bounty_id":"0x650ab64c02cba5dd8d3d4f3efcc1bf4c2c8125d2fea0e13b69e8e326f1b8b81f","gmv_base_units":900000,"status":"eligible"}
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
node benchmarks/standing-meta-v2/gmv-forward-fortnight-20260824/self-test.mjs
```
