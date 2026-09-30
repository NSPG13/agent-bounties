# Broad verification and saved evidence

This release candidate adds reusable checks and durable evidence. It is not an activated V3 payment profile. Existing contracts, clocks, verifier quorums and signed payment rules continue to apply. No model provider is used.

A winning-submission page records an outcome and may link to a solver's files. A link is not a retained copy. Private S3-compatible storage keeps the exact bytes used for review, with a SHA-256 manifest and PostgreSQL permissions. It keeps those bytes available when an external link changes or disappears. The production default is a private R2 Standard bucket; no bucket is automatically provisioned.

## Reuse before adding checks

This extends existing verification. It does not replace the contract, signature
or payment systems, and it does not introduce another sandbox implementation.

| Existing capability | Addition in this release candidate |
| --- | --- |
| Submission references and declared digests | Store and read back the exact files; derive the manifest from received bytes. |
| `DockerCliRegressionExecutor` and its bounded policy | Reuse that executor for committed tests against stored submissions. |
| Supported deterministic and GitHub checks | Add reusable required-file, text, JSON, CSV and cross-file rules where applicable. |
| Authorized verdicts, canonical history and winning pages | Attach saved criterion explanations and exact supporting files to the existing flow. |

Select only checks relevant to the accepted requirements. A file hash cannot
prove quality, a passing test cannot prove coverage, and an advisory report
cannot approve payment. Existing funded verification rules remain authoritative.

## Creator and solver flow

1. Read the brief and query the versioned check catalog. Select checks that actually apply. Do not use CAD-specific claims for other work.
2. Map every criterion to required checks and any questions still needing review. Diagnostics and ranking cannot stand in for required checks. Ask for missing details instead of inventing them.
3. Show the exact checks, parameters, test code, pinned image, limits, remaining questions, reviewers and actual deadlines to the creator before funding. Structural validation cannot prove the brief was understood correctly.
4. Put that accepted plan in the immutable funded terms. The same plan goes to solvers. The self-check endpoint accepts exploratory plans too, so a self-check is never evidence that the funded plan was followed without comparing its `plan_hash` to the funded commitment.
5. Upload the actual files, or import selected paths from an exact public GitHub commit. Preserve the returned artifact ID and manifest hash. Reusing a key with changed content is a conflict.
6. Run the shared checks on the saved artifact. Read every result and remaining review question. A solver's claimed test output is not used as a runner receipt.
7. For supported legacy contracts, use the artifact's prepare-submission endpoint. It reads the stored bytes back before building unsigned calls. Wallet signing stays on first-party pages. Confirm the canonical submission before publishing file links.
8. The authorized reviewer or accepted assigned agent saves each criterion's reason before the verifier wallet signs. Assignment does not grant signing authority. Quorum and canonical settlement remain authoritative.

Legacy ordinary submission endpoints remain compatible with funded terms. New mixed-verification funding must stay unavailable until its factory, immutable plan binding, capacity admission, indexer and review gates are released together. The experimental V3 source is not a production deployment.

## Check claims

| Family | Checks | What passes | What remains unproven |
| --- | --- | --- | --- |
| Delivery and integrity | `integrity_v1` | Exact saved bytes match the manifest | Authorship, quality, truth |
| Required contents | `contents_v1` | Named files and nonempty folders exist | Correct contents |
| Structured rules | `text_v1`, `json_v1`, `csv_v1` | Agreed fields, values, types, integer bounds and counts match | Meaning and real-world accuracy |
| Consistency | `consistency_v1` | Equal values, matching counts, unique IDs or complete references | Semantic equivalence |
| Published tests | `published_tests_v1` | Exact committed tests execute on exact files | Complete coverage or platform approval |

The catalog includes full JSON schemas, versions, execution limits and exclusions. Results are **Pass**, **Fail**, **Needs review**, **Unsupported**, **Blocked**, or **Check unavailable**. Unsupported opaque files can still be delivered and reviewed. Malformed JSON declared as JSON fails that agreed format rule; duplicate fields are rejected as ambiguous. JSON integer comparisons preserve signed 64-bit integers. CSV headers are unique, row widths exact, and values are not silently coerced between strings and numbers.

