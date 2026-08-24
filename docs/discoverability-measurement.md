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
dispatched manually. It:

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

## Credential boundary (separate R3 action)

Code deployment does not provision access. A maintainer must separately create
and grant three narrowly scoped secrets:

- `REPOSITORY_TRAFFIC_TOKEN`: GitHub repository-traffic read access;
- `GSC_SERVICE_ACCOUNT_JSON`: a Search Console restricted-user service account
  for the `https://agentbounties.app/` property; and
- `DISCOVERABILITY_INGEST_TOKEN`: one matching ingestion-only value in Render
  and GitHub Actions.

Never put credential values, Search Console query rows, GitHub path/referrer
rows, or raw provider payloads in issues, workflow logs, public artifacts, or
the public Metrics page.

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
