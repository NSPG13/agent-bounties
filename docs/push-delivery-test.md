# Check browser notification delivery

Sign in at [Account](https://agentbounties.app/#account), enable browser
notifications on the current device, then select **Send a test notification**.
The test says that no review or payment action is required. Selecting it opens
account preferences. It creates no bounty, submission, verdict or payment record.

The account page distinguishes push-provider acceptance from a notification
received by this browser. If the provider accepts it but the browser does not
confirm receipt within ten seconds, check browser/OS notification settings.
The ten-second check is not a delivery guarantee: a later receipt is possible.
Closing a notification immediately can also prevent the page observing it.

Tests require a signed account session, an allowed first-party origin, explicit
consent and that account's active subscription. They are limited to one per
minute and five per hour across all of the account's devices. The same request
ID never sends again, including after a restart or interrupted provider request.
No provider URL, encryption key or arbitrary message can be supplied to the test
action. Existing subscribe/unsubscribe clients remain compatible.

The existing private endpoint `POST /v1/site-auth/push-notifications` accepts
`{"action":"test","id":"<owned-device-uuid>","request_id":"<fresh-uuid>","consent":true}`.
Its no-store response includes `request_id` and `status`: `accepted` means provider
acceptance; `attempting`/`unknown` do not establish receipt; `failed`/`cancelled`
are not retried. The browser checks its own service worker for the exact test
tag before displaying receipt confirmation. A test UUID is correlation only,
never authorization. Account changes cancel the page's pending receipt check.

The additive `0057_push_delivery_tests.sql` migration stores the bounded attempt
and subscription generation separately from real review notifications. The
account lock serializes admissions with subscription renewal; send-time checks
reject opted-out, expired, renewed or old reservations. A late rejection cannot
disable a newer subscription. Ambiguous sends remain consumed. Test payloads
expire after 60 seconds and use a separate schema; older workers discard them.
The page waits for a service-worker update before sending its test.

Rollback: retain the additive table and revert the application or website
revision. Disable `CREATOR_PUSH_ENABLED` if the provider or consent boundary
fails. No wallet key, contract, funded term or gas-budget change is involved.

Validation: database fixtures cover ownership, concurrent duplicate requests,
restart replay, account quotas, opt-out, renewal and delayed provider failure.
Worker fixtures check encryption, expiry and endpoint restrictions. Browser
fixtures check explicit action, receipt versus acceptance, stale payloads and
fixed navigation. A real provider/device canary remains separate evidence.
