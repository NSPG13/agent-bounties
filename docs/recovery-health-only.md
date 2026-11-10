# API recovery staging

Set `API_BOOT_MODE=recovery-health-only` to start only the API liveness server.
The absent setting or exact `normal` preserves ordinary startup. Any other value
fails before dependency access. Recovery mode also rejects command arguments,
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
ordinary Dockerfile remains unchanged. The Containers workflow builds this
additional recipe and checks both a successful shell override and a failing one.
This validates Docker command behavior, not every hosting-platform failure path.

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
