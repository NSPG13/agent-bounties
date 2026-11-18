# API recovery staging

Set `API_BOOT_MODE=recovery-health-only` to start only the API liveness server.
The absent setting or exact `normal` preserves ordinary startup. Unsupported values
fail before dependency access. Recovery mode also rejects command arguments,
including the database-backed notification diagnostic.

Recovery mode returns before reading database, signer or provider configuration,
building application state, running migrations or starting background jobs. It
does not attach ordinary API routes or analytics middleware. `GET /health` and
`HEAD /health` succeed; every other route/method, including marketplace readiness,
authentication and writes, returns `503` with `ready: false`. Responses have
`Cache-Control: no-store` and `x-agent-bounties-boot-mode: recovery-health-only`.
Liveness retains the ordinary revision/protocol headers and `ok` body. Its
revision header is platform metadata, not independent proof of the built source.

`Dockerfile.recovery-api` builds the same locked public API sources and defaults
to this mode with financial flags false. It has a `CMD` and no `ENTRYPOINT`, so a
platform pre-deploy shell command is not appended as arguments to the API. The
ordinary Dockerfile remains unchanged.

For the pre-deploy command, use the installed executable
`/usr/local/bin/check-recovery-revision` followed by the reviewed 40-character
lowercase commit as its sole argument. The payload is a plain path and argument,
without an additional `sh -c`, quotes or environment expansion. The script reads
`RENDER_GIT_COMMIT` itself and exits 0 only for an exact valid match. Missing,
malformed or mismatched deployed revisions exit 1; malformed expected revisions
or argument counts exit 2. Input values are neither evaluated nor printed.

The first Render attempt used a quoted shell expression that passed local image
checks but failed with exit 127 when the provider treated the expression as a
command name. That failure did not establish a revision mismatch or qualify an
API runtime. The executable avoids that nested quoting boundary. Containers CI
now exercises the actual installed path with exact success/failure codes, plus
one shell dispatch of the plain command. These checks do not establish every
hosting-platform failure or rollback path.

This mode is a startup setting, not a live switch. It does not protect an older
artifact that predates the mode, enable another service, or qualify marketplace
readiness. API/MCP financial signing remains separately disabled by default.

For a new service, the first reviewed deployment should have no database URL,
environment groups, financial keys, payment/provider credentials or RPC URLs.
Keep automatic deploys off and financial flags explicitly false. Pin and verify
the first public source revision using the deployment platform's admission
mechanism before allowing it to start. Verify the native deployment revision,
build recipe, actual process, effective selected configuration and liveness;
exercise functional endpoints to confirm `503`. This gives a later failed
deployment a harmless retained artifact. A later attachment must account for
both candidate and retained command/environment behavior; this document does
not assume saved dashboard settings change an older running artifact.

Switching to `normal` with `DATABASE_URL` is a separate production transition.
Ordinary startup unconditionally replays the database migrations and launches
GitHub draft cleanup and claim maintenance. It also starts posting reconciliation
and managed refund observation when their dependencies are configured. Artifact
verification/cleanup and email delivery start when their storage/provider
configuration is available. Financial signing being off does not make those
database operations read-only. Qualify schema compatibility, intended writes,
job configuration and private credential attachment before that transition.
Exclude financial keys and Stripe execution credentials throughout recovery;
preserve the independent signing, broadcast, checkout and refund holds.

Existing factories, bounty addresses, durable events and integrity quarantines
remain authoritative. A new service does not deploy a contract, clear a reader
quarantine or certify old history. The combined reader still accepts only its
source-pinned Render identity; a replacement needs a reviewed direct ID update
consistent with `ops/public-runtime-services.json`. Domain/callback cutover and
the public release inventory must be reconciled before declaring recovery.

Run `cargo test -p api --test recovery_boot` for real-process containment and
restart tests, and `cargo test -p api recovery_boot::tests` for mode parsing.
The process test supplies invalid dependency credentials and an observed local
connection trap. Recovery must remain live, deny functional requests, and never
contact that trap. Run the normal API tests as the compatibility gate.
Run `python3 -m unittest scripts.test_recovery_revision_guard` for the executable
revision boundary. The copied script is included in public runtime source parity.

## Public read recovery

