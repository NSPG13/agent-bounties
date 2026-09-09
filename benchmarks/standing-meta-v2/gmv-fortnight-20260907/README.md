# Forward GMV Fortnight 20260907 Settlement Attribution Benchmark

This benchmark defines one deterministic child bounty verification harness for the fortnight 20260907 forward GMV campaign (contract `0x81f0dd1f7da5f53ab6317e27131f9af45392b84c`, bounty ID `0x6f4939b0e12c8dd5d84efbfec0c7933bcdfcac9fb012c1b63014c6a6be7cad04`).
The child solver must provide:

`scripts/check-agent-bounties-gmv-20260907.mjs`

The script accepts exactly one argument: a path to a settlement attribution payload file. It must use only Node.js built-ins, perform no network access, and write exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.app/schemas/gmv-settlement-attribution.v1.json`
- network: `base-mainnet`
- chain ID: `8453`
- competition: `0x81f0dd1f7da5f53ab6317e27131f9af45392b84c` (case-insensitive)
- bounty_id: `0x6f4939b0e12c8dd5d84efbfec0c7933bcdfcac9fb012c1b63014c6a6be7cad04` (case-insensitive)
- epoch_id: `0x34750799f9f543ea8b35feaceb2b50b248ce358b3aad071f482900399c394b8c` (case-insensitive)
- verification_policy_hash: `0x617eb9935be1dd5d7287068ec5a9ce3e15b92851a84ec8633813199863b05e73` (case-insensitive)
- scoring window: `settled_at` must fall between `1788739200` (inclusive) and `1789948800` (exclusive)
- non-self-dealing: `creator` !== `solver` (case-insensitive)
- excluded wallets: creator, solver, and funder must not be in the excluded cohort addresses
- excluded contracts: `child_bounty_contract` must not be in the excluded reward contracts
- gmv_base_units: integer >= 1

On success, exit zero and print:

```json
{"ready":true,"network":"base-mainnet","competition":"0x81f0dd1f7da5f53ab6317e27131f9af45392b84c","bounty_id":"0x6f4939b0e12c8dd5d84efbfec0c7933bcdfcac9fb012c1b63014c6a6be7cad04","gmv_base_units":900000,"status":"eligible"}
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
node benchmarks/standing-meta-v2/gmv-fortnight-20260907/self-test.mjs
```
