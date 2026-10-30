# Discoverability measurement

This document defines the 30-day search and agent-discoverability sprint. Reach
is the optimization objective. Truthful funding, claimability, verification,
and payment claims are release guardrails and must never be traded for traffic.

## Frozen day-zero baseline and day-30 targets

| Signal | Provider window | Day zero | Day-30 target |
| --- | --- | ---: | ---: |
| Google Search impressions | rolling 28 days | 116 | at least 350 |
| Google organic clicks | rolling 28 days | 0 | at least 5 |
| Google average position | rolling 28 days | 7.8 | observe; do not optimize in isolation |
| Captured ChatGPT referrals | first-party rolling 30 days | 19 | at least 38 in rolling 28 days |
| GitHub unique visitors | GitHub rolling 14 days | 202 | at least 300 |
| GitHub unique cloners | GitHub rolling 14 days | 530 | operational context only |
| GitHub page views | GitHub rolling 14 days | 487 | operational context only |
| GitHub clone operations | GitHub rolling 14 days | 31,808 | operational volume only |
| Market-to-funded-opportunity CTR | first-party rolling 30 days | 5.8% | at least 5.8% |

External discovery-route interactions must grow at least 25% from the first
complete seven-day baseline. A2A, MCP, API/CLI, and feed values are interactions,
not unique people, clients, or independent agents.

## Storage and public boundary

Migration `0029_discoverability_measurement.sql` creates two aggregate stores:

- `discoverability_snapshots` retains operator-only provider payloads for 18
  months. Idempotency is the provider, observation time, source window, and
  canonical payload checksum.
- `discovery_route_usage_hourly` stores counts by interface, route family,
  attribution reliability, and UTC hour. It stores no IP address, user agent,
  prompt, task content, wallet, client identifier, or request body.

The operator ingestion endpoint is
`POST /v1/operator/discoverability/snapshots`. It accepts only a dedicated
ingestion token and validated Search Console, GitHub, and first-party payloads.
The API derives the route-interaction snapshot from its own aggregate store;
clients cannot supply or rewrite that provider. The detailed operator report is
`GET /v1/operator/discoverability/report?window_days=30` and requires the
separate operator credential. The ingestion credential is write-only and cannot
read raw snapshots, query/page dimensions, paths, or referrers.

`GET /v1/discoverability/summary` returns a fixed allowlist of delayed aggregate
headlines. It never returns Search Console queries, Google pages, GitHub popular
paths, GitHub referrers, checksums, or raw provider payloads. If any of Search
Console, GitHub, first-party analytics, or route-interaction data is missing or
more than nine days stale, its status is `unavailable` and the website displays
no partial values.

## Collection schedule

`.github/workflows/discoverability-snapshot.yml` runs every Monday and can be
dispatched manually. Manual runs default to `dry_run=true`: collection and
validation only, with no snapshot upload. A scheduled run retains the existing
upload behavior. A manual upload requires deliberately setting `dry_run=false`
and the already-authorized ingestion configuration. The collector:

1. queries 28-day Search Console headline totals through D-3 while retaining an
   overlapping 35-day private query/page recovery window;
2. retrieves the full GitHub 14-day views, clones, popular paths, and referrers;
3. snapshots 28-day first-party acquisition; the API simultaneously snapshots
   30-day route usage and the current seven-day route baseline/comparison;
4. validates nonnegative counts, calculates canonical JSON SHA-256 checksums,
   and uploads idempotently; and
5. logs only provider coverage and aggregate upload status.

Overlapping windows recover one missed weekly run. The operator report derives
and exposes longer coverage gaps rather than silently interpolating them.

First-party response windows end at the API's `generated_at`; their observation
time is captured after the response arrives. A later-generated response must
not be rejected just because collection started earlier. Time-zone-free,
inverted or future response windows remain invalid. Each provider's exact
request is validated before upload or dry-run success, so an invalid provider
does not prevent valid providers from being processed.

The collector requires the documented first-party schema and collections,
integer nonnegative counts, and a defined session-based market click-through
rate consistent with its numerator and denominator. Missing evidence, malformed
counts and an undefined rate are unavailable, not zero. Under the existing
numeric-rate ingestion contract, a zero-denominator rate leaves the
first-party provider unavailable. A measured zero rate with a positive
denominator is valid. An absent event row inside a valid grouped event-count
collection means zero occurrences of that event.

## Diagnose missing snapshots without uploading

```bash
python scripts/discoverability_snapshot.py --dry-run
```

This mode needs no ingestion credential. It collects first-party analytics
and uses existing read credentials for other providers when configured. Missing
provider credentials are named as configuration gaps; private credential
values, exception text, query/page rows and GitHub path/referrer rows are never
printed. Search Console's read-only query API uses POST, but dry-run never
calls the snapshot ingestion endpoint or changes provider configuration.
Exit status remains nonzero when a provider is unavailable or invalid. A
partial dry-run is useful diagnostic evidence, not a healthy combined pipeline.

Normal upload mode stops before collection if `DISCOVERABILITY_INGEST_TOKEN`
is missing. Safe diagnostics distinguish missing configuration from collection
or validation failures. Inspect the relevant workflow's dated step result and
these aggregate diagnostics before drawing a production conclusion; a past
missing-configuration observation does not establish current secret state.

The public summary reads stored snapshots, not the live analytics endpoint.
The API derives its route-interaction snapshot when it accepts an ingestion
batch. Therefore direct first-party analytics and route counters can work while
the public summary has no provider observations. All-null provider timestamps
indicate missing retained snapshots; stale snapshots keep their dated windows.
Do not substitute direct analytics for missing stored provider coverage or
disable the all-provider freshness requirement to make the headline appear.

