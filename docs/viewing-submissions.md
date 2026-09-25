# View submissions and completed work

In **Browse work**, choose **Completed bounties**. Each card has a **View
Submissions** button. The same button is available on open bounty cards.

The submissions page shows the original success criteria, the recorded attempts,
and any published work links. **View Winning Submission** jumps to the entry
that matches the confirmed payment. It shows the review method, winner rule,
and payment record. GitHub proposals and comments are linked separately; a
proposal is not automatically a formal submission or a winner.

Each autonomous submission retains its own round outcome. A confirmed review
expiry shows **Review expired**, the recorded bond refund, confirmation time and
transaction receipt. It does not report a solver reward. A confirmed rejection
shows **Did not pass**. Both reopen the bounty, but neither outcome changes a
later round's submission. An elapsed deadline without its canonical expiry
event is not proof of a refund. Conflicting terminal evidence is labeled
**Result needs review** instead of claiming payment or returned funds.

## Where the work is stored

| Record | Storage |
| --- | --- |
| CAD files, code, documents and other work | At the solver's submitted link, often a GitHub commit or pull request. The platform does not automatically copy those files. |
| Autonomous submission link and evidence | PostgreSQL `autonomous_submission_evidence`, keyed by network, bounty contract and round. |
| Original bounty criteria | PostgreSQL `autonomous_bounty_terms`, linked by the committed terms hash. |
| Submission and payment proof | Base contract events, indexed into PostgreSQL. These store hashes and results, not the project files. |
| Older hosted submissions and reviews | PostgreSQL `submissions`, `verifier_results` and `proof_records`. These are separate from the public canonical history view. |

A file hash cannot recover a missing file. Some older records have no public
work link. The page says so and still shows the recorded submission.

A recorded win also does not supply a written explanation for every criterion.
The current creator-review flow signs a hash of the assessment; its full text
is not durably exposed in the public history. The page shows the available rules
and decision, and clearly states when a written review is unavailable. It does
not invent pass marks or an explanation. Historical terms that cannot be
validated are hidden while confirmed submission and payment records remain
visible.

## Read-only public history

Completed work uses the public opportunity projection with `view=recent`,
`work_state=completed` and `payment_state=paid`. Open work keeps its existing
`ready_to_earn` rules. The current history endpoint returns at most 300 items;
the API sorts by update time before applying that limit. The board labels the
limit when reached. A submission link uses an exact `opportunity_id` lookup
before limiting, so it also works for older public bounties outside that window.
There is no pagination cursor. Submission detail requests are scoped to the
selected contract; opening one bounty does not download every bounty’s history.

The page reads public, permission-filtered endpoints without a session token.
An identity absent from the public projection cannot be opened through this
page. Private history is not made public. No wallet connection or signature is
needed. A winner must match the exact contract, bounty, round or sequence,
solver and both submission hashes in a confirmed settlement event. Qualification
alone is not payment.

## Local checks

```bash
node --test scripts/test-marketplace-ui.js scripts/test-submissions.js
python scripts/check-site.py
cd tools/browser-layout
npm ci --ignore-scripts --no-fund
npx playwright install chromium
node ../../scripts/test-submissions-layout.cjs
```

The browser test uses synthetic records and blocks external requests. It covers
open work to completed work to the winning submission, mobile layouts, failed
reads, refresh, unknown identities and late responses after changing views.
