# Measured posting-flow verification

**Ready for local review; production remains blocked.** The supported Coinbase
creator-review journey now completes after cancellation, top-up and reload.
Local API/database behavior is real; OAuth, Coinbase transport and Base events
in the automated journeys are simulated. No real bounty, payment, legal
acceptance or deployment was performed.

## Acceptance scorecard

Targets were fixed before implementation: 100% eligible local completion,
continuity and recoverable completion; zero unauthorized/duplicate effects or
false success. All ratios below name their actual denominators.

| Metric | Baseline | Final result | Evidence in `target/posting-metrics/` | Status |
| --- | --- | --- | --- | --- |
| Eligible posting completion | 12/26 (46.15%) | 26/26 (100%) | `baseline-corrected.json`, `final.json` | PASS locally |
| Recovery completes same operation | 8/22 (36.36%) | 22/22 (100%) | Same scenario records | PASS locally |
| Required fields preserved | 144/144 observed; other comparisons not reached | 234/234 (100%) | Per-field comparisons in `final.json` | PASS locally |
| Unchanged approvals survive | Reload invalidated terms approval in affected baseline paths | Terms retained throughout 26/26 journeys; one legal acceptance each, including reload | `final.json`, `legal_acceptances` | PASS locally |
| Changed approvals invalidated; no unauthorized/duplicate SDK effects or false success | Dedicated safety matrix not run at baseline | 24/24 scenarios pass; zero excess/unauthorized SDK calls or false-success observations | `safety-final.json` | PASS locally |
| First-time comprehension | No participants | Pilot waived by the user/maintainer on 2026-09-17; no claimed pass rate | User instruction; `docs/posting-novice-study.md` | NOT MEASURED; not a release gate |
| Review choices explained | Dedicated content measures not run at baseline | 2/2 options include decision-maker, evidence, costs, limits and protocol; 4/4 recommendations give protocol and task reason | `communication-final.log` | PASS for templates |
| Payment information | Dedicated field measures not run at baseline | 10/10 fields across the signature and transaction reviews: amount, network, recipient, expiry, fee | Same log; wallet screenshots | PASS locally |
| Routine guidance | Dedicated content measures not run at baseline | 6/6 routine guidance templates have at most three sentences; detailed consent is explicitly excepted | `communication-final.log` | PASS for sentence count |
| WebMCP and browser continuity | Registration shim only | 92/92 recorded eligible preparation/status calls use actual registered handlers through the shim; 7/7 additional native calls succeed; 0 external handoffs in 26 journeys; 2/2 simulated sign-in returns preserve draft | `final.json`, `native-webmcp.json` | PASS for tested coverage |
| Live Coinbase and canonical Base funding | Not attempted | Not attempted | `docs/posting-live-canary.md` | UNVERIFIED |

The routine guidance policy requires one primary next action. Structured tools
return one `next_action`; the tests check this guidance, not every possible
assistant's generated answer. Sentence count and complete choice metadata do
not establish human comprehension. Automated-review posting, child bounties,
external wallets and other browsers are outside the 26-journey completion
matrix; existing regressions are not a substitute for live coverage of them.

## Scenario and evidence boundaries

Desktop 1440×1000 and phone 390×844 each run: normal, sign-in return,
cancel-before-signature with/without reload, cancel-after-signature with/without
reload, USDC top-up with/without reload, zero-ETH top-up with/without reload,
insufficient-positive-ETH top-up with/without reload, and lost SDK reply followed
by reconciliation. Completion requires matching creation, full funding,
claimability and this exact bounty in ready-to-earn inventory. The final record
captures these observed checks, operation ID and bounty contract.

The nine compared fields are operation ID, goal, acceptance criteria, worker
reward, review reserve, task window, review mode, exact ISO deadline with offset
and frozen reference. References are actual local image bytes with timestamp
and SHA-256 binding. The example is a fixed 2.00 + 0.01 USDC creator-review bounty.
The requested 200 USDC, 1-per-response/10-per-referral campaign remains unsupported
and was not converted or implemented.

Real boundaries: Rust API and isolated PostgreSQL, account wallet-ownership
challenge and EIP-191 verification, legal receipt persistence, terms publication,
creation/authorized-plan endpoints, and EIP-712 verification of genuine
signatures made with a public unfunded fixture key. The React confirmation UI
uses trusted browser events. Coinbase authentication/SDK transport, Base RPC,
receipts, indexed events and inventory are fixture responses. They do not prove
actual blockchain funding or payment.

Native WebMCP was separately discovered in the Codex inline browser: 22 tools
were advertised. Seven calls covered page context, task-specific options,
journey creation, staging, review, same-tab sign-in navigation and review after
return. The same operation, rewards and offset deadline survived; no approvals
were granted. OAuth providers were disabled on that local preview, so browser
back was explicitly recorded as the return fallback. The original production
tab remained untouched. Native OAuth completion is UNVERIFIED.

## Fixes supported by the measurements

- WebMCP staging no longer overwrites the normalized reviewed draft with raw
  input. This caused approval hashes to change on reload.
- Known-unsent signed requests can continue after cancellation or top-up.
  The exact signed payload stays in the original tab, bound to account, wallet,
  draft and operation; only its immutable digest is synchronized.
- A unique PostgreSQL submission reservation must succeed immediately before
  SDK invocation. Concurrent attempts have one winner. After that boundary,
  uncertain replies remain reconciliation-only; no new nonce or resend.
- Unchanged legal receipts survive reload within the same binding. Changed
  task terms disable payment immediately; changed legal policy blocks reuse.
- Page, agent guidance and native tools share the two review choices. Both
  payment reviews display Base explicitly and preserve small nonzero fees.
