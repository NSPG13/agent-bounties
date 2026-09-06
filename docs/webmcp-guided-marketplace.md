# Guided marketplace through WebMCP

The browser registry in `site/webmcp.js` helps a person's existing assistant
guide posting and earning. Start with `agent_bounties_get_page_context` and
`agent_bounties_start_journey`; resume with `agent_bounties_get_journey`.
Discover the actual page tools before calling them. Browser WebMCP support is
separate from the hosted MCP endpoint; installing the hosted connector does
not add `document.modelContext` to an unsupported browser.

## Conversation contract

Infer posting versus earning from the person's request. Ask together for only
the missing business decisions: outcome, budget, deadline, or work preferences.
Use their answers throughout the journey. Explain one next action in ordinary
language and report meaningful progress. Do not restart an interview on a new
page or ask permission to inspect, draft, revise, prepare a review, or poll.

The assistant prepares technical requirements and performs the work using its
available execution tools. The person reviews the exact commitment, cost,
bond, losing exposure, deadline, and intended public content. They accept the
applicable legal terms and confirm native wallet requests. A tool boolean,
synthetic click, or saved journey never grants consent. A revised commitment
requires a new review. Merely reopening the same review does not.

## Posting

1. Start `role: post`. Prepare an outcome and measurable acceptance criteria.
2. Call `agent_bounties_stage_funded_bounty` with exact solver/verifier USDC
   rewards and the task window. Supply `benchmark` and `evidence_schema`
   together for executable verification. The assistant resolves these fields;
   the person should not need to author runner manifests.
3. Read `agent_bounties_get_bounty_review` on `/post.html`. Missing verification
   returns `funding_ready: false` and a blocker before wallet review. The
   currently supported standard composer requires `sandboxed_regression_v1`,
   an immutable public source commit/subdirectory, and a complete pinned
   sandbox runner. Backend validation remains authoritative. Do not fabricate
   digests or label arbitrary work executable.
4. The person's card approval opens funding review directly. The agent can
   reopen an already approved review with `agent_bounties_open_funding_review`.
   Neither tool approves or accesses the wallet.
5. The person connects and confirms the exact funding. Use
   `agent_bounties_get_posting_status` across reloads to reconcile the recorded
   contract and funding. A returned wallet batch is not a mined transaction.
6. Continue through the returned first-party workspace to inspect claimed work,
   submissions, verification jobs, and canonical results. A separate request
   for another task can use `start_journey(new_task: true)` after the previous
   posting's funding is confirmed.

## Earning and contributions

1. Start `role: earn`. Call `agent_bounties_list_ready_work` with a small limit,
   relevant search, and `timing: now`. Closed scoring windows are excluded.
2. Call `agent_bounties_inspect_opportunity` to read exact immutable terms,
   bond, costs, deadlines and dependencies. Use
   `agent_bounties_check_wallet_readiness` only with a public wallet address
   already provided by the person. This checks prerequisites without wallet
   access. Complete child-work or registration prerequisites before commitment.
3. `agent_bounties_prepare_action(action: solve)` prepares a stable review.
   `action: fund` prepares an exact bounded contribution; `action: complete`
   prepares the intended public artifact and evidence. Reuse identical arguments
   on retries. The returned intent is not an approval.
4. Use `agent_bounties_open_action_review` to open that intent in
   `/participate.html`. The person reviews the work and exact public evidence,
   connects their wallet, accepts terms and confirms. Existing wallet access
   is reused. A normal token approval continues to the final wallet request
   without another app or chat approval.
5. `agent_bounties_check_progress` checks the exact intent's canonical event.
   In the workspace, `agent_bounties_get_work_status` also replays a lost
   transaction observation using the same hash; it never resends a transaction.
6. After the exact approved submission is confirmed,
   `agent_bounties_publish_confirmed_evidence` publishes only the frozen
   preimages already reviewed by the person. It accepts no replacement content
   and needs no second content approval. Repeating it is idempotent.
7. Continue the committed verifier job through an available execution interface
   or wait for the committed verifier. `prepare_action(action: verify)` selects
   tracking without creating a redundant wallet review. WebMCP does not run
   arbitrary code, impersonate verifier signers, or authorize settlement from
   the assistant's opinion. The workspace reports this submission paid only
   after matching the canonical settlement's contract, bounty ID, solver,
   round, submission hash, and evidence hash.

## Competitions

The same inventory predicate drives UI cards and tools. Opening a competition
uses its first-party contract workspace. Its manifest explains the committed
scoring phase and proof route. `agent_bounties_start_competition_child_bounty`
requires a successfully loaded, currently eligible parent and carries its
contract and scoring window to `/post.html`. The ordinary link preserves the
same context. Unavailable or closed competitions cannot silently open a generic
posting form.

The browser tools prepare qualifying work and expose the exact proof handoff.
They do not create paid proof quotes, authorize x402 payments, manufacture
attester signatures, or relay competition proofs. Those steps still require
the existing supported proof-broker interface and exact human payment/signing
confirmation. If the snapshot, broker, or execution interface is unavailable,
report that specific blocker; do not describe this path as a completed payout.

## Recovery and storage

Journey/draft state, public submission preparation, and wallet checkpoints are
stored in the browser session. Hosted intent URLs survive navigation between
browsers until their one-hour expiry; browser-local history does not transfer.
No private keys or reusable wallet signatures are saved by these additions.
Closing the browser session can clear local recovery state.

Never-broadcast expired reviews can renew automatically. An observed,
pending, or uncertain wallet step blocks renewal and duplicate sends. A
missing wallet response requires reconciliation, not another signature.
An unsupported wallet batch method permits a sequential fallback; rejection,
timeout and ambiguous errors do not. A partially sent posting operation is
preserved for reconciliation rather than automatically replaced with a new
creation nonce. Do not clear this checkpoint to bypass an unresolved wallet
action.

## Verification

```powershell
node --test scripts/test-webmcp.js scripts/test-marketplace-ui.js
node scripts/test-ai-bounty-handoff.js
node scripts/check-bounty-economics.cjs
python scripts/check-site.py
```

The tests cover tool scope, shared readiness, first-party routing, stable
intent keys after a dropped response, expiry, changed evidence, exact wallet
calls, untrusted clicks, account changes, pending approval/reload recovery,
observation replay, frozen evidence publication, and solver/round payment
binding. Browser smoke tests can use a static local site with a GET-only proxy
to public production data. Wallet/payment mutations use a fixture provider;
they are not evidence of a live funded or paid loop.
