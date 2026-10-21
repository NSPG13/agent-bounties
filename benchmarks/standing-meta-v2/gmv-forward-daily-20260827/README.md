# Forward GMV Daily 20260827 Settlement Attribution Benchmark

This benchmark defines one deterministic child bounty verification harness for the daily August 27, 2026 forward GMV campaign (contract `0x979782be0bfefb78ac0459d4695d619932ecdda4`, bounty ID `0xce85cd119ce7f9fedabbc3aa866bb1951719e0a3dd2115c805ef59e16374d47c`).
The child solver must provide:

`scripts/check-agent-bounties-gmv-forward-daily-20260827.mjs`

The script accepts exactly one argument: a path to a settlement attribution payload file. It must use only Node.js built-ins, perform no network access, and write exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.app/schemas/gmv-settlement-attribution.v1.json`
- network: `base-mainnet`
- chain ID: `8453`
- competition: `0x979782be0bfefb78ac0459d4695d619932ecdda4` (case-insensitive)
- bounty_id: `0xce85cd119ce7f9fedabbc3aa866bb1951719e0a3dd2115c805ef59e16374d47c` (case-insensitive)
- epoch_id: `0x6a26927d5353f491fe377ed6edaf5c3618b9f95d4bc10d5a42e905c96bbe2b62` (case-insensitive)
- verification_policy_hash: `0x40b88ef7e73817ef5a784c78870d62b361aa4ad39d046ef6534337239dd2fb5f` (case-insensitive)
- scoring window: `settled_at` must fall between `1787788800` (inclusive) and `1787875200` (exclusive)
- non-self-dealing: `creator` !== `solver` (case-insensitive)
- excluded wallets: creator, solver, and funder must not be in the excluded cohort addresses
- excluded contracts: `child_bounty_contract` must not be in the excluded reward contracts
- gmv_base_units: integer >= 1

On success, exit zero and print:

```json
{"ready":true,"network":"base-mainnet","competition":"0x979782be0bfefb78ac0459d4695d619932ecdda4","bounty_id":"0xce85cd119ce7f9fedabbc3aa866bb1951719e0a3dd2115c805ef59e16374d47c","gmv_base_units":900000,"status":"eligible"}
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
node benchmarks/standing-meta-v2/gmv-forward-daily-20260827/self-test.mjs
```
