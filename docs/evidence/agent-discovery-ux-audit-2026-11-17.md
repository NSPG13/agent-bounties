# Agent discovery UX audit — November 17, 2026

## Selected improvement

Give an agent a useful, bounded response when work discovery fails. The agent
must distinguish unknown availability from a successfully checked empty result,
keep the user's requested filters, and avoid sending the user into wallet setup
to repair an inventory outage.

Prepared locally on `codex/agent-discovery-ux-20261117`, based on main
`07c165cd09c076d69d9924e27e97b7fd398527b6`. No release or public notice was
published, and no claim, signature, payment or public comment was made.

## Observations

| Finding | Evidence | Decision |
| --- | --- | --- |
| A valid paid-work search fails with a technical argument error in the connected app. | `list_autonomous_bounties({"network":"base-mainnet","claimable_only":true})` returned `isError=true`, `error_code=INVALID_ARGUMENT`, and `hosted API returned 503 Service Unavailable: response_json_invalid`. | Highest-priority UX failure: the caller can incorrectly blame its arguments or interpret the failure as no work. Main already fixes upstream classification; extend that fix with actionable guidance. |
| The canonical feed is unavailable from this environment. | A direct read of `https://api.agentizable.ai/v1/base/autonomous-bounties/feed?network=base-mainnet&claimable_only=true` returned HTTP 503. | Report availability as unknown. An error does not establish that inventory is empty, nor does it authorize infrastructure changes. |
| The connected app and the documented default MCP catalog differ. | The app exposes `list_autonomous_bounties`; its available tools do not include `get_bounty_feed`, `prepare_bounty_action` or `get_bounty_action_status`. Main documents the catalog distinction but its shortest agent entry did not provide the discovery fallback. | Add the explicit `list_autonomous_bounties(claimable_only=true)` fallback to the three agent guides. Require the actual session catalog for later actions. |
| Some static agent entry points cannot be read from this environment. | `https://agentizable.ai/agent/index.md` and `https://agentizable.ai/.well-known/agent-bounties.json` returned HTTP 403 to Python URL reads; the web reader also could not access them. `https://mcp.agentizable.ai/tools` returned HTTP 200. | Record an access limitation, not a universal website outage. Do not change access controls without a separate diagnosis. |
| Posting and reader recovery already have active contributor work. | Open PRs include #1789 (v2 wallet posting), #1764 (read-only API/reader recovery) and #1765 (funding recovery). | Keep this patch confined to discovery diagnostics and agent guidance. It does not solve or replace their deployment work. |

The open-PR search and first REST page were inspected. REST returned 100 open
entries, so it is a bounded queue snapshot, not a claim of full queue review.
A request for GraphQL review/check/mergeability metadata failed with HTTP 502;
REST list records did not supply mergeability. No review decisions were made.
The required maintainer notice remains a local draft outside this checkout;
publishing it requires an explicitly authorized public message.

## Resulting behavior

On an upstream failure, the read-only HTTP inventory adapters include
`agent-bounties/discovery-problem-v1`. Remote MCP clients receive the same
guidance in JSON-RPC error data or MCP tool-error `structuredContent`, following
their existing response path. Text-only tool clients receive both the plain
explanation and serialized guidance. Existing HTTP/JSON-RPC codes, successful
inventory and real argument failures retain their behavior.

The response explains:

> Agentizable's work inventory is temporarily unavailable. Work availability
> is unknown; this does not mean there are no matching tasks.

The problem has `coverage=unavailable`, `degraded=true`, a maximum of one
identical read retry, and an explicit next action. It does not invent empty
`items`, `bounties` or zero counts. Retry guidance is restricted to discovery;
it never applies to claims, signatures, publication, funding or payment-status
calls. The service does not automatically retry. The hosted feed, opportunity
and inventory-summary requests have a 15-second total request timeout.

Example structured guidance (diagnostic text omitted):

```json
{
  "schema_version": "agent-bounties/discovery-problem-v1",
  "error_code": "upstream_unavailable",
  "coverage": "unavailable",
  "degraded": true,
  "retryable": true,
  "retry_policy": {"maximum_retries": 1, "same_arguments": true}
}
```

## Validation

- Core preflight passed.
- Full MCP suite: 123 passed, 4 ignored. The ignored tests require separate
  Postgres or explicit fixture/review inputs; no database behavior changed.
- The new default-suite wire-contract test launches the existing isolated
  fixture against a closed local port, a non-JSON HTTP 503 and a JSON HTTP 503.
  Each case exercises direct HTTP adapters, both MCP eras, the compatibility
  bounty-list tool, actual invalid arguments and diagnostic redaction.
- A text-only client test verifies that structured guidance is also present in
  text content. Unit checks preserve valid empty results and argument errors,
  exclude writes from retry guidance and reject fabricated inventory counts.
- Site checks passed, including 154 Node checks and navigation on 48 pages.
- Agent-discovery contract check, its 10 unit tests, and the 2 MCP registry tests
  passed. `cargo run -p cli -- docs-contract-check` passed across 210 files,
  454 API routes and 158 MCP tools. Formatting and whitespace checks passed.

This is deterministic local validation. It does not demonstrate restored
production inventory, improved conversion or any paid outcome. Production
verification after release should repeat the exact read above and inspect the
new problem metadata during an outage, then confirm that a healthy response
preserves its authoritative inventory and coverage.

## Release and rollback

This is a read-only diagnostic change, class R0. It adds no database migration,
tool name or wallet authority. Roll back by reverting the patch. Publish the
required maintainer notice and recheck overlapping PRs before proposing a public
merge. Restoring live reader availability remains a separate infrastructure
work item.
