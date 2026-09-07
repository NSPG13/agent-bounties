# Website account setup

An account requires sign-in and at least one verified wallet. Users choose **Link wallet** or **Link another** to use a phone wallet (QR), an installed browser wallet, or create a wallet with the configured embedded provider. Phone pairing has no separate floating button. Closing setup saves progress; it does not complete the account.

The wallet connection is followed by one bounded ownership message. The server verifies the signature and stores the link before account setup completes. Pairing, an address, a signature alone, or a successful sign-in does not prove completion. No deposit, token approval or transaction is needed to create an account. Already verified wallets do not need another ownership signature. Removing the final wallet returns the account to unfinished setup.

## API contract

The website uses `/v1/site-auth/session`, `/v1/site-auth/account`, `/v1/site-auth/wallet/challenge`, `/v1/site-auth/wallet/verify` and `/v1/site-auth/wallet/unlink`. The loopback development server provides equivalent routes below `/auth`.

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
