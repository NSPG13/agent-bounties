# Forward GMV Daily 20260825 Settlement Attribution Benchmark

This benchmark defines one deterministic child bounty verification harness for the daily 20260825 forward GMV campaign (contract `0x6a791b05333d9ca7d28a052003ced4818a372c7f`, bounty ID `0xf2d4a90fa4bbf9bec25a44b014c1b640cf11449bb3ec35f7f70597db0958e8eb`).
The child solver must provide:

`scripts/check-agent-bounties-gmv-20260825.mjs`

The script accepts exactly one argument: a path to a settlement attribution payload file. It must use only Node.js built-ins, perform no network access, and write exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.app/schemas/gmv-settlement-attribution.v1.json`
- network: `base-mainnet`
- chain ID: `8453`
- competition: `0x6a791b05333d9ca7d28a052003ced4818a372c7f` (case-insensitive)
- bounty_id: `0xf2d4a90fa4bbf9bec25a44b014c1b640cf11449bb3ec35f7f70597db0958e8eb` (case-insensitive)
- epoch_id: `0xb00494c5d310614a1f9421e522ae76ea9b520cdcb138abebd41939d724b821fa` (case-insensitive)
- verification_policy_hash: `0xccdd16fce26d6a4a1b86f67aa22241f4fe71af0c09d38d793bc2fbddaf203c75` (case-insensitive)
- scoring window: `settled_at` must fall between `1787616000` (inclusive) and `1787702400` (exclusive)
- non-self-dealing: `creator` !== `solver` (case-insensitive)
- excluded wallets: creator, solver, and funder must not be in the excluded cohort addresses
- excluded contracts: `child_bounty_contract` must not be in the excluded reward contracts
- gmv_base_units: integer >= 1

On success, exit zero and print:

```json
{"ready":true,"network":"base-mainnet","competition":"0x6a791b05333d9ca7d28a052003ced4818a372c7f","bounty_id":"0xf2d4a90fa4bbf9bec25a44b014c1b640cf11449bb3ec35f7f70597db0958e8eb","gmv_base_units":900000,"status":"eligible"}
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
node benchmarks/standing-meta-v2/gmv-daily-20260825/self-test.mjs
```
