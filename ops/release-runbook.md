# Release Runbook — Pinned Production Revision

The production smoke and operational control loop workflows compare the deployed
revision against an expected SHA. When production is intentionally pinned to a
revision other than `main` HEAD (e.g. a delayed promotion or a hotfix rollback),
set the `PRODUCTION_EXPECTED_REVISION` repository variable to the exact SHA that
is currently deployed.

## Precedence

The expected revision is resolved in this order:

1. `vars.PRODUCTION_EXPECTED_REVISION` (repository variable — persistent pin)
2. `inputs.expected_revision` (workflow dispatch input — manual override)
3. `github.sha` (the commit the workflow is running on — default)

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

## Stale pin detection

A scheduled workflow (`check-pinned-revision.yml`) runs daily and warns if:

- The pinned SHA is not an ancestor of `main` (orphaned pin).
- The pinned SHA is older than 7 days (forgotten pin).

If either condition fires, open an issue to update or remove the pin.
