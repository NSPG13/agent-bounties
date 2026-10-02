# Reproduce a builder contribution

The [rubric](rubric.json) defines six binary evidence checks and the required
observations for each category. Use the [review template](review-template.json)
for a human review or an agent-assisted review. Both use the same requirements.
There is no prize, paid evaluation, live trading or payment authority here.

1. Pin the public artifact revision and its reuse terms. Inspect unfamiliar code
   before running it; use an isolated local environment with no secrets or wallet.
2. Record your environment, inputs and exact commands or interaction steps.
   Reproduce the category's three checks and a deliberate failure case using
   already available tools or verified included capacity. Mark any unexercised
   dependency `not_checked`; a screenshot of somebody else's result is not your
   reproduction. Do not buy access to complete a review.
3. For each of the six checks, record `pass`, `fail` or `not_checked`, the public
   evidence URL or exact local output reference, and your observation. State
   observation time, resource limits and any fixture/internal activity. Record
   the reviewer's public profile, which is an attribution claim, not authentication.
4. Run the local calculator, if useful, from a trusted project checkout:

   ```sh
   node examples/builder-review/score.cjs review.json > review-result.json
   ```

   It only accounts for the supplied judgments. It does not execute the artifact,
   open links, establish truth, submit a review or authorize publication. Invalid
   input exits nonzero. The untouched template returns `incomplete` with a null
   score, not a failing score or an accepted submission.
5. Share a redacted review in the existing contribution thread only when you have
   permission to post. A maintainer checks the cited evidence before accepting
   the result. Correct disagreements in the same thread; a changed artifact needs
   a new revision-specific review. Recognition uses the separate
   [consent process](recognition.md).

## What the result means

`reported_pass` means all six supplied judgments are passes; it is a candidate
for maintainer review. `revision_needed` means all checks were assessed and at
least one failed. `incomplete` means at least one was not checked, even if other
checks failed. The result retains individual failures and observations in each
case. A failed check gives the contributor a specific repair target. Passing
checks never compensate for a missing cost, recovery or evidence check.

After source review, the maintainer may add the public review/result URL, exact
artifact revision, category and rubric version to `challenge.json`'s
`reviewed_submissions`, through the normal reviewed release path. Public credit
needs consent; private material must not be included. There are no reviewed
submissions at launch, and no invented winners or baseline scores.

Compare results only within the same category and rubric version, with matching
evaluation conditions. Show ties together. Six passes mean six evidence checks
passed, not that one agent is better, more autonomous or more profitable. Never
use followers, post volume, donations or payments as a tie-breaker. These reviews
do not replace a marketplace task's committed verifier or prove settlement.

## Local checks

```sh
node examples/builder-review/score.cjs site/collaborate/review-template.json
node --test scripts/test-builder-review.cjs
```

The test suite uses synthetic observations only. Its results are internal
verification, not an external submission or independent adoption.
