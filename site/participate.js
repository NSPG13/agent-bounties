(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root?.document) api.start(root, root.document);
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";
  const TOKEN = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
  const HASH = /^0x[0-9a-f]{64}$/i;
  const lower = (value) => String(value || "").toLowerCase();
  const stable = (value) => value && typeof value === "object" ? Array.isArray(value) ? `[${value.map(stable).join(",")}]` : `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${stable(value[key])}`).join(",")}}` : JSON.stringify(value);
  // A URL selects presentation only; it never establishes verifier authority.
  function verifierReviewState(item, now = Math.floor(Date.now() / 1000)) {
    if (item.status !== "submitted") return { title: "Check the current bounty", summary: "There is no work to review right now. Check the latest status below.", deadline: "" };
    const submission = (item.events || []).filter((event) => event.kind === "submission_added"
      && lower(event.contract_address) === lower(item.bounty_contract) && event.bounty_id === item.bounty_id
      && event.id && Number.isSafeInteger(event.block_number) && event.block_number > 0
      && Number.isSafeInteger(event.log_index) && event.log_index >= 0)
      .sort((a, b) => b.block_number - a.block_number || b.log_index - a.log_index)[0];
    const deadline = submission?.data?.verification_expires_at;
    if (item.terms_valid !== true || !Number.isSafeInteger(submission?.data?.round) || submission.data.round <= 0
      || !Number.isSafeInteger(deadline) || deadline <= 0 || !Number.isFinite(new Date(deadline * 1000).getTime())) return {
      title: "Refresh review details", summary: "We can’t load the work or its review date. Try again before you decide.", deadline: "",
    };
    const date = new Date(deadline * 1000).toLocaleString();
    if (now >= deadline) return { title: "The review deadline has passed", summary: "The time to review this work has ended. Refresh to see what happened next.", deadline: `Review deadline: ${date} (your local time).` };
    return { title: "Review the solution", summary: "Read the work below and check it against the bounty’s rules. Your AI can help. Only the chosen review wallet can sign your decision.", deadline: `Review by ${date} (your local time). This date is for your review.` };
  }
  function recoveryStatus(item, now = Math.floor(Date.now() / 1000)) {
    if (["paid", "cancelled"].includes(item.status)) return null;
    const events = (item.events || []).filter((event) => lower(event.contract_address) === lower(item.bounty_contract)
      && event.bounty_id === item.bounty_id && event.id && Number.isSafeInteger(event.block_number) && event.block_number > 0
      && Number.isSafeInteger(event.log_index) && event.log_index >= 0)
      .sort((a, b) => b.block_number - a.block_number || b.log_index - a.log_index);
    const claim = events.find((event) => event.kind === "bounty_claimed");
    const submission = events.find((event) => event.kind === "submission_added"
      && event.data?.round === claim?.data?.round && lower(event.data?.solver) === lower(claim?.data?.solver));
    const active = item.status === "claimed" ? claim : item.status === "submitted" ? submission : null;
    const deadline = active?.data?.[item.status === "claimed" ? "claim_expires_at" : "verification_expires_at"];
    if (Number.isSafeInteger(deadline) && deadline > 0 && Number.isSafeInteger(now) && now > deadline) {
      const isClaim = item.status === "claimed", action = isClaim ? "expire_claim" : "expire_submission";
      return { action, status: "timeout_review_available", deadline, round: active.data.round,
        plan_endpoint: `/v1/base/autonomous-bounties/${isClaim ? "expire-claim-plan" : "expire-submission-plan"}`,
        effect: isClaim ? "The work deadline passed. Expiry releases the claim and moves its bond to the bounty bonus pool."
          : "The verification deadline passed. Expiry returns the bond to the original solver and releases the submission.",
        instructions: "Prepare the exact timeout review. The contract rechecks its deadline and current state; a plan is not an executed expiry. Do not keep working or wait indefinitely for a verifier after expiry.",
        expected_event: isClaim ? "claim_expired" : "submission_expired", confirmed: false };
    }
    if (["claimed", "submitted"].includes(item.status) && (!active || !Number.isSafeInteger(deadline) || deadline <= 0)) {
      return { action: "refresh_canonical_state", status: "recovery_evidence_unavailable", instructions: "Current round and deadline evidence is incomplete. Refresh canonical state before proposing recovery or more work.", confirmed: false };
    }
    if (item.verification_ready !== true) return { action: "resolve_verification_blocker", status: "verification_blocked",
      instructions: item.verification_readiness_reason || "Restore the exact committed verifier or prepare an owner-authorized cancellation when the contract allows it. Do not claim, fund a child, or replace immutable verifier authority.", confirmed: false };
    return null;
  }
  function validateSubmission(submission, details, contract, wallet, bountyId, requireFileCheck = false) {
    const evidence = submission?.evidence_publication;
    let expectedEvidence = details.evidence;
    const checked = submission?.checked_artifact;
    if (requireFileCheck && !checked) throw new Error("The files have not been checked yet. Try again before signing.");
    if (checked) {
      if (checked.artifact_reference !== details.artifact_reference
        || checked.source_subdirectory !== (details.evidence.source_subdirectory ?? ".")
        || !/^sha256:[0-9a-f]{64}$/.test(checked.source_snapshot_digest)
        || !Number.isSafeInteger(checked.file_count) || checked.file_count < 1
        || !Number.isSafeInteger(checked.total_bytes) || checked.total_bytes < 0
        || evidence?.evidence?.source_snapshot_digest !== checked.source_snapshot_digest
        || (details.evidence.source_snapshot_digest != null && details.evidence.source_snapshot_digest !== checked.source_snapshot_digest)) throw new Error("The checked files differ from the work you chose. Do not sign.");
      expectedEvidence = { ...details.evidence, source_snapshot_digest: checked.source_snapshot_digest };
    }
    if (submission?.network?.chain_id !== 8453 || lower(submission.bounty_contract) !== contract || lower(submission.solver) !== wallet
      || submission.bounty_id !== bountyId || !Number.isSafeInteger(submission.round) || submission.round <= 0
      || !HASH.test(submission.submission_hash) || !HASH.test(submission.evidence_hash)
      || evidence?.network !== "base-mainnet" || lower(evidence.bounty_contract) !== contract || lower(evidence.solver_wallet) !== wallet
      || evidence.bounty_id !== bountyId || evidence.round !== submission.round
      || evidence.artifact_reference !== details.artifact_reference || stable(evidence.evidence) !== stable(expectedEvidence)) throw new Error("The prepared submission differs from the exact public evidence you reviewed.");
  }
  function validateCalls(calls, { action, contract, wallet, amount, submission }, evm) {
    const selector = (signature) => evm.keccak256Hex(evm.textHex(signature)).slice(0, 10);
    const approve = selector("approve(address,uint256)") + evm.addressWord(contract) + evm.uint256Word(amount || 0);
    const expected = action === "solve" ? selector("claim()")
      : action === "fund" ? selector("fund(uint256)") + evm.uint256Word(amount)
      : action === "complete" ? selector("submit(bytes32,bytes32)") + evm.bytes32Word(submission.submission_hash) + evm.bytes32Word(submission.evidence_hash) : null;
    if (!expected || !Array.isArray(calls) || calls.length < 1 || calls.length > 2) throw new Error("Unsupported wallet plan.");
    calls.forEach((call, index) => {
      const isApproval = calls.length === 2 && index === 0;
      if (isApproval && action === "complete") throw new Error("Submission must not request a token approval.");
      if (lower(call.from) !== lower(wallet) || lower(call.to) !== lower(isApproval ? TOKEN : contract)
        || String(call.value_wei) !== "0" || lower(call.data) !== lower(isApproval ? approve : expected)) throw new Error("The wallet request differs from the reviewed action. No transaction was sent.");
    });
    return calls;
  }
  function validateSubmissionSignature(submission, contract, wallet, now = Math.floor(Date.now() / 1000)) {
    const typed = submission?.signing_payload, m = typed?.message, domain = typed?.domain;
    const fields = (pairs) => pairs.map(([name, type]) => ({ name, type }));
    const types = {
      EIP712Domain: fields([["name", "string"], ["version", "string"], ["chainId", "uint256"], ["verifyingContract", "address"]]),
      Submit: fields([["bounty", "address"], ["bountyId", "bytes32"], ["solver", "address"], ["round", "uint64"], ["submissionHash", "bytes32"], ["evidenceHash", "bytes32"], ["policyHash", "bytes32"], ["deadline", "uint256"]]),
    };
    if (typed?.primaryType !== "Submit" || stable(typed.types) !== stable(types)
      || domain?.name !== "Agent Bounties" || domain.version !== (submission.protocol_version === "agent-bounties/autonomous-v2" ? "2" : "1") || Number(domain.chainId) !== 8453 || lower(domain.verifyingContract) !== contract
      || lower(m?.bounty) !== contract || lower(m.solver) !== wallet || lower(m.bountyId) !== lower(submission.bounty_id)
      || String(m.round) !== String(submission.round) || lower(m.submissionHash) !== lower(submission.submission_hash)
      || lower(m.evidenceHash) !== lower(submission.evidence_hash) || lower(m.policyHash) !== lower(submission.policy_hash)
      || !HASH.test(m.policyHash) || !HASH.test(m.bountyId) || !Number.isSafeInteger(Number(m.deadline))
      || Number(m.deadline) !== submission.authorization_deadline || Number(m.deadline) <= now || Number(m.deadline) > now + 1800
      || Number(m.deadline) > submission.claim_expires_at) throw new Error("The submission signature differs from your exact claim, evidence or deadline. Refresh before signing.");
    return typed;
  }
  function validateTransferSignature(typed, { contract, wallet, amount, nonce, expiry }, evm, now = Math.floor(Date.now() / 1000)) {
    const m = typed?.message, d = typed?.domain;
    if (typed?.primaryType !== "TransferWithAuthorization" || stable(typed.types) !== stable(evm.transferWithAuthorizationTypes())
      || d?.name !== "USD Coin" || d.version !== "2" || Number(d.chainId) !== 8453 || lower(d.verifyingContract) !== TOKEN
      || lower(m?.from) !== wallet || lower(m.to) !== contract || String(m.value) !== String(amount) || String(m.validAfter) !== "0"
      || !HASH.test(m.nonce) || nonce && lower(m.nonce) !== lower(nonce) || !Number.isSafeInteger(Number(m.validBefore))
      || expiry && Number(m.validBefore) !== expiry || Number(m.validBefore) <= now || Number(m.validBefore) > now + 3600) {
      throw new Error("The wallet signature differs from your exact USDC amount, destination or expiry. No signature was requested.");
    }
    return typed;
  }
  async function start(win, doc) {
    const flow = win.AgentBountiesWorkflow, client = flow.createClient(win), evm = win.AgentBountiesEvm;
    const find = (s) => doc.querySelector(s), put = (s, value) => { find(s).textContent = value; };
    const params = new URLSearchParams(win.location.search), contract = lower(params.get("bountyContract"));
    const network = params.get("network") || flow.NETWORK;
    const reviewVisit = params.get("role") === "verifier";
    if (reviewVisit) doc.body?.classList.add("verifier-review-visit");
    let intentId = params.get("intent"), intent = null, item = null, wallet = null, provider = null, calls = null, submission = null, busy = false;
    const recordKey = `agent-bounties.wallet-step.v1:${intentId || contract}`;
    let record;
    try { record = JSON.parse(win.sessionStorage.getItem(recordKey) || "{}"); } catch (_) { record = {}; }
    const saveRecord = () => win.sessionStorage.setItem(recordKey, JSON.stringify(record));
    function message(error) { put("[data-work-status]", error.message || String(error)); }
    async function canonicalItem() {
      const items = await client.request(`/v1/base/autonomous-bounties/feed?network=${network}&claimable_only=false`);
      const current = Array.isArray(items) && items.find((entry) => lower(entry.bounty_contract) === contract);
      if (!current || current.terms_valid !== true || current.validation_errors?.length) throw new Error("Current verified terms are unavailable. Return to the marketplace and choose another bounty.");
      return current;
    }
    function scopeEvents(events) {
      return events.filter((event) => lower(event.contract_address) === contract && event.bounty_id === item.bounty_id);
    }
    async function refresh() {
      if (!flow.ADDRESS.test(contract) || network !== flow.NETWORK) throw new Error("Open a supported bounty from the marketplace.");
      item = await canonicalItem();
      const terms = item.terms.document;
      put("[data-work-title]", terms.title); put("[data-work-goal]", terms.goal);
      put("[data-work-reward]", `${Number(item.solver_reward) / 1e6} USDC`);
      put("[data-work-bond]", `${Number(item.claim_bond) / 1e6} USDC`);
      put("[data-work-funding]", `${Number(item.funded_amount) / 1e6} / ${Number(item.target_amount) / 1e6} USDC`);
      put("[data-work-costs]", "The claim bond is at risk if you miss the work deadline. Agent Bounties pays transaction gas. Any required child-bounty funding is part of your approved USDC commitment.");
      const criteria = find("[data-work-criteria]"); criteria.replaceChildren();
      for (const criterion of terms.acceptance_criteria) { const li = doc.createElement("li"); li.textContent = criterion; criteria.append(li); }
      const events = scopeEvents(item.events || []);
      const recovery = recoveryStatus(item);
      const claim = events.filter((event) => event.kind === "bounty_claimed").sort((a, b) => b.block_number - a.block_number || b.log_index - a.log_index)[0];
      const claimOwner = lower(claim?.data?.solver) || null;
      const ownsClaim = Boolean(record.wallet && claimOwner === record.wallet);
      put("[data-work-deadline]", terms.benchmark?.engine === "creator_review_v1" ? `Deliver by ${new Date(terms.benchmark.delivery_deadline * 1000).toLocaleString()}. Creator review is due within ${terms.contract_terms.verification_window_seconds / 3600} hours after submission. The claim timeout is a separate relative window.` : item.status === "claimed" && claim?.data?.claim_expires_at
        ? `Work is due ${new Date(claim.data.claim_expires_at * 1000).toLocaleString()}.`
        : terms.contract_terms?.claim_window_seconds ? `After claiming, complete the work within ${terms.contract_terms.claim_window_seconds / 86400} days.` : "Read the committed terms for the work deadline.");
      const paidEvent = record.submission && events.find((event) => event.kind === "bounty_settled" && event.id && event.block_number != null
        && lower(event.data?.solver) === record.wallet && event.data?.round === record.submission.round
        && lower(event.data?.submission_hash) === lower(record.submission.submission_hash) && lower(event.data?.evidence_hash) === lower(record.submission.evidence_hash));
      let jobs = [];
      if (item.status === "submitted") {
        jobs = (await client.request(`/v1/base/autonomous-bounties/verification-jobs?network=${network}`)).filter((job) => lower(job.bounty_contract) === contract);
      }
      if (intentId) {
        // A dropped observation response must replay the same transaction, never resend it.
        if (record.finalHash && !record.observed) {
          await client.request(`/v1/chatgpt/action-intents/${intentId}/observations`, { transaction_hash: record.finalHash, bounty_contract: contract, actor_wallet: record.wallet });
          record.observed = true; saveRecord();
        }
        intent = await client.progress(intentId);
        if (lower(intent.bounty_contract) !== contract || intent.network !== network) throw new Error("This review belongs to another bounty or network.");
        find("[data-action-review]").hidden = (intent.status !== "review_required" && !record.submissionRequest && !record.sponsoredRequest?.signature) || intent.action === "verify" || Boolean(record.finalHash || record.sending);
        put("[data-review-summary]", intent.action === "complete" ? "Review the exact public artifact and evidence, then sign your submission. Agent Bounties pays gas."
          : intent.action === "fund" ? `Contribute exactly ${intent.amount_base_units / 1e6} USDC to this bounty. Sign the exact contribution; Agent Bounties pays gas.`
          : `Claim this work with a ${Number(item.claim_bond) / 1e6} USDC bond. Sign the exact bond authorization; Agent Bounties pays gas.`);
        find("[data-public-evidence-note]").hidden = intent.action !== "complete";
        put("[data-action-details]", JSON.stringify(intent.details, null, 2));
        const help = new URL("onramp.html", win.location.href);
        help.searchParams.set("purpose", intent.action === "fund" ? "fund" : "earn");
        help.searchParams.set("amount", String(Number(intent.action === "fund" ? intent.amount_base_units : item.claim_bond) / 1e6));
        help.searchParams.set("return", win.location.href);
        find("[data-wallet-help]").href = help.href;
        put("[data-work-status]", intent.paid ? "Payment confirmed for this review." : `${intent.status.replaceAll("_", " ")}. ${intent.next_step || ""}`);
      } else put("[data-work-status]", `Current work status: ${item.status}.`);
      if (paidEvent) put("[data-work-status]", `Payment confirmed: ${Number(paidEvent.data.solver_payout) / 1e6} USDC to your submission wallet.`);
      if (recovery) {
        put("[data-work-status]", recovery.status === "verification_blocked" && item.status === "claimable"
          ? `Funded · waiting for verification readiness. ${recovery.instructions}`
          : recovery.effect || recovery.instructions);
        if (recovery.deadline) put("[data-work-deadline]", `The ${item.status === "claimed" ? "work" : "verification"} deadline was ${new Date(recovery.deadline * 1000).toLocaleString()}. Expiry still requires a confirmed on-chain event.`);
      }
      const claimable = item.status === "claimable" && item.verification_ready === true;
      find("[data-work-prepare]").hidden = Boolean(intentId) || !claimable;
      put("[data-step-title]", recovery ? "Review recovery" : item.status === "paid" ? "Canonical result" : item.status === "claimed" ? "Track the agreed work" : item.status === "submitted" ? "Verification in progress" : "Review the next step");
      put("[data-step-summary]", recovery ? recovery.instructions : item.verification_ready ? "Your AI can read the exact requirements and prepare the next action."
        : item.verification_readiness_reason || "The verifier is not ready. Do not commit to new work yet.");
      if (reviewVisit && !intentId) {
        const review = verifierReviewState(item);
        put("[data-step-title]", review.title);
        put("[data-step-summary]", review.summary);
        put("[data-work-deadline]", review.deadline);
        find("[data-work-prepare]").hidden = true;
        find("[data-work-evidence]").parentElement.open = true;
      }
      put("[data-work-evidence]", JSON.stringify({ verification_readiness: item.verification_details || { ready: item.verification_ready, reason: item.verification_readiness_reason }, verification: terms.verification_policy, benchmark: terms.benchmark, evidence_schema: terms.evidence_schema, events, jobs }, null, 2));
      const next = paidEvent ? null
        : item.status === "paid" ? { action: "show_canonical_result", instructions: "Show the recorded settlement and its actual recipient. This is not a claim that the current person earned money unless their wallet and submission are matched." }
        : item.status === "cancelled" ? { action: "review_contributor_refunds", instructions: "The bounty was cancelled. Each contributor must reconcile their own RefundWithdrawn event; cancellation alone is not a refund." }
        : recovery ? recovery
        : intent?.status === "confirmed" && intent.action === "complete" && !record.evidencePublished ? { tool: "agent_bounties_publish_confirmed_evidence", input: {} }
        : item.status === "claimed" && ownsClaim ? { action: "complete_agreed_work", instructions: "Use the current assistant's execution tools to complete and test the exact accepted work. Prepare action complete with the public artifact and evidence when it passes. No permission is needed for routine preparation." }
        : item.status === "claimed" ? { action: "track_claimed_work", instructions: "This work is reserved by the displayed claim owner. Track that solver's progress; do not start duplicate work or represent the claim as yours without matching the person's wallet." }
        : item.status === "submitted" && terms.benchmark?.engine === "creator_review_v1" ? { tool: "agent_bounties_get_creator_review", input: {}, instructions: "The creator signs the verdict. Prepare their assessment only after examining the exact submitted artifacts; a solver cannot approve their own payment." }
        : item.status === "submitted" ? { action: "continue_committed_verification", instructions: reviewVisit
          ? "Read the current submission evidence and committed verification policy. Check that the person's wallet is a designated verifier before preparing their verdict; this URL grants no authority. Use the committed verification flow and first-party signing pages."
          : "Read the returned verification job. Execute its exact committed flow through an available interface, or wait for the committed verifier. Do not ask for a new approval just to check status." }
        : { action: "review_current_requirements", instructions: "Resolve the listed prerequisites before preparing a claim. If posting, wait for a solver or continue preparing an authorized contribution." };
      return { bounty_contract: contract, bounty_id: item.bounty_id, status: item.status, terms, events, claim_owner: claimOwner, claim_owned_by_review_wallet: ownsClaim, verification_jobs: jobs,
        action: intent, paid: Boolean(paidEvent), payment_evidence: paidEvent || null, wallet_connected: Boolean(wallet), poll_after_seconds: 15, evidence_boundary: flow.BOUNDARY,
        recovery, next_action: next };
    }
    async function prepareRecovery(input = {}) {
      const current = await refresh(), recovery = current.recovery;
      if (!recovery?.plan_endpoint) return { status: recovery?.status || "no_timeout_recovery", next_action: current.next_action, paid: false };
      if (input.caller && !flow.ADDRESS.test(input.caller)) throw new Error("A valid caller wallet is required.");
      const plan = await client.request(recovery.plan_endpoint, { network, bounty_contract: contract, ...(input.caller ? { caller: input.caller } : {}) });
      const signature = recovery.action === "expire_claim" ? "expireClaim()" : "expireSubmission()";
      const selector = evm.keccak256Hex(evm.textHex(signature)).slice(0, 10);
      if (lower(plan.to) !== contract || String(plan.value_wei) !== "0" || lower(plan.data) !== selector
        || lower(plan.from) !== lower(input.caller) || plan.function !== signature) throw new Error("The recovery plan differs from the current bounty and timeout action.");
      return { status: "recovery_prepared", network, bounty_contract: contract, round: recovery.round, deadline: recovery.deadline,
        plan, effect: recovery.effect, expected_event: recovery.expected_event, confirmed: false, paid: false,
        user_confirmation_required: true, next_action: "Review the exact expiry and its bond effect before authorizing execution. No wallet, relay or payment was invoked." };
    }
    async function publishEvidence() {
      await refresh();
      if (record.evidencePublished) return { status: "evidence_published", already_published: true };
      if (intent?.status !== "confirmed" || intent.action !== "complete" || !record.submission || record.finalHash !== intent.transaction_hash
        || lower(intent.actor_wallet) !== record.wallet || lower(record.submission.bounty_contract) !== contract) throw new Error("Wait for the exact approved submission to be confirmed before publishing evidence.");
      await client.request("/v1/base/autonomous-bounties/submission-evidence", record.submission.evidence_publication);
      record.evidencePublished = true; saveRecord();
      put("[data-work-status]", "Submission confirmed and evidence published. The committed verifier can now check your work.");
      return { status: "evidence_published", paid: false, next_action: "Read the committed verifier job and continue its exact verification flow.", user_confirmation_required: false };
    }
    let loginToken = null;
    async function claimRequest(body) {
      try { return await client.request("/v1/base/autonomous-bounties/claims", body, {account:!loginToken,token:loginToken}); }
      catch (error) {
        if (![401,403].includes(error.status) || !win.AgentBountiesWalletSession) throw error;
        if (loginToken) win.AgentBountiesWalletSession.clear(win);
        put("[data-wallet-status]", "Confirm a 15-minute wallet login. This message cannot move funds; the bond uses a separate exact authorization.");
        loginToken = await win.AgentBountiesWalletSession.authenticate(win, provider, wallet, client.request, flow.apiBase(win.location));
        return client.request("/v1/base/autonomous-bounties/claims",body,{token:loginToken});
      }
    }
    async function prepareWallet() {
      if (!intent || record.finalHash || record.sending) throw new Error("Refresh the current confirmation before preparing another wallet request.");
      if (record.wallet && record.wallet !== wallet) throw new Error("Reconnect the wallet used for this pending review.");
      if (record.submissionRequest || record.sponsoredRequest?.signature) {
        calls = []; submission = record.submission;
        find("[data-wallet-confirm]").disabled = false;
        put("[data-wallet-status]", "Your signed request is preserved. Continue with the same signature; Agent Bounties pays gas.");
        return;
      }
      if (intent.status !== "review_required") throw new Error("Refresh the current action before preparing a new signature.");
      if (record.pendingHash && !await receipt(record.pendingHash)) throw new Error("Your previous wallet transaction is still pending. Reconcile it before preparing another request.");
      const actionKey = { solve: "claim", fund: "contribution", complete: "submission" }[intent.action];
      const gas = await client.request("/v1/base/gas-sponsorship");
      if (!actionKey || gas?.[actionKey]?.available !== true || gas[actionKey].customer_gas_wei !== "0") throw new Error("Gas sponsorship is temporarily unavailable. Keep this review and retry; no paid wallet transaction is needed.");
      if (intent.action === "solve") {
        const body = { idempotency_key: `participate:${intentId}:${wallet}`, network, bounty_contract: contract, solver_wallet: wallet, request_bond_sponsorship: false };
        const plan = await claimRequest(body);
        if (plan.schema_version !== "agent-bounties/agent-native-claim-v1" || plan.candidate?.network !== network
          || lower(plan.candidate.bounty_contract) !== contract || lower(plan.candidate.solver_wallet) !== wallet
          || String(plan.claim_bond) !== String(item.claim_bond)) throw new Error("Claim terms changed. Refresh and review the current bond.");
        if (plan.claim_transaction_hash) { record.wallet = wallet; acceptClaim(plan); await refresh(); return; }
        validateTransferSignature(plan.signing_payload, { contract, wallet, amount: item.claim_bond }, evm);
        record.sponsoredRequest = { action: "solve", body, typed: plan.signing_payload };
      } else if (intent.action === "fund") {
        const contribution = { bounty_contract: contract, contributor: wallet, amount: { amount: intent.amount_base_units, currency: "usdc" },
          authorization_nonce: evm.randomBytes32(), authorization_valid_before: Math.floor(Date.now() / 1000) + 1800 };
        const plan = await client.request("/v1/base/autonomous-bounties/contribution-plan", { network, contribution });
        if (plan.network?.chain_id !== 8453) throw new Error("The funding plan uses another chain.");
        validateTransferSignature(plan.eip3009_authorization, { contract, wallet, amount: intent.amount_base_units, nonce: contribution.authorization_nonce, expiry: contribution.authorization_valid_before }, evm);
        record.sponsoredRequest = { action: "fund", body: { network, contribution }, typed: plan.eip3009_authorization };
      } else if (intent.action === "complete") {
        submission = await client.request("/v1/base/autonomous-bounties/submission-preparation", { network, bounty_contract: contract, solver_wallet: wallet, ...intent.details });
        validateSubmission(submission, intent.details, contract, wallet, item.bounty_id, item.terms?.document?.benchmark?.engine === "sandboxed_regression_v1");
        if (submission.checked_artifact) {
          put("[data-work-evidence]", JSON.stringify({ files_checked: submission.checked_artifact, evidence_to_sign: submission.evidence_publication.evidence }, null, 2));
          find("[data-work-evidence]").parentElement.open = true;
        }
        validateSubmissionSignature(submission, contract, wallet);
      } else throw new Error("The committed verifier handles this step. Refresh verification status.");
      calls = []; record.calls = []; record.wallet = wallet; record.submission = submission;
      record.reviewedIntent = { action: intent.action, network, bounty_contract: contract, details: intent.details, amount_base_units: intent.amount_base_units };
      saveRecord();
      put("[data-wallet-status]", submission?.checked_artifact ? "Your files were checked. Review the file hash below, then sign. This does not mean the work passed its tests." : "Sign the exact action once. Agent Bounties pays the network fee.");
      find("[data-wallet-confirm]").disabled = false;
    }
    const providers = [];
    win.addEventListener("eip6963:announceProvider", (event) => { if (event.detail?.provider && !providers.some((entry) => entry.provider === event.detail.provider)) providers.push(event.detail); });
    async function connect(selected) {
      provider = selected;
      let accounts = await provider.request({ method: "eth_accounts" });
      if (!accounts?.length) accounts = await provider.request({ method: "eth_requestAccounts" });
      wallet = lower(accounts[0]);
      if (!flow.ADDRESS.test(wallet)) throw new Error("Choose a valid wallet.");
      if (lower(await provider.request({ method: "eth_chainId" })) !== "0x2105") await provider.request({ method: "wallet_switchEthereumChain", params: [{ chainId: "0x2105" }] });
      await refresh();
      await prepareWallet();
    }
    win.addEventListener("agent-bounties:phone-wallet-state", (event) => {
      if (event.detail?.connected && !provider && !busy && !record.finalHash && !record.sending && win.AgentBountiesPhoneWallet) connect(win.AgentBountiesPhoneWallet.provider).catch(message);
    });
    find("[data-wallet-connect]").addEventListener("click", async (event) => {
      if (!event.isTrusted || busy) return;
      try {
        if (record.finalHash || record.sending) throw new Error("A wallet submission has already started. Refresh its status or inspect the wallet; do not send a duplicate.");
        win.dispatchEvent(new win.Event("eip6963:requestProvider"));
        await new Promise((resolve) => win.setTimeout(resolve, 200));
        if (win.ethereum && !providers.some((entry) => entry.provider === win.ethereum)) providers.push({ provider: win.ethereum, info: { name: "Browser wallet" } });
        if (providers.length === 1) await connect(providers[0].provider);
        else {
          const options = find("[data-wallet-options]"); options.replaceChildren();
          for (const entry of providers) { const button = doc.createElement("button"); button.textContent = entry.info?.name || "Wallet"; button.addEventListener("click", (click) => { if (click.isTrusted) connect(entry.provider).catch(message); }); options.append(button); }
          if (!providers.length) put("[data-wallet-status]", "Open this same review link in a browser with your wallet. The prepared action is preserved; you will not need to repeat the interview.");
        }
      } catch (error) { message(error); }
    });
    async function receipt(hash) {
      const result = await provider.request({ method: "eth_getTransactionReceipt", params: [hash] });
      if (!result) return false;
      if (result.status !== "0x1") throw new Error("The transaction reverted. Refresh the bounty before trying again.");
      return true;
    }
    async function relaySubmission() {
      if (!record.submissionRequest) {
        const typed = validateSubmissionSignature(submission, contract, wallet);
        const m = typed.message;
        const signature = await provider.request({ method: "eth_signTypedData_v4", params: [wallet, JSON.stringify(typed)] });
        if (!/^0x(?:[0-9a-f]{2}){1,4096}$/i.test(signature)) throw new Error("The wallet returned an invalid or oversized signature. No relay was requested.");
        record.submissionRequest = { network, signature, submission: { bounty_contract: contract, bounty_id: m.bountyId, solver: wallet, round: Number(m.round),
          submission_hash: m.submissionHash, evidence_hash: m.evidenceHash, policy_hash: m.policyHash, deadline: Number(m.deadline) } };
        record.submission = submission; saveRecord();
      }
      const result = await client.request("/v1/base/autonomous-bounties/submission-relay", record.submissionRequest);
      const r = record.submissionRequest.submission;
      if (result.schema !== "agent-bounties/sponsored-submission-v1" || result.network !== network || lower(result.bounty_contract) !== contract
        || lower(result.bounty_id) !== lower(r.bounty_id) || lower(result.solver) !== wallet || result.round !== r.round
        || lower(result.submission_hash) !== lower(r.submission_hash) || lower(result.evidence_hash) !== lower(r.evidence_hash)
        || result.customer_gas_wei !== "0" || result.solver_paid !== false
        || result.transaction_hash && !HASH.test(result.transaction_hash)) throw new Error("The sponsor returned a different submission. Keep the signed request and reconcile; do not sign another.");
      if (!result.transaction_hash) throw new Error("Your exact submission is reserved. Retry this same signed request to check progress; no new signature is needed.");
      record.finalHash = result.transaction_hash; saveRecord();
      put("[data-work-status]", "Submission relayed with platform gas. Checking canonical confirmation.");
    }
    function acceptClaim(result) {
      if (result.schema_version !== "agent-bounties/agent-native-claim-v1" || result.candidate?.network !== network
        || lower(result.candidate.bounty_contract) !== contract || lower(result.candidate.solver_wallet) !== record.wallet
        || String(result.claim_bond) !== String(item.claim_bond)
        || result.claim_transaction_hash && !HASH.test(result.claim_transaction_hash)) throw new Error("The sponsor returned a different claim. Keep this request and reconcile.");
      if (result.claim_transaction_hash) { record.finalHash = result.claim_transaction_hash; saveRecord(); }
    }
    async function relayFundingOrClaim() {
      const request = record.sponsoredRequest;
      if (!request.signature) {
        const amount = request.action === "fund" ? request.body.contribution.amount.amount : item.claim_bond;
        validateTransferSignature(request.typed, { contract, wallet, amount }, evm);
        request.signature = await provider.request({ method: "eth_signTypedData_v4", params: [wallet, JSON.stringify(request.typed)] });
        if (!/^0x[0-9a-f]{130}$/i.test(request.signature)) { delete request.signature; throw new Error("The wallet returned no usable signature. No relay was requested."); }
        saveRecord();
      }
      if (request.action === "solve") {
        acceptClaim(await claimRequest({ ...request.body, wallet_signature: request.signature }));
      } else {
        const result = await client.request("/v1/base/autonomous-bounties/contribution-relay", { ...request.body, signature: request.signature });
        const r = result.relay;
        if (result.schema !== "agent-bounties/sponsored-contribution-v1" || result.customer_gas_wei !== "0" || result.solver_paid !== false
          || r?.network !== network || lower(r.bountyContract) !== contract || lower(r.contributor) !== record.wallet
          || String(r.amount) !== String(request.body.contribution.amount.amount)
          || r.transaction && !HASH.test(r.transaction)) throw new Error("The sponsor returned a different contribution. Keep the signed request and reconcile.");
        if (r.transaction) { record.finalHash = r.transaction; saveRecord(); }
      }
      if (!record.finalHash) throw new Error("Your signed request is reserved. Continue this same request to check progress; no new signature is needed.");
    }
    find("[data-wallet-confirm]").addEventListener("click", async (event) => {
      if (!event.isTrusted || busy || !calls || !wallet || !intent) return;
      busy = true; find("[data-wallet-confirm]").disabled = true;
      try {
        if (record.finalHash || record.sending) throw new Error("A wallet submission has already started. Refresh its status or inspect the wallet; do not send a duplicate.");
        if (record.submissionRequest) { await relaySubmission(); await refresh(); return; }
        if (record.sponsoredRequest?.signature) { await relayFundingOrClaim(); await refresh(); return; }
        const accounts = await provider.request({ method: "eth_accounts" });
        if (lower(accounts[0]) !== wallet || lower(await provider.request({ method: "eth_chainId" })) !== "0x2105") throw new Error("Your wallet or network changed. Connect again to prepare an exact review.");
        intent = await client.progress(intentId);
        const current = { action: intent.action, network: intent.network, bounty_contract: lower(intent.bounty_contract), details: intent.details, amount_base_units: intent.amount_base_units };
        if (intent.status !== "review_required" || stable(current) !== stable(record.reviewedIntent)) throw new Error("This action has changed or is already pending. Refresh progress instead of signing again.");
        const legal = { solve: "claim_bounty", fund: "fund_bounty", complete: "submit_result" }[intent.action];
        const acceptance = await win.AgentBountiesLegal.requireAcceptance({ action: legal, walletAddress: wallet });
        if (!acceptance.durable) throw new Error("The agreement could not be recorded. Retry when the service is available; no transaction was sent.");
        if (intent.action === "complete") await relaySubmission();
        else await relayFundingOrClaim();
        await refresh();
      } catch (error) { message(error); }
      finally { busy = false; find("[data-wallet-confirm]").disabled = Boolean(record.finalHash || record.sending) || intent?.status !== "review_required" && !record.submissionRequest && !record.sponsoredRequest?.signature; }
    });
    find("[data-work-prepare]").addEventListener("click", async (event) => {
      if (!event.isTrusted) return;
      try {
        const prepared = await client.prepareAction({ action: "solve", opportunity_id: `canonical:${network}:${contract}` });
        win.location.assign(prepared.authorization_url);
      } catch (error) { message(error); }
    });
    find("[data-work-refresh]").addEventListener("click", () => refresh().catch(message));
    win.AgentBountiesParticipation = Object.freeze({ refresh, publishEvidence, prepareRecovery });
    try { await refresh(); } catch (error) { message(error); }
    win.setInterval(() => { if (!doc.hidden && !busy && (intentId || reviewVisit)) refresh().catch(message); }, 15000);
  }
  return { validateCalls, validateSubmission, validateSubmissionSignature, validateTransferSignature, recoveryStatus, verifierReviewState, start };
});
