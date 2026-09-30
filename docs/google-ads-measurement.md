# Google Ads outcome measurement

This implementation is disabled by default and covers autonomous-v1 only. See
[the design, privacy boundary and launch gates](adr/0006-consented-google-ads-outcomes.md).

## Configuration (API only)

| Setting | Default / meaning |
| --- | --- |
| `GOOGLE_ADS_MEASUREMENT_ENABLED` | false; accepts consented website clicks when true |
| `GOOGLE_ADS_VALIDATE_ENABLED` | false; sends eligible events with validateOnly if export is off |
| `GOOGLE_ADS_EXPORT_ENABLED` | false; sends real eligible conversion events |
| `GOOGLE_ADS_MEASUREMENT_KEY` | dedicated 32-byte AES key, 64 hex characters |
| `DISTRIBUTION_ATTRIBUTION_SIGNING_SECRET` | existing acquisition HMAC secret |
| `GOOGLE_ADS_CUSTOMER_ID` | actual ten-digit Ads customer ID, no hyphens; not URL ocid |
| `GOOGLE_ADS_FUNDED_ACTION_ID` | numeric External bounty funded import action ID |
| `GOOGLE_ADS_PAID_ACTION_ID` | numeric Verified bounty paid import action ID |
| `GOOGLE_ADS_OAUTH_CLIENT_ID`, `GOOGLE_ADS_OAUTH_CLIENT_SECRET`, `GOOGLE_ADS_OAUTH_REFRESH_TOKEN` | separately authorized Data Manager credentials |

Use `https://www.googleapis.com/auth/datamanager`, not the website login scope or
an assumed Google Ads API credential. Obtain and store credentials through the
approved private workflow; never paste them into a public issue, report or command
history. Validation mode still shares identifiers. No remarketing or enhanced
lead/user-data uploads are enabled.

## Private API

All website writes require an allowed first-party Origin. Capture requires the
new explicit consent version, a random 64-hex nonce, bounded campaign token,
`identifier_kind`, `click_id`, and optional existing `acquisition`. Responses return
only a signed opaque acquisition and its true expiry. Capture is retry-safe.

- `POST /v1/distribution/website-acquisitions`
- `POST /v1/distribution/website-handoffs`: acquisition, operation_id, bounded stage.
- `POST /v1/distribution/website-consent/revoke`: acquisition; allowed even with
  GPC/DNT or the capture flag disabled. Erases identifiers and suppresses exports.

These endpoints are browser/private measurement plumbing, not agent tools. Do not
put click IDs in MCP requests, bounty descriptions, prompts or agent-visible feeds.

Operator authentication is required for:

- `PUT /v1/operator/google-ads/costs`: one verified campaign-day, body
  `{ "campaign":"exact-landing-campaign-token", "day":"2026-09-30",
  "cost_micros":0, "clicks":0 }`. Zero is valid only when verified from Ads, not
  a placeholder. Re-importing replaces that day's figures without duplication.
- `GET /v1/operator/google-ads/report?start=<RFC3339>&end=<RFC3339>`: end exclusive,
  maximum 366 days. Use midnight in the verified account's UTC-06:00 timezone
  (06:00 UTC) and import every day, including real zero-spend days, for valid CAC.

Use the **same campaign token** as the landing page (`utm_campaign`, otherwise
`gad_campaignid`, otherwise the named landing-page fallback). Confirm the mapping
against each campaign's actual final URL before cost imports; never join by a
similar-looking display name. Existing September totals must not be turned into
invented daily rows. Import the actual Ads day-level export.

The report separates original-campaign results from later-click assists and
includes wallet-review join coverage, unknown-source totals, funnel stage counts,
queue errors, processing reasons and costs. Missing spend is null. CAC is null
when cost dates are incomplete or there are no outcomes. It is spend divided by
outcomes in the selected period, not a causal or lifetime cohort estimate.
Funnel stages are counts of observed steps, not assertions that absent events are
confirmed abandonment. Google-credit results remain null until separately proven
by Google's reporting. LTV is not reported.

## Recovery

Disable export/validation to stop sends without disrupting posting. Keep this
compatible release running so withdrawal and 90-day erasure continue. Watch
pending/rejected/suppressed and processing reason counts. Resolve destination,
OAuth or policy errors before retrying. A reviewed retry must reuse the same
outbox UUID; never delete/recreate rows to get a fresh conversion ID.

`accepted_now_ineligible > 0` requires investigation of revoked consent, late
wallet classification or corrected canonical history. Do not claim an automatic
Google retraction. Backup restoration must apply erasure before exports resume.

## Local checks

```sh
node --test scripts/test-google-ads-measurement.js scripts/test-posting-session.js
cargo test -p api google_ads::tests
cargo test -p api ad_recovery
# Set AGENT_BOUNTIES_TEST_DATABASE_URL to an isolated disposable PostgreSQL.
cargo test -p db google_ads_postgres -- --ignored --nocapture
python scripts/check-migration-history.py
python scripts/check-site.py
```

No local fixture is uploaded to Google. Passing local checks does not establish
production wallet usability, >=95% site-wide coverage, live conversion credit or
permission to spend.

Google primary references:
[send events](https://developers.google.com/data-manager/api/devguides/events/google-ads/offline/send-events),
[retrieve processing status](https://developers.google.com/data-manager/api/reference/rest/v1/requestStatus/retrieve).
