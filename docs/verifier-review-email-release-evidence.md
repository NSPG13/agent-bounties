# Verifier review email release evidence

The reviewed email notification implementation at `295fc16a` changes the
guarded worker build inputs. This release refreshes their expected fingerprints
using the process in [posting journey release evidence](posting-journey-release-evidence.md).

An isolated archive of public main `d38999ae` reproduced the previous worker
digest. The complete input set grows from 165 to 169 files. Its nine changed
inputs are:

- `crates/api/src/main.rs`
- `crates/api/src/site_auth.rs`
- `crates/db/src/lib.rs`
- `crates/db/src/review_notifications.rs`
- `crates/worker/src/lib.rs`
- `crates/worker/src/open_competition_v2_shadow.rs`
- `crates/worker/src/review_email.rs`
- `crates/worker/src/review_notifications.rs`
- `migrations/0040_verifier_review_notifications.sql`

These contain the notification feature and tests, its additive migration, and
the reviewed compiler-lint cleanup in the existing shadow module. The Cargo
manifests and lockfile are unchanged.

The worker-build digest changes from
`7a55133c0025be81feac6354942bf655e5358572114c622e5a921bf549b18bb6` to
`a284b56b70522433b5ec3a3711e1f110aeb9d321bae3792fc07b56d512beda24`.
The eight expected worker digests in the runner, signer and reusable signing
workflows are refreshed together. Their steps, authorization, secrets, signing,
relay and before/after-build checks are unchanged.

The signing-runtime digest remains
`6b0c5073097e9612f94e35089285729d0c0f5db580f75b351d19a97f8bffd2f1`.
The guard executable, guarded scopes, pipeline and their tests are unchanged.

The 1200-second watchdog `unfunded_precommit` receives the same worker pin,
six corresponding rehearsal literals and three derived fixture workflow hashes.
Its exact benchmark directory digest changes from
`sha256:86cc4c1f5f213793811bef9a4d82efe0b4db08130492b6525f2843867b5ed8f7` to
`sha256:160967d64d509cfc70dcc7de599b9e9bed559c6a0cf7fe06ca29bc0d3084f795`.
The checker and rehearsal logic are unchanged.

The existing [activation record](../ops/releases/qicswu-growth-release-v1/pinned-regression-catalog-activation-v1.json)
explicitly classifies that precommit as non-authoritative for the separate,
immutable 900-second catalog tuple. It remains blocked with production
activation unauthorized. The catalog, browser mirror, historical benchmark
bindings, economics, verifier identity and funding gates are unchanged. This
refresh does not activate, fund or settle either artifact and makes no new
claim about live inventory.

Local validation passes the 12 source-guard tests, 36 pipeline tests and four
precommit tests, both expected source-digest checks, and the exact benchmark
directory check. The pipeline tests use local fixtures; their simulated relay
output is not a blockchain transaction.

The full watchdog rehearsal accepts the known-good implementation and rejects
the adversarial workflow/build-input mutations and the known-bad implementation.
Any subsequent guarded source edit requires a new reviewed digest, synchronized
pins and these checks again. No live signing, relay or provider email request is
part of this fingerprint validation.

## Review follow-up

A further review found that queue admission discarded deadline ordering and
that email links did not identify the verifier's task. The follow-up prioritizes
due reviews by their earliest deadline, keeps retry backoff, adds email context
and preference links, and gives email visitors current review guidance.
The new PostgreSQL test fails under the previous admission order and passes
with the correction. No contract, recipient authority, migration, signing flow,
or provider retry payload from an earlier attempt changes.

Only `crates/db/src/review_notifications.rs` and
`crates/worker/src/review_email.rs` change guarded build inputs in this follow-up.
The reviewed worker fingerprint is now `e11407bce1db901466da6da7b18bbdc5057812de538b71d5299d4e7f827086d9`;
the inactive 1200-second benchmark digest is `sha256:4d64e0548471aef9284c3b7f30312a3ac148f3538d3ea6bc0f808ee101138582`.
The same eight workflow pins, six rehearsal literals and derived fixture hashes
are refreshed. Signing runtime, guard/checker behavior, funded tuples and the
900-second catalog are unchanged.

Focused validation: 13 real PostgreSQL notification tests, 15 mailer/projection
tests, strict affected-crate Clippy, 82 frontend/WebMCP tests, mobile/desktop
review-link browser checks, site checks, 52 source/pipeline/precommit tests and
the known-good/known-bad rehearsal. Production delivery remains disabled.

## Plain language update

Email, review-page and account text now use shorter sentences and everyday words.
Behavior and authority checks are unchanged. The mail template and its existing
text checks change the worker fingerprint to `bc044a20d8e6c47e7511d7ee9e495a65589d55010f1a7180342e73dd9c6278bd`.
The matching inactive benchmark digest is `sha256:403bc9d0c7461f86f7592d360702e50badaebae22c08343a29e06ed87b79a49c`.
The existing pin refresh process applies; no signing logic or funded record changes.
