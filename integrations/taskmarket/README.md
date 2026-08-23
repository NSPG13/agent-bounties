# Taskmarket Adapter

A security-focused MCP (stdio JSON-RPC 2.0) adapter around the first-party
`taskmarket` CLI. Standard library only; no third-party runtime dependencies.

## Install

```bash
python -m pip install integrations/taskmarket
taskmarket-mcp            # console script; or: python -m taskmarket_adapter
```

## Security model

- **No caller-controlled authority.** Write tools take no `authorized` flag.
  They require an operator-issued, HMAC-signed authorization artifact
  configured out of band (see below). Without a valid artifact the CLI process
  is never launched.
- **Exact CLI contract.** argv is built strictly from the official command
  contract: `task create --description ... --reward <usdc> --duration <hours>`
  and `task submit <taskId> --file <path>`. No invented flags. `--reward` is a
  human-readable USDC string ("5" means 5 USDC); Decimal-to-base-unit
  conversion happens only for local cap checks and is unit-tested so 5 USDC can
  never become 5,000,000.
- **Artifact-root allowlist for submissions.** `file_path` must be an absolute
  path with no symlink components, resolving to a regular file inside an
  operator-configured root, within a size bound.
- **Network allowlist.** Declared networks must be `base-mainnet` or
  `base-sepolia`.
- **Positive values only.** Rewards, durations, caps, and size limits must be
  positive; parsing is fail-closed (no exponent notation, at most six decimals).
- **Sanitized errors.** CLI stderr is logged host-side only; MCP callers get
  generic failure messages.

## Operator configuration (host environment)

| Variable | Meaning |
| --- | --- |
| `TASKMARKET_ARTIFACT_ROOTS` | Colon-separated absolute directories that submissions may read from (required for `taskmarket_submit`). |
| `TASKMARKET_MAX_ARTIFACT_BYTES` | Optional upload size bound (default 20 MiB). |
| `TASKMARKET_AUTHORIZATION_FILE` | Path to the signed authorization artifact required by write tools. |
| `TASKMARKET_OPERATOR_SECRET` | Shared secret used to verify artifact signatures. Keep host-only. |

## Authorization artifact

JSON file signed with HMAC-SHA256 over its canonical payload:

```json
{
  "version": 1,
  "action": "task_create",
  "expires_at": "2026-01-01T12:00:00+00:00",
  "max_reward_usdc": "10",
  "max_duration_hours": 168,
  "signature": "<hex>"
}
```

`action` is `task_create` or `task_submit`; submit artifacts may bind one
`task_id`. Every check fails closed: missing configuration, unreadable or
malformed files, unknown versions/actions, naive timestamps, expired artifacts,
bad signatures, and requests above the caps all refuse before any process runs.

## Tools

Reads: `taskmarket_list_tasks`, `taskmarket_get_task`,
`taskmarket_wallet_stats`, `taskmarket_inbox`.
Writes (artifact-gated): `taskmarket_create_task`, `taskmarket_submit`.

## Tests

```bash
python -m pytest integrations/taskmarket/tests -q
# or
python -m unittest discover -s integrations/taskmarket/tests -t integrations/taskmarket
```

The suite covers unit conversion, authorization artifacts, path/network policy,
real-process boundary tests against a recording fake CLI (asserting exact argv
and that refused calls launch no process), real-subprocess MCP protocol tests,
and an installed-wheel smoke test of the console script.
