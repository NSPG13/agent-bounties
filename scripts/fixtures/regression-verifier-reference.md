# Frozen verifier test fixture

`regression-verifier-1998b4e.tar.gz` contains the exact historical source bytes
needed by the unchanged watchdog rehearsal. It is a test fixture, never a
deployment bundle or permission to sign with the current app.

The reviewed source was recovered from local mirror commit
`1998b4ebe57ddbc82ffb72653f0c4b498e74cb42`. That mirror commit is not available
from the public repository. Keep this fixture in the private runtime release.
Tests must work without fetching it from GitHub or relying on local Git history.

The archive contains 205 regular files: every file selected by the original
worker-build source guard, plus the original checker, rehearsal, signing
runtime, tests and three existing verifier workflows. There are no generated
build outputs, credentials or user submissions. Paths and bytes come from
`git show <reference>:<path>`; tar entries are sorted, have mode 0644, zero
timestamps/owner IDs, and use USTAR. Gzip uses an empty filename and timestamp 0.

Exact commitments:

- Archive SHA-256: `74162e0c58870d73907360150af37eb8ba6737e41de493e2403dc662793e77ea`.
- Worker-build SHA-256: `f3a66b73cbb03164a4372a13c3b859e0ef8fe8629f7c076be8f016f5950cf9e5`.
- Signing-runtime SHA-256: `6b0c5073097e9612f94e35089285729d0c0f5db580f75b351d19a97f8bffd2f1`.

Run `python scripts/rehearse_frozen_verifier.py`. It checks the archive before
extraction, verifies the original executable and source commitments, then runs
the unchanged known-good/known-bad and adversarial mutation suite without
inherited cloud, wallet, database or provider credentials. Missing or changed
fixtures fail the gate. The current repository must retain the same frozen
checker, signing runtime and workflow bytes too.

The regular repository gate independently tests the current application.
Production runner/signer workflows still require their original source hashes;
their current-app drift remains a separate operational restriction. This test
does not make those workflows ready or change any funded acceptance terms.
Do not refresh the fixture or hashes merely to accept new application code.
