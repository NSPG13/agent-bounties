# Forward GMV Week 20260831 Settlement Attribution Benchmark

This benchmark defines one deterministic child bounty verification harness for the week 20260831 forward GMV campaign (contract `0xdc1bbcbcb149b07262565c8b9caa1ae5e2058f76`, bounty ID `0x46a5a34d8596f6f54efae2487e4ef7906ff3940583a97b9d266ef15c45c3df67`).
The child solver must provide:

`scripts/check-agent-bounties-gmv-20260831.mjs`

The script accepts exactly one argument: a path to a settlement attribution payload file. It must use only Node.js built-ins, perform no network access, and write exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.app/schemas/gmv-settlement-attribution.v1.json`
- network: `base-mainnet`
- chain ID: `8453`
- competition: `0xdc1bbcbcb149b07262565c8b9caa1ae5e2058f76` (case-insensitive)
- bounty_id: `0x46a5a34d8596f6f54efae2487e4ef7906ff3940583a97b9d266ef15c45c3df67` (case-insensitive)
- epoch_id: `0xb2f9f8ba04bfcbcf4858ccf32338f9f807a5fa57e37d31d3fd0d46b60a8fef3f` (case-insensitive)
- verification_policy_hash: `0xbf60c6e630dc123f02115765623bb882f661a9b8594448678b39e774b2c9831a` (case-insensitive)
- scoring window: `settled_at` must fall between `1788134400` (inclusive) and `1788739200` (exclusive)
- non-self-dealing: `creator` !== `solver` (case-insensitive)
- excluded wallets: creator, solver, and funder must not be in the excluded cohort addresses
- excluded contracts: `child_bounty_contract` must not be in the excluded reward contracts
- gmv_base_units: integer >= 1

On success, exit zero and print:

```json
{"ready":true,"network":"base-mainnet","competition":"0xdc1bbcbcb149b07262565c8b9caa1ae5e2058f76","bounty_id":"0x46a5a34d8596f6f54efae2487e4ef7906ff3940583a97b9d266ef15c45c3df67","gmv_base_units":900000,"status":"eligible"}
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
node benchmarks/standing-meta-v2/gmv-week-20260831/self-test.mjs
```
