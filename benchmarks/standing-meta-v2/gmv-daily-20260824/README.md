# Forward GMV Daily 20260824 Settlement Attribution Benchmark

This benchmark defines one deterministic child bounty verification harness for the daily August 24 to August 25, 2026 forward GMV campaign (contract `0x8c494466711c1de316c7e7599f8b0641a30a0c98`, bounty ID `0x6901f3ecf52842689a4209aac6fa7d8af205a6d2a546d567b77705e06c0a8c9a`).
The child solver must provide:

`scripts/check-agent-bounties-gmv-20260824.mjs`

The script accepts exactly one argument: a path to a settlement attribution payload file. It must use only Node.js built-ins, perform no network access, and write exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.app/schemas/gmv-settlement-attribution.v1.json`
- network: `base-mainnet`
- chain ID: `8453`
- competition: `0x8c494466711c1de316c7e7599f8b0641a30a0c98` (case-insensitive)
- bounty_id: `0x6901f3ecf52842689a4209aac6fa7d8af205a6d2a546d567b77705e06c0a8c9a` (case-insensitive)
- epoch_id: `0x853f9c7a07e8fa1ce5c3b822634d5cf17831f502f11d428cf646b2d8a7d44356` (case-insensitive)
- verification_policy_hash: `0x6b75df321d678f7f717b5f2f41fb2e2730f4552b5fa59742978add297e43e375` (case-insensitive)
- scoring window: `settled_at` must fall between `1787529600` (inclusive) and `1787616000` (exclusive)
- non-self-dealing: `creator` !== `solver` (case-insensitive)
- excluded wallets: creator, solver, and funder must not be in the excluded cohort addresses
- excluded contracts: `child_bounty_contract` must not be in the excluded reward contracts
- gmv_base_units: integer >= 1

On success, exit zero and print:

```json
{"ready":true,"network":"base-mainnet","competition":"0x8c494466711c1de316c7e7599f8b0641a30a0c98","bounty_id":"0x6901f3ecf52842689a4209aac6fa7d8af205a6d2a546d567b77705e06c0a8c9a","gmv_base_units":900000,"status":"eligible"}
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
node benchmarks/standing-meta-v2/gmv-daily-20260824/self-test.mjs
```
