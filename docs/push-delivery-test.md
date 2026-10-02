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

If setup times out, a late browser subscription is unsubscribed and a late
server registration is removed. Another setup waits for pending cleanup. If
you change accounts, server cleanup resumes when the original account signs
back in. Each deadline stage has its own alert; duplicate delivery of the same
stage replaces that alert without sounding again.

Tests require a signed account session, an allowed first-party origin, explicit
consent and that account's active subscription. They are limited to one per
minute and five per hour across all of the account's devices. The same request
ID never sends again, including after a restart or interrupted provider request.
No provider URL, encryption key or arbitrary message can be supplied to the test
action. Existing subscribe/unsubscribe clients remain compatible.

The hosted service reserves each test before delivery. Opted-out, expired or
renewed subscriptions cannot be used by an old pending test. Tests expire after
60 seconds; older service workers discard them. The page waits for its
service-worker update before sending the test.

Operators can roll back the website and disable push delivery independently.
This check does not change wallet keys, contracts, funded terms or gas budgets.

Validation: database fixtures cover ownership, concurrent duplicate requests,
restart replay, account quotas, opt-out, renewal and delayed provider failure.
Worker fixtures check encryption, expiry and endpoint restrictions. Browser
fixtures check explicit action, receipt versus acceptance, late setup cleanup,
account changes, distinct deadline alerts, stale payloads and fixed navigation.
A real provider/device canary remains separate evidence.
