# Forward GMV Daily 20260826 Settlement Attribution Benchmark

This benchmark defines one deterministic child bounty verification harness for the daily 20260826 forward GMV campaign (contract `0x5a096ba0dc8f647cc499d14cd82ff49eef05b828`, bounty ID `0xc9507d73785ce4abfcfb147d135849e8df560df4c201506c657fcafd2bcb257d`).
The child solver must provide:

`scripts/check-agent-bounties-gmv-20260826.mjs`

The script accepts exactly one argument: a path to a settlement attribution payload file. It must use only Node.js built-ins, perform no network access, and write exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.app/schemas/gmv-settlement-attribution.v1.json`
- network: `base-mainnet`
- chain ID: `8453`
- competition: `0x5a096ba0dc8f647cc499d14cd82ff49eef05b828` (case-insensitive)
- bounty_id: `0xc9507d73785ce4abfcfb147d135849e8df560df4c201506c657fcafd2bcb257d` (case-insensitive)
- epoch_id: `0x66ea8b217cc2cc08c45382405d7bae18cbd9eebc96e3745c8d928562254c68df` (case-insensitive)
- verification_policy_hash: `0x6ead994859b7e72db2f9d621a03aa0c2d685d20573f56b326717b33a3ce93e23` (case-insensitive)
- scoring window: `settled_at` must fall between `1787702400` (inclusive) and `1787788800` (exclusive)
- non-self-dealing: `creator` !== `solver` (case-insensitive)
- excluded wallets: creator, solver, and funder must not be in the excluded cohort addresses
- excluded contracts: `child_bounty_contract` must not be in the excluded reward contracts
- gmv_base_units: integer >= 1

On success, exit zero and print:

```json
{"ready":true,"network":"base-mainnet","competition":"0x5a096ba0dc8f647cc499d14cd82ff49eef05b828","bounty_id":"0xc9507d73785ce4abfcfb147d135849e8df560df4c201506c657fcafd2bcb257d","gmv_base_units":900000,"status":"eligible"}
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
node benchmarks/standing-meta-v2/gmv-daily-20260826/self-test.mjs
```
