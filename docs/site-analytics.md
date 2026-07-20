# Site analytics

Agent Bounties keeps a deliberately small first-party measurement stream from
the public pages on `agentbounties.app`. It is the authoritative product-funnel
source because it can distinguish confirmed interface transitions without
sending wallet addresses or bounty evidence to an advertising platform.

GA4 is an optional acquisition layer. It loads only after explicit browser
consent, disables advertising signals and ad personalization, and receives only
page views plus allowlisted interface event names and page paths. The site never
sends wallet addresses, bounty contracts, evidence, payments, email addresses,
or task content to GA4.

The Pages deployment reads the public `G-...` measurement ID from the
`GA_MEASUREMENT_ID` repository variable and writes it into
`site/analytics-config.js` in the deployment artifact. An empty variable keeps
GA4 disabled without affecting first-party analytics.

## Endpoints and compatibility

- `POST /v1/analytics/events` accepts events only from the production website
  origins (plus explicit localhost development origins). The body has a
  client-generated `event_id`, random browser-local `visitor_id`, random
  session-local `session_id`, an allowlisted event name, page path, optional
  privacy-safe attribution, optional fixed context, optional public opportunity
  or bounty reference, and occurrence time. Replaying an `event_id` is
  idempotent. The body cannot supply `site_host`; the server derives that
  dimension from the exact allowlisted `Origin` and canonicalizes `www` to the
  apex host.
- `GET /v1/analytics/site?window_hours=720` returns aggregate visitors,
  returning visitors, sessions, page views, event counts, daily series,
  first-touch channels, current-touch channels, fixed-context breakdowns, the
  allowlisted site-host breakdown, original session rates, and ordered
  conversion cohorts. The supported
  lookback is 1 through 8,760 hours.
- MCP `get_site_analytics`, TypeScript `getSiteAnalytics`, and Python
  `get_site_analytics` expose the same read-only aggregate report.

The v2 aggregate response preserves every v1 field. TypeScript callers accept
the `agent-bounties/site-analytics-v1` or `agent-bounties/site-analytics-v2`
schema and treat v2-only aggregates as optional, so a rolling deployment does
not break older API instances. The API must emit v2 only after migration 0012
has been applied. The aggregate endpoint is public and never returns event-level
identifiers.

The `hosts` rows group traffic and interface events under one of
`bountyboard.global`, `agentbounties.app`, `localhost`, or `unknown`. `unknown`
contains rows recorded before the host dimension existed; it is never selected
from a production request. The response contains aggregate counts only and
never exposes event, visitor, or session identifiers.

## Event contract

The collector accepts only:

- `page_view`
- `market_view` after the live opportunity projection and claim evidence load
- `opportunity_exposed` after one eligible live opportunity satisfies the
  visibility rules below
- `funded_bounty_click` on a canonically funded, claimable card
- `unfunded_post_started` and `unfunded_post_completed` for compatible future
  first-party no-wallet publishing interfaces
- `canonical_post_started` and `canonical_post_confirmed`
- `funding_started`
- `claim_started` and `claim_confirmed`

`canonical_post_confirmed` is emitted only after indexed
`CanonicalBountyCreated`. `claim_confirmed` is emitted only after indexed
`BountyClaimed`. These interface events are useful for diagnosing user flow,
but the canonical event index remains authoritative. Only confirmed
`BountySettled` proves solver payment.

### Fixed context fields

Every event may include these optional, normalized tokens:

- `placement`: the stable UI surface, such as `live-market` or `home-featured`.
- `variant`: a predeclared presentation or experiment variant. It is not free
  text and must not contain a visitor identifier.
- `opportunity_class`: a deterministic product class, such as
  `funded-claimable`; it must not contain task content.

The browser also sends `current_source`, `current_campaign`, and
`current_referrer_host`. These six values are nullable and constrained to short,
privacy-safe tokens or a hostname. Arbitrary metadata is not accepted.

`site_host` is different from these client context fields. It is not accepted
in the JSON body. The API maps only the exact approved request origins to
`bountyboard.global`, `agentbounties.app`, or `localhost`, and the database
enforces that finite set plus the migration-only `unknown` value.

### Opportunity exposure semantics

Rendering an element is not enough to count an exposure. The site tracks
`opportunity_exposed` only when all of the following are true:

1. The element is explicitly marked
   `data-analytics-exposure="opportunity_exposed"` after live canonical data and
   its eligibility evidence have been rendered. Placeholder, loading, fallback,
   demo, and static snapshot cards must not carry this marker.
2. It has a valid public `data-analytics-opportunity-id` or
   `data-analytics-bounty-contract` so later events can match the same work.
3. At least 50% of the element remains inside the viewport for one uninterrupted
   second while the document is visible. CSS-hidden elements do not qualify.
4. The same page-path, opportunity/contract, placement, variant, and opportunity
   class tuple has not already qualified in the current tab session.

An `IntersectionObserver` applies the rule to both initial and dynamically
inserted cards. Unsupported browsers do not synthesize exposures. Leaving the
viewport or hiding the tab resets the one-second timer. The event records an
observed interface opportunity, not a unique person and not proof that the user
read or understood the card.