- Shared asset cache versions move together so new tools cannot load stale
  workflow helpers. Other pages change only those version references.

A server reservation followed by a browser crash remains conservatively
uncertain, even if the SDK never actually ran. Another device cannot recover
this tab's retained signature. Expired or changed-policy signed operations do
not auto-renew. These boundaries preserve authority; they are not tested as
recoverable known-unsent journeys.

## Test corrections and scope additions

Before product edits, the sign-in harness queried a nonexistent `.draft`
property; it was corrected to documented `status=staged` and the two affected
baseline cases rerun. Original and corrected logs remain retained. Baseline
failures had no rate-limit errors.

An early smoke run reached the fixture account's draft limit after accumulated
runs; cleanup now targets only that public fixture account in the guarded
loopback test database. A concurrency test sometimes serialized naturally and
made only one attempt; a test-only request barrier now forces both distinct
reservations to contend. The required one-winner threshold did not change.

The added legal-policy-change case expands the separate safety matrix from
22 to 24 scenarios. It does not change the baseline/final 26-journey denominator.
The initial safety run found an enabled payment button after changed terms;
that UI defect was fixed and retested. No SDK request occurred in that failure.

## Safety and supporting gates

Final results: **24/24 safety scenarios**, **11/11 original browser regressions**,
**181/181 focused regressions**, **9/9 API auth tests**, **3/3 database tests**
(including real concurrency), and **10/10 content/payment-boundary checks**.
Overlapping checks are not added into a single success total. The safety suite covers missing legal acceptance,
synthetic clicks, repeated clicks, changed wallet, expired authorization, SDK
timeout, wrong inventory, concurrent tabs, changed terms, private payload
exclusion, tampering and changed legal policy, on both viewports. It measures
SDK effects, not actual financial effects on Base.

Other gates: Rust API builds; API auth and real database tests pass; the locked
Coinbase bundle builds; dependency compatibility and site/asset checks pass.
Wallet-origin configuration checks use deterministic fixtures, despite their
console wording; they are not a live Coinbase origin authorization check.
Logs, screenshots, source revision and timestamped SHA-256 evidence bindings
are retained locally in `target/posting-metrics/` (not committed).

## Reproduce the real API/database scenarios

Requirements: Node 22+, installed locked wallet dependencies and Playwright
Chromium, Rust, and PostgreSQL (`initdb`, `pg_ctl`, `createdb`, `psql` on PATH).
Run from the repository root with free ports 38130–38132. Use only a disposable
database; the fixture key and session secret below are public test values.

```bash
npm ci --prefix tools/coinbase-embedded-wallet --ignore-scripts --no-audit --no-fund
npm rebuild --prefix tools/coinbase-embedded-wallet esbuild
(cd tools/coinbase-embedded-wallet && npx playwright install chromium)
cargo build -p api --locked

posting_pg=$(mktemp -d /tmp/ab-posting-test.XXXXXX)
initdb -D "$posting_pg" -U posting_test -A trust --no-locale
pg_ctl -D "$posting_pg" -l "$posting_pg/server.log" \
  -o '-h 127.0.0.1 -p 38132 -k /tmp' -w start
createdb -h 127.0.0.1 -p 38132 -U posting_test posting_test

env -i API_BIND_ADDR=127.0.0.1:38131 \
  DATABASE_URL=postgres://posting_test@127.0.0.1:38132/posting_test \
  AUTH_SESSION_SECRET=local-posting-test-only-not-a-real-secret \
  SITE_POSTING_DRAFTS_ENABLED=true \
  PUBLIC_BASE_URL=http://127.0.0.1:38131 \
  WEBSITE_BASE_URL=http://127.0.0.1:38130 \
  "${CARGO_TARGET_DIR:-target}/debug/api" > "$posting_pg/api.log" 2>&1 &
posting_api_pid=$!
# Wait until the API reports its loopback listener in that log.
POSTING_TEST_API_URL=http://127.0.0.1:38131 \
  POSTING_TEST_DATABASE_URL=postgres://posting_test@127.0.0.1:38132/posting_test \
  PSQL_BIN="$(command -v psql)" \
  POSTING_METRICS_OUT=target/posting-metrics/final.json \
  node --test tools/coinbase-embedded-wallet/test-posting-metrics.mjs

kill "$posting_api_pid"
pg_ctl -D "$posting_pg" -m fast -w stop
```

Run the metrics, safety and original browser suites **sequentially**: they
share a fixture port and reset only the public fixture account's drafts. Set
`POSTING_SAFETY_OUT` for the safety JSON, and `POSTING_TEST_EVIDENCE_DIR` for
wallet screenshots. The fixture deliberately rejects hosted API/database URLs.

## Remaining release gates

| Blocker | What clears it |
| --- | --- |
| Real embedded-wallet authentication and Base completion unverified | Human-approved staged canary using the exact selected Coinbase wallet and confirmed canonical creation/funding/claimability/inventory evidence; see `posting-live-canary.md` |
| Live test decision | The user identifies as maintainer and authorized actual Coinbase testing on 2026-09-17. Wallet confirmations remain theirs; no separate maintainer identity approval is required. Production promotion follows the observed canary result. |

No deployment or real spending is authorized by this report. Only a confirmed
canonical settlement event can prove solver payment.

## Maintainer deployment decision — 2026-09-17

The user identifies as maintainer and explicitly instructed: “if it would be
easier to test live after deployment then do so”. This authorizes deploying
the tested application changes and running the bounded live Coinbase test
after deployment, replacing the prior staging-first requirement for this
change. The five-person pilot is waived. Human publication, legal acceptance
and wallet confirmations remain required; no payment is claimed in advance.
