# Website account setup

An account requires sign-in only. A signed-in account can prepare, approve and
post a bounty, and pay for it by card, without a crypto wallet. Card buyers are
authenticated by their account; crypto funding proves its funding wallet at the
funding step.

A verified wallet is required only to **solve**: to claim a bounty, sign a
solver bond, submit work, enter solver surfaces such as Workspace, and receive
payouts. Users choose **Link wallet** or **Link another** to use a phone wallet
(QR), an installed browser wallet, or create a wallet with the configured
embedded provider. Phone pairing has no separate floating button.

The wallet connection is followed by one bounded ownership message. The server verifies the signature and stores the link before solver readiness is reported. Pairing, an address, a signature alone, or a successful sign-in does not prove a linked wallet. No deposit, token approval or transaction is needed to link one. Already verified wallets do not need another ownership signature. Removing the final wallet ends solver readiness; the account can still post and pay by card.

## API contract

The website's internal session, account and wallet-link handlers are defined in [site_auth.rs](../crates/api/src/site_auth.rs). The [loopback development server](../scripts/serve-solarpunk-auth.py) provides equivalent handlers. These browser-session routes are separate from the public marketplace API.

Session, account, successful verification and unlink responses include:

| Field | Meaning |
| --- | --- |
| `account_status: signed_out` | No authenticated identity; returned by the session route. |
| `account_status: ready` | Signed in and account storage is readable. The account can post and pay by card, with or without a wallet. |
| `account_status: unavailable` | Account storage could not be checked. Do not report a ready account. |
| `account_complete` | True only for `account_status: ready`. |
| `solver_status: ready` | At least one stored, ownership-verified wallet. The account can claim, submit and receive payouts. |
| `solver_status: wallet_required` | Ready account with no stored verified wallet. Link one before solving. |
| `solver_status: unavailable` / `signed_out` | Wallet storage could not be checked, or nobody is signed in. |
| `solver_ready` | True only for `solver_status: ready`. |

`authenticated` continues to describe the signed-in identity. Website clients must use the server's completion fields and verified wallet list before showing a ready account or a linked solver wallet; a `solver_ready` claim that disagrees with the wallet list is treated as unavailable. The session's opaque `user.id` binds resumable setup to the same identity.

Neither readiness flag is authority. Card quotes still require an authenticated account and an allowed Origin. Claims, submissions, terms publication and payouts still require server-side proof of the exact wallet (`authorize_verified_wallet`), whatever these fields say.

### Rolling release

Release the website before the API. The current website also understands the
previous API, which reported a signed-in account without a wallet as
`account_status: wallet_required` with `account_complete: false`, and reported
`ready` only with a verified wallet. The website treats that previous
`wallet_required` response as a ready posting account whose solver wallet is
missing. A previous website reading the current API would show a wallet-less
account as unavailable, so the API must not be released first.

An unavailable marketplace activity feed does not revoke a verified link. Account readiness and activity availability are separate. Wallet-derived stats and settlement history appear only after a wallet is linked. Card orders stay on their order page. Signature and transaction confirmations remain in the user's wallet. These account links grant no authority to spend, fund, publish or settle work.

New ownership-verification requests may include `provider_id`. Responses preserve
this user-selected hint, `wallet_type`, `chain_ids`, and `last_verified_at`.
Legacy links remain provider-unknown. This metadata cannot establish a live
signing session, ownership of another address, or funding readiness.

## Continue your work

The account panel lists saved drafts whether or not a wallet is linked. It groups confirmed
activity by next action, recovery, working, awaiting review, paid and past
activity. A deadline is a reminder; only confirmed terminal events establish
payment, refund, rejection or expiry. Rounds remain separate after reopening.

`GET /v1/site-auth/account` includes `saved_drafts`, `activity_inbox` and
`generated_at`. Each source reports availability independently. Drafts remain
accessible when no wallet is linked or chain evidence is unavailable.
`GET /v1/site-auth/posting-drafts?offset=...` lists only the signed-in account's
saved-operation summaries, 50 per page. Follow `next_offset`; the listing
excludes raw draft contents, approvals and recovery payloads. Both reads use
the existing browser session, no-store responses and the existing draft
rollout gate. A continuation URL is not authorization.

A pending posting checkpoint moves to past activity only when canonical
activity matches its bounty contract and ID, proves funding, and identifies
the account as poster. It remains a posting record, not a solver payment.
Older draft pages may still ask to check posting status; resume the same
operation to reconcile them before retrying.

To return with an assistant, use the [saved-work request and browser read tool](webmcp-guided-marketplace.md#resume-without-prior-conversation).
It shares your current browser session and preserves the selected operation;
installing the hosted MCP connector alone does not grant it access to your
private inbox. You can also open **Your activity** and choose the saved item
yourself. Checking progress neither creates a new task nor authorizes payment.

## Saved posting journeys

`GET` and `POST /v1/site-auth/posting-drafts/{operation_id}` use the existing
authenticated browser session. Writes also require an allowed first-party
Origin. The opaque UUID in `post.html?operation_id=...` identifies the journey;
it is not a bearer credential. Another account receives 404, and signed-out
users must sign in before any draft content loads.

POST accepts `draft`, `expected_revision` (0 for a new draft), optional
`approved_draft_hash`, and optional `recovery_state`. The response returns the
same fields, `draft_hash`, `revision`, timestamps, and `continuation_url`.
`draft_hash` is SHA-256 of RFC 8785 canonical JSON for `draft` only (UTF-16
property ordering and ECMAScript number formatting). Integer values outside
the browser-safe range must be strings; decimal annotations remain supported.
Approval must bind to that exact hash. Changed terms clear approval unless the
person approves the new hash. `recovery_state` is separate from the terms hash
and must contain only public operation identifiers and reconciliation hints.
Wallet signatures, credentials, and pairing secrets are rejected.

Replaying an identical request returns the same record. A stale conflicting
revision receives 409, requiring a reload before another write or wallet action.
Each account is limited to 50 active drafts, 64 KiB per draft and 16 KiB of
recovery state. Unsubmitted drafts expire after 30 days without a change;
reads exclude expired records and subsequent saves remove expired rows.
Records containing a posting journal remain available for recovery beyond that
deadline, and an update cannot erase their bounty identity or transaction IDs.
Browser caches must
be scoped to the authenticated account and must not silently overwrite a
different account's journey.

Saved approval records express review of draft contents only. They do not
replace legal consent, wallet confirmation, or canonical evidence. A saved
transaction hash must be reconciled instead of creating another transaction.

During a rolling deployment, the website also understands the previous API's explicit verified-identity and wallet-link receipts. New completion and solver fields take precedence when present. A wallet address or empty successful response alone never proves a linked solver wallet.
