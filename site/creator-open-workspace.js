/* First-party creator-open workspace. No client path sends a gas-paying transaction. */
(function (win, doc) {
  "use strict";
  const core = win.AgentBountiesCreatorOpen, evm = win.AgentBountiesEvm;
  const $ = id => doc.getElementById(id), lower = v => String(v || "").toLowerCase();
  const api = ["localhost", "127.0.0.1", "[::1]"].includes(win.location.hostname) ? "http://127.0.0.1:3000" : "https://api.agentbounties.app";
  const query = new URL(win.location.href).searchParams;
  let network = query.get("network") === "base-sepolia" ? "base-sepolia" : "base-mainnet";
  let contract = core.ADDRESS.test(query.get("bounty")) ? lower(query.get("bounty")) : null;
  let capabilities, release, provider, wallet, record = null, item = null, busy = false, generation = 0, recoveryBlocked = false;
  $("open-network").value = network;
  const storageKey = () => `agentbounties:creator-open:v1:${network}`;
  const now = () => Math.floor(Date.now() / 1000);
  const message = text => { $("open-status").textContent = text; };
  const signed = () => Boolean(record && (record.signatures.token !== "0x" || record.signatures.action !== "0x" || record.submitted));
  const complete = () => record && ["token_authorization", "action_authorization"].every((key, i) => !record.local[key] || record.signatures[i ? "action" : "token"] !== "0x");
  function save() {
    // Refuse signing or sending if the exact continuation cannot be retained.
    win.sessionStorage.setItem(storageKey(), JSON.stringify(record));
  }
  function available() { return capabilities?.networks?.find(n => n.network === network)?.available === true; }
  function controls() {
    const ready = !busy && available() && Boolean(wallet) && !record && !recoveryBlocked;
    $("open-create-prepare").disabled = !ready;
    const blocker = core.actionBlocker(item, $("open-action").value, wallet, now());
    $("open-action-prepare").disabled = !ready || Boolean(blocker);
    $("open-action-hint").textContent = blocker || "Prepare the exact action. You review it before anything is signed.";
    $("open-network").disabled = busy || Boolean(record);
    $("open-connect").disabled = busy;
    $("open-refresh").disabled = busy;
    $("open-import-submit").disabled = busy || Boolean(record) || recoveryBlocked || !release;
    for (const input of [...$("open-create-form").elements, ...$("open-action-form").elements]) if (input.tagName !== "BUTTON") input.disabled = busy || Boolean(record);
    $("open-confirm").disabled = busy || !record || record.confirmed || record.submitted || !available() || !wallet || !$("open-consent").checked || record.shortfall !== "0" || record.request.valid_before <= now();
    $("open-retry").hidden = !record || !complete() || record.confirmed;
    $("open-retry").disabled = busy || !available() || !wallet || !$("open-consent").checked || record?.request.valid_before <= now();
    $("open-check").disabled = busy || !record;
    $("open-edit").hidden = !record || signed();
    $("open-edit").disabled = busy;
    $("open-finish").hidden = !record?.confirmed;
    $("open-finish").disabled = busy;
  }
  async function request(path, body) {
    const controller = new AbortController(), timer = win.setTimeout(() => controller.abort(), 25000);
    try {
      const response = await win.fetch(`${api}/v1/base/creator-open${path}`, { method: body === undefined ? "GET" : "POST", headers: { Accept: "application/json", ...(body === undefined ? {} : { "Content-Type": "application/json" }) }, body: body === undefined ? undefined : JSON.stringify(body), cache: "no-store", credentials: "omit", referrerPolicy: "no-referrer", signal: controller.signal });
      const value = await response.json().catch(() => null);
      if (!response.ok) throw Object.assign(new Error(response.status === 429 ? "The sponsorship quota is currently used. Keep this saved request and retry later." : response.status === 409 ? "The bounty state or deadline changed. Refresh before preparing another action." : response.status === 404 ? "This exact action or bounty is not indexed yet. Keep your saved request and refresh; do not create another funding attempt." : "Sponsorship or canonical evidence is unavailable. Your exact review stays saved; no customer-paid gas is requested."), { status: response.status, body: value });
      return value;
    } catch (error) { if (error instanceof TypeError) throw new Error("The service reply was lost. Check the saved transaction, then retry this exact signed request if needed."); throw error; } finally { win.clearTimeout(timer); }
  }
  async function run(action) {
    if (busy) return;
    busy = true; controls();
    try { await action(); } catch (error) { message(error.name === "AbortError" ? "The reply timed out. Check this saved transaction, then retry the same signed request if needed." : error.message || "The action could not finish. Preserve this exact review and retry."); }
    finally { busy = false; controls(); }
  }
  function click(id, action) { $(id).addEventListener("click", event => { if (event.isTrusted) run(action); }); }
  function fieldVisibility() {
    const action = $("open-action").value;
    $("open-submission-fields").hidden = action !== "submit";
    $("open-entry-field").hidden = !["review", "refund_bond"].includes(action);
    $("open-review-fields").hidden = action !== "review";
    $("open-increase-field").hidden = action !== "increase"; controls();
  }
  function publicEvidence(parent, document, commitment) {
    const hash = core.evidence(document, evm);
    if (hash !== lower(commitment)) throw new Error("The artifact details do not match the signed evidence hash.");
    const link = doc.createElement("a"); link.textContent = document.artifact_url; link.href = document.artifact_url; link.target = "_blank"; link.rel = "noopener noreferrer";
    const digest = doc.createElement("p"); digest.textContent = `Artifact checksum: ${document.artifact_sha256}`;
    const notes = doc.createElement("p"); notes.textContent = document.notes;
    parent.append(link, digest, notes);
  }
  function facts(values) {
    $("open-facts").replaceChildren();
    for (const [label, value] of values) { const dt = doc.createElement("dt"), dd = doc.createElement("dd"); dt.textContent = label; dd.textContent = value; $("open-facts").append(dt, dd); }
  }
  function renderBounty() {
    $("open-create").hidden = Boolean(contract); $("open-bounty").hidden = !item;
    if (!item) return;
    $("open-bounty-title").textContent = item.terms.title; $("open-goal").textContent = item.terms.goal;
    $("open-criteria").replaceChildren(...item.terms.acceptance_criteria.map(value => { const li = doc.createElement("li"); li.textContent = value; return li; }));
    $("open-bounty-address").textContent = `${network} · ${contract} · revision ${item.revision}`;
    facts([["Status", item.status], ["Solver reward", core.money(item.terms.solver_reward_micro_usdc)], ["Each submission bond", core.money(item.terms.review_reward_micro_usdc)], ["Submission deadline", new Date(item.terms.submission_deadline * 1000).toLocaleString()], ["Review window", `${item.terms.review_window_seconds / 3600} hours per submission`], ["Creator", item.terms.creator], ["Customer network fee", "0 ETH"]]);
    $("open-entries").replaceChildren();
    for (const { id, entry, public_evidence } of item.entries) {
      const section = doc.createElement("section"); section.className = "open-entry open-wrap";
      const title = doc.createElement("h3"), details = doc.createElement("p"); title.textContent = `Submission ${id} · ${entry.status}`;
      details.textContent = `Solver ${entry.solver}. Review deadline: ${new Date(entry.review_deadline * 1000).toLocaleString()}.`;
      section.append(title, details);
      if (item.status === "open" && entry.status === "pending" && entry.review_deadline > now()) {
        const reminder = doc.createElement("button"); reminder.type = "button"; reminder.textContent = "Add review deadline to calendar";
        reminder.addEventListener("click", event => { if (event.isTrusted) run(async () => {
          await loadBounty(); const latest = item, current = latest.entries.find(value => value.id === id)?.entry;
          if (latest.status !== "open" || !current || current.status !== "pending" || current.review_deadline !== entry.review_deadline || current.evidence_hash !== entry.evidence_hash || current.submission_hash !== entry.submission_hash) throw new Error("This submission changed. Refresh before saving its deadline.");
          const calendar = win.AgentBountiesReviewDeadline.calendar({ protocol: "creator-open-v1", network, bounty_contract: contract, round: id, verification_expires_at: current.review_deadline });
          const url = win.URL.createObjectURL(new win.Blob([calendar], { type: "text/calendar;charset=utf-8" }));
          const link = doc.createElement("a"); link.href = url; link.download = `creator-review-${contract}-${id}.ics`; doc.body.append(link); link.click(); link.remove(); win.setTimeout(() => win.URL.revokeObjectURL(url), 1000);
          message("Calendar reminder downloaded for the verified deadline. Check this bounty for any later status change.");
        }); }); section.append(reminder);
      }
      if (public_evidence) publicEvidence(section, public_evidence, entry.evidence_hash);
      else { const missing = doc.createElement("p"); missing.textContent = "The committed artifact details are unavailable here. Obtain and verify the evidence before deciding."; section.append(missing); }
      $("open-entries").append(section);
    }
    if (!item.entries.length) $("open-entries").textContent = "No pending or winning submissions are indexed. Use an exact submission number to recover an older eligible bond.";
  }
  async function loadBounty(entry) {
    if (!contract) { item = null; renderBounty(); return; }
    const result = await request(`/bounties?network=${network}&bounty_contract=${contract}${entry ? `&entry=${entry}` : ""}`);
    const candidate = result.items?.find(value => lower(value.bounty_contract) === contract);
    if (result.protocol !== core.PROTOCOL || !candidate || candidate.network !== network || candidate.customer_gas_wei !== "0"
      || core.predict(release, { original_terms: candidate.original_terms, user_salt: candidate.user_salt }, evm) !== contract
      || core.hashJson(candidate.terms, evm) !== lower(candidate.terms_hash)) throw new Error("The canonical bounty terms could not be verified. Refresh before acting.");
    if (entry) return candidate.entries.find(value => value.id === entry);
    item = candidate; renderBounty();
  }
  async function refresh() {
    capabilities = await request("/capabilities");
    if (capabilities.protocol !== core.PROTOCOL || capabilities.customer_gas_wei !== "0") throw new Error("The sponsored creator-review service is unavailable.");
    release = capabilities.networks?.find(value => value.network === network)?.release;
    if (!release) { item = null; renderBounty(); message("Open creator review is not active on this network yet. Your draft stays here; no wallet authorization is requested."); return; }
    if (!record) {
      const saved = win.sessionStorage.getItem(storageKey());
      if (saved) {
        recoveryBlocked = true;
        const value = JSON.parse(saved), local = core.expected(release, value.request, evm, now(), true);
        if (value.approval_hash !== local.approval_hash || value.network !== network || !value.signatures || ![value.signatures.token, value.signatures.action].every(v => /^0x(?:[0-9a-f]{2}){0,4096}$/i.test(v))) throw new Error("The saved request failed its integrity check. Keep this tab for recovery; do not start another payment.");
        record = { ...value, local, confirmed: false }; recoveryBlocked = false; renderReview();
      }
    }
    if (record) { if (!signed()) await recheckBalance(); else await checkSaved(); return; }
    await loadBounty();
    message(available() ? "Review the terms, then connect a wallet. Every supported action uses platform-paid gas." : "New sponsorship is paused. Existing outcomes can still be checked; keep your saved review.");
  }
  async function ensureWallet() {
    if (!provider || !wallet) throw new Error("Connect the wallet that will authorize this action.");
    const chain = core.network(network).chain;
    if (Number(BigInt(await provider.request({ method: "eth_chainId" }))) !== chain) throw new Error("Your wallet changed networks. Reconnect to the selected Base network.");
    const accounts = await provider.request({ method: "eth_accounts" });
    if (!accounts?.some(address => lower(address) === wallet)) throw new Error("Your wallet account changed. Reconnect and review the exact request.");
    for (const key of ["token_authorization", "action_authorization"]) if (record?.local[key] && lower(record.local[key].signer) !== wallet) throw new Error("Connect the exact wallet named in this saved authorization.");
  }
  function invalidateWallet() { generation++; wallet = null; $("open-consent").checked = false; $("open-wallet").textContent = "Wallet or network changed. Reconnect before authorizing anything."; controls(); }
  async function connect() {
    const choice = await win.AgentBountiesWalletLink.select({ purpose: "creator-open", chainId: core.network(network).chain });
    if (choice.provider.agentBountiesCapabilities?.reviewedPostingOnly) throw new Error("This wallet has not enabled open-review signatures. Choose a browser wallet or supported phone wallet.");
    provider?.removeListener?.("accountsChanged", invalidateWallet); provider?.removeListener?.("chainChanged", invalidateWallet);
    provider = choice.provider;
    const accounts = await provider.request({ method: "eth_requestAccounts" }), address = lower(accounts?.[0]);
    if (!core.ADDRESS.test(address)) throw new Error("Your wallet did not provide a valid account.");
    const chain = `0x${core.network(network).chain.toString(16)}`;
    if (Number(BigInt(await provider.request({ method: "eth_chainId" }))) !== core.network(network).chain) await provider.request({ method: "wallet_switchEthereumChain", params: [{ chainId: chain }] });
    wallet = address; provider.on?.("accountsChanged", invalidateWallet); provider.on?.("chainChanged", invalidateWallet);
    await ensureWallet(); $("open-wallet").textContent = `Connected: ${wallet}. Connecting moved no money.`;
    message("Wallet connected. Prepare an exact review before signing.");
  }
  function renderReview() {
    $("open-review").hidden = !record; if (!record) return;
    const r = record.request, a = r.action, local = record.local;
    $("open-summary").textContent = `${a.kind.replaceAll("_", " ")} · ${core.money(local.principal)} from your wallet · 0 ETH network fee. Bounty ${local.bounty}.`;
    const effects = { create: "Fund the exact solver reward and review reserve. Anyone may submit; the first passing creator verdict pays one solver.", submit: "Post the submission bond. Rejection transfers it to the creator. An accepted result returns the bond and pays the reward. Unreviewed entries can recover their bond after the review deadline or another winner.", review: a.passed ? "Accept this exact submission and pay its solver. This selects the single winner and closes further submissions." : "Reject this exact submission. Its bond goes to the creator; the reward remains available for another result.", increase: `Increase the solver reward to ${core.money(a.new_reward_micro_usdc || 0)}. Only the additional amount leaves your wallet. The first submission prevents later increases.`, cancel: "Close this bounty before any submission. Recover the remaining funds in a separate sponsored refund action.", expire: "Close after the work deadline and review window. Bond and principal refunds remain separate sponsored actions.", refund_bond: `Return submission ${a.entry}'s eligible bond to its original solver. You cannot redirect it.`, refund_principal: "Return the closed bounty’s remaining principal to its creator. You cannot redirect it." };
    $("open-balance").textContent = record.shortfall && record.shortfall !== "0" ? `Add ${core.money(record.shortfall)} to ${local.token_authorization.signer} on Base${network === "base-sepolia" ? " Sepolia (test funds)" : ""}. Current balance: ${core.money(record.balance)}. Required: ${core.money(local.principal)}. Receive USDC on this exact network, then refresh this review. This action needs 0 ETH in the receiving wallet.` : "The USDC requirement was checked at preparation. Agent Bounties pays this action’s network fee.";
    $("open-copy-wallet").hidden = !record.shortfall || record.shortfall === "0";
    $("open-effect").textContent = effects[a.kind]; $("open-disclosure").textContent = core.DISCLOSURE;
    $("open-deadline").textContent = `This exact authorization expires ${new Date(r.valid_before * 1000).toLocaleString()}. Work deadline: ${new Date(r.original_terms.submission_deadline * 1000).toLocaleString()}.`;
    $("open-public-preview").replaceChildren();
    if (r.public_evidence) publicEvidence($("open-public-preview"), r.public_evidence, a.evidence_hash);
    if (record.reviewed_entry?.public_evidence) publicEvidence($("open-public-preview"), record.reviewed_entry.public_evidence, a.evidence_hash);
    if (record.review_reason) { const reason = doc.createElement("p"); reason.textContent = `Your verdict reason: ${record.review_reason}`; $("open-public-preview").append(reason); }
    $("open-details").textContent = JSON.stringify({ request: r, bounty_contract: local.bounty, approval_hash: local.approval_hash, token_authorization: local.token_authorization, action_authorization: local.action_authorization }, null, 2);
    // Signatures deliberately never enter the DOM, URLs or analytics.
    controls();
  }
  function shortfall(prepared, local) {
    if (BigInt(local.principal) === 0n) return "0";
    if (!/^[0-9]+$/.test(prepared.wallet_balance_micro_usdc || "")) throw new Error("The wallet USDC balance is unavailable. Recheck before signing.");
    const value = BigInt(local.principal) - BigInt(prepared.wallet_balance_micro_usdc);
    return String(value > 0n ? value : 0n);
  }
  async function recheckBalance() {
    const prepared = await request("/prepare", record.request);
    const local = core.validatePrepared(prepared.prepared, release, record.request, evm, now());
    record.shortfall = shortfall(prepared, local); record.balance = prepared.wallet_balance_micro_usdc; save(); renderReview();
    message(record.shortfall === "0" ? "The exact request is ready for your review. You do not need ETH." : "Add the exact Base USDC shortfall shown below, then refresh. Keep the same review.");
  }
  async function prepareReview(r, extra = {}) {
    if (recoveryBlocked) throw new Error("The saved recovery record needs inspection before any new authorization.");
    if (record) throw new Error("Finish checking the saved action before preparing another.");
    await ensureWallet();
    core.expected(release, r, evm, now());
    const captured = generation, prepared = await request("/prepare", r), local = core.validatePrepared(prepared.prepared, release, r, evm, now());
    if (captured !== generation) throw new Error("Your wallet changed during preparation. Review again.");
    record = { network, request: r, approval_hash: local.approval_hash, local, signatures: { token: "0x", action: "0x" }, submitted: false, confirmed: false, shortfall: shortfall(prepared, local), balance: prepared.wallet_balance_micro_usdc, ...extra };
    await ensureWallet(); save(); $("open-consent").checked = false; renderReview();
    $("open-review-title").scrollIntoView({ block: "start", behavior: "smooth" }); message("Check the exact terms and costs below. Nothing has been signed or sent.");
  }
  function createRequest() {
    const f = $("open-create-form"), get = name => f.elements.namedItem(name).value;
    const reward = core.units(get("reward")), reserve = core.units(get("reserve")), deadline = Math.floor(new Date(get("deadline")).getTime() / 1000);
    return { original_terms: { schema: core.PROTOCOL, network, bounty_id: evm.randomBytes32(), creator: wallet, title: get("title").trim(), goal: get("goal").trim(), acceptance_criteria: get("criteria").split("\n").map(v => v.trim()).filter(Boolean), solver_reward_micro_usdc: reward, review_reward_micro_usdc: reserve, submission_deadline: deadline, review_window_seconds: Number(get("reviewHours")) * 3600, max_pending_entries: Number(get("queue")), disclosure: core.DISCLOSURE }, user_salt: evm.randomBytes32(), expected_revision: 1, expected_solver_reward_micro_usdc: reward, valid_before: now() + 3600, action: { kind: "create", token_nonce: evm.randomBytes32() } };
  }
  async function actionRequest() {
    await loadBounty();
    const kind = $("open-action").value, blocker = core.actionBlocker(item, kind, wallet, now());
    if (blocker) throw new Error(blocker);
    const r = { original_terms: item.original_terms, user_salt: item.user_salt, expected_revision: item.revision, expected_solver_reward_micro_usdc: item.terms.solver_reward_micro_usdc, valid_before: now() + 3600, action: { kind } }, extra = {};
    if (kind === "submit") {
      const document = { artifact_url: $("open-artifact").value.trim(), artifact_sha256: $("open-artifact-hash").value.trim(), notes: $("open-notes").value.trim() }, hash = core.evidence(document, evm);
      r.public_evidence = document; Object.assign(r.action, { solver: wallet, submission_hash: hash, evidence_hash: hash, token_nonce: evm.randomBytes32() });
    } else if (kind === "increase") Object.assign(r.action, { new_reward_micro_usdc: core.units($("open-new-reward").value), token_nonce: evm.randomBytes32() });
    else if (["review", "refund_bond"].includes(kind)) {
      const id = Number($("open-entry").value); if (!Number.isSafeInteger(id) || id < 1) throw new Error("Enter an exact submission number.");
      r.action.entry = id;
      if (kind === "review") {
        const found = item.entries.find(e => e.id === id) || await loadBounty(id), verdict = $("open-verdict").value, reason = $("open-response").value.trim();
        if (!found || found.entry.status !== "pending" || !found.public_evidence) throw new Error("The pending submission and its public evidence must be available before review.");
        if (!["pass", "reject"].includes(verdict) || !reason) throw new Error("Review the artifact, choose a verdict and explain your decision.");
        r.valid_before = Math.min(r.valid_before, found.entry.review_deadline);
        core.evidence(found.public_evidence, evm);
        Object.assign(r.action, { solver: found.entry.solver, submission_hash: found.entry.submission_hash, evidence_hash: found.entry.evidence_hash, passed: verdict === "pass", response_hash: core.hashJson({ review: reason }, evm) });
        extra.reviewed_entry = found; extra.review_reason = reason;
      }
    }
    return { r, extra };
  }
  function validateOutcome(value) {
    const expected = { create: "CreatorOpenBountyCreated", submit: "OpenSubmissionAdded", review: record.request.action.passed ? "OpenBountySettled" : "OpenSubmissionRejected", increase: "OpenRewardIncreased", cancel: "OpenBountyCancelled", expire: "OpenBountyCancelled", refund_bond: "OpenBondRefunded", refund_principal: "OpenPrincipalRefunded" }[record.request.action.kind];
    if (value?.protocol !== core.PROTOCOL || value.network !== network || lower(value.approval_hash) !== record.approval_hash || lower(value.bounty_contract) !== record.local.bounty || value.customer_gas_wei !== "0"
      || (value.confirmed && value.evidence?.event !== expected) || (value.solver_paid && (!value.confirmed || expected !== "OpenBountySettled"))) throw new Error("The service reply does not match this saved action. Keep it for recovery.");
    if (record.transaction_hash && value.operation?.transaction_hash && record.transaction_hash !== value.operation.transaction_hash) throw new Error("The saved transaction hash changed. Stop and reconcile the original transaction.");
    record.transaction_hash = value.operation?.transaction_hash || record.transaction_hash;
    record.confirmed = value.confirmed === true; record.outcome = value.confirmed ? expected : null; save();
    const confirmed = { create: "Your bounty is funded. Continue to view its submissions.", submit: "Your submission and bond are confirmed. The creator can now review your result.", review: record.request.action.passed ? "The accepted result and solver payment are confirmed." : "The rejection is confirmed. The bounty remains funded for another result.", increase: "The larger reward is confirmed and funded.", cancel: "Your bounty is closed. Prepare the refund to return its remaining funds.", expire: "The bounty is closed after its deadline. Eligible bonds and remaining funds can be returned.", refund_bond: "The eligible bond was returned to its original solver.", refund_principal: "The remaining bounty funds were returned to the creator." };
    message(record.confirmed ? confirmed[record.request.action.kind] : `Saved transaction ${record.transaction_hash || "awaiting submission"}. Confirmation is still pending. Check status or retry the same signed request.`);
  }
  async function checkSaved() { if (!record) return; validateOutcome(await request("/status", { network, approval_hash: record.approval_hash })); renderReview(); }
  async function send() {
    await ensureWallet();
    if (!$("open-consent").checked || !available() || !complete() || record.request.valid_before <= now()) throw new Error("Review the saved request and current sponsorship before retrying. Expired requests remain available for status checks.");
    record.submitted = true; save();
    validateOutcome(await request("/relay", { request: record.request, approval_hash: record.approval_hash, signatures: record.signatures })); renderReview();
  }
  async function confirm() {
    if (!record || !$("open-consent").checked) throw new Error("Read and approve the exact commitment first.");
    const current = record, captured = generation;
    await ensureWallet(); if (!signed()) await recheckBalance(); if (record.shortfall !== "0") throw new Error("Add the exact USDC shortfall and refresh this same review before signing."); save();
    for (const [key, signature] of [["token_authorization", "token"], ["action_authorization", "action"]]) {
      if (!record.local[key] || record.signatures[signature] !== "0x") continue;
      await ensureWallet();
      if (generation !== captured || record !== current || !$("open-consent").checked || record.request.valid_before <= now()) throw new Error("The wallet or review changed. Keep the original request and reconnect.");
      const result = await provider.request({ method: "eth_signTypedData_v4", params: [wallet, JSON.stringify(record.local[key].typed_data)] });
      if (!/^0x(?:[0-9a-f]{2}){1,4096}$/i.test(result)) throw new Error("The wallet returned an invalid signature. Keep this review and reconnect.");
      current.signatures[signature] = result; save();
      if (generation !== captured || record !== current) throw new Error("Your wallet changed while signing. The signed request is saved, and nothing was sent by this page.");
    }
    await send();
  }
  click("open-copy-wallet", async () => { if (record?.local.token_authorization) { await win.navigator.clipboard.writeText(record.local.token_authorization.signer); message("Receiving address copied. Send only USDC on the selected Base network, then refresh this saved review."); } });
  click("open-connect", connect); click("open-refresh", refresh); click("open-check", checkSaved); click("open-confirm", confirm); click("open-retry", send);
  click("open-edit", async () => { if (signed()) throw new Error("Keep this signed request until its canonical outcome is known."); win.sessionStorage.removeItem(storageKey()); record = null; $("open-review").hidden = true; message("Edit the draft, then prepare a new review. No signature was discarded."); });
  click("open-finish", async () => { if (!record?.confirmed) return; await checkSaved(); if (!record.confirmed) return; contract = record.local.bounty; win.history.replaceState(null, "", `creator-open.html?network=${network}&bounty=${contract}`); win.sessionStorage.removeItem(storageKey()); record = null; $("open-review").hidden = true; await loadBounty(); message("Confirmed action saved. The next action uses the latest canonical bounty state."); });
  $("open-consent").addEventListener("change", controls); $("open-action").addEventListener("change", fieldVisibility);
  $("open-import-form").addEventListener("submit", event => { event.preventDefault(); if (event.isTrusted) run(async () => {
    if (record || recoveryBlocked || !release) throw new Error("Finish the saved action before importing a bounty.");
    let terms; try { terms = JSON.parse($("open-import-terms").value); } catch (_) { throw new Error("Paste the exact original terms as valid JSON."); }
    const salt = lower($("open-import-salt").value.trim()), hash = lower($("open-import-hash").value.trim());
    if (terms.network !== network || !core.HASH.test(salt) || !core.HASH.test(hash)) throw new Error("The original terms, salt and transaction must match the selected network.");
    const predicted = core.predict(release, { original_terms: terms, user_salt: salt }, evm);
    const imported = await request("/import", { original_terms: terms, user_salt: salt, creation_transaction_hash: hash });
    if (imported.protocol !== core.PROTOCOL || imported.network !== network || lower(imported.bounty_contract) !== predicted || lower(imported.creation_transaction_hash) !== hash || imported.imported !== true) throw new Error("The imported creation does not match the supplied original terms.");
    contract = predicted; item = null; win.history.replaceState(null, "", `creator-open.html?network=${network}&bounty=${contract}`); renderBounty();
    message("Creation verified. Refresh status after the canonical index catches up. Import is not a current-state or payment confirmation.");
  }); });
  $("open-create-form").addEventListener("submit", event => { event.preventDefault(); if (event.isTrusted) run(async () => prepareReview(createRequest())); });
  $("open-action-form").addEventListener("submit", event => { event.preventDefault(); if (event.isTrusted) run(async () => { const { r, extra } = await actionRequest(); await prepareReview(r, extra); }); });
  $("open-network").addEventListener("change", () => run(async () => { if (record) throw new Error("Finish the saved action before changing networks."); network = $("open-network").value; contract = null; item = null; invalidateWallet(); await refresh(); }));
  $("open-create-form").addEventListener("input", () => { if (record) return; try { win.sessionStorage.setItem(`agentbounties:creator-open-draft:${network}`, JSON.stringify(Object.fromEntries(new FormData($("open-create-form"))))); } catch (_) { message("Browser storage is unavailable. Keep this page open; signing stays blocked until the exact request can be saved."); } });
  try {
    const draft = JSON.parse(win.sessionStorage.getItem(`agentbounties:creator-open-draft:${network}`) || "{}");
    for (const [key, value] of Object.entries(draft)) if (typeof value === "string" && $("open-create-form").elements.namedItem(key)) $("open-create-form").elements.namedItem(key).value = value;
    if (!$("open-create-form").elements.namedItem("deadline").value) { const date = new Date(Date.now() + 7 * 86400000); date.setMinutes(date.getMinutes() - date.getTimezoneOffset()); $("open-create-form").elements.namedItem("deadline").value = date.toISOString().slice(0, 16); }
  } catch (_) { message("The local draft could not be restored. Keep any signed recovery record before starting again."); }
  const linkedEntry = Number(query.get("entry"));
  if (Number.isSafeInteger(linkedEntry) && linkedEntry > 0) { $("open-entry").value = String(linkedEntry); $("open-action").value = "review"; }
  async function restoreOrigin() {
    const id = query.get("github_origin"); if (!/^[0-9a-f-]{36}$/i.test(id || "") || contract || record || recoveryBlocked) return;
    const target = `/creator-open.html?github_origin=${id}`, login = $("open-origin-login");
    login.href = `${api}/v1/site-auth/login/github?return_to=${encodeURIComponent(target)}`;
    const control = new AbortController(), timer = win.setTimeout(() => control.abort(), 12000);
    try {
      const response = await win.fetch(`${api}/v1/site-auth/github-origin-drafts/${id}`, { headers: { Accept: "application/json" }, credentials: "include", cache: "no-store", redirect: "error", referrerPolicy: "no-referrer", signal: control.signal });
      if ([401, 403].includes(response.status)) { login.hidden = false; message("Sign in with the GitHub account that requested this private draft. No wallet is needed to restore it."); return; }
      if (!response.ok) throw new Error("The private issue draft is unavailable or expired. Return to GitHub setup and refresh your drafts.");
      const value = await response.json(), source = value.items?.find(item => item.id === id), draft = source?.plan?.draft;
      if (value.schema !== "agent-bounties/github-origin-v1" || value.funding_authority !== false || draft?.ready_for_publish !== false || typeof draft.title !== "string" || typeof draft.goal !== "string" || !Array.isArray(draft.acceptance_criteria) || draft.acceptance_criteria.some(v => typeof v !== "string")) throw new Error("The issue draft could not be verified.");
      const form = $("open-create-form");
      // A reload must not overwrite the person's edits with the original issue.
      const marker = `agentbounties:github-origin:${network}:${id}`;
      if (!win.sessionStorage.getItem(marker)) {
        if (form.elements.namedItem("title").value.trim() || form.elements.namedItem("goal").value.trim()) throw new Error("A different local draft is already open. Preserve or finish it before importing this issue.");
        form.elements.namedItem("title").value = draft.title; form.elements.namedItem("goal").value = draft.goal; form.elements.namedItem("criteria").value = draft.acceptance_criteria.join("\n");
        win.sessionStorage.setItem(`agentbounties:creator-open-draft:${network}`, JSON.stringify(Object.fromEntries(new FormData(form)))); win.sessionStorage.setItem(marker, "loaded");
      }
      $("open-origin-notice").hidden = false; $("open-origin-notice").textContent = "This is a private GitHub issue snapshot. Review and remove any confidential content before publishing. Choose the budget, deadline and review terms yourself; nothing is approved or funded."; login.hidden = true;
    } finally { win.clearTimeout(timer); }
  }
  fieldVisibility(); renderBounty(); controls(); run(async () => { await refresh(); await restoreOrigin(); });
})(window, document);