Checks run cheaply first. Missing required prechecks block expensive tests. Runner code has no network, read-only source/tests/root, a pinned image, an unprivileged user, dropped capabilities, bounded CPU/memory/processes/time/output, and a fixed platform/seed. Images must already exist on the dedicated runner. Missing runners return Check unavailable. These are trusted-runner results, not cryptographic computation proofs. Change the checker version when behavior changes; never reuse a version for different semantics.

## API and MCP

The shared API verifies website or wallet sessions. MCP forwards only the caller's credentials; it does not attach a configured operator secret. The core MCP catalog adds these sixteen tools; the nine-tool ChatGPT app catalog is unchanged.

| MCP tool | API route under the verification prefix |
| --- | --- |
| `find_verification_checks` | `GET /checkers` |
| `validate_verification_plan` | `POST /plans/validate` |
| `upload_verification_artifact` | `POST /artifacts` |
| `import_verification_github` | `POST /artifacts/import-github` |
| `get_verification_artifact` | `GET /artifacts/:id` |
| `run_verification_checks` | `POST /runs` |
| `get_verification_run` | `GET /runs/:id` |
| `list_review_tasks` | `GET /review-tasks` |
| `assign_review_task` | `POST /review-tasks/:id/assign` |
| `accept_review_task` | `POST /review-tasks/:id/accept` |
| `save_verification_review` | `POST /reviews` |
| `get_verification_review` | `GET /reviews/:id` |
| `prepare_artifact_submission` | `POST /artifacts/:id/prepare-submission` |
| `confirm_artifact_submission` | `POST /artifacts/:id/confirm-submission` |
| `open_artifact_dispute` | `POST /artifacts/:id/disputes` |
| `resolve_artifact_dispute` | `POST /disputes/:id/resolve` |

Direct and remote MCP requests have a 2 MiB body limit, including base64 encoding. Use the API, CLI, website or SDK for larger uploads up to 100 MiB; an exact GitHub import also avoids sending file bytes through MCP. These transport limits do not change funded submission limits.

An artifact owner or explicitly granted reader may open a retention dispute with a reason. Repeated requests return that participant's existing open dispute. Each participant may withdraw only their own hold, with a recorded reason; another open hold continues to stop cleanup. Reasons are private. Holds do not approve work, grant file access, or rewrite canonical outcomes.

File downloads use `/artifacts/:id/files/:index`, attachment disposition and no-store. Owners can grant or revoke expiring read access through `/artifacts/:id/participants` and `/participants/revoke` with `principal_key` and `expires_at`. Grants do not grant writes or signatures. Private reads return 404 to everyone else. Completed check caches use the same artifact permissions.

New fixed-cutoff plans may include `time_windows` with Unix-second cutoffs for funding, submissions, checks, review and final recovery. Validation requires at least 24 hours for checks, then at least 48 hours for review, followed by exactly 24 hours of recovery. Unused stages must have equal cutoffs. These dates are part of the plan hash and shown in the plan preview. This does not reserve capacity, activate funding or alter an existing bounty’s deadlines.

The first-party review page uses `/reviews/:id/signature` and `/confirm`. Draft, signed, submitted and confirmed are distinct. The API verifies the stored assessment digest, EIP-712 signature and actual canonical transaction/attestation array. Corrections create new records; canonical outcomes are never edited. A designated reviewer can read an agent's draft and sign it, without receiving unrelated agent-account data.

`GET /bounties/:network/:contract/submissions` supplements existing public legacy history with saved files and confirmed review explanations. Missing historical records remain labeled; payment does not itself explain qualification or selection. A separate creator award remains separate.

Python and TypeScript SDKs expose the corresponding operations. The CLI supports:

