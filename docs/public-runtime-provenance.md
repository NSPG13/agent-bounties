# Public runtime provenance

Production application releases originate from `NSPG13/agent-bounties` main. The public repository contains the runtime implementation, including managed payment paths and all advertised MCP tools. Internal operational notes may remain private; executable runtime source may not.

`python scripts/check_public_runtime_parity.py --published` fetches public main from the fixed repository and compares source content, Cargo manifests/lockfile, migrations, contracts, programs, Dockerfile, Cargo configuration and embedded workspace Rust inputs. Missing, extra or changed build inputs fail the check. A local `--public-root` comparison is review assistance, never deployment evidence. A private runtime checkout must publish matching source before it can pass the release gate.

`application_release_gate.py` records the public-source check with the repository, disposable PostgreSQL and production-compose gates. Its `verify` operation repeats the public-source check immediately before a release. Records are evidence, not permission to sign contracts or move money.

The `Public Runtime Release` workflow replaces the legacy environment-reconciling deploy controller. It deploys only an exact public-main push whose latest CI and container checks succeeded, verifies every service ID/name/type/workspace in `ops/public-runtime-services.json`, and waits for the exact commit to become live on every service. It verifies both public health revision headers eight times. Before any source migration or deployment, all four marketplace readers must be current and ready; a reader integrity halt or unresolved outage requires a separate investigated recovery before this controller can run. After deployment, the controller requires eight consecutive fresh readiness observations from the exact new revision within five minutes. Its only credential is the Render API key, scoped to the deploy step in the main-only production environment. It does not provision services, access environment values, rotate keys or change payment flags.

The initial manual `migrate_source=true` dispatch changes only repository, branch and native auto-deploy settings on the reviewed service inventory. Every service is validated before any change and all bindings are reread before deploying. Normal releases reject private or unexpected source bindings. A manual dispatch can supply `approved_revision`, which must equal the exact checked-out public-main SHA. This authorizes only that operation and does not change stored expected-live monitor values. Automatic runs retain the production revision-pin check. Update live monitor expectations only after the full release is independently verified, or remove a release pin only when continuous main releases are explicitly intended. Keep the legacy `Render Deploy Recovery` workflow disabled.

For the initial autonomous-history certification, dispatch with both
`migrate_source=true`, `bootstrap_autonomous_readers=true` and the exact reviewed
`approved_revision`. Public-main, CI, container and full-inventory identity checks
still apply, but only the
reviewed base-indexer service changes source and deploys. API, MCP and all other
services keep their current deployments. The receipt ends at
`bootstrap_deployed_pending_acceptance`, never at a verified full release.
Independently verify both autonomous checkpoints, their canonical block hashes,
unchanged historical event counts and successful current reader heartbeats before
dispatching the full migration without the bootstrap flag. Supply the accepted
reader's deployment ID as `accepted_autonomous_deploy_id` to avoid restarting it.
Reuse requires the original public binding, reviewed build recipe, and newest
deployment to be live at that exact ID and approved source SHA; historical matching
deployments, newer pending deploys or mismatches fail before another deploy starts.
A failed or quarantined
reader blocks promotion; do not rewind cursors or clear quarantine to proceed.
Keep the old expected-live pins throughout bootstrap and promotion. The explicit
manual approval names the deployment target separately from observed live state.

Legacy history requires an RPC with historical log coverage. Qualify the oldest
stored events and current cursor before setting a service-local
`BASE_INDEXER_RPC_URL` override through a separately reviewed configuration change.
The posting-indexer launcher applies it only to autonomous-v1; autonomous-v2 keeps
`BASE_MAINNET_RPC_URL`. The release controller never sets either variable. An old
latest-minus-confirmations cursor ahead of safe remains unavailable and uses the
existing bounded retry policy until safe catches up. Regression below an already
certified checkpoint remains a terminal integrity failure.

`Public source provenance` runs hourly. Its public HTTP check needs no credentials and fails when API/MCP revisions differ, are absent, or are not reachable from public main. A separate main-only production-environment job uses the Render credential only for GET requests, verifying the source binding and publicly reachable live revision of every listed service, including workers. Neither observer deploys or repairs production. GitHub administrators and Render operators can bypass controls outside this path; their access remains an administrative trust boundary, and the observer detects resulting public revision drift.

An emergency rollback must restore a reviewed prior tree through a public-main revert, pass the same gates, and account for forward-only database migrations. A green deploy does not prove payment, verifier readiness, inventory replenishment or refund completion.
