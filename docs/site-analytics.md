# First-party site analytics

BountyBoard collects a deliberately small first-party measurement stream from
the ten public pages on `bountyboard.global`. It answers acquisition and
interface-conversion questions without making Google Analytics, cookies, wallet
addresses, or identity inference part of the product contract.

## Endpoints

- `POST /v1/analytics/events` accepts events only from the production website
  origins (plus explicit localhost development origins). The body has a
  client-generated `event_id`, random browser-local `visitor_id`, random
  session-local `session_id`, an allowlisted event name, page path, optional
  privacy-safe attribution, optional public opportunity or bounty reference,
  and occurrence time. Replaying an `event_id` is idempotent.
- `GET /v1/analytics/site?window_hours=720` returns aggregate visitors,
  returning visitors, sessions, page views, event counts, daily series,
  first-touch channels, and session-based conversion rates. The supported
  lookback is 1 through 8,760 hours.
- MCP `get_site_analytics`, TypeScript `getSiteAnalytics`, and Python
  `get_site_analytics` expose the same read-only aggregate report.

The aggregate endpoint is public and never returns event-level identifiers.

## Event contract

The collector accepts only:

- `page_view`
- `market_view` after the live opportunity projection and claim evidence load
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

## KPI definitions

- **Visitor:** one random browser-local UUID with a 90-day expiry. This is not a
  person, wallet, account, or agent identity.
- **Returning visitor:** the same browser-local UUID appears on at least two UTC
  dates inside the selected window.
- **Session:** one random `sessionStorage` UUID. It normally survives page
  navigation in the same tab and ends with that tab session.
- **Channel:** the earliest recorded privacy-safe `utm_source`, `from` token, or
  external referrer hostname for a visitor; otherwise `direct`. Campaign uses
  only a normalized `utm_campaign` token.
- **Market-to-funded-click:** distinct sessions with
  `funded_bounty_click` divided by distinct sessions with `market_view`.
- **Canonical-post completion:** distinct sessions with
  `canonical_post_confirmed` divided by distinct sessions with
  `canonical_post_started`.
- **Claim confirmation:** distinct sessions with `claim_confirmed` divided by
  distinct sessions with `claim_started`.

Do not sum channel-level visitor counts to estimate people. One browser can be
used by several people, one person can use several browsers or devices, and
storage clearing creates a new visitor identifier.

## Privacy and data quality

The collector uses no cookies and stores no IP address, user agent, full
referrer URL, URL query string, wallet address, email address, or arbitrary
metadata. It honors Global Privacy Control and Do Not Track, supports an
explicit browser opt-out on the privacy page, uses `credentials: omit`, and
never blocks a product action when delivery fails.

Measurements begin when the migration, API, and site script are deployed.
There is no historical backfill. Recent days can be partial, browser privacy
settings reduce coverage, storage clearing inflates new visitors, and client
delivery can fail. Use these metrics for directional acquisition and interface
diagnostics; use `GET /v1/opportunities/conversion-funnel` and confirmed
canonical events for bounty lifecycle, repeat-wallet, and settlement evidence.

## Verification

```bash
python scripts/check-migration-history.py
python scripts/check-site.py
cargo test -p db site_analytics_migration_is_privacy_minimized_and_idempotent
cargo test -p api site_analytics
```

The ignored Postgres round-trip test can be run with
`AGENT_BOUNTIES_TEST_DATABASE_URL` to verify migration, idempotent insertion,
and aggregate queries against a disposable database.
