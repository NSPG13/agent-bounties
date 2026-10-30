# Cloudflare website operations

## Search routes and missing pages

Cloudflare Pages redirects file-shaped HTML URLs such as `/about.html` to
`/about` and directory indexes such as `/install/index.html` to `/install/`.
Canonical tags, social URL metadata, structured page URLs and the sitemap use
those final routes. The original HTML files and working legacy links remain
available through the provider's redirects; no API or MCP aliases change.

The top-level `site/404.html` is required. Without it, Pages treats this static
website as a single-page application and returns the homepage for missing
paths. The independent fallback uses fully qualified recovery links because
it can render at any URL depth. It has no analytics, account or wallet scripts,
declares `noindex, follow`, and is omitted from the sitemap. The site gate checks
these source requirements; after deployment, verify an unknown top-level and
nested path return HTTP 404 and that canonical/sitemap targets return HTTP 200
without a redirect. A local file check does not establish live HTTP behavior.

Provider behavior: [Serving Pages](https://developers.cloudflare.com/pages/configuration/serving-pages/).

## Build and publication

Pages builds `main` with `bash scripts/build-static-site.sh` and publishes `site`.
The build verifies wallet assets and origin, prepares licensed fonts and schemas,
validates the reviewed local participation aggregate, and runs site tests. It
makes no GitHub activity collection calls and needs no GitHub metrics token.
The manifest at [site-build.json](https://agentizable.ai/.well-known/site-build.json)
records the source revision, aggregate hash, original observation time and
whether the aggregate was reused or unavailable. A new build never refreshes
an old observation timestamp.

The durable input is the versioned `site/generated/github-participation.json`
in the reviewed release source. The first fresh daily collection was published in
release `5495345`, observed October 29 at 00:07:32 UTC (October 28 locally).
Its exact bytes, original observation time and policy digest were verified on
both production domains and the immutable deployment. This replaces the earlier
recovered snapshot without rewriting its historical evidence. When no trusted aggregate is supplied, the
build uses unavailable with no counts and no invented observation time. A fresh checkout therefore makes no assumption
about a previous deployment's files. Malformed input, raw fields, unreviewed
metadata, wrong schema or changed exclusion policy stop validation; they do not
silently become unavailable.

The existing explicit collector, `python3 scripts/refresh_site_metrics.py`, can
prepare a replacement using a public-repository read-only `GITHUB_METRICS_TOKEN`.
It validates aggregate-only output and binds it to the current exclusion policy,
locks concurrent collectors and atomically replaces the file only on success.
Raw records and the token remain in memory. Use `--output` to write a candidate under the existing private operations
directory, then review and publish only that aggregate with its original
observation windows in the release source. Do not commit
private raw collection records. A failed attempt preserves the last success and
must be reported separately. The existing 3 p.m. task now performs reviewed
daily collection and publication; no incremental collection cache is used.

Public build variables remain the existing Coinbase and WalletConnect project
IDs, analytics measurement ID, `NODE_VERSION=22`, `PYTHON_VERSION=3.12` and
`SKIP_DEPENDENCY_INSTALL=true`. Metrics describe a daily snapshot and show its
actual age. The existing two-hour delay threshold is unchanged; delayed data
cannot supply a current complete account headline. Canonical payment metrics
remain independent of optional GitHub participation coverage.

The redundant Worker's daily 12:17 UTC rebuild trigger was removed after the
first fresh daily artifact publication was verified on October 28, 2026.
Its saved production settings show no cron triggers, and `ops/cloudflare/wrangler.toml`
keeps the trigger list empty so a later deployment cannot restore it by accident.
The Worker code and scoped encrypted hook remain available; do not restore the
schedule. Rebuilding does not collect GitHub activity. The legacy GitHub Pages workflow is retained as manual validation only, with
its deployment job disabled in source. Its live disabled setting must remain
disabled; no paid duplicate push or daily builds are needed. Neither schedule is proof of fresh metrics.
The Worker hook stays restricted to this project and production branch; it has
no signing credentials or general Cloudflare API token. A hook success is not a
completed deployment: inspect Pages status and the live manifest's source,
aggregate hash and observation time. New source pushes still deploy normally.

## Existing 3 p.m. task: trusted aggregate publication

The existing daily task owns this procedure. No new schedule or storage provider
is required:

1. After GitHub review, inspect that day's report watermark. Reuse a successful
   same-cycle collection; do not scan again for every deployment or retry.
2. On trusted reviewed source, use the existing read-only collector with the
   existing authenticated local GitHub credential and `--output` pointing to
   `metrics-aggregate-candidate.json` in that day's private operations directory.
   Pass the token privately through the environment, never in logs or a command
   argument. The competitor collector is a different tool and supplies no
   GitHub participation artifact.
3. Validate the candidate with `prepare_site_metrics.prepare(candidate,
   policy_path)`. Record collection time, exact source/policy and aggregate hash
   in the daily evidence. On failure keep the last success, record failed source
   coverage and do not advance its successful watermark. Never label old data
   fresh or infer that an unavailable collector means zero participation.
4. Copy only the validated aggregate into `site/generated/github-participation.json`
   on a reviewed release branch. Inspect the aggregate-only diff, run preparation,
   audience/dashboard and site tests, and use the existing reviewed merge process.
   Publication is a source change under existing release authority, not authority
   granted by a scheduled collection. Preserve no-publication holds when present.
5. Verify Cloudflare's resulting deployment and compare live manifest/aggregate
   hashes and observation time to the candidate before recording publication as
   verified. Keep the old source artifact until that succeeds. The redundant
   Cloudflare daily rebuild is already retired; keep it disabled.

The first complete daily collection and publication cycle is verified. Each
later run still requires its own source, aggregate and live-publication evidence;
a successful past cycle does not make a future observation fresh.
The full-history collector reads issue/comment pages through REST and batches
review queries for 20 exact PR node IDs at a time. Each review connection fetches
up to 100 records per page; overflow is paginated individually, with a hard
100-page/10,000-review limit per PR. Node identity, repository, complete page
counts, distinct review IDs and cursor progress must all agree. Any missing or
partial response, count drift, unsupported access or limit exhaustion fails the
collection; it never falls back to another full REST scan or treats missing
reviews as zero. The GraphQL request is a fixed read-only query, with no review
text or mutation fields. Bots and deleted authors retain the same metric rules.

A shared `RequestBudget` admits at most 1,200 total REST and GraphQL requests in
210 seconds, including calls from concurrent threads. Each network timeout is
bounded by the remaining time. REST and GraphQL response quota headers must be
valid nonnegative integers when present. A value below 500 stops collection;
a value of 500 permits that completed response but blocks another request from
that quota pool. Missing headers are tolerated. Precheck both quota pools before
an operational run. These checks stop on observed quota and locally reserve the
minimum unit before each shared-reader request; they are not an account-wide
reservation. Other account activity or a higher GraphQL query cost can change
remaining quota between observations. The actual fresh collector is sequential.
No rate-limit retry or broader credential fallback is permitted.
An operational wrapper should pass the same
budget through `refresh(..., budget=budget)` and record
`budget.requests_started`; wrapping only `Reader.page` misses GraphQL requests.
The budget does not authorize more attempts. Keep the daily receipt's existing
exclusive creation and reuse a successful same-cycle aggregate only when its
exact collector source, exclusion policy and content hash match. A source or
policy change requires a separately recorded bounded run, not silent reuse or a
reset of an exhausted cycle. On restart or failure, do not retry in a tight loop.

No incremental raw cache is introduced. Each allowed collection reads the full
current review set, so edits, deleted reviews and deleted authors are reflected
without relying on PR update timestamps to invalidate saved reviews. There is
no cross-run checkpoint to mislabel as a complete new observation. A GitHub
change during pagination may cause a safe failure; this is an observed scan,
not a promise of an atomic repository snapshot. Only a completely validated
aggregate is atomically published, and failed collection never blocks builds.

Keep the previous host available until both production domains serve the exact
source, wallet configuration, notification handler and manifest over valid TLS.
Then disable the old Pages deployment schedule to avoid competing publishers.
API/MCP hosting, mail records, account origins, budgets and signing keys are
separate from this website cutover and remain unchanged.

The public homepage canary also checks the existing account-level Web Analytics
edge injection on `agentizable.ai`. The reviewed exact script is recorded in
`ops/cloudflare/site-canary-edge.json`, including its URL, integrity, public site
identifier, settings and attributes. The account excludes EU visitors, so zero
or one exact beacon is allowed, only as the final direct child of the body. This
is separate from Pages-project analytics and does not enable tracking or change
provider settings. All original asset order/bytes and script attributes/content
remain checked; other, duplicate or changed scripts fail. Re-review the profile
when the provider changes its script. The immutable Pages URL check remains a
separate build check and does not replace the custom-domain check.