## KPI definitions

- **Visitor:** one random browser-local UUID with a 90-day expiry. This is not a
  person, wallet, account, or agent identity.
- **Returning visitor:** the same browser-local UUID appears on at least two UTC
  dates inside the selected window.
- **Session:** one random `sessionStorage` UUID. It normally survives page
  navigation in the same tab and ends with that tab session.
- **First-touch channel (`channels`):** the earliest recorded privacy-safe
  `utm_source`, `from` token, or external referrer hostname for a visitor;
  otherwise `direct`. It stays fixed for the browser identifier's 90-day life.
- **Current-touch channel (`current_channels`):** the acquisition values active
  for the tab session. A later explicit UTM, `from`, or external referral starts
  a new current touch; ordinary internal navigation retains the session value.
- **Context (`contexts`):** raw event, session, and visitor counts grouped by the
  exact nullable `placement`, `variant`, and `opportunity_class` tuple.
- **Site host (`hosts`):** aggregate traffic and interaction counts grouped by
  the server-selected first-party origin. This makes old-domain and new-domain
  activity comparable without trusting a caller-supplied hostname.
- **Legacy session rates (`rates`):** the original v1 distinct-session ratios.
  They remain available for continuity but do not enforce event order or
  opportunity identity.

Do not sum channel-level visitor counts to estimate people. One browser can be
used by several people, one person can use several browsers or devices, and
storage clearing creates a new visitor identifier.

The domain cutover is an explicit identity-series break. `localStorage` and
`sessionStorage` are origin-scoped, so the same browser receives a new visitor
and session identifier on `agentbounties.app`; the API neither links nor
deduplicates identities across the old and new hosts. During an overlap or
redirect period, summing host-level visitors may double-count the same browser.
Compare sessions, page views, exposures, and ordered actions by host, but report
cross-host unique visitors as separate browser-origin identifiers rather than
people retained or lost.

## Ordered conversion contracts

`ordered_conversions` closes the main ambiguity in the legacy session rates.
For each row, denominators are start events inside the selected report window.
`numerator_events` counts those start events for which at least one later
matching outcome exists. The session and visitor numerator/denominator fields
are the corresponding distinct browser identifiers, and all raw counts are
returned even when a rate would have a zero denominator.

The ordered contracts are:

1. `opportunity_exposed -> funded_bounty_click`: the outcome occurs at or after
   the exposure in the same session and has the same non-null bounty contract or
   opportunity ID. There is no separate clock window beyond the session.
2. `opportunity_exposed -> claim_confirmed`: the outcome occurs at or after the
   exposure for the same visitor, has the same non-null bounty contract or
   opportunity ID, and is no more than 24 hours later.
3. `canonical_post_started -> canonical_post_confirmed`: the outcome occurs at
   or after the start for the same visitor and is no more than seven days later.
   When both events contain a bounty contract, the contracts must match; a
   missing contract on either event does not prevent the match because the
   contract may not be known at flow start.

Outcomes are searched through report generation time, including outcomes after
the report window start. Consequently, the newest 24-hour and seven-day start
cohorts have not had their full opportunity to convert. These are observational
browser cohorts, not causal attribution: compare variants or placements only
after checking traffic mix, sample size, and cohort maturity.

## Privacy and data quality

The first-party collector uses no cookies and stores no IP address, user agent,
full referrer URL, URL query string, wallet address, email address, or arbitrary
metadata. It honors Global Privacy Control and Do Not Track, supports an
explicit browser opt-out on the privacy page, uses `credentials: omit`, and
never blocks a product action when delivery fails.

Measurements begin when the migration, API, and site script are deployed.
There is no historical backfill. Recent days can be partial, browser privacy
settings reduce coverage, storage clearing inflates new visitors, and client
delivery can fail. Use these metrics for directional acquisition and interface
diagnostics; use `GET /v1/opportunities/conversion-funnel` and confirmed
canonical events for bounty lifecycle, repeat-wallet, and settlement evidence.

Exact-Origin allowlisting is a browser boundary, not authentication. A client
can forge an Origin outside normal browser enforcement, generate arbitrary
UUIDs, or submit plausible interface events. Event-ID idempotency limits exact
replays but cannot prove a person, action, or economic outcome. Public host
aggregates can therefore be spoofed and must be treated as directional. Use
confirmed canonical chain events for funded, claimed, settled, and paid truth;
only `BountySettled` proves solver payment.

GA4 can use cookies and Google can process network, device, and usage data after
consent. Declining GA4 does not affect the product. Global Privacy Control, Do
Not Track, explicit opt-out, or `?analytics=off` prevents GA4 from loading.

## Verification

```bash
python scripts/check-migration-history.py
python scripts/check-site.py
cargo test -p db site_analytics_migrations_are_privacy_minimized_and_idempotent
cargo test -p api site_analytics
```

The ignored Postgres round-trip test can be run with
`AGENT_BOUNTIES_TEST_DATABASE_URL` to verify migration, idempotent insertion,
fixed-context and site-host aggregation, and ordered matching against a
disposable database.
