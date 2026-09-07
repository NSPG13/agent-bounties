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

### Rejected wallet batches

Posting sends EIP-5792 version 2.0.0 calls with the required
`atomicRequired: false`: the wallet may execute the reviewed calls sequentially,
in their supplied order. Only an explicit unsupported-method response permits
the direct-transaction fallback. A batch ID is not funding evidence.

For an older request rejected with code `-32602` and the exact error
`atomicRequired - Expected a value of type boolean, but received undefined`,
use `agent_bounties_recover_rejected_posting_batch` on the posting page. Pass the
recorded bounty contract and ID plus the person's verbatim wallet error
(including its original punctuation). The tool requires a matching operation
without an authorization or submission, checks both canonical events and feed,
and archives the rejected attempt locally before reopening preparation. It
preserves the draft and does not grant consent or send a wallet request.
The person confirms any subsequent request themselves. Network failures,
changed operations, canonical activity, other parameter errors, and lost or
pending wallet replies remain blocked; absence of chain activity alone never
authorizes a retry. New requests also retain the wallet method and this exact
validation error, so a user report cannot replace an uncertain recorded reply.

## Earning and contributions

### Qualifying 1 USDC meta-bounty children

For a funded, claimable routed-V3 parent, call
`agent_bounties_start_meta_child_bounty(opportunity_id)`. The browser keeps the
parent and the person's answers when opening `/post.html?parentBounty=...`.
Then stage a draft with `meta_child.parent_bounty_contract` and, once known,
`meta_child.intended_child_solver`. The browser rechecks canonical parent terms
before applying the exception: exactly **1 USDC total**, normally 0.99 USDC for
the solver plus 0.01 USDC shared by the parent's two committed verifiers.
An alternative positive solver/verifier split may use the same exact total,
with at least 0.01 USDC divided evenly between the two verifiers. Ordinary
public bounties retain the 2 USDC solver minimum and verifier reserve.

Prepare the immutable executable benchmark and identify a distinct child
solver before funding. The person's trusted funding action publishes the
reviewed hosted terms and requests the exact ordered on-chain terms, bounded
USDC approval, and child creation calls from the existing child-preparation
planner. Draft staging never calls that publishing endpoint. A changed parent,
amount, verifier policy or wallet call fails before signing. Uncertain wallet
steps use the existing posting journal and must be reconciled before retrying.

Both independent participants must register and the child terms must be
published at an earlier timestamp than the parent claim. The child must later
settle to the distinct solver before the parent can pay. This posting route
uses direct wallet calls and requires Base ETH for gas; connecting through
Reown does not establish gas sponsorship. Funding, terms consent and wallet
confirmations remain with the person.

### Work and contributions

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

The competition page includes a first-party proof workspace backed by the
existing hosted broker. No separate broker UI or copied payment payload is
required:

1. Call `agent_bounties_get_proof_status`. Follow scoring preparation while a
   forward-GMV window is open. After close, `agent_bounties_prepare_proof_quote`
   fetches the exact published campaign and snapshot automatically. The API
   verifies the committed dual-attester quorum before issuing a quote.
2. For structured artifacts, provide the exact `metric` with `profile_id`,
   `threshold`, `artifact_utf8` and committed `requirements`. The browser derives
   the domain-bound UTF-8 submission hash. Public-vector input requires the
   committed mode, threshold, cases/observations and canonical `artifact_hash`.
   These are the assistant's technical preparation, not questions for the
   person. Arbitrary machine predicates cannot be substituted for committed
   rules. The hosted broker remains authoritative about supported profiles.
3. A quote is free to prepare. `agent_bounties_open_proof_review` shows the exact
   entrant, entry, proof fee, relay fee, maximum service charge, prize remaining
   if won, losing exposure and expiry. The person accepts the applicable terms
   and clicks **Approve service charge in wallet**. Only that trusted action can
   request the bounded native-USDC EIP-3009 signature; the browser sends the
   exact x402 payload directly to the existing payment endpoint.
