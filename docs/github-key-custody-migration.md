# One-time legacy key custody migration

This R3 operation completes the environment-custody work left open by
[the public security remediation](https://github.com/NSPG13/agent-bounties/pull/1738).
The repository owner explicitly authorized copying the four existing GitHub
secrets into the prepared restricted environments without exposing their values,
and keeping signing disabled until verification completes. This authorization
does not activate any signer, approve a worker release, rotate an identity, spend
funds, deploy a contract, or change a hosted service.

The November 7 open-PR inventory was inspected (123 open PRs). No contributor
proposal is superseded. Historical workflow work such as #568 must preserve the
custody boundary and cannot restore a disabled signer as part of routine repair.
The original remediation notice and PR already identified this custody follow-up.

## Exact destination map

| Original repository secret | Destination environment | Destination secret names |
| --- | --- | --- |
| `BASE_KEEPER_PRIVATE_KEY` | `legacy-keeper` | `BASE_KEEPER_PRIVATE_KEY` |
| `PARTICIPANT_ATTESTER_PRIVATE_KEY` | `legacy-keeper` | `PARTICIPANT_ATTESTER_PRIVATE_KEY` |
| `REGRESSION_VERIFIER_ONE_PRIVATE_KEY` | `legacy-verifier-one` | `REGRESSION_VERIFIER_ONE_PRIVATE_KEY`, `VERIFIER_PRIVATE_KEY`, `SIGNER_PRIVATE_KEY` |
| `REGRESSION_VERIFIER_TWO_PRIVATE_KEY` | `legacy-verifier-two` | `REGRESSION_VERIFIER_TWO_PRIVATE_KEY`, `VERIFIER_PRIVATE_KEY`, `SIGNER_PRIVATE_KEY` |

The two verifier keys never enter the same job or environment. The aliases
preserve the existing reusable-workflow inputs. GitHub environment secrets take
precedence over same-name passed secrets; the final verification must run after
repository copies are absent, proving that no repository fallback is needed.
Neither verifier key is copied into `legacy-keeper`.

## Authority and data flow

```text
Existing GitHub repository secret
  -> one isolated GitHub-hosted job, protected main, exact approved revision
  -> libsodium sealed box addressed to GitHub's environment public key
  -> short-lived encrypted artifact
  -> local administrator client relays ciphertext to GitHub's secrets API
  -> destination environment secret
  -> separate protected verification job -> byte-equality receipt only
```

The owner/API administrator authorizes the exact workflow revision through
`KEY_CUSTODY_MIGRATION_REVISION` and the current phase through
`KEY_CUSTODY_MIGRATION_PHASE`. Both are unset outside the migration window.
`KEY_CUSTODY_SOURCE_RUN_ID` pins the completed source run during verification.
The reviewed plan pins each existing environment ID, public encryption key and
key ID, a random challenge, and a finite expiry. Changed keys or policies require
a new reviewed plan; never substitute a recipient through workflow input.

The runner has read-only repository/Actions token permissions. Its signing-key
environment exists only on the encryption or comparison step. The owner token
stays in the local authenticated GitHub CLI. There is no API that returns saved
GitHub secret values. No plaintext key enters this workstation, command arguments,
logs, outputs, summaries, artifacts, or repository files. The runner necessarily
holds plaintext in process memory; trusted GitHub infrastructure, pinned actions,
the distribution crypto library and repository administrators remain trust
boundaries. Core dumps and debug execution are rejected. Private scalar validity
is checked without generating a signature or making a chain call.

Sealed boxes protect ciphertext confidentiality but do not authenticate a sender.
The administrator therefore checks the exact owner-dispatched workflow, source
revision, run attempt, successful four jobs, artifact names, recipient bindings
and plan digest. A fresh keyed challenge proves exact byte equality; a separate
normalized-scalar commitment proves the two verifier identities are distinct.
These commitments never authorize a transaction.

## Staged procedure and evidence

1. Record secret **metadata only**, environment controls/public keys and workflow
   states. Require exactly the `main` branch policy, no tag or wildcard access,
   empty destination environments and no environment authorization overrides.
   Disable every workflow consuming these keys or their reusable aliases. Verify
   there are no queued, pending, waiting or running consumer jobs.
2. Publish this reviewed workflow through normal PR/CI gates after the current
   public-main release freeze ends. Do not weaken environment policies to run a
   feature branch. Keep the application release pin unchanged.
3. On the exact public-main checkout, set the revision and `seal` phase variables.
   Manually dispatch `github-key-custody.yml` on `main` with `phase=seal` exactly
   once. The workflow's secret-free checks and synthetic crypto rehearsal must
   pass before key access. Record its run ID. Never rerun a secret-bearing job;
   a second attempt is denied.
4. Run `scripts/migrate_github_key_custody.py inspect`, then `import`, supplying
   `--gh`, `--revision`, `--source-run`, the original `--before` metadata snapshot,
   and a private `--evidence` directory. Import sends only sealed ciphertext via
   authenticated API stdin. It creates exactly eight entries and advances the
   authorization to `verify` only after metadata reconciliation.
5. Dispatch `phase=verify` on the same revision. Run the local `verify` operation
   with its `--verification-run`. All eight aliases must match; neither verifier
   may share the other's scalar. Recheck restrictions, metadata and workflow holds.
6. Run `retire` against that verification run. The local script records a deletion
   intent before removing the four repository-wide entries. All destination
   entries remain. Dispatch a fresh verification run after removal and require
   all four isolated jobs to succeed again. A missing alias fails closed.
7. Run `complete` with the **second** verification run. Save final reconciliation
   before cleanup, disable the migration workflow, remove the three temporary
   authorization variables, and delete the temporary source/verification
   artifacts. Read back these settings and all consumer holds. Signing remains
   disabled; migration completion is not activation approval.

Metadata snapshots and final receipts contain no secret values. Source artifacts
contain only ciphertext, challenge commitments and provenance, have one-day
retention and are deleted after acceptance. No new privileged token, wallet,
contract or monetary action is required. Existing Actions spending limits apply.

## Failure, retry and containment

Wrong actor, branch, revision, debug mode, runner, expiry, environment key,
artifact, source run, missing alias or changed controls stop the operation.
Do not clear a hold to make the migration pass. Preserve original repository
entries until destination verification succeeds. A partial import can repeat the
same ciphertext PUTs while still held; it must retain the original source run.
After the phase advances, verify instead of repeating import.

A partial retirement resumes against its saved exact intent and unchanged
destination metadata; it deletes only remaining originals. Never recreate a
repository-wide secret to repair a consumer. If final cleanup stops midway, the
`complete-before-cleanup.json` record is the acceptance evidence: finish removing
only its migration authorization and artifacts using metadata. No key export or
signing run is needed. Preserve a record of every completed external write.

Historical `leaderboard-reward-deploy` and `standing-meta-v2-deploy` jobs expect
multiple verifier keys in the keeper environment. Other historical jobs bind
`v2-beta2-sepolia` or standing-meta-v4 environments. They remain disabled and do
not receive broader copies. Any later activation requires a separate reviewed
workflow repair and its existing release, health, obligations and value gates.

## Verification

Run the synthetic round-trip and failure tests, keeper-workflow security check
and actionlint. The dedicated PR check installs the distribution libsodium
library and executes synthetic tests without any production secret. The manual
workflow also rehearses encryption before secret access. No runtime, contract,
database or payment behavior changes in this migration.

References: [GitHub environment secrets API](https://docs.github.com/en/rest/actions/secrets),
[GitHub reusable workflows](https://docs.github.com/en/actions/how-tos/reuse-automations/reuse-workflows),
[libsodium sealed boxes](https://doc.libsodium.org/public-key_cryptography/sealed_boxes).