`API_BOOT_MODE=recovery-read-only` is a separate early startup path. It requires
`DATABASE_URL`, skips migrations and normal application initialization, and
starts no background jobs or analytics middleware. It constructs disabled
authentication, signing, payment and provider state without reading their keys.
Its only routes are `GET`/`HEAD` `/health`, `/readiness/marketplace`,
`/v1/opportunities`, and `/v1/base/autonomous-bounties/feed`. Every other method
or path returns `503`, including OAuth callbacks and evaluation GETs. Only
first-party OPTIONS preflights for these four paths, GET/HEAD and the Accept or
Cache-Control request headers are accepted; this supports the existing homepage
client's `cache-control: no-cache` header without enabling another application route.
Public browser reads permit only the established first-party origins and never
allow credentialed CORS. Existing authenticated UI flows are not restored by
this mode. Responses identify the boot mode and use `Cache-Control: no-store`.

The existing public projection handlers retain their privacy, source coverage,
canonicality, quarantine, verification and freshness rules. Legacy opportunity
requests hydrate real durable state for that request and fail unavailable if
hydration fails; they do not substitute an empty in-memory inventory. Canonical
and offchain source filters use their existing independent durable reads.
Unavailable or quarantined readers remain unavailable: a working API is not
proof of a fresh marketplace, claimable work or payment. No mutation action in a
returned opportunity is enabled by this router.

Every new and reused PostgreSQL connection enters read-only transaction mode
before application queries, with a ten-second statement timeout, one-second
lock timeout, five-second acquisition timeout and at most four connections.
The request boundary is fifteen seconds. This guards accidental durable writes;
it does **not** revoke the database role's privileges or sandbox a compromised
process. Any production attachment must separately identify and authorize the
exact existing credential and role exposure. A restricted read role, where
available, remains preferable; this mode creates no role or grant.

The normal Dockerfile and recovery image's health-only default are unchanged.
Retained public `c3096b440269517b18f7385d7e0a3a5373fb84fa` rejects the new mode
before database access; with its old configuration it remains health-only.
Qualify those candidate/retained configuration paths before attaching a database.
Actual runtime/source, selected configuration, durable-state preservation and
bounded public reads still require separate deployment verification. Login,
reader catch-up, quarantine repair, domain cutover, public release inventory and
API/MCP revision agreement are separate recovery requirements. Signing stays off.

Local tests require a fresh, isolated PostgreSQL fixture. Run
`cargo test -p api --test recovery_reads -- --include-ignored --test-threads=1`,
then `cargo test -p db recovery_read_tests:: -- --ignored --test-threads=1`.
The process test proves no startup migrations, real public data, private-data
filtering, unavailable canonical reads, denied methods and paths, restart and
unchanged table contents. The database test rejects durable DML/DDL and verifies
the policy on fresh and reused connections. Neither test qualifies production
credentials, schema or provider rollback behavior.

## Base mainnet reader cadence

Base's [Denim upgrade](https://www.base.org/blog/denim) uses 200ms mainnet
blocks. Readiness allows a maximum 200-block lag from the independently observed
safe head on `base-mainnet`, retaining the prior approximately 40-second
allowance. Other networks retain 20 blocks. Actual admission, lag-only progress
waiting and the marketplace response's reported limit use the same policy.
The 90-second heartbeat limit, 300-second head timestamp limit, 12-second
observation deadline, exact canonical hashes and quarantine checks remain in
force. Projection reads remain capped at the lesser of the durable cursor and
the provider's safe head. Freshness does not establish payment or finality.

The recovery supervisor passes explicitly selected `BASE_INDEXER_POLL_SECONDS`
and `BASE_INDEXER_CONFIRMATIONS` to both autonomous readers. Unset values retain
their existing defaults. The proposed recovery selection is 15 seconds and 20
confirmations; configuration requires separate deployment verification. The V2
deployment start and its independent chunk limit remain unchanged. Confirmation
depth still combines with the provider-safe cap; it does not replace it. This
change introduces no faster polling or catch-up drain loop. Safe batches may
arrive in jumps, and scan time adds to the poll delay.

Public metrics intentionally retain their separate existing policy: a heartbeat
up to 300 seconds old and a cursor within 20 blocks of that heartbeat's observed
latest block. That metric does not use the independently observed safe head and
can report delay even when the safe-capped readiness check succeeds.

The read-only marketplace monitor accepts only the reviewed 20- or 200-block
response limits with a 90-second heartbeat limit. The 200-block policy requires
an explicit `base-mainnet` network; legacy responses without limits retain 20.
Malformed limits fail unavailable. The generic self-healing policy's independent
20-block alert remains conservative and is unchanged. Neither monitor authorizes
a payment or restart merely because reader freshness succeeds.
