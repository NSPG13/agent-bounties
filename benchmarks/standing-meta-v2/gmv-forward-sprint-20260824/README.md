# Forward GMV Sprint 20260824 Settlement Attribution Benchmark

This benchmark defines one deterministic child bounty verification harness for the sprint 20260824 forward GMV campaign (contract `0x36e45ec89b61f551724558ed799ad4e07c288175`, bounty ID `0x652887c4b67fe6fe8b414074112107347f7e8480d433022252da032f3bcec4cb`).
The child solver must provide:

`scripts/check-agent-bounties-gmv-forward-sprint-20260824.mjs`

The script accepts exactly one argument: a path to a settlement attribution payload file. It must use only Node.js built-ins, perform no network access, and write exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.app/schemas/gmv-settlement-attribution.v1.json`
- network: `base-mainnet`
- chain ID: `8453`
- competition: `0x36e45ec89b61f551724558ed799ad4e07c288175` (case-insensitive)
- bounty_id: `0x652887c4b67fe6fe8b414074112107347f7e8480d433022252da032f3bcec4cb` (case-insensitive)
- epoch_id: `0x34c8b8362111ed2a48f8e73e6b555c2df3e217bf77337d692182545652086647` (case-insensitive)
- verification_policy_hash: `0x800333bd3f461b8fca3eb71b6024cd728726d9dfb8e284f6a5a10e161299c2ff` (case-insensitive)
- scoring window: `settled_at` must fall between `1787529600` (inclusive) and `1787788800` (exclusive)
- non-self-dealing: `creator` !== `solver` (case-insensitive)
- excluded wallets: creator, solver, and funder must not be in the excluded cohort addresses
- excluded contracts: `child_bounty_contract` must not be in the excluded reward contracts
- gmv_base_units: integer >= 1

On success, exit zero and print:

```json
{"ready":true,"network":"base-mainnet","competition":"0x36e45ec89b61f551724558ed799ad4e07c288175","bounty_id":"0x652887c4b67fe6fe8b414074112107347f7e8480d433022252da032f3bcec4cb","gmv_base_units":900000,"status":"eligible"}
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
node benchmarks/standing-meta-v2/gmv-forward-sprint-20260824/self-test.mjs
```