```sh
cargo run -p cli -- verification catalog
cargo run -p cli -- verification validate examples/verification/document.json
cargo run -p cli -- verification check examples/verification/document.json ./work
# Use your short-lived wallet session through AGENT_BOUNTIES_SESSION_TOKEN.
cargo run -p cli -- verification upload ./work --idempotency-key deliverable-v1
cargo run -p cli -- verification run ARTIFACT_UUID examples/verification/document.json
cargo run -p cli -- verification result RUN_UUID
cargo run -p cli -- verification tasks
```

The website supports individual files or a whole folder, retaining internal paths. Upload retries use an exact-content key, including after reload; changing paths or bytes creates a different upload. Owners can open or close their own file-retention dispute from a saved check run. Local CLI checks cannot claim published-test execution. The website's Check your work page uses the shared runner. Document, dataset and opaque-design examples are starting points, not platform-approved acceptance terms. Replace filenames and limits and resolve all review questions before acceptance.

## Operations and limits

Required storage variables: `ARTIFACT_S3_ENDPOINT` (HTTPS), `ARTIFACT_S3_REGION` (default `auto`), `ARTIFACT_S3_BUCKET`, `ARTIFACT_S3_ACCESS_KEY_ID`, `ARTIFACT_S3_SECRET_ACCESS_KEY`. Keep the bucket private and credentials scoped to it. API and dedicated worker share the PostgreSQL database and storage. `worker --broad-verification` uses `BROAD_CHECKS_DOCKER_BINARY` only on a dedicated bounded runner; API workers never enable Docker. Never mount a Docker socket into the public API container.

Defaults: 100 MiB uploaded, 1 GiB expanded, 1,024 files, 8 MiB per structured input, 100,000 records, 256 KiB per plan, 600 seconds total test time. Storage admission defaults to 10 GiB globally (`ARTIFACT_STORAGE_CAPACITY_BYTES`), 1 GiB and 30 artifact records per principal per day. Shared checks allow 30 new runs per principal per day and 1,000 pending runs globally. Exact retries reuse stored records without using a second allowance. GitHub imports replay completed results without downloading again; unfinished downloads have a separate 10-attempts-per-minute limit.

Uploads are reserved durably before object writes and become ready only after readback. Workers use database leases and exact-input keys, retry boundedly after restarts and reject late leases or changed/missing check results. Abandoned uploads expire after seven days; active or pending submissions and dispute holds prevent cleanup. Confirmed nonwinning files stay one year after canonical closure. Paid winning files stay while the platform runs. Cleanup uses batches of 100.

Pending submissions are reconciled against exact canonical round, solver and commitment evidence. A matching submission is retained privately even if the client never returns to confirm it. Only canonical evidence that the round cannot accept the prepared commitment releases an unused reservation, with seven further days before cleanup. Unavailable chain reads retain the files. Publication still requires the solver's explicit request.

An optional `ARTIFACT_S3_CA_CERTIFICATE` PEM adds a trusted root for a private S3 installation or TLS test fixture. Certificate and hostname checks remain enabled; R2 uses its usual public certificate and needs no custom root.

Before connecting a new private bucket to production, run the explicit storage smoke check with its scoped credentials in the process environment:

```sh
ARTIFACT_STORAGE_SMOKE=1 cargo test -p worker configured_private_storage_round_trip -- --ignored
```

This uploads two small synthetic files using the actual application storage client, reads them through a fresh client, checks isolation between artifact IDs and removes only those test objects. It does not list the bucket, touch existing submissions or print credentials. A failed cleanup reports the random artifact ID for follow-up. Verify that public access is disabled in the storage dashboard separately. Never put credentials in a command argument, test log or committed configuration.

Review reminders use existing verified contact preferences. They send at 24, 6 and 1 hour remaining, and once overdue. Restarted workers skip stale reminder stages. Retries preserve exact payload and provider idempotency key; confirmed closure stops notifications. Tasks expose deadlines and readiness without exposing contact addresses. Overdue notices never extend a contract's clock.

Release gates still include repository, PostgreSQL and production-container checks. New V3 contracts additionally need independent review, meaningful invariants, testnet rehearsal and a bounded production trial. Do not change live terms or enable V3 from local unit tests alone.
