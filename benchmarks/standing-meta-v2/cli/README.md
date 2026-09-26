# Cli Benchmark

This benchmark defines one deterministic child bounty for standing-meta-v2,
bound to parent issue [#333](https://github.com/NSPG13/agent-bounties/issues/333)
([META] Earn 1 USDC margin with a CLI bounty).

The child solver must add:

`scripts/check-agent-bounties-cli.mjs`

The script accepts exactly one argument: a path to a CLI manifest.
It must use only Node.js built-ins, perform no network access, and write
exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.org/schemas/cli-manifest.v2.json`
- verify a CLI manifest: dependency-free Node CLI named agent-bounty-cli with list/claim/register/status commands, exit codes 0/1/2, --help, and Node >= 20.

On success, exit zero and print:

```json
{"ready": true, "binary": "agent-bounty-cli", "commands": ["list", "claim", "register", "status"], "exit_codes": [0, 1, 2], "help": true, "node_version": ">=20"}
```

For input errors (missing argument, unreadable file, malformed JSON, non-object
root), exit 2. For validation failures, exit 1 with `{"ready":false,"errors":[...]}`
where every error is one of: schema_mismatch, binary_mismatch, command_missing:list, command_missing:claim, command_missing:register, exit_code_mismatch:2, help_missing, node_version_mismatch.

## Immutable Runner

- image: `docker.io/library/node@sha256:b74031e546d7f4fafd797ac1b76beccac856a042815ca77db4fd047581605`
- platform: `linux/amd64`
- command: `node /benchmark/test.mjs /workspace`
- network: disabled by the sandbox
- workdir: `/workspace`
- timeout: 30 seconds

## Fixtures

- `fixtures/valid.json` — must pass with exit 0 and `ready: true`
- `fixtures/wrong-protocol.json` — wrong binary name; must fail with exit 1
- `fixtures/missing-field.json` — missing required field; exit 1
- `fixtures/not-an-object.json` — non-object root; exit 2
- `fixtures/malformed.json` — invalid JSON; exit 2
- `fixtures/absent.json` — unreadable path; exit 2

Run the benchmark harness self-test with:

```sh
node benchmarks/standing-meta-v2/cli/self-test.mjs
```
