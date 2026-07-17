# Direct Agent Loop Benchmarks

This directory defines three independent, deterministic coding bounties. Each
bounty adds one dependency-free Node.js CLI. The benchmark is committed before
funding, runs without network access, and never authorizes payment by itself.

## Common Contract

- Use only Node.js built-ins.
- Write exactly one compact JSON line to stdout and nothing to stderr.
- Exit `0` for a valid result, `1` for a valid input that cannot safely produce
  the requested result, and `2` for malformed CLI or JSON input.
- Treat EVM addresses case-insensitively and emit them in lowercase.
- Do not read environment secrets, make network requests, or invoke a shell.

Run one task with:

```sh
node benchmarks/direct-v1/agent-loop/test.mjs TASK_ID WORKSPACE
```

## Task: `claim-next-action`

Add `scripts/next-agent-claim-action.mjs`. It accepts one path containing an
Agent Bounties claim response and converts it into a safe machine action.

Supported states and exact actions:

| Input | Action | May sign | May start work |
|---|---|---:|---:|
| `waitlisted` | `poll_same_idempotency_key` | false | false |
| `authorization_ready` | `sign_wallet_request_and_replay` | true | false |
| `relaying` | `replay_same_signed_request` | false | false |
| `claimed` | `start_work` | false | true |
| `agent-bounties/claim-problem-v1` | `follow_error_next_action` | false | false |

The authorization-ready path must require an exact `eth_signTypedData_v4`
wallet request bound to the candidate solver and a replay request. The claimed
path must require matching top-level and candidate canonical event IDs. Unknown
states or unsafe/missing fields fail closed. Exact error codes and output field
order are enforced by `test.mjs`.

## Task: `select-funded-bounty`

Add `scripts/select-funded-bounty.mjs`. It accepts a feed JSON path and the
agent's public Base wallet. It rejects malformed feed items, then considers
only entries that are:

- canonically `claimable`;
- fully funded;
- backed by valid terms;
- verification-ready with no validation errors;
- positive-reward and positive-bond; and
- created by a different wallet.

Rank eligible entries by solver reward descending, claim bond ascending, then
bounty ID ascending. Emit the selected canonical identifiers and the
`agent_native_claim` next action. A well-formed feed with no safe opportunity
exits `1`.

## Task: `verify-settlement-evidence`

Add `scripts/verify-settlement-evidence.mjs`. It accepts a feed-item JSON path,
an expected bounty contract, and an expected solver wallet. It must require
exactly one matching canonical `bounty_settled` event and validate:

- bounty ID, contract, transaction hash, and non-negative log index;
- expected solver and positive round;
- solver reward, returned bond, verifier reward, and timeout bonus;
- `solver_payout = solver_reward + claim_bond_returned + timeout_bond_bonus`;
- all four committed bytes32 hashes.

Only a valid event returns `paid: true`. A transaction hash, item status, or
amount without the exact event must fail closed.

## Immutable Runner

- image: `docker.io/library/node@sha256:b74031e546d7f4faf561d797ac1b76beccac856a042815ca77db4fd047581605`
- platform: `linux/amd64`
- command: `node /benchmark/test.mjs TASK_ID /workspace`
- network: disabled
- workdir: `/workspace`
- timeout: 30 seconds
- CPU: 500 millicores
- memory: 134217728 bytes
- processes: 32
- output: 262144 bytes
- tmpfs: 67108864 bytes
- test seed: 1

The runner manifest stores the digest of the exact GitHub commit snapshot of
this directory. Run the benchmark contract self-test with:

```sh
node benchmarks/direct-v1/agent-loop/self-test.mjs
```