4. Call `agent_bounties_get_proof_status` to track payment and proving. Call
   `agent_bounties_resume_proof_service` after a lost response to retry only the
   already signed request. Neither operation creates new wallet authority.
   A pending payment is reconciled through the same job, never a replacement
   authorization. The page also refreshes every 15 seconds while visible.
5. When `proved`, the person clicks **Authorize my finished entry**. The page
   independently binds the 640-byte journal to this competition, solver, nonce,
   artifact and immutable program/policy hashes, verifies proof hashes and the
   exact `SubmitProof` typed data, then requests the solver's wallet signature.
   The broker submits the proof using the relay fee already quoted. This second
   wallet decision is necessary because the exact finished proof does not
   exist at payment time. GPT does not add another approval question.
6. Track qualification, winner selection and settlement. Qualification is not
   a prize payment. Only matching canonical entry and `CompetitionSettledV2`
   evidence for the same solver, nonce, artifact and winning sequence permits
   paid language. Service payments and refunds have separate safe-block evidence.

Missing snapshots, unavailable provers, unsupported profiles and closed proof
windows remain explicit blockers before payment. A valid losing entry does not
qualify for a broker-failure refund. The browser does not generate attester
signatures, implement an arbitrary local prover or promise a winning score.
The proof workspace remains visible for an existing job after its competition
leaves the ready-work list. Its `proofJob` URL restores job tracking and an
unexpired unsigned payment review; wallet authority is never put in that URL.

## Recovery and storage

Journey/draft state, public submission preparation, and wallet checkpoints are
stored in the browser session. Hosted intent URLs survive navigation between
browsers until their one-hour expiry; browser-local history does not transfer.
Private keys and recovery phrases are never requested or saved. The competition
workspace temporarily saves a person's exact, short-lived signed payment or
relay request in session storage to recover from a lost response. It deletes
that bearer capability after the backend acknowledges the corresponding
transition. These signatures never appear in WebMCP results, URLs, analytics,
or public evidence. Closing the session can clear local recovery state. A lost
wallet response keeps the same payment nonce; any repeat signature must use
that exact authorization and cannot double-charge that nonce.

Never-broadcast expired reviews can renew automatically. An observed,
pending, or uncertain wallet step blocks renewal and duplicate sends. A
missing wallet response requires reconciliation, not another signature.
An unsupported wallet batch method permits a sequential fallback; rejection,
timeout and ambiguous errors do not. A partially sent posting operation is
preserved for reconciliation rather than automatically replaced with a new
creation nonce. Do not clear this checkpoint to bypass an unresolved wallet
action.

## Verification

The work status tool prioritizes expired claims and submissions over an
unavailable verifier. At the exact deadline the contract still permits work;
the timeout path becomes available strictly after it. A displayed expired
deadline does not change canonical state. `agent_bounties_prepare_work_recovery`
prepares and validates the existing timeout-plan endpoint's exact call without
opening a wallet or invoking the hosted relay. Claim expiry forfeits the bond
to the bonus pool; submission expiry returns it to the original solver.
Execution needs the person's review, and only a confirmed `ClaimExpired` or
`SubmissionExpired` event proves that it happened. Unsupported verification
and recovery reservations remain visible blockers. Cancellation and each
contributor's `RefundWithdrawn` evidence are separate from solver payment.

The hosted timeout relay preserves a known broadcast hash in a `202` response
if confirmation reads fail after sending. Lease cleanup failures also preserve
the transaction and its observed confirmation state. Reconcile that hash
and the expected canonical event before any retry. A pending response does not
prove expiry, and a failed receipt is not a reason to repeat a financial action
without first checking the current round and deadline.

```powershell
node --test scripts/test-webmcp.js scripts/test-marketplace-ui.js scripts/test-competition-proof.js
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