## Credential boundary (separate R3 action)

Code deployment does not provision access. A maintainer must separately grant
the narrowly scoped provider and ingestion access:

- `REPOSITORY_TRAFFIC_TOKEN`: GitHub repository-traffic read access;
- Search Console: the service account below has Restricted access to the exact
  `https://agentizable.ai/` URL-prefix property, using keyless federation; and
- `DISCOVERABILITY_INGEST_TOKEN`: one matching ingestion-only value in Render
  and GitHub Actions.

### Keyless Search Console configuration

The workflow uses GitHub OIDC and Google Workload Identity Federation to mint
an OAuth access token immediately before collection. It requests only
`https://www.googleapis.com/auth/webmasters.readonly` for 600 seconds. The token
is passed to the collector as `GSC_ACCESS_TOKEN`; it is not a repository secret,
credentials file, exported ambient Google credential, or log artifact. The
workflow runs only on `refs/heads/main`, retains `contents: read`, and adds
`id-token: write` only to the snapshot job. Its Monday schedule and manual
`dry_run=true` default are unchanged.

Set these nonsecret repository variables after the separate access approval:

| Variable | Value |
| --- | --- |
| `GCP_WORKLOAD_IDENTITY_PROVIDER` | `projects/782448447339/locations/global/workloadIdentityPools/agentizable-discovery/providers/github-discovery` |
| `GCP_SEARCH_CONSOLE_SERVICE_ACCOUNT` | `agentizable-search-console@agent-bounties-local-auth.iam.gserviceaccount.com` |

The provider issuer is `https://token.actions.githubusercontent.com`. Use the
provider's default audience check; the pinned Google auth action derives its
audience from the configured provider. Map `google.subject=assertion.sub`, plus
`attribute.repository_id`, `attribute.repository_owner_id`, `attribute.ref`, and
`attribute.workflow_ref` to their corresponding assertion claims. Require this
exact provider condition (the numeric IDs were verified against GitHub):

```text
assertion.repository_id == '1293030696' &&
assertion.repository_owner_id == '189029667' &&
assertion.ref == 'refs/heads/main' &&
assertion.workflow_ref == 'NSPG13/agent-bounties/.github/workflows/discoverability-snapshot.yml@refs/heads/main'
```

Grant only `roles/iam.workloadIdentityUser` on the named service account to:

```text
principalSet://iam.googleapis.com/projects/782448447339/locations/global/workloadIdentityPools/agentizable-discovery/attribute.repository_id/1293030696
```

This grants no project role, service-account-key creation, domain-wide
delegation, Search Console ownership, or permission to change the property.
The ordinary workflow claim is `workflow_ref`; `job_workflow_ref` describes
reusable workflows and must not replace it here. Numeric repository and owner
IDs prevent name-reuse impersonation; reviewed changes to this workflow on
main remain part of the trusted boundary.

If either federation variable is set, the authentication step must succeed;
partial configuration or a rejected identity stops the job before collection.
Do not add `continue-on-error` or fall back to a JSON key after WIF failure.
The collector prefers a supplied access token and cannot refresh it or fall
back after rejection. It rejects Search Console redirects. If collection
outlasts the token, a new authorized run must mint a new token.

The preexisting `GSC_SERVICE_ACCOUNT_JSON` path remains compatible only when no
access token is supplied and the workflow has no federation configuration.
Do not create a key or weaken an organization policy to use that fallback.
With neither credential, safe diagnostics name both alternatives and leave
Search Console unavailable; they never substitute zero traffic for a failed
request.

Official references: [Google GitHub auth action](https://github.com/google-github-actions/auth),
[Google deployment-pipeline federation](https://docs.cloud.google.com/iam/docs/workload-identity-federation-with-deployment-pipelines#github-actions),
and [GitHub OIDC claims](https://docs.github.com/en/actions/reference/security/oidc).

Never put credential values, Search Console query rows, GitHub path/referrer
rows, or raw provider payloads in issues, workflow logs, public artifacts, or
the public Metrics page.

Changing credential grants or property access is separate from a read-only
diagnostic or collector repair. Claim pipeline restoration only after an
authorized ingestion result and fresh public provider observations verify it.
First dispatch the exact reviewed main revision with `dry_run=true`, and verify
all three collected providers with no upload. Then an authorized
`dry_run=false` run can verify ingestion and all four fresh public summary
providers. A fresh stored Search Console window may show zero for a recently
migrated property; compare its D-3 dates before interpreting growth. To contain
a federation incident, disable this workflow or provider and revoke the
service-account-only binding; do not rotate unrelated credentials or enable
the legacy key path automatically.

## Weekly operating rule and day-30 readout

Review coverage before conclusions. Compare matching provider windows with the
frozen baseline and report absolute value, change, target, freshness, and known
coverage gaps. Change at most one title/snippet cluster and one distribution
surface per week. Keep market-to-funded-opportunity CTR at or above 5.8%, and
audit every published funding, claimability, verification, and payment statement.

The day-30 readout must state which targets were met, which changes plausibly
contributed, which evidence is only directional, and whether the zero-false-claim
guardrail held. GitHub clone operations must never be converted into people,
agents, participation, or earnings.

## Verification

```powershell
python scripts/test_discoverability_snapshot.py
python scripts/check-migration-history.py
python scripts/check-render-blueprint.py
node --test scripts/test-metrics-dashboard.js
cargo test -p api discoverability -- --nocapture
cargo test -p db discoverability -- --nocapture
```

With disposable Postgres configured, also run the ignored durability test and
`scripts/check-postgres.ps1`.
