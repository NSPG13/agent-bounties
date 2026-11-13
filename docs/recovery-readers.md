# Separate recovery readers

`Dockerfile.recovery-indexers` builds the dedicated `recovery-reader` binary.
It never enters the general worker dispatcher, runs migrations, constructs
signers or starts notification delivery. The supervisor clears inherited
environment variables for each process and allows only the configuration that
reader needs. Autonomous v1 and v2 use the existing canonical ingestion,
checkpoint, heartbeat, reorg and bounded provider-recovery logic. The recovery
caller explicitly suppresses discovery enqueue after ingestion; ordinary worker
callers retain their existing enqueue behavior. The creator reader uses its
existing ingestion and safe-head recovery logic.

The reader writes ingestion state and health observations to PostgreSQL. It is
not a read-only database connection. Existing invalidated checkpoints remain
authoritative, and this binary does not clear them. Provider lag and rate limits
retain the existing bounded backoff behavior. An integrity halt waits for
operator investigation; it does not start a repair or signing operation.

## Artifact admission

Every production image requires three nonsecret build inputs:

- `RENDER_SERVICE_ID`: the newly allocated native service identity.
- `RENDER_GIT_COMMIT`: the exact public source revision being built.
- `RECOVERY_READER_ARTIFACT_CAPABILITY`: `standby` or `reader`.

The Docker build rejects missing or malformed identities and all existing
service IDs in `ops/recovery-reader-denied-services.txt`. Rust embeds the three
values in the binary. The build inputs are not private keys or access tokens.
An ordinary workspace build without these inputs is inert and cannot index.

The initial production artifact must have `standby` capability. Its immutable
capability refuses active mode even if later runtime flags request activation.
This keeps a retained standby artifact inert if a subsequent active build fails.
At runtime the baked ID and revision must equal `RENDER_SERVICE_ID` and
`RENDER_GIT_COMMIT` before any database or RPC access. No runtime expected-ID
override changes the compiled values. `--build-identity` reports these public
values; `--check-binding` checks admission without dependency access.

`RECOVERY_READER_MODE` defaults to `standby`. Active mode also requires a
`reader` artifact and correct runtime binding. The supervisor starts autonomous
v1, and starts creator/v2 only when their respective reviewed enable flags are
exactly `true`. V1 and v2 factories and the v2 deployment block are fixed. When
one child exits, the supervisor terminates and reaps the other children.

These source controls do not attest that provider metadata is unforgeable.
Before activation, a separate admitted recovery procedure must check native
creation identity, workspace, actual build artifact, source and recipe, settings,
credentials, retired service containment and runtime identity. A local build or
one successful readiness request does not authorize that transition. Signing
must remain disabled throughout recovery verification.

## Held public inventory

The presence of `transition_hold` in `ops/public-runtime-services.json` blocks
the ordinary public-runtime deployment controller. This applies even to null,
false or malformed values. The CLI checks it before approval checks, public CI,
credential lookup, provider clients, readiness or receipt creation. Imported
`prepare`, `prepare_release` and `repair_inventory` paths enforce the same hold.
It has no expiry, runtime override or manual-release bypass.

`scripts/materialize_recovery_inventory.py` is a pure post-allocation transform.
It accepts only the frozen base bytes and the exact native-binding shape. Its
only variable value is the once-bound newly created reader ID. It changes six
reviewed fields: API and reader IDs, names and Dockerfile paths. It preserves
the hold, actual paused Blueprint resource IDs and historical autonomous
bootstrap ID. It rejects old or duplicate IDs, extra edits, changed workspace,
missing or misspelled hold, a second substitution and changes to the frozen base.
No live reader ID is invented or stored in the source template.

The binding JSON is supplied only after the separate recovery procedure verifies
its immutable native creation receipt. The JSON alone is not evidence of creation
or authorization. The transform cannot create resources, resolve dynamic IDs,
call ordinary release preparation or deploy anything. Its output is a reviewable
held inventory, not a release receipt. The separate procedure must preserve the
retired service identities and check that they remain stopped.

Because `ops/` is included in public runtime provenance, materializing the final
inventory changes the source digest. The active build must bind the exact final
public revision; it need not equal the earlier standby revision. The initial
inert build and later active build are separate operations. Removing the hold is
another reviewed source change: review every active service, all held/retired
exclusions and ordinary release dependencies first. Ignored row status fields
cannot keep a service out of the ordinary controller's deployment loop.

The hold affects this controller only. It does not retroactively stop an older
controller, disable services or prove production recovery. Operator/run
coordination, routing and TLS, all required readers' freshness, public API/MCP
behavior and independent acceptance remain part of production verification.

## Verification gates

Lightweight local gates cover the actual shell supervisor, build-input checks,
ordinary-release compatibility, the global hold and strict inventory transform.
The required PostgreSQL suite compiles a synthetic reader-bound binary and
executes it against an isolated database and local RPC server. That test checks
no migrations, eligible ordinary discovery enqueue versus zero recovery enqueue,
canonical persistence, restart from the durable checkpoint, quarantine retention
and absence of unrelated durable writes.

The Containers workflow builds standby A, reader A and reader B sequentially
with shared Docker cache. `scripts/check-recovery-reader-images.py` checks the
actual images' embedded identities, immutable capability, A-to-B cache changes,
runtime identity mismatch rejection, inert defaults and direct predeploy command
execution. These are fixture identities only. Production native identity and
final source acceptance still require independent checks after deployment.
