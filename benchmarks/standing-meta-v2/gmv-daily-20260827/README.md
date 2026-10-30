# Forward GMV Daily 20260827 Settlement Attribution Benchmark

This benchmark defines one deterministic child bounty verification harness for the daily 20260827 forward GMV campaign (contract `0xee01479015026afc2b09dea37d2ed805926c3c0d`, bounty ID `0x184ac3c4046a401263cc2ba4217a5aa58e47d2304619e1fc112620cd7598a45e`).
The child solver must provide:

`scripts/check-agent-bounties-gmv-20260827.mjs`

The script accepts exactly one argument: a path to a settlement attribution payload file. It must use only Node.js built-ins, perform no network access, and write exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.app/schemas/gmv-settlement-attribution.v1.json`
- network: `base-mainnet`
- chain ID: `8453`
- competition: `0xee01479015026afc2b09dea37d2ed805926c3c0d` (case-insensitive)
- bounty_id: `0x184ac3c4046a401263cc2ba4217a5aa58e47d2304619e1fc112620cd7598a45e` (case-insensitive)
- epoch_id: `0x8ea3eec80412bbe126fad9e043c1d40e41adaeeff13017947e5877484db173fe` (case-insensitive)
- verification_policy_hash: `0x438a46f8103de81b96fe48de248c8908f509b21fa8254a5355e8a04c94277528` (case-insensitive)
- scoring window: `settled_at` must fall between `1787788800` (inclusive) and `1787875200` (exclusive)
- non-self-dealing: `creator` !== `solver` (case-insensitive)
- excluded wallets: creator, solver, and funder must not be in the excluded cohort addresses
- excluded contracts: `child_bounty_contract` must not be in the excluded reward contracts
- gmv_base_units: integer >= 1

On success, exit zero and print:

```json
{"ready":true,"network":"base-mainnet","competition":"0xee01479015026afc2b09dea37d2ed805926c3c0d","bounty_id":"0x184ac3c4046a401263cc2ba4217a5aa58e47d2304619e1fc112620cd7598a45e","gmv_base_units":900000,"status":"eligible"}
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
node benchmarks/standing-meta-v2/gmv-daily-20260827/self-test.mjs
```
