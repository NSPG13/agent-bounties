# Discovery growth cycle

Use an evidence-backed cycle to improve useful discovery across search, AI
answers, agent clients and social referrals. Select a small fixed set of
opportunities, write acceptance checks, implement, and distinguish local
validation from publication and externally observed results. Do not treat
traffic, tool requests, plans or drafts as payments or independent users.

## Evidence sources

- [Site analytics](site-analytics.md) defines the public first-party browser
  and interface aggregates. Browser-local identifiers are not people. MCP
  requests are not independent agents; unexcluded operator tests may be counted.
- [Discoverability measurement](discoverability-measurement.md) defines private
  provider snapshots. Its historical sprint targets are not a new baseline.
  A public `unavailable` state stays unavailable even when one upstream source
  can be inspected directly.
- [Opportunity conversion](opportunity-conversion-analytics.md) defines the
  unfunded-publication cohort separately from canonical marketplace activity.
- Search Console domain properties must be recorded separately during a domain
  move. Record selected dates, last update, search type and actual coverage.
  An empty new-domain report does not establish failed migration or zero
  visibility after the move.
- GA4 is consent-dependent acquisition evidence, not the product-funnel source.
  Record property, dates, dimensions and definitions; do not merge its user or
  session totals with first-party identifiers.
- Social public reactions/comments are engagement observations, not impressions,
  reach or conversion. Unavailable account analytics remain unavailable.

## Reproduce a baseline

Save receipts outside the public repository; provider query/page dimensions
and private account reports must not be published in PRs or workflow logs.

```bash
python scripts/discovery-growth-baseline.py --output /tmp/growth-baseline.json
python scripts/agent-discovery-benchmark.py --output /tmp/agent-discovery.json
python scripts/check-social-preview.py --live-image
```

The first command performs four fixed public GETs and never uploads snapshots,
provisions credentials or adds tracking. It retains the rolling 28-day report
and retrieves enough history for two complete 28-day UTC windows. Only additive
event counts are compared; daily visitors and sessions are never summed into
period identities. Missing dates, invalid rates and unavailable providers must
remain explicit. The rolling totals and calendar totals have different bounds.

The agent benchmark performs bounded synthetic read-only probes. Use an existing
scoped `AGENT_BOUNTIES_ANALYTICS_EXCLUSION_TOKEN` when available and check the
server acknowledgement; a self-test user agent alone does not exclude traffic.
Do not provision broader credentials as part of a read-only benchmark.

Record manual observations against the fixed cases in
[`ops/distribution/discovery-query-cases.json`](../ops/distribution/discovery-query-cases.json).
Save timestamp, provider/engine, locale, exact input, returned links, brand
accuracy, destination, and coverage limitations. A web-search result is not a
ChatGPT/Claude/Gemini answer. Record direct answer-engine cases as `not_run`
unless actually observed. Do not manufacture citations, ranking or visibility
percentages from a limited query tool.

## Prioritization and attribution

Fix broken discovery and misleading claims before increasing distribution.
Choose changes by observed impact, evidence, effort and risk. If no matching
ready-to-earn inventory exists, direct builders toward useful integration and
posting workflows without promising immediate work or earnings.

Use clean canonical destination URLs and bounded `utm_source`/`utm_campaign`
tags for distribution. The first-party collector may normalize sources; use
distinct campaign values to distinguish platform-specific experiments. It does
not retain arbitrary UTM content or full referrer URLs. Tags are self-declared
attribution, not proof a campaign caused an outcome.

Respect the existing weekly operating rule: change at most one title/snippet
cluster and one distribution surface per week. Technical consistency repairs
and truthful expired-event corrections should be logged separately. Preserve
nonrenewal/cancellation instructions and never activate paid campaigns as an
implicit part of growth work.

## Release and observation

Before release, check the current PR queue for overlapping work, run the source
checks, and prepare a reviewable notice and PR. Publication needs the relevant
existing user authorization and release gates; permission to collect evidence
does not grant permission to merge, deploy or post.

```bash
python scripts/test_discovery_growth_baseline.py
python scripts/test_agent_discovery_benchmark.py
python scripts/check-agent-discovery-contract.py
python scripts/check-public-handoffs.py
python scripts/check-site.py
python scripts/check-social-preview.py
git diff --check
```

After authorized deployment, record the live site-build revision and verify:

- Each sitemap destination returns 200 without redirect and matches its
  canonical, social URL and structured page URL.
- Unknown top-level and nested paths return 404 with useful recovery links.
- Legacy website aliases still redirect with path/query intact; API/MCP aliases
  remain usable under their own compatibility contract.
- Social image URLs return the expected bytes; static metadata is present.
  Platform-specific preview rendering remains unverified until observed there.
- Agent discovery and safe representative read-only operations still work.

Separate `drafted`, `committed`, `PR opened`, `merged`, `deployed`, `submitted`,
`indexed`, and `externally observed`. A source check is not a live release check.
Compare results only after equivalent observation windows, account for the
domain move and product/inventory changes, and retain a prioritized next-cycle
backlog. Do not attribute baseline trends to changes not yet deployed.
