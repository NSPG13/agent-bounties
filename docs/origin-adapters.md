# GitHub and Linear origin adapters

`github-app::origin` is the provider-neutral contract for converting source
issues into reviewable bounty drafts and returning reconciled results to the
originating issue. It is a pure planner: it performs no GitHub, Linear, wallet,
verifier, contract, or settlement writes.

## Inbound draft contract

Both adapters return `OriginDraftPlan` with the same normalized fields:

- source provider, workspace, stable external id, display id, and HTTPS URL;
- trigger, title, goal, explicit acceptance criteria;
- solver reward, verifier reward, and complete target;
- verifier instructions and fields still requiring review; and
- a stable provider-event idempotency key.

The GitHub adapter wraps the existing `/agent-bounty create <amount> USDC`
planner. A second planner accepts authenticated GitHub assignment and mention
webhook projections with a stable provider event id and explicit solver
reward. Both extract only explicit `Acceptance criteria` and
`Verifier`/`Verification` Markdown sections, keep absent fields in review, and
deduplicate webhook deliveries.
The Linear adapter accepts an already
authenticated webhook projection and extracts only explicit `Goal`,
`Acceptance criteria`, and `Verifier`/`Verification` Markdown sections. It does
not infer payout conditions from prose. Public origin drafts require a solver
reward of at least 2 USDC.

## Intentional provider-execution boundary

`github-app::origin::runtime` is the deployable request-planning boundary. It:

- verifies GitHub `sha256=` and Linear unprefixed SHA-256 HMAC signatures over
  the exact raw body with a 1 MiB limit;
- accepts only GitHub issue assignment/app mention and Linear issue
  delegation/agent mention projections;
- invokes the shared review-only adapters after authentication; and
- translates status/proof and close intents into exact GitHub REST or Linear
  GraphQL request plans with stable idempotency keys and dependency ordering.

GitHub deduplication identities come from signed event fields, not solely from
the unsigned delivery header. Outbound batches contain exactly one stable
status upsert and, only for complete evidence, one dependent issue close.

The request plans contain no bearer token and perform no network write. They
name only the credential kind an executor must inject. Provider URLs, methods,
headers, mutations, identifiers, status-comment markers, comment size, and the
proof-before-close dependency are allowlisted and validated.

This is still not a complete hosted multi-tenant GitHub or Linear integration.
Remaining provider infrastructure must register apps, complete OAuth/install
flows, encrypt installation/access tokens, rotate webhook secrets, map tenants
to configured rewards and Linear completed-state ids, durably persist event and
operation idempotency, discover an existing status-comment id, execute request
plans with rate-limit/backoff handling, verify provider responses, and expose
health/audit telemetry. The canonical lifecycle indexer must separately supply
trusted funding, claim, submission, verification, and settlement evidence.
None of that grants the provider runtime wallet or payment authority.

A ready draft is
still not published, funded, claimable, verified, accepted, or settled. The
first-party review and wallet flow owns those later steps.

## Outbound result contract

`plan_origin_progress_callback` upserts the same stable status comment for
draft, canonically funded, canonically claimed, and canonically submitted
states. Canonical states require an HTTPS indexed-evidence URL and never close
the origin issue.

`plan_origin_result_callback` accepts the source, canonical bounty id, status
URL, artifact URL, precommitted-verifier evidence, and canonical settlement
receipt. It returns ordered `OriginWriteOperation` values:

1. upsert one stable status/proof comment;
2. close the source issue only when the artifact is HTTPS, verification passed
   and matched the committed policy, and the receipt identifies a confirmed
   canonical `BountySettled` or `CompetitionSettledV2` event.

The close operation depends on the status-comment idempotency key, so an
authorized worker must publish the receipt before closing the issue. Replays
reuse stable operation keys and markers. Invalid, pending, or failed evidence
produces an open-issue status plan rather than a false conversion.

The receipt planner validates shape and evidence boundaries; it does not query
the chain. Its input must come from the lifecycle-complete canonical indexer,
not from a provider comment, signature, transaction broadcast, transaction
hash alone, database intention row, or AI output.

## Authority boundary

Every plan exposes an all-false `OriginAuthorityBoundary`. Origin adapters:

- never receive or store private keys;
- never request or produce wallet signatures;
- never fund, claim, verify, accept, pay, or settle;
- never call provider APIs; and
- never close an issue without ordered proof publication plus valid
  verification and confirmed canonical settlement evidence.

Run the focused contract tests with:

```bash
cargo test -p github-app --test origin_adapters
```
