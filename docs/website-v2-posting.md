# Website V2 wallet posting

New website USDC postings use the existing autonomous-v2 factory on Base. The
retired `protocol.json` remains a historical V1 descriptor; new posting loads
`protocol-v2.json`. This change does not deploy a contract or change the API.

The creator approves the draft, reviews the reward plus the factory's 7.5% fee
on the solver reward (rounded up to one micro-USDC), and explicitly accepts that
the wallet pays Base ETH network fees. The website publishes the exact immutable
terms, checks the V2 quote and creation plan, then requests two wallet
transactions: an allowance for the exact full target and creation with full
funding. It waits for the first transaction before requesting the second.
A signature, allowance, receipt or returned hash alone cannot mark the bounty
funded; the existing canonical event and inventory reconciliation still applies.

`posting-v2-plan.js` independently encodes the requested creation, computes its
bounty ID and CREATE2 address, checks the fixed factory and implementation, and
compares every wallet call. A `ReceiveWithAuthorization` in the response is not
sent to the retired V1 sponsored relay. V2 sponsored creation remains unavailable.
Embedded wallets limited to the old reviewed signature route must choose a
compatible external Base wallet or the separate card route. Held verifier and
Workspace profiles remain held by their existing API checks.

If approval succeeds and creation fails or its reply is lost, the existing
operation and allowance transaction stay recorded. The website blocks a fresh
payment; it does not clear that record or automatically replay either call.
The old signed V1 continuation can reconcile canonical success, but cannot start
or replay retired V1 creation. A separate recovery procedure is needed for a
partially completed direct transaction sequence; do not claim automatic resume.

Local checks (no production payments):

```sh
node --test scripts/test-posting-v2-plan.cjs scripts/test-funding-readiness.js scripts/test-sponsored-posting.cjs scripts/test-posting-session.js scripts/test-terms-publication.cjs scripts/test-bounty-composer-retirement.cjs
node scripts/test-posting-v2-browser.cjs
node scripts/test-manual-posting-browser.cjs
python3 scripts/check-site.py
```

The ABI and predicted-address test uses the retained Foundry/Anvil fixture.
Browser tests isolate every external request and cover exact payment calls,
changed calldata, a rejected or lost second request, a wrong network, no ETH,
account or chain changes during estimation, and another device changing the
saved draft revision while the quote is pending. These are local qualification,
not evidence of production deployment or a live customer payment.
