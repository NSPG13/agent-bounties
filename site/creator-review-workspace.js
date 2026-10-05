(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root?.document) api.start(root);
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";
  const lower = (value) => String(value || "").toLowerCase();
  const stable = (value) => value && typeof value === "object" ? Array.isArray(value) ? `[${value.map(stable).join(",")}]` : `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${stable(value[key])}`).join(",")}}` : JSON.stringify(value);
  // Autonomous-v2 bounties verify verdicts under EIP-712 domain version "2"; v1 bounties under "1".
  const domainVersion = (item) => item?.protocol_version === "agent-bounties/autonomous-v2" ? "2" : "1";
  function typedData(request, version = "1") {
    const fields = (entries) => entries.map(([name, type]) => ({ name, type }));
    return { domain: { name: "Agent Bounties", version, chainId: 8453, verifyingContract: request.bounty_contract }, primaryType: "VerificationAttestation",
      types: { EIP712Domain: fields([["name", "string"], ["version", "string"], ["chainId", "uint256"], ["verifyingContract", "address"]]),
        VerificationAttestation: fields([["bounty", "address"], ["bountyId", "bytes32"], ["round", "uint64"], ["verifier", "address"], ["submissionHash", "bytes32"], ["evidenceHash", "bytes32"], ["policyHash", "bytes32"], ["passed", "bool"], ["responseHash", "bytes32"], ["deadline", "uint256"]]) },
      message: { bounty: request.bounty_contract, bountyId: request.bounty_id, round: String(request.round), verifier: request.verifier, submissionHash: request.submission_hash, evidenceHash: request.evidence_hash, policyHash: request.policy_hash, passed: request.passed, responseHash: request.response_hash, deadline: String(request.deadline) } };
  }
  function settlementData(attestation, evm) {
    if (!/^0x(?:[0-9a-f]{2}){1,4096}$/i.test(attestation.signature)) throw new Error("A bounded verifier signature is required.");
    const word = evm.uint256Word;
    const length = (attestation.signature.length - 2) / 2;
    return evm.keccak256Hex(evm.textHex("settleWithAttestations((address,bool,bytes32,uint256,bytes)[])")).slice(0, 10)
      + word(32) + word(1) + word(32) + evm.addressWord(attestation.verifier) + word(attestation.passed ? 1 : 0)
      + evm.bytes32Word(attestation.response_hash) + word(attestation.deadline) + word(160) + word(length) + attestation.signature.slice(2).padEnd(Math.ceil(length / 32) * 64, "0");
  }
  function assessment(job, input) {
    const criteria = job.terms.document.acceptance_criteria;
    if (!Array.isArray(input.checks) || input.checks.length !== criteria.length) throw new Error("Assess every published acceptance criterion, in its published order.");
    const checks = input.checks.map((check, index) => {
      if (check.criterion !== criteria[index] || typeof check.passed !== "boolean" || typeof check.reason !== "string" || !check.reason.trim() || check.reason.length > 1000) throw new Error("Every check must match the published criterion and include a bounded reason.");
      return { criterion: criteria[index], passed: check.passed, reason: check.reason.trim() };
    });
    const terms = job.terms.document;
    const submittedAt = job.verification_expires_at - terms.contract_terms.verification_window_seconds;
    const onTime = Number.isSafeInteger(submittedAt) && submittedAt > 0 && submittedAt <= terms.benchmark.delivery_deadline;
    return { checks, on_time: onTime, passed: onTime && checks.every((check) => check.passed), round: job.round, submission_hash: job.submission_evidence.artifact_hash, evidence_hash: job.submission_evidence.evidence_hash };
  }
  function start(win) {
    const doc = win.document, root = doc.querySelector("[data-creator-review]");
    if (!root) return;
    const client = win.AgentBountiesWorkflow.createClient(win), evm = win.AgentBountiesEvm;
    const contract = lower(new URLSearchParams(win.location.search).get("bountyContract"));
    const key = `agent-bounties.creator-review.v1:${contract}`;
    const output = root.querySelector("output"), confirm = root.querySelector("button");
    let job = null, staged = null, busy = false, importedDraft = null, version = "1";
    const calendarButton = doc.createElement("button"); calendarButton.type = "button"; calendarButton.textContent = "Add review deadline to calendar"; calendarButton.hidden = true;
    root.append(calendarButton);
    calendarButton.addEventListener("click", (event) => {
      if (!event.isTrusted || !job) return;
      try {
        const body = win.AgentBountiesReviewDeadline.calendar(job);
        const url = win.URL.createObjectURL(new win.Blob([body], { type: "text/calendar;charset=utf-8" }));
        const link = doc.createElement("a"); link.href = url; link.download = `bounty-review-${job.round}.ics`; link.click();
        win.setTimeout(() => win.URL.revokeObjectURL(url), 1000);
      } catch (error) { output.textContent = error.message; }
    });
    const load = () => JSON.parse(win.sessionStorage.getItem(key) || "null");
    const save = (record) => win.sessionStorage.setItem(key, JSON.stringify(record));
    async function reportRequest(path, body) {
      const response = await win.fetch(`${win.AgentBountiesWorkflow.apiBase(win.location)}${path}`, { method: body === undefined ? "GET" : "POST", credentials: "include", cache: "no-store", headers: {"Accept":"application/json", "Content-Type":"application/json"}, ...(body === undefined ? {} : {body:JSON.stringify(body)}) });
      if (!response.ok) throw new Error(response.status === 401 || response.status === 403 ? "Sign in with the account linked to your reviewer wallet, then try again. Your review is still here." : "Your review could not be saved. Try this same step again before signing.");
      return response.json();
    }

    async function refresh() {
      const feed = await client.request("/v1/base/autonomous-bounties/feed?network=base-mainnet&claimable_only=false");
      const item = feed.find((entry) => lower(entry.bounty_contract) === contract);
      if (!item?.terms_valid || item.terms.document.benchmark?.engine !== "creator_review_v1") return { status: "not_creator_review" };
      root.hidden = false;
      version = domainVersion(item);
      const operation = load();
      const submitted = operation && item.events.find((entry) => entry.kind === "submission_added" && lower(entry.contract_address) === contract && entry.bounty_id === item.bounty_id && entry.data.round === operation.request.round
        && lower(entry.data.submission_hash) === lower(operation.request.submission_hash) && lower(entry.data.evidence_hash) === lower(operation.request.evidence_hash));
      const event = submitted && item.events.find((entry) => entry.kind === (operation.request.passed ? "bounty_settled" : "submission_rejected") && lower(entry.contract_address) === contract && entry.bounty_id === item.bounty_id
        && entry.data.round === operation.request.round && lower(entry.data.solver) === lower(submitted.data.solver) && entry.id && entry.block_number > 0
        && (entry.block_number > submitted.block_number || entry.block_number === submitted.block_number && entry.log_index > submitted.log_index)
        && (entry.kind === "submission_rejected" || lower(entry.data.submission_hash) === lower(operation.request.submission_hash) && lower(entry.data.evidence_hash) === lower(operation.request.evidence_hash)));
      if (event) { if (operation.report_id) await reportRequest(`/v1/verification/reviews/${operation.report_id}/confirm`, {transaction_hash:event.tx_hash}); output.textContent = event.kind === "bounty_settled" ? "Payment confirmed by canonical settlement." : "Rejection confirmed; the bounty is available again."; confirm.disabled = true; return { status: "confirmed", paid: event.kind === "bounty_settled", event }; }
      const jobs = item.status === "submitted" ? await client.request("/v1/base/autonomous-bounties/verification-jobs?network=base-mainnet") : [];
      job = jobs.find((entry) => lower(entry.bounty_contract) === contract && entry.bounty_id === item.bounty_id && entry.terms.terms_hash === item.terms_hash && entry.threshold === 1
        && entry.eligible_verifiers.length === 1 && lower(entry.eligible_verifiers[0]) === lower(item.creator)) || null;
      calendarButton.hidden = !job || job.verification_expires_at * 1000 <= Date.now();
      if (job) calendarButton.title = `Review by ${new Date(job.verification_expires_at * 1000).toISOString()}`;
      if (operation?.phase === "signed" && operation.request.deadline <= Date.now() / 1000) {
        // This phase proves no broadcast was attempted. An expired signature cannot settle.
        win.sessionStorage.setItem(`${key}.previous`, JSON.stringify(operation)); win.sessionStorage.removeItem(key);
        staged = null; return refresh();
      }
      if (["signed", "relay_pending"].includes(operation?.phase)) {
        staged = operation.assessment; if (job && staged) renderAssessment(); confirm.disabled = !job || !staged;
        output.textContent = "Your verdict is already signed. Continue to reconcile and relay this same verdict; Agent Bounties pays the network fee.";
      } else if (operation) output.textContent = "A verdict wallet step is recorded. Check its canonical status; do not repeat it.";
      else if (!staged) output.textContent = job ? "A submission is ready. Your AI can check every criterion and prepare your verdict here." : "The creator will confirm the verdict here after a submission arrives.";
      return { status: operation ? "pending_confirmation" : job ? "review_available" : "waiting_for_submission", paid: false, job, operation: operation ? { phase: operation.phase, transaction_hash: operation.hash || null } : null, user_confirmation_required: false };
    }
    function renderAssessment() {
      const list = root.querySelector("ol"); list.replaceChildren();
      for (const check of staged.checks) { const li = doc.createElement("li"); li.textContent = `${check.passed ? "Pass" : "Fail"}: ${check.criterion} — ${check.reason}`; list.append(li); }
      output.textContent = `${staged.passed ? "Proposed pass" : "Proposed rejection"}. ${staged.on_time ? "Submitted by the delivery deadline." : "The submission missed the delivery deadline."} Review every check before confirming.`;
      root.querySelector("[data-review-cost]").textContent = `On Base: a pass pays ${Number(job.current_solver_payout) / 1e6} USDC to solver ${job.solver_wallet}; either verdict pays ${Number(job.verifier_reward) / 1e6} USDC to reviewer ${job.eligible_verifiers[0]}. A rejection forfeits the solver bond. Review expires ${new Date(job.verification_expires_at * 1000).toLocaleString()}. Agent Bounties pays the network fee. If sponsorship is unavailable, the verdict stays saved.`;
    }
    async function importDraft(id) {
      if (!/^[0-9a-f-]{36}$/i.test(id) || busy || load()) throw new Error("Check the existing wallet step before opening another review.");
      await refresh(); if (!job) throw new Error("No current submission is ready for this review.");
      const report = await reportRequest(`/v1/verification/reviews/${id}`);
      if (report.status !== 'draft' || report.network !== 'base-mainnet' || lower(report.bounty_contract) !== contract || report.round !== job.round || lower(report.reviewer) !== lower(job.eligible_verifiers[0])) throw new Error("This saved review does not match the current task.");
      const checked = assessment(job, report.assessment);
      if (stable(checked) !== stable(report.assessment)) throw new Error("The saved review no longer matches the submitted files.");
      importedDraft = report; staged = checked; renderAssessment(); confirm.disabled = false;
      return {status:'saved_review_ready', paid:false, user_confirmation_required:true};
    }
    async function stage(input) {
      if (busy || load()) throw new Error("Reconcile the recorded verdict before preparing another.");
      await refresh(); if (!job) throw new Error("No current creator-review submission is available.");
      importedDraft = null; staged = assessment(job, input);
      renderAssessment();
      confirm.disabled = false;
      return { status: "verdict_staged", assessment: staged, paid: false, user_confirmation_required: true, next_action: "The creator reviews these checks and confirms the verdict in the page and wallet. Do not click their confirmation." };
    }
    confirm.addEventListener("click", async (event) => {
      if (!event.isTrusted || busy || !staged || (load() && !["signed", "relay_pending"].includes(load().phase))) return;
      busy = true; confirm.disabled = true;
      try {
        const priorJob = job, existing = load(); await refresh();
        if (!job || job.round !== priorJob.round || job.submission_evidence.evidence_hash !== priorJob.submission_evidence.evidence_hash || job.submission_evidence.artifact_hash !== priorJob.submission_evidence.artifact_hash) throw new Error("The submission changed. Ask your AI to refresh its review.");
        await win.AgentBountiesPhoneWallet?.restore();
        const provider = win.AgentBountiesPhoneWallet?.state().connected ? win.AgentBountiesPhoneWallet.provider : win.ethereum;
        if (!provider) throw new Error("Connect your phone wallet using the QR button, then confirm this review.");
        let accounts = await provider.request({ method: "eth_accounts" });
        if (!accounts.length) accounts = await provider.request({ method: "eth_requestAccounts" });
        const wallet = lower(accounts[0]);
        if (wallet !== lower(job.eligible_verifiers[0])) throw new Error("Connect the creator wallet named in this bounty.");
        if (lower(await provider.request({ method: "eth_chainId" })) !== "0x2105") await provider.request({ method: "wallet_switchEthereumChain", params: [{ chainId: "0x2105" }] });
        const request = existing?.request || (importedDraft?.request?.deadline > Date.now()/1000 ? importedDraft.request : null) || { bounty_contract: contract, bounty_id: job.bounty_id, round: job.round, verifier: wallet, submission_hash: staged.submission_hash, evidence_hash: staged.evidence_hash, policy_hash: job.terms.policy_hash, passed: staged.passed,
          response_hash: evm.keccak256Hex(evm.textHex(stable(staged))), deadline: Math.min(job.verification_expires_at, Math.floor(Date.now() / 1000) + 600) };
        if (request.round !== job.round || request.submission_hash !== job.submission_evidence.artifact_hash || request.evidence_hash !== job.submission_evidence.evidence_hash || request.deadline <= Date.now() / 1000) throw new Error("The signed review is stale; check its canonical status.");
        const storedReport = existing?.report_id ? {id:existing.report_id} : importedDraft && stable(importedDraft.request) === stable(request) ? importedDraft : await reportRequest("/v1/verification/reviews", {network:"base-mainnet", attestation:request, assessment:staged, previous_report_id:importedDraft?.id || null});
        const report_id = storedReport.id;
        let attestation = existing?.attestation;
        if (!attestation) {
          const sponsor = await client.request("/v1/base/gas-sponsorship");
          if (sponsor.schema !== "agent-bounties/gas-sponsorship-v1" || !sponsor.creator_verdict?.available || sponsor.creator_verdict.customer_gas_wei !== "0") throw new Error("Gas sponsorship is unavailable. Your review is saved; recheck before signing. You do not need to buy ETH.");
          const plan = await client.request("/v1/base/autonomous-bounties/verification-attestation-plan", { network: "base-mainnet", attestation: request });
          if (stable(plan) !== stable(typedData(request, version))) throw new Error("The requested signature differs from your reviewed verdict.");
          save({ phase: "signing", request, assessment: staged, report_id });
          const signature = await provider.request({ method: "eth_signTypedData_v4", params: [wallet, JSON.stringify(plan)] });
          attestation = { verifier: wallet, passed: request.passed, response_hash: request.response_hash, deadline: request.deadline, signature };
          save({ phase: "signed", request, attestation, assessment: staged, report_id });
        }
        await reportRequest(`/v1/verification/reviews/${report_id}/signature`, {signature:attestation.signature});
        const tx = await client.request("/v1/base/autonomous-bounties/attestation-settlement-plan", { network: "base-mainnet", bounty_contract: contract, caller: wallet, attestations: [attestation] });
        if (lower(tx.from) !== wallet || lower(tx.to) !== contract || String(tx.value_wei) !== "0" || lower(tx.data) !== lower(settlementData(attestation, evm))) throw new Error("The settlement transaction differs from the signed verdict.");
        if (lower((await provider.request({ method: "eth_accounts" }))[0]) !== wallet || lower(await provider.request({ method: "eth_chainId" })) !== "0x2105") throw new Error("Wallet or chain changed; no transaction was sent.");
        save({ phase: "relay_pending", request, attestation, assessment: staged, report_id });
        const result = await client.request("/v1/base/autonomous-bounties/creator-verdict-relay", { network: "base-mainnet", attestation: request, signature: attestation.signature });
        if (result.schema !== "agent-bounties/sponsored-verdict-v1" || result.network !== "base-mainnet" || lower(result.bounty_contract) !== contract || result.round !== request.round || result.customer_gas_wei !== "0") throw new Error("The relay returned a different review. Preserve this signed verdict and check its canonical status.");
        const hash = result.transaction_hash;
        if (!/^0x[0-9a-f]{64}$/i.test(hash)) throw new Error("The transaction response is uncertain. Check status before retrying.");
        save({ phase: "submitted", request, hash, attestation, assessment: staged, report_id });
        await reportRequest(`/v1/verification/reviews/${report_id}/confirm`, {transaction_hash:hash});
        output.textContent = "Verdict submitted. Your AI can check canonical confirmation; payment is not confirmed yet.";
      } catch (error) {
        const record = load();
        if (error.code === 4001 && record?.phase === "signing") win.sessionStorage.removeItem(key);
        output.textContent = error.message || String(error);
        confirm.disabled = Boolean(load() && !["signed", "relay_pending"].includes(load().phase));
      } finally { busy = false; }
    });
    win.AgentBountiesCreatorReviewWorkspace = Object.freeze({ refresh, stage, importDraft });
    (new URLSearchParams(win.location.search).has("reviewReport") ? importDraft(new URLSearchParams(win.location.search).get("reviewReport")) : refresh()).catch((error) => { output.textContent = error.message; });
  }
  return { typedData, domainVersion, settlementData, assessment, start };
});
