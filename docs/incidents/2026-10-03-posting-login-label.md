# Posting card retained “Checking login…” after successful login

The account and saved draft were available, but restoring a stale proposal
returned from `syncPrimaryAction` before replacing the old login button label.
Brief and reference invalidation also disabled the button without rendering its
new reason. The apparent login hang was a proposal requiring regeneration.

Recovery uses the existing WebMCP journey and staging tools to update the same
operation from its saved brief, preserving rewards, deadline, criteria and
reference bindings. Recovery does not approve terms, publish or fund the bounty.
The live affected session was recovered and displayed “Approve bounty card”.

The fix labels every blocked proposal state and rerenders immediately on
invalidation. Session and optional wallet-account reads now time out after
8 seconds each, including response-body decoding; the completed login state is
rendered before optional wallet discovery, after account draft hydration has
completed. Until then the action explicitly says “Restoring saved draft…” and
approval remains disabled, including on failed or conflicting restoration. Failed reads expose account recovery
without treating failure as authentication or authorizing a wallet operation.

Regression tests execute the browser controller with a stale restored proposal,
brief/reference invalidation, hung headers, hung JSON, unavailable wallet
metadata and a successful retry. The stale-label fixtures fail on the original
revision. Run:

```
node --test scripts/test-posting-auth.js scripts/test-account-navigation.js scripts/test-posting-brief.cjs scripts/test-posting-reference.js scripts/test-posting-session.js scripts/test-funding-readiness.js scripts/test-webmcp.js scripts/test-creator-review.js
python scripts/check-site.py
```

Scope: reversible frontend presentation/read recovery; no API, authentication,
wallet, payment or storage contract changes. The open PR queue was inspected;
posting UI PRs may need a normal rebase if they touch the same controller, with
no migration required. This is an incident repair; no third-party messaging or
payment authority is implied. Rollback is a revert of this frontend change.
Production prevention is verified only after the reviewed commit is deployed.
