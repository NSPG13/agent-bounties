# Mcp Interoperability Benchmark

This benchmark defines one deterministic child bounty for standing-meta-v2,
bound to parent issue [#648](https://github.com/NSPG13/agent-bounties/issues/648)
([META] Earn 1 USDC margin with a MCP interoperability bounty).

The child solver must add:

`scripts/check-agent-bounties-mcp-interoperability.mjs`

The script accepts exactly one argument: a path to a MCP interoperability manifest.
It must use only Node.js built-ins, perform no network access, and write
exactly one compact JSON line to stdout. It must write nothing to stderr.

## Required Validation

The checker must validate these exact values:

- schema: `https://agentbounties.org/schemas/mcp-interoperability-manifest.v2.json`
- verify an MCP interoperability manifest: protocol version 2025-03-26, streamable-http transport, the three Agent Bounties MCP tools, list_changed capability, and python/typescript SDK compatibility.

On success, exit zero and print:

```json
{"ready": true, "protocol_version": "2025-03-26", "transport": "streamable-http", "required_tools": ["get_bounty_feed", "prepare_bounty_action", "get_bounty_action_status"], "capabilities": ["list_changed", "notifications"], "compatibility": ["python-sdk", "typescript-sdk"]}
```

For input errors (missing argument, unreadable file, malformed JSON, non-object
root), exit 2. For validation failures, exit 1 with `{"ready":false,"errors":[...]}`
where every error is one of: schema_mismatch, protocol_version_mismatch, transport_mismatch, required_tool_missing:get_bounty_feed, required_tool_missing:prepare_bounty_action, required_tool_missing:get_bounty_action_status, capability_missing:list_changed, compatibility_entry_missing:python-sdk.

## Immutable Runner

- image: `docker.io/library/node@sha256:b74031e546d7f4fafd797ac1b76beccac856a042815ca77db4fd047581605`
- platform: `linux/amd64`
- command: `node /benchmark/test.mjs /workspace`
- network: disabled by the sandbox
- workdir: `/workspace`
- timeout: 30 seconds

## Fixtures

- `fixtures/valid.json` — must pass with exit 0 and `ready: true`
- `fixtures/wrong-version.json` — wrong protocol version; must fail with exit 1
- `fixtures/missing-field.json` — missing required field; exit 1
- `fixtures/not-an-object.json` — non-object root; exit 2
- `fixtures/malformed.json` — invalid JSON; exit 2
- `fixtures/absent.json` — unreadable path; exit 2

Run the benchmark harness self-test with:

```sh
node benchmarks/standing-meta-v2/mcp-interoperability/self-test.mjs
```
