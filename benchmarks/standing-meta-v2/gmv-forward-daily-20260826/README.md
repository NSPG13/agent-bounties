# Forward GMV Daily 20260826 Settlement Attribution Benchmark

This benchmark defines one deterministic child bounty verification harness for the daily August 26, 2026 forward GMV campaign (contract `0x441e8bded29917999fc0e8330c83553262fa2f08`, bounty ID `0x2bf21872cbf17425daf5e3231c242185ce89fc601320af5d785b87fc50329b8f`).
The child solver must provide:

`scripts/check-agent-bounties-gmv-forward-daily-20260826.mjs`

The script accepts exactly one argument: a path to a settlement attribution payload file. It must use only Node.js built-ins, perform no network access, and write exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.app/schemas/gmv-settlement-attribution.v1.json`
- network: `base-mainnet`
- chain ID: `8453`
- competition: `0x441e8bded29917999fc0e8330c83553262fa2f08` (case-insensitive)
- bounty_id: `0x2bf21872cbf17425daf5e3231c242185ce89fc601320af5d785b87fc50329b8f` (case-insensitive)
- epoch_id: `0xdf2ca136c2bfcf42dfac78584d6b2293bd51339e4fac7373b24f41eadba6288d` (case-insensitive)
- verification_policy_hash: `0x0da57d77cd3483cdc3a49e6f3ed05d6d8aff9a346152aca6c03f82b524fd78d8` (case-insensitive)
- scoring window: `settled_at` must fall between `1787702400` (inclusive) and `1787788800` (exclusive)
- non-self-dealing: `creator` !== `solver` (case-insensitive)
- excluded wallets: creator, solver, and funder must not be in the excluded cohort addresses
- excluded contracts: `child_bounty_contract` must not be in the excluded reward contracts
- gmv_base_units: integer >= 1

On success, exit zero and print:

```json
{"ready":true,"network":"base-mainnet","competition":"0x441e8bded29917999fc0e8330c83553262fa2f08","bounty_id":"0x2bf21872cbf17425daf5e3231c242185ce89fc601320af5d785b87fc50329b8f","gmv_base_units":900000,"status":"eligible"}
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
node benchmarks/standing-meta-v2/gmv-forward-daily-20260826/self-test.mjs
```
