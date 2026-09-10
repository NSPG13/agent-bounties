# Website account setup

An account requires sign-in and at least one verified wallet. Users choose **Link wallet** or **Link another** to use a phone wallet (QR), an installed browser wallet, or create a wallet with the configured embedded provider. Phone pairing has no separate floating button. Closing setup saves progress; it does not complete the account.

The wallet connection is followed by one bounded ownership message. The server verifies the signature and stores the link before account setup completes. Pairing, an address, a signature alone, or a successful sign-in does not prove completion. No deposit, token approval or transaction is needed to create an account. Already verified wallets do not need another ownership signature. Removing the final wallet returns the account to unfinished setup.

## API contract

The website's internal session, account and wallet-link handlers are defined in [site_auth.rs](../crates/api/src/site_auth.rs). The [loopback development server](../scripts/serve-solarpunk-auth.py) provides equivalent handlers. These browser-session routes are separate from the public marketplace API.

Session, account, successful verification and unlink responses include:

| Field | Meaning |
| --- | --- |
| `account_status: signed_out` | No authenticated identity; returned by the session route. |
| `account_status: wallet_required` | Signed in, with no stored verified wallet. Setup is unfinished. |
| `account_status: ready` | At least one stored, ownership-verified wallet. |
| `account_status: unavailable` | Wallet storage could not be checked. Do not report completion. |
| `account_complete` | True only for `ready`. |

`authenticated` continues to describe the signed-in identity so pending users can request a wallet challenge and finish setup. It is not an account-completion flag. Website clients must use the server's completion fields and verified wallet list before showing a completed account. The session's opaque `user.id` binds resumable setup to the same identity.

An unavailable marketplace activity feed does not revoke a verified link. Account completion and activity availability are separate. Signature and transaction confirmations remain in the user's wallet. These account links grant no authority to spend, fund, publish or settle work.

New ownership-verification requests may include `provider_id`. Responses preserve
this user-selected hint, `wallet_type`, `chain_ids`, and `last_verified_at`.
Legacy links remain provider-unknown. This metadata cannot establish a live
signing session, ownership of another address, or funding readiness.

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

During a rolling deployment, the website also understands the previous API's explicit verified-identity and wallet-link receipts. New completion fields take precedence when present. A wallet address or empty successful response alone never completes setup.
