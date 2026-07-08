# Funding Comment Planner

This directory contains the deterministic funding-comment planner used by
the `Funding Comment Planner` GitHub Actions workflow
(`.github/workflows/funding-comment-planner.yml`).

## What it does

When a comment of the form

```
/agent-bounty fund <amount> <currency> via <rail>
```

is posted on an issue labeled `bounty`, the workflow runs
`scripts/funding_comment_planner.py` and posts a Markdown result back to the
issue. The planner is **read-only with respect to settlement state**:

- It never credits balances.
- It never marks a bounty as funded.
- It never authorizes claimability.
- It never releases payout.

Every successful planner result includes:

- `amount`
- `currency`
- `rail`
- `contributor_login`
- `idempotency_key`
- `requires_operator_reconciliation: true`

Invalid, duplicate, unsupported, or non-bounty comments receive constructive,
action-required feedback instead.

## Supported rails and currencies

| Rail   | Supported currencies |
|--------|-----------------------|
| base   | usdc                  |
| stripe | usd, eur, gbp          |

## Local testing (no GitHub secrets required)

Run the planner against any of the replay fixtures in
`scripts/fixtures/funding_comment_planner/`:

```bash
# Valid Base USDC funding signal
python3 scripts/funding_comment_planner.py \
  --fixture scripts/fixtures/funding_comment_planner/valid_base_usdc.json

# Valid Stripe fiat funding signal
python3 scripts/funding_comment_planner.py \
  --fixture scripts/fixtures/funding_comment_planner/valid_stripe_fiat.json

# Invalid amount
python3 scripts/funding_comment_planner.py \
  --fixture scripts/fixtures/funding_comment_planner/invalid_amount.json

# Duplicate idempotency key
python3 scripts/funding_comment_planner.py \
  --fixture scripts/fixtures/funding_comment_planner/duplicate_idempotency_key.json

# Non-bounty issue
python3 scripts/funding_comment_planner.py \
  --fixture scripts/fixtures/funding_comment_planner/non_bounty_issue.json
```

Each command prints the exact Markdown that would be posted back to the
issue by the GitHub Action, so contributors and reviewers can validate
behavior without needing privileged GitHub secrets or triggering a real
workflow run.

## Adding new fixtures

Fixture files are plain JSON with the following shape:

```json
{
  "comment_body": "/agent-bounty fund 25 USDC via base",
  "comment_author": "github-login",
  "comment_id": "1001",
  "issue_number": "42",
  "issue_labels": ["bounty"],
  "repo_full_name": "example-org/agent-bounties",
  "seen_idempotency_keys": []
}
```

`seen_idempotency_keys` simulates previously reconciled funding signals for
duplicate-detection testing.
