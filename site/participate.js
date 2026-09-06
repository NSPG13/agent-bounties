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
  function validateSubmission(submission, details, contract, wallet, bountyId) {
    const evidence = submission?.evidence_publication;
    if (submission?.network?.chain_id !== 8453 || lower(submission.bounty_contract) !== contract || lower(submission.solver) !== wallet
      || submission.bounty_id !== bountyId || !Number.isSafeInteger(submission.round) || submission.round <= 0
      || !HASH.test(submission.submission_hash) || !HASH.test(submission.evidence_hash)
      || evidence?.network !== "base-mainnet" || lower(evidence.bounty_contract) !== contract || lower(evidence.solver_wallet) !== wallet
      || evidence.bounty_id !== bountyId || evidence.round !== submission.round
      || evidence.artifact_reference !== details.artifact_reference || stable(evidence.evidence) !== stable(details.evidence)) throw new Error("The prepared submission differs from the exact public evidence you reviewed.");
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
  async function start(win, doc) {
    const flow = win.AgentBountiesWorkflow, client = flow.createClient(win), evm = win.AgentBountiesEvm;
    const find = (s) => doc.querySelector(s), put = (s, value) => { find(s).textContent = value; };
    const params = new URLSearchParams(win.location.search), contract = lower(params.get("bountyContract"));
    const network = params.get("network") || flow.NETWORK;
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
      put("[data-work-costs]", "The claim bond is at risk if you miss the work deadline. Gas and any required child-bounty funding are additional costs. Your AI should explain these before you commit.");
      const criteria = find("[data-work-criteria]"); criteria.replaceChildren();
      for (const criterion of terms.acceptance_criteria) { const li = doc.createElement("li"); li.textContent = criterion; criteria.append(li); }
      const events = scopeEvents(item.events || []);
      const claim = events.filter((event) => event.kind === "bounty_claimed").sort((a, b) => b.block_number - a.block_number || b.log_index - a.log_index)[0];
      const claimOwner = lower(claim?.data?.solver) || null;
      const ownsClaim = Boolean(record.wallet && claimOwner === record.wallet);
      put("[data-work-deadline]", item.status === "claimed" && claim?.data?.claim_expires_at
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
        find("[data-action-review]").hidden = intent.status !== "review_required" || intent.action === "verify" || Boolean(record.finalHash || record.sending);
        put("[data-review-summary]", intent.action === "complete" ? "Review the exact public artifact and evidence, then confirm submission in your wallet."
          : intent.action === "fund" ? `Contribute exactly ${intent.amount_base_units / 1e6} USDC to this bounty. Your wallet shows the destination and gas.`
          : `Claim this work with a ${Number(item.claim_bond) / 1e6} USDC bond. Your wallet shows the exact bond and gas.`);
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
      const claimable = item.status === "claimable" && item.verification_ready === true;
      find("[data-work-prepare]").hidden = Boolean(intentId) || !claimable;
      put("[data-step-title]", item.status === "paid" ? "Canonical result" : item.status === "claimed" ? "Track the agreed work" : item.status === "submitted" ? "Verification in progress" : "Review the next step");
      put("[data-step-summary]", item.verification_ready ? "Your AI can read the exact requirements and prepare the next action."
        : item.verification_readiness_reason || "The verifier is not ready. Do not commit to new work yet.");
      put("[data-work-evidence]", JSON.stringify({ verification: terms.verification_policy, benchmark: terms.benchmark, evidence_schema: terms.evidence_schema, events, jobs }, null, 2));
      const next = paidEvent ? null
        : item.status === "paid" ? { action: "show_canonical_result", instructions: "Show the recorded settlement and its actual recipient. This is not a claim that the current person earned money unless their wallet and submission are matched." }
        : intent?.status === "confirmed" && intent.action === "complete" && !record.evidencePublished ? { tool: "agent_bounties_publish_confirmed_evidence", input: {} }
        : item.status === "claimed" && ownsClaim ? { action: "complete_agreed_work", instructions: "Use the current assistant's execution tools to complete and test the exact accepted work. Prepare action complete with the public artifact and evidence when it passes. No permission is needed for routine preparation." }
        : item.status === "claimed" ? { action: "track_claimed_work", instructions: "This work is reserved by the displayed claim owner. Track that solver's progress; do not start duplicate work or represent the claim as yours without matching the person's wallet." }
        : item.status === "submitted" ? { action: "continue_committed_verification", instructions: "Read the returned verification job. Execute its exact committed flow through an available interface, or wait for the committed verifier. Do not ask for a new approval just to check status." }
        : { action: "review_current_requirements", instructions: "Resolve the listed prerequisites before preparing a claim. If posting, wait for a solver or continue preparing an authorized contribution." };
      return { bounty_contract: contract, bounty_id: item.bounty_id, status: item.status, terms, events, claim_owner: claimOwner, claim_owned_by_review_wallet: ownsClaim, verification_jobs: jobs,
        action: intent, paid: Boolean(paidEvent), payment_evidence: paidEvent || null, wallet_connected: Boolean(wallet), poll_after_seconds: 15, evidence_boundary: flow.BOUNDARY,
        next_action: next };
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
    async function prepareWallet() {
      if (!intent || intent.status !== "review_required" || record.finalHash || record.sending) throw new Error("Refresh the current confirmation before preparing another wallet request.");
      if (record.calls && record.wallet) {
        if (record.wallet !== wallet) throw new Error("Reconnect the wallet used for this pending review.");
        calls = record.calls; submission = record.submission;
        find("[data-wallet-confirm]").disabled = false;
        put("[data-wallet-status]", "Your existing wallet step is restored. Confirm to continue from the last transaction without repeating it.");
        return;
      }
      if (intent.action === "solve") {
        const plan = await client.request("/v1/base/autonomous-bounties/claim-plan", { network, bounty_contract: contract, solver: wallet });
        if (plan.network?.chain_id !== 8453 || lower(plan.bounty_contract) !== contract || String(plan.claim_bond) !== String(item.claim_bond)) throw new Error("Claim terms changed. Refresh and review the current bond.");
        calls = plan.wallet_calls;
      } else if (intent.action === "fund") {
        const plan = await client.request("/v1/base/autonomous-bounties/contribution-plan", { network, contribution: { bounty_contract: contract, contributor: wallet, amount: { amount: intent.amount_base_units, currency: "usdc" } } });
        if (plan.network?.chain_id !== 8453) throw new Error("The funding plan uses another chain.");
        calls = plan.wallet_calls;
      } else if (intent.action === "complete") {
        submission = await client.request("/v1/base/autonomous-bounties/submission-preparation", { network, bounty_contract: contract, solver_wallet: wallet, ...intent.details });
        validateSubmission(submission, intent.details, contract, wallet, item.bounty_id);
        calls = [await client.request("/v1/base/autonomous-bounties/submission-plan", { network, bounty_contract: contract, solver: wallet, submission_hash: submission.submission_hash, evidence_hash: submission.evidence_hash })];
      } else throw new Error("The committed verifier handles this step. Refresh verification status.");
      validateCalls(calls, { action: intent.action, contract, wallet, amount: intent.action === "fund" ? intent.amount_base_units : item.claim_bond, submission }, evm);
      record.calls = calls; record.wallet = wallet; record.submission = submission; saveRecord();
      put("[data-wallet-status]", `Ready on Base: ${wallet}. ${calls.length} wallet confirmation${calls.length === 1 ? "" : "s"}; only the exact displayed amount is authorized.`);
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
    find("[data-wallet-confirm]").addEventListener("click", async (event) => {
      if (!event.isTrusted || busy || !calls || !wallet || !intent) return;
      busy = true; find("[data-wallet-confirm]").disabled = true;
      try {
        if (record.finalHash || record.sending) throw new Error("A wallet submission has already started. Refresh its status or inspect the wallet; do not send a duplicate.");
        if (record.pendingHash) {
          if (!await receipt(record.pendingHash)) throw new Error("Your transaction is still pending. Wait; do not sign it again.");
          record.pendingHash = null; record.next = (record.next || 0) + 1; saveRecord();
        }
        const accounts = await provider.request({ method: "eth_accounts" });
        if (lower(accounts[0]) !== wallet || await provider.request({ method: "eth_chainId" }) !== "0x2105") throw new Error("Your wallet or network changed. Connect again to prepare an exact review.");
        intent = await client.progress(intentId);
        if (intent.status !== "review_required" || lower(intent.bounty_contract) !== contract) throw new Error("This action has changed or is already pending. Refresh progress instead of signing again.");
        const legal = { solve: "claim_bounty", fund: "fund_bounty", complete: "submit_result" }[intent.action];
        const acceptance = await win.AgentBountiesLegal.requireAcceptance({ action: legal, walletAddress: wallet });
        if (!acceptance.durable) throw new Error("The agreement could not be recorded. Retry when the service is available; no transaction was sent.");
        validateCalls(calls, { action: intent.action, contract, wallet, amount: intent.action === "fund" ? intent.amount_base_units : item.claim_bond, submission }, evm);
        for (let index = record.next || 0; index < calls.length; index++) {
          const current = await client.progress(intentId);
          if (current.status !== "review_required" || current.network !== network || lower(current.bounty_contract) !== contract
            || current.action !== intent.action || stable(current.details) !== stable(intent.details) || current.amount_base_units !== intent.amount_base_units) throw new Error("The review expired or changed. Reconcile the current wallet step before continuing.");
          const call = calls[index];
          record.sending = true; saveRecord();
          let hash;
          try { hash = await provider.request({ method: "eth_sendTransaction", params: [{ from: wallet, to: call.to, data: call.data, value: "0x0" }] }); }
          catch (error) {
            if (error.code === 4001) { record.sending = false; saveRecord(); }
            throw error;
          }
          if (!HASH.test(hash)) throw new Error("The wallet returned no valid transaction hash. Inspect the wallet before retrying.");
          record.sending = false; record.pendingHash = hash; record.lastHash = hash; record.next = index; record.submission = submission;
          if (index === calls.length - 1) record.finalHash = hash;
          saveRecord();
          if (index === calls.length - 1) {
            await client.request(`/v1/chatgpt/action-intents/${intentId}/observations`, { transaction_hash: hash, bounty_contract: contract, actor_wallet: wallet });
            record.observed = true; saveRecord();
            put("[data-work-status]", "Sent. Your AI will check confirmation; no new signature is needed.");
            break;
          }
          // Continue the already-confirmed sequence once the token approval lands.
          // Only an unusually long pending approval needs a later resume click.
          let mined = await receipt(hash);
          for (let attempt = 0; !mined && attempt < 10; attempt++) {
            put("[data-wallet-status]", "Waiting for token approval. The wallet will show the final action next; no extra chat approval is needed.");
            await new Promise((resolve) => win.setTimeout(resolve, 2000));
            mined = await receipt(hash);
          }
          if (!mined) throw new Error("Token approval is still pending. Resume with Confirm in wallet after it confirms; the approval will not repeat.");
          const currentAccounts = await provider.request({ method: "eth_accounts" });
          if (lower(currentAccounts[0]) !== wallet || lower(await provider.request({ method: "eth_chainId" })) !== "0x2105") throw new Error("The wallet changed while approval was pending. Reconnect the original wallet to continue.");
          record.pendingHash = null; record.next = index + 1; saveRecord();
        }
        await refresh();
      } catch (error) { message(error); }
      finally { busy = false; find("[data-wallet-confirm]").disabled = intent?.status !== "review_required" || Boolean(record.finalHash || record.sending); }
    });
    find("[data-work-prepare]").addEventListener("click", async (event) => {
      if (!event.isTrusted) return;
      try {
        const prepared = await client.prepareAction({ action: "solve", opportunity_id: `canonical:${network}:${contract}` });
        win.location.assign(prepared.authorization_url);
      } catch (error) { message(error); }
    });
    find("[data-work-refresh]").addEventListener("click", () => refresh().catch(message));
    win.AgentBountiesParticipation = Object.freeze({ refresh, publishEvidence });
    try { await refresh(); } catch (error) { message(error); }
    win.setInterval(() => { if (!doc.hidden && !busy && intentId) refresh().catch(message); }, 15000);
  }
  return { validateCalls, validateSubmission, start };
});
