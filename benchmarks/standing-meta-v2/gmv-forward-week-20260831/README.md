# Forward GMV Week 20260831 Settlement Attribution Benchmark

This benchmark defines one deterministic child bounty verification harness for the week 20260831 forward GMV campaign (contract `0x1692fecdb678537eb4cc0093e9e4e99dbded9806`, bounty ID `0x51e1f4e05e3ef3dd61f0867a378005780b8882ed4b991fe0f927e6be44bf29f0`).
The child solver must provide:

`scripts/check-agent-bounties-gmv-forward-20260831.mjs`

The script accepts exactly one argument: a path to a settlement attribution payload file. It must use only Node.js built-ins, perform no network access, and write exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.app/schemas/gmv-settlement-attribution.v1.json`
- network: `base-mainnet`
- chain ID: `8453`
- competition: `0x1692fecdb678537eb4cc0093e9e4e99dbded9806` (case-insensitive)
- bounty_id: `0x51e1f4e05e3ef3dd61f0867a378005780b8882ed4b991fe0f927e6be44bf29f0` (case-insensitive)
- epoch_id: `0x3b20496282476095677bba3800efafd147a6c728f08eb69f3a1ba59a53ccaf2d` (case-insensitive)
- verification_policy_hash: `0x8e32aa4f1c355ad59c7c2047a2fa8d29e84619008c1c75626cbb4335c423e178` (case-insensitive)
- scoring window: `settled_at` must fall between `1788134400` (inclusive) and `1788739200` (exclusive)
- non-self-dealing: `creator` !== `solver` (case-insensitive)
- excluded wallets: creator, solver, and funder must not be in the excluded cohort addresses
- excluded contracts: `child_bounty_contract` must not be in the excluded reward contracts
- gmv_base_units: integer >= 1

On success, exit zero and print:

```json
{"ready":true,"network":"base-mainnet","competition":"0x1692fecdb678537eb4cc0093e9e4e99dbded9806","bounty_id":"0x51e1f4e05e3ef3dd61f0867a378005780b8882ed4b991fe0f927e6be44bf29f0","gmv_base_units":900000,"status":"eligible"}
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
node benchmarks/standing-meta-v2/gmv-forward-week-20260831/self-test.mjs
```
