# Measured posting correction

Objective: complete the existing posting journey under the attached seven-part
scorecard. Targets remain 100% eligible local completion, 100% draft continuity,
100% recoverable completion, zero unauthorized/duplicate effects and zero false
successes. Human comprehension requires actual participants, not an AI judgment.

## Baseline and matrix

The immutable baseline evidence is in `target/posting-metrics/`: source hashes,
API discovery documents, raw scenario logs and `baseline-corrected.json`.
Desktop (1440×1000) and phone (390×844) each run 13 journeys: normal, sign-in
return, cancellation before signing with/without reload, cancellation after
signing with/without reload, USDC top-up with/without reload, zero-ETH top-up
with/without reload, post-signature gas shortfall top-up with/without reload,
and a lost transaction reply followed by canonical reconciliation. References
are captured from real local image bytes and bound to their timestamp/hash.

Before implementation: **12/26 completion, 8/22 recovery**. The 144/144 observed
field comparisons passed, but reload failures prevented the remaining comparisons;
full continuity is therefore unverified at baseline. A harness-only correction
replaced a nonexistent review `.draft` property with documented `status=staged`;
the two affected sign-in cases were rerun before production code changed.

Independent safety scenarios will cover missing approvals, repeated clicks,
concurrent tabs, wrong wallets, expired authorizations, SDK timeout/lost replies,
tampered continuation state and mismatched public inventory. Changes in coverage
will be listed separately instead of changing the baseline denominator.

## Continuation design and threat model (R3)

`approved draft → exact funding signature → authorized/unsent → user transaction
review → durable unique submission reservation → Coinbase SDK → reconciliation`

- Store the bounded signed creation request in this tab's session storage,
  bound to the account, operation, wallet and approved draft hash. Do not put
  signatures in account drafts, public terms, logs or WebMCP output.
- Store only its digest in the account recovery journal. The digest cannot
  change once recorded. Reload validates the local bytes against that digest.
- Before invoking the SDK, atomically reserve a unique submission attempt via
  the existing PostgreSQL revision check. Concurrent attempts must not share
  a reservation. Once reserved, the server rejects downgrade or replacement.
- Cancellation and insufficient gas before that reservation leave the request
  authorized/unsent. Resume revalidates the same request and asks for a new
  transaction confirmation; it neither re-signs nor creates a new bounty.
- A reservation with a crash or uncertain SDK reply remains reconciliation-only.
  Absence of an on-chain event does not authorize a resend. Expiry, identity,
  approval or integrity failures stop wallet invocation.
- The browser already holds this purpose-limited signature to request a creation
  plan. Retaining it across reload extends exposure to the current tab/session;
  it authorizes only the committed amount/recipient/nonce/deadline. Existing XSS
  defenses remain necessary. No private key or seed enters application storage.
- Other devices can read the draft and reconcile progress but cannot silently
  acquire this tab's signer or signed payload. Missing local continuation bytes
  produce a specific recovery message rather than a replacement transaction.

Deploy browser/adapter/API together only after R3 review and a staged canary.
Rollback disables new continuation and preserves journals; old uncertain
requests must never be retried to work around an application rollback.

## Contributor coordination

The current public PR list was read and retained locally. #1438 overlaps posting
layout; preserve it. #1446/#1437 concern inventory filtering; do not change those
contracts. #1447 concerns the OpenAPI digest helper and #1436 verifier email;
neither is changed here. No contributor code was executed or PR mutated.

Notice draft, not sent: "Continuing the local posting correction with measured
recovery, preserved approvals and beginner guidance. Reuse the current posting
layout and inventory contracts. Browser and API recovery metadata must ship
together after R3 review and canary. How did you find the project, what motivated
participation, and which integration or workflow should we protect?"

Human research and live Coinbase/Base behavior remain explicitly UNVERIFIED
until direct evidence is collected. Local tests cannot clear those gates.
