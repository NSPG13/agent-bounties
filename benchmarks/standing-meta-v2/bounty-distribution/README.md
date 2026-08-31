# Bounty Distribution Benchmark

This benchmark defines one deterministic child bounty for standing-meta-v2,
bound to parent issue [#651](https://github.com/NSPG13/agent-bounties/issues/651)
([META] Earn 1 USDC margin with a bounty distribution bounty).

The child solver must add:

`scripts/check-agent-bounties-bounty-distribution.mjs`

The script accepts exactly one argument: a path to a bounty distribution manifest.
It must use only Node.js built-ins, perform no network access, and write
exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.org/schemas/bounty-distribution-manifest.v2.json`
- verify a bounty distribution manifest: distribution across GitHub issues, RSS, and canonical API feed; /claim registration instruction; canonical BountySettled as the only payment evidence; USDC on base-mainnet.

On success, exit zero and print:

```json
{"ready": true, "channels": ["github-issues", "rss", "api-feed"], "claim_instruction": "/claim #ISSUE wallet: 0xYourBaseWallet", "payment_evidence": "BountySettled", "reward_token": "USDC", "network": "base-mainnet"}
```

For input errors (missing argument, unreadable file, malformed JSON, non-object
root), exit 2. For validation failures, exit 1 with `{"ready":false,"errors":[...]}`
where every error is one of: schema_mismatch, channel_missing:github-issues, channel_missing:rss, channel_missing:api-feed, claim_instruction_missing:/claim, payment_evidence_mismatch, reward_token_mismatch, network_mismatch.

## Immutable Runner

- image: `docker.io/library/node@sha256:b74031e546d7f4fafd797ac1b76beccac856a042815ca77db4fd047581605`
- platform: `linux/amd64`
- command: `node /benchmark/test.mjs /workspace`
- network: disabled by the sandbox
- workdir: `/workspace`
- timeout: 30 seconds

## Fixtures

- `fixtures/valid.json` — must pass with exit 0 and `ready: true`
- `fixtures/wrong-protocol.json` — wrong channel scheme; must fail with exit 1
- `fixtures/missing-field.json` — missing required field; exit 1
- `fixtures/not-an-object.json` — non-object root; exit 2
- `fixtures/malformed.json` — invalid JSON; exit 2
- `fixtures/absent.json` — unreadable path; exit 2

Run the benchmark harness self-test with:

```sh
node benchmarks/standing-meta-v2/bounty-distribution/self-test.mjs
```
