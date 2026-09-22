# Release Runbook — Pinned Production Revision

The production smoke and operational control loop workflows compare the deployed
revision against an expected SHA. When production is intentionally pinned to a
revision other than `main` HEAD (e.g. a delayed promotion or a hotfix rollback),
set the `PRODUCTION_EXPECTED_REVISION` repository variable to the exact SHA that
is currently deployed.

## Precedence

The expected revision is resolved in this order (most specific first):

1. `inputs.expected_revision` (workflow dispatch input — manual override when explicitly provided)
2. `vars.PRODUCTION_EXPECTED_REVISION` (repository variable — persistent pin set by the promotion workflow)
3. `github.sha` (the commit the workflow is running on — default when no pin or override is set)

A manual dispatch input beats the persistent pin when it is explicitly provided. The promotion workflow sets the persistent pin; smoke and control-loop workflows read it. The pin is not an override of an explicit manual dispatch.

## When to update

- **Pin promotion:** After merging to `main` but before the next production
  deployment, set the variable to the SHA that *remains* deployed.
- **New deployment:** After confirming the new revision is live, update the
  variable to the new SHA.
- **Unpin:** When `main` HEAD matches the deployed revision, remove the variable
  (or set it to empty) so the workflows fall back to `github.sha`.

## How to set the variable

### Via GitHub CLI

```bash
# Set a pin
gh api repos/NSPG13/agent-bounties/actions/variables/PRODUCTION_EXPECTED_REVISION \
  -X PATCH -f name='PRODUCTION_EXPECTED_REVISION' \
  -f value='abc123def456...'

# Remove a pin
gh api repos/NSPG13/agent-bounties/actions/variables/PRODUCTION_EXPECTED_REVISION \
  -X DELETE
```

### Via GitHub UI

1. Go to **Settings → Secrets and variables → Actions → Variables**.
2. Click **New repository variable** (or edit the existing one).
3. Name: `PRODUCTION_EXPECTED_REVISION`.
4. Value: The exact 40-character SHA currently deployed to production.

## Verifying a pin

Before updating the pinned revision, confirm that the promotion workflow smoke checks
pass against the target revision. See `docs/production-smoke.md` for the smoke check
definitions and the expected value of `PRODUCTION_EXPECTED_REVISION`.

When a pinned revision becomes stale (the artefact it references is no longer deployed),
update `PRODUCTION_EXPECTED_REVISION` to the new artefact SHA as part of the next
promotion. Do not rely on a separate scheduled pin-check workflow — the pin is managed
directly by the promotion workflow that sets the variable.
