# Maintainer release notice: guided wallet funding

This notice accompanies the user-authorized website deployment. It describes
contributor impact and recovery; it is not a bounty or payment approval.

## Scope and reason

Replace generic wallet purchase links with a saved, guided top-up flow. Add
provider-neutral account APIs and WebMCP preparation/status tools, live Coinbase
hosted sessions and MoonPay quotes, matching-wallet MetaMask continuation,
durable purchase guards and a narrowly scoped existing-factory creation relay.
The reported desktop failures left people without clear next actions and risked
losing the wallet, bounty and pending purchase across providers.

## Impact and repair path

Affected areas are posting/onramp assets, account APIs, existing x402 relay nonce
coordination, additive migration 0041, OpenAPI digest and agent guidance. No
contract ABI, custody model, reward rates or gas budgets change. Existing MoonPay
callers remain compatible. Both rollout flags start disabled.

The open PR queue was inspected before edits: 50 open results were returned,
including #1447 (digest automation), #1446 (discovery filtering), #1438
(homepage palette), #1437 (claim deadline), #1436 (verifier email), #1196
(credential flow), and #910 (analytics migration reservation). No external PR
code was executed and no maintainer review or status approval is implied.
The open queue was rechecked before preparing the website release on September 17.

Potential overlap is in account flow, public guidance, API annotations/OpenAPI
digest and shared frontend assets; no known bounty artifact submission needs
reworking. Preserve unrelated PRs. Rebase affected changes and rerun:

```sh
python3 scripts/check-migration-history.py
python3 scripts/check-site.py
python3 scripts/check-agent-discovery-contract.py
cargo test -p api
node scripts/test-guided-topup.cjs
node scripts/test-posting-layout.cjs
```

Keep the migration additive, regenerate the OpenAPI digest from the actual API,
and do not replace pending purchase/relay records. A collaboration branch is
appropriate for useful overlapping work that is not ready for main.

## Release gate

See [the implementation and release checklist](guided-wallet-funding.md).
The local creation rehearsal exceeds the default gas cap after the existing
safety margin; changing the cap requires explicit policy review. Live reserve,
provider eligibility, physical wallet/browser compatibility and the separately
authorized live canary remain release gates. Flags can be rolled back separately
while reconciliation continues.

## Distribution feedback request

* How did you find Agent Bounties?
* What made this bounty or project worth participating in?
* Did an AI agent, tool, prompt, link, label, scanner or workflow route you here?
* What would make participation easier or more trustworthy?
