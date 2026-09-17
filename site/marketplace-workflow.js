(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.AgentBountiesWorkflow = api;
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";
  const NETWORK = "base-mainnet";
  const ADDRESS = /^0x[0-9a-f]{40}$/i;
  const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
  const SESSION_KEY = "agent-bounties.guided-journey.v1";
  const stableJson = (value) => value && typeof value === "object"
    ? Array.isArray(value) ? `[${value.map(stableJson).join(",")}]`
      : `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${stableJson(value[key])}`).join(",")}}`
    : JSON.stringify(value);
  const BOUNDARY = "Only confirmed canonical BountySettled or CompetitionSettledV2 events prove payment. A draft, approval, signature or transaction hash does not.";
  const GUIDANCE = {
    style: "Use at most three short sentences per routine reply and give one primary next action. Define necessary terms: the worker does the task, the reviewer checks it, and gas means the network fee. Ask together only for missing outcome, budget, deadline or work preferences; detailed payment approvals may be longer.",
    autonomy: "Inspect, compare, draft, revise, check readiness, prepare evidence and poll unchanged requests without asking permission again. Do the work in the current assistant; never send the person to another chat to copy technical instructions.",
    consent: "Show one concise review of the exact public terms or submission, total cost, refundable bond, losing exposure and deadline. The person approves the commitment and accepts legal terms in the first-party page, then confirms native wallet requests. Never click their consent controls or accept consent through a tool boolean. Ask again only when the commitment, amount, recipient, wallet, scope or expiry changes.",
    progress: "Report meaningful changes only. Preserve the journey and stable request keys across navigation/retries. A pending transaction means wait, never ask to sign it again.",
    verification: "Prepare the exact committed verifier and evidence yourself. Never invent a benchmark or claim payment from an AI assessment. Explain unsupported work honestly and help adapt it to a supported verifier before funding.",
    posting_choices: "Before staging, explain why completion needs a check and offer the choices from agent_bounties_get_posting_options. Recommend one with its protocol, task-specific reason, review reward and total cost. Preserve per-result rates and campaign caps; changes need the person's agreement.",
    sign_in: "Use agent_bounties_open_account_setup to show sign-in in this same browser tab, preserving the draft. The person completes credentials and consent there. Prefer validated WebMCP actions, record any necessary UI fallback, and keep routine setup in this browser.",
  };
  const POSTING_OPTIONS = {
    explanation: "A bounty holds the reward until someone checks the finished work. The verifier is the person or test that makes that check.",
    choices: [
      { review_mode: "creator", label: "I review the work", suitable_for: "Outreach, research, design, and work that needs your judgment.", explanation: "You check the evidence and confirm pass or fail in your wallet within the review window. This is your decision, not an independent review.",
        decision_maker: "You, the bounty creator", evidence: "The delivered artifact and evidence for every agreed acceptance check.", costs: "At least 2 USDC for the worker and a positive review reserve of at least 0.01 USDC; the reserve pays you after either completed verdict.", limitations: "You must review within the published review window; no independent reviewer is provided.", protocol_id: "agent-bounties/autonomous-v1" },
      { review_mode: "automated", label: "An automated test checks it", suitable_for: "Work with a supported, repeatable pass/fail test, such as a code fix.", explanation: "The exact test and verifier must be supported and ready before funding. An assistant's opinion alone cannot release money.",
        decision_maker: "The precommitted verifier quorum running the supported test", evidence: "The exact pinned benchmark, its required submission evidence and matching verifier verdicts.", costs: "At least 2 USDC for the worker and at least 0.01 USDC for the review; confirm the actual supported verifier's reward and readiness before funding.", limitations: "An unsupported or unavailable test cannot be funded; this option cannot establish whether a real-company conversation is genuine.", protocol_id: "agent-bounties/autonomous-v1" },
    ],
    protocol: { id: "agent-bounties/autonomous-v1", explanation: "This form posts one fixed-reward bounty on Base. Its contract holds the funds and pays after the agreed review. The review method is fixed before funding." },
    other_protocols: [{ id: "agent-bounties/open-competition-v2-beta3", explanation: "An opt-in competition can reward the first proven result or the best score. It needs a supported deterministic proof program and live release readiness. It does not verify real-company conversations or implement per-referral campaign payouts; it is not a substitute for creator review." }],
    costs: "Ordinary bounties need at least 2 USDC for the worker plus a positive review reward of at least 0.01 USDC. You approve that split. The worker separately supplies a bond equal to the review reward. Creator review still needs this reserve: it pays you after either completed verdict, not automatically. Platform fee: 0 USDC. Creation also needs Base ETH for a network fee shown before sending.",
    campaign_limit: "This form does not implement an open-ended per-response or referral campaign with a shared budget cap. Preserve those requested rates and explain this limit; only propose separate fixed bounties with the person's agreement.",
    wallets: "Use or recover a Coinbase embedded wallet with email or social sign-in, or choose a browser or phone wallet. Linking proves ownership; payment requires a separate confirmation.",
  };
  function postingOptions(workType) {
    const reasons = {
      outreach: ["creator", "You need to judge the company identity, response and referral evidence."],
      research: ["creator", "You need to judge the sources and whether the findings answer your question."],
      creative: ["creator", "You need to judge the delivered design against your acceptance checks."],
      software: ["automated", "A supported pinned regression test can measure whether the requested code behavior works."],
    };
    if (workType !== undefined && !Object.hasOwn(reasons, workType)) throw new Error("Choose outreach, research, creative or software work.");
    const match = reasons[workType];
    return { ...POSTING_OPTIONS, recommendation: match ? { review_mode: match[0], protocol_id: POSTING_OPTIONS.protocol.id, reason: match[1], approval_required: true } : null,
      next_action: match ? "Review this recommendation and choose how the work will be checked." : "Describe the outcome to get a review recommendation." };
  }
  function apiBase(location) {
    return ["localhost", "127.0.0.1", "::1", "[::1]"].includes(location?.hostname)
      ? "http://127.0.0.1:3000" : "https://api.agentbounties.app";
  }
  function units(amount) {
    if (!amount || amount.unit !== "base_units" || amount.decimals !== 6 || !/^\d+$/.test(String(amount.amount))) return null;
    return BigInt(amount.amount);
  }
  function isV2(item) {
    return String(item?.opportunity_id || "").startsWith("open-competition-v2:")
      || item?.evidence_requirements?.protocol_version === "agent-bounties/open-competition-v2-beta3"
      || String(item?.next_action?.action || "").includes("open_competition_v2");
  }
  function ready(item) {
    const reward = units(item?.reward), funded = units(item?.funded_amount), target = units(item?.funding_target);
    if (!item || item.network !== NETWORK || item.source_type !== "canonical_base" || !ADDRESS.test(item.source_id)
      || item.work_state !== "claimable" || item.payment_state !== "escrowed" || item.payment_committed !== true
      || item.verification_ready !== true || reward === null || reward <= 0n || target === null || target <= 0n
      || funded === null || funded < target) return false;
    return isV2(item)
      ? item.source_status === "active" && ["first_proven", "best_score"].includes(item.competition_mode)
        && Boolean(item.evidence_requirements?.program_profile) && Boolean(item.evidence_requirements?.verification_policy_hash)
        && item.evidence_requirements?.participation_metadata_ready !== false
      : item.source_status === "claimable" && Boolean(item.terms_hash);
  }
  function participationKind(item) {
    if (item?.standing_meta_bounty === true) return "child_funding";
    if (isV2(item) || ["best_score", "first_proven", "first_valid_submission", "open_competition"].includes(item?.competition_mode)) return "competition";
    return item?.competition_mode === "exclusive_claim" ? "direct" : "unknown";
  }
  function phase(item, now = Date.now()) {
    const scoring = item?.evidence_requirements?.scoring_window;
    if (!Number.isFinite(now) || participationKind(item) === "unknown") return "unavailable";
    if (scoring) {
      const start = Date.parse(scoring.starts_at), end = Date.parse(scoring.ends_at);
      if (!Number.isFinite(start) || !Number.isFinite(end) || start >= end) return "unavailable";
      if (now < start) return "upcoming";
      if (now < end) return "now";
      return "ended";
    }
    // A V2 proof deadline cannot substitute for its missing scoring window.
    if (isV2(item)) return "unavailable";
    const deadline = Date.parse(item?.deadline || "");
    if (!Number.isFinite(deadline)) return "unavailable";
    return now < deadline ? "now" : "closed";
  }
  function opportunityDeadline(item, now = Date.now()) {
    const scoring = item?.evidence_requirements?.scoring_window;
    const end = Date.parse(scoring?.ends_at || ""), start = Date.parse(scoring?.starts_at || "");
    // Once scoring closes, participants need the remaining proof deadline.
    if (Number.isFinite(start) && Number.isFinite(end) && start < end && now < end) return end;
    const deadline = Date.parse(item?.deadline || "");
    return Number.isFinite(deadline) ? deadline : Infinity;
  }
  function sortOpportunities(items, now = Date.now()) {
    return [...items].sort((a, b) =>
      Number(participationKind(b) === "direct") - Number(participationKind(a) === "direct")
      || opportunityDeadline(a, now) - opportunityDeadline(b, now)
      || String(a.opportunity_id).localeCompare(String(b.opportunity_id)));
  }
  function inventoryCounts(items, now = Date.now()) {
    const counts = { now: 0, upcoming: 0, ended: 0, closed: 0, unavailable: 0, direct: 0, competition: 0, child_funding: 0, unknown: 0 };
    for (const item of items) {
      const timing = phase(item, now);
      counts[timing]++;
      if (timing === "now") counts[participationKind(item)]++;
    }
    return counts;
  }
  function fundedInventory(payload) {
    if (payload?.schema_version !== "agent-bounties/opportunity-projection-v1" || payload.network !== NETWORK
      || payload.applied_view !== "ready_to_earn" || payload.degraded !== false || !Array.isArray(payload.items)) {
      throw new Error("Funded inventory evidence is incomplete.");
    }
    const canonical = payload.source_statuses?.find?.((source) => source?.source_type === "canonical_base");
    if (canonical?.available !== true || payload.items.some((item) => !ready(item))) {
      throw new Error("Canonical inventory evidence is unavailable.");
    }
    return { payload, items: payload.items };
  }
  function opportunityFeedUrl(location) {
    return `${apiBase(location)}/v1/opportunities?network=${NETWORK}&view=ready_to_earn&source_type=canonical_base&limit=300`;
  }
  async function loadFundedInventory(win) {
    const response = await win.fetch(opportunityFeedUrl(win.location), { cache: "no-store", credentials: "omit", referrerPolicy: "no-referrer", headers: { Accept: "application/json" } });
    if (!response.ok) throw new Error(`Funded inventory request failed (${response.status}).`);
    return fundedInventory(await response.json());
  }
  function detailUrl(item) {
    if (!ADDRESS.test(item?.source_id) || item.network !== NETWORK) throw new Error("The opportunity has no supported canonical address.");
    return `${isV2(item) ? "competition" : "participate"}.html?bountyContract=${encodeURIComponent(item.source_id)}&network=${NETWORK}`;
  }
  function text(value, label, max) {
    if (typeof value !== "string" || !value.trim() || [...value.trim()].length > max) throw new Error(`${label} is required and must be at most ${max} characters.`);
    return value.trim();
  }
  function publicJson(value) {
    const json = JSON.stringify(value);
    if (!value || typeof value !== "object" || Array.isArray(value) || json.length > 16000) throw new Error("Provide a bounded public evidence object (at most 16 KB).");
    const visit = (object, depth = 0) => {
      if (depth > 6) throw new Error("Public evidence is too deeply nested.");
      if (object && typeof object === "object") for (const [key, child] of Object.entries(object)) {
        if (/^(?:password|passphrase|secret|token|api[_ -]?key|private[_ -]?key|seed(?:[_ -]?phrase)?|mnemonic|wallet[_ -]?signature|payment[_ -]?authorization|access[_ -]?token|refresh[_ -]?token|card[_ -]?number|cvv|otp)$/i.test(key)) throw new Error("Keep credentials, signatures and payment details out of public evidence.");
        visit(child, depth + 1);
      }
    };
    visit(value);
    return JSON.parse(json);
  }
  function summarize(item) {
    return {
      opportunity_id: item.opportunity_id, title: item.title, goal: item.goal,
      bounty_contract: item.source_id, network: item.network, ready: ready(item), phase: phase(item),
      reward_usdc: units(item.reward) === null ? null : Number(units(item.reward)) / 1e6,
      bond_usdc: units(item.bond) === null ? null : Number(units(item.bond)) / 1e6,
      cash_economics: item.cash_economics, deadline: item.deadline, standing_meta_bounty: item.standing_meta_bounty === true,
      competition_mode: item.competition_mode, verification_method: item.verification_method,
      participation_kind: participationKind(item),
      evidence_requirements: item.evidence_requirements, terms_hash: item.terms_hash,
      participation_url: detailUrl(item), next_action: item.next_action, payment_boundary: BOUNDARY,
    };
  }
  function createClient(win) {
    let memory = null;
    async function request(path, body) {
      if (!path.startsWith("/v1/")) throw new Error("Unsupported platform request.");
      const response = await win.fetch(`${apiBase(win.location)}${path}`, {
        method: body === undefined ? "GET" : "POST", cache: "no-store", credentials: "omit", referrerPolicy: "no-referrer",
        headers: { Accept: "application/json", ...(body === undefined ? {} : { "content-type": "application/json" }) },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      });
      const payload = await response.json().catch(() => null);
      if (!response.ok) throw new Error(payload?.message || payload?.next_action || `The platform could not complete this step (${response.status}). Retry this same step; no new permission is needed.`);
      if (!payload) throw new Error("The platform returned no usable result.");
      return payload;
    }
    async function inventory(view = "ready_to_earn") {
      const payload = await request(`/v1/opportunities?network=${NETWORK}&view=${view}&source_type=canonical_base&limit=300`);
      if (payload.schema_version !== "agent-bounties/opportunity-projection-v1" || !Array.isArray(payload.items)) throw new Error("The marketplace returned an unsupported inventory format.");
      return { payload, items: payload.items.filter(view === "ready_to_earn" ? ready : (item) => item.network === NETWORK && ADDRESS.test(item.source_id)) };
    }
    async function opportunity(id, requireReady = false) {
      let { items } = await inventory(requireReady ? "ready_to_earn" : "recent");
      let item = items.find((entry) => entry.opportunity_id === id || entry.source_id?.toLowerCase() === String(id).toLowerCase());
      // An older funded item can be outside the recent page. Do not substitute a cached card.
      if (!item && !requireReady) {
        ({ items } = await inventory());
        item = items.find((entry) => entry.opportunity_id === id || entry.source_id?.toLowerCase() === String(id).toLowerCase());
      }
      if (!item) throw new Error("This opportunity is unavailable in the current marketplace. Refresh the list and choose another; do not start or spend from an old card.");
      return item;
    }
    function load() {
      try { const raw = win.sessionStorage.getItem(SESSION_KEY); if (raw) memory = JSON.parse(raw); } catch (_) { /* Session-only fallback. */ }
      if (!memory || memory.schema !== "agent-bounties/guided-journey-v1") return null;
      return memory;
    }
    function save(value) {
      memory = JSON.parse(JSON.stringify(value));
      try { win.sessionStorage.setItem(SESSION_KEY, JSON.stringify(memory)); } catch (_) { /* Return state so the assistant can preserve it. */ }
      win.dispatchEvent?.(new win.CustomEvent("agent-bounties:journey", { detail: memory }));
      return memory;
    }
    function start(input) {
      if (!["post", "earn"].includes(input.role)) throw new Error("Choose post or earn.");
      let current = load();
      if (input.new_task === true) {
        createPostingJournal(win).archiveConfirmed();
        current = null;
      }
      return save({ ...current, schema: "agent-bounties/guided-journey-v1", id: current?.id || win.crypto.randomUUID(),
        role: input.role, goal: input.goal ? text(input.goal, "Goal", 4000) : current?.goal || "",
        preferences: input.preferences ? text(input.preferences, "Preferences", 1000) : current?.preferences || "",
        steps: current?.steps || {}, updated_at: new Date().toISOString() });
    }
    async function inspect(id) {
      const item = await opportunity(id);
      const result = { ...summarize(item), terms: null };
      if (!isV2(item) && /^0x[0-9a-f]{64}$/i.test(item.terms_hash || "")) result.terms = await request(`/v1/base/autonomous-bounties/terms/${item.terms_hash}`);
      return result;
    }
    async function prepareAction(input) {
      if (!["solve", "fund", "complete", "verify"].includes(input.action)) throw new Error("Unsupported action.");
      const item = await opportunity(input.opportunity_id, input.action === "solve");
      if (isV2(item)) return { status: "competition_workflow", opportunity: summarize(item), next_action: { tool: "agent_bounties_open_opportunity", input: { opportunity_id: item.opportunity_id } }, user_confirmation_required: false };
      if (input.action === "verify") return { status: "verification_tracking", opportunity: summarize(item), next_action: { tool: "agent_bounties_open_opportunity", input: { opportunity_id: item.opportunity_id } },
        user_confirmation_required: false, guidance: "Read get_work_status in the workspace. Continue through the committed verifier job using an available execution interface, or wait for the committed verifier. Tracking requires no approval. Do not invent verifier signatures or turn an AI opinion into settlement." };
      const journey = load() || start({ role: input.action === "fund" ? "post" : "earn" });
      const details = input.action === "complete"
        ? { artifact_reference: text(input.artifact_reference, "Public artifact reference", 4000), evidence: publicJson(input.evidence) }
        : input.action === "solve" ? { title: item.title, claim_bond_base_units: String(units(item.bond) ?? 0n), verification_ready: item.verification_ready }
        : input.action === "verify" ? { title: item.title, verification_method: item.verification_method }
        : { title: item.title };
      let amount;
      if (input.action === "fund") {
        amount = Number(input.amount_base_units);
        if (!Number.isSafeInteger(amount) || amount <= 0) throw new Error("Choose the exact contribution in USDC base units.");
        const remaining = (units(item.funding_target) ?? 0n) - (units(item.funded_amount) ?? 0n);
        if (BigInt(amount) > remaining) throw new Error("The contribution exceeds the remaining funding target. Refresh the amount before review.");
      }
      const body = { action: input.action, network: NETWORK, opportunity_id: item.opportunity_id, bounty_contract: item.source_id, details, ...(amount ? { amount_base_units: amount } : {}) };
      const fingerprint = stableJson(body);
      const existing = journey.steps[fingerprint];
      // Reserve before the network call: a lost response retries the same durable key.
      const step = existing || { key: `webmcp:${journey.id}:${win.crypto.randomUUID()}` };
      journey.steps[fingerprint] = step;
      journey.selected = item.opportunity_id;
      save(journey);
      const reserve = () => {
        try {
          win.sessionStorage.setItem(SESSION_KEY, JSON.stringify(journey));
          if (JSON.parse(win.sessionStorage.getItem(SESSION_KEY)).steps[fingerprint].key !== step.key) throw new Error("Checkpoint mismatch");
        } catch (_) { throw new Error("This browser cannot preserve a safe retry checkpoint. Open the same review in a browser with session storage before preparing a wallet action."); }
      };
      reserve();
      let intent = step.intent_id ? await progress(step.intent_id)
        : await request("/v1/chatgpt/action-intents", { ...body, idempotency_key: step.key });
      if (intent.status === "expired" && !intent.transaction_hash) {
        // Renew only a never-broadcast review. Local pending/unknown wallet state also blocks renewal.
        const walletStep = JSON.parse(win.sessionStorage.getItem(`agent-bounties.wallet-step.v1:${intent.intent_id}`) || "{}");
        if (walletStep.sending || walletStep.lastHash || walletStep.finalHash) throw new Error("The review expired after a wallet step started. Reconcile that transaction before preparing another review.");
        step.key = `webmcp:${journey.id}:${win.crypto.randomUUID()}`; delete step.intent_id; save(journey);
        reserve();
        intent = await request("/v1/chatgpt/action-intents", { ...body, idempotency_key: step.key });
      }
      if (!UUID.test(intent.intent_id) || intent.action !== input.action || intent.network !== NETWORK || String(intent.bounty_contract).toLowerCase() !== item.source_id.toLowerCase()) throw new Error("The returned review does not match the requested bounty action.");
      step.intent_id = intent.intent_id;
      journey.current_intent = intent.intent_id;
      save(journey);
      return { ...intent, authorization_url: `${detailUrl(item)}&intent=${encodeURIComponent(intent.intent_id)}`,
        user_confirmation_required: intent.status === "review_required", next_action: intent.status === "pending_confirmation" ? "Check status with the same intent; do not sign again." : "Open the first-party review. The person confirms the exact commitment there, with no extra chat approval.", guidance: GUIDANCE.consent };
    }
    async function progress(intentId) {
      if (!UUID.test(intentId || "")) throw new Error("A valid review identifier is required.");
      const intent = await request(`/v1/chatgpt/action-intents/${intentId}`);
      const paid = intent.status === "confirmed" && intent.canonical_event_id && intent.confirmed_block != null
        && ["bounty_settled", "competition_settled_v2"].includes(intent.canonical_event_kind) && intent.paid === true;
      return { ...intent, paid: Boolean(paid), user_confirmation_required: false, poll_after_seconds: 15,
        next_step: paid ? "Payment confirmed. Show the evidence and offer another suitable task."
          : intent.status === "confirmed" ? "This step is confirmed. Inspect the bounty and continue to the next step."
          : intent.status === "pending_confirmation" ? "Wait and check again; keep the same request and do not ask for another signature." : intent.next_action };
    }
    return { request, inventory, opportunity, inspect, prepareAction, progress, load, save, start };
  }
  function createPostingJournal(win) {
    const key = "agent-bounties.posting-operation.v1";
    const load = () => JSON.parse(win.sessionStorage.getItem(key) || "null");
    const save = (value) => { win.sessionStorage.setItem(key, JSON.stringify(value)); win.dispatchEvent?.(new win.CustomEvent("agent-bounties:posting-journal", { detail: value })); return value; };
    const atomicParamsRejected = (error) => error?.code === -32602 && typeof error.message === "string"
      && error.message.replace(/\s+/g, " ").trim() === "Invalid params 0 > atomicRequired - Expected a value of type `boolean`, but received: `undefined`";
    const metaMaskBatchRejected = (error) => error?.code === 4001 && error.message === "MetaMask Tx Signature: User denied transaction signature.";
    function validateRejectedBatch(expected, error) {
      const current = load();
      if (!atomicParamsRejected(error) && !metaMaskBatchRejected(error)) throw new Error("Only the wallet's exact missing atomicRequired validation error or MetaMask signature rejection can recover this legacy batch. Keep uncertain requests recorded.");
      if (!current || !ADDRESS.test(expected?.bounty_contract || "") || !/^0x[0-9a-fA-F]{64}$/.test(expected?.bounty_id || "")
        || current.bounty_contract.toLowerCase() !== expected.bounty_contract.toLowerCase() || current.bounty_id !== expected.bounty_id)
        throw new Error("The rejected batch does not match the recorded posting operation.");
      const capturedMatches = current.wallet_error?.code === error.code
        && (atomicParamsRejected(current.wallet_error) && atomicParamsRejected(error) || metaMaskBatchRejected(current.wallet_error) && metaMaskBatchRejected(error));
      const legacyReport = !current.error_capture_version && (atomicParamsRejected(error) && !current.wallet_method
        || metaMaskBatchRejected(error) && current.wallet_method === "wallet_sendCalls");
      if (current.phase !== "sending" || current.authorizationIssued || !Array.isArray(current.transactions) || current.transactions.length
        || current.wallet_method && current.wallet_method !== "wallet_sendCalls" || !capturedMatches && !legacyReport)
        throw new Error("An authorized, submitted or uncertain posting step cannot be cleared by this recovery.");
      return current;
    }
    return {
      load,
      validateRejectedBatch,
      archiveRejectedBatch(expected, error, snapshot) {
        const current = validateRejectedBatch(expected, error);
        if (JSON.stringify(current) !== JSON.stringify(snapshot)) throw new Error("The posting operation changed during recovery. Check its status again.");
        const historyKey = `${key}.rejected`;
        const history = JSON.parse(win.sessionStorage.getItem(historyKey) || "[]");
        const archived = { ...current, phase: atomicParamsRejected(error) ? "batch_params_rejected" : "batch_wallet_rejected", wallet_error: { code: error.code, message: error.message },
          evidence_source: current.wallet_error ? "wallet_response" : "user_reported_wallet_response", archived_at: new Date().toISOString() };
        win.sessionStorage.setItem(historyKey, JSON.stringify([...history, archived]));
        win.sessionStorage.removeItem(key);
        return archived;
      },
      prepare(plan) {
        if (load()) throw new Error("A posting wallet step is already recorded. Check its canonical status before starting another.");
        return save({ bounty_contract: plan.predicted_bounty_contract, bounty_id: plan.bounty_id, phase: "prepared", transactions: [], error_capture_version: 1 });
      },
      checkpoint(phase, hash, walletMethod) {
        const current = load();
        if (!current) throw new Error("Posting recovery state is unavailable; no new wallet request can be sent.");
        return save({ ...current, phase, ...(walletMethod ? { wallet_method: walletMethod } : {}), authorizationIssued: current.authorizationIssued || phase === "authorized", transactions: hash ? [...current.transactions, hash] : current.transactions });
      },
      authorizeContinuation(hash) {
        const current = load();
        if (!current || current.submission_attempt_id || current.transactions.length || !["signing", "authorized"].includes(current.phase)
          || !/^0x[0-9a-f]{64}$/.test(hash) || current.continuation_hash && current.continuation_hash !== hash)
          throw new Error("The signed posting request cannot replace an existing operation.");
        return save({ ...current, phase: "authorized", authorizationIssued: true, continuation_hash: hash });
      },
      reserveContinuation(attempt) {
        const current = load();
        if (!current?.continuation_hash || current.phase !== "authorized" || current.submission_attempt_id || current.transactions.length || !UUID.test(attempt))
          throw new Error("This operation is already reserved or submitted. Check its status; do not send it again.");
        return save({ ...current, phase: "sending", wallet_method: "eth_sendTransaction", submission_attempt_id: attempt });
      },
      reject(error) {
        const current = load();
        if (current?.phase === "sending" && current.wallet_method === "wallet_sendCalls" && atomicParamsRejected(error))
          save({ ...current, wallet_error: { code: error.code, message: error.message } });
        if (current && !current.authorizationIssued && !current.transactions.length
          && (current.phase === "prepared" || ["signing", "sending"].includes(current.phase) && error.code === 4001)) win.sessionStorage.removeItem(key);
      },
      archiveConfirmed() {
        const current = load();
        if (!current) return;
        if (current.phase !== "funding_confirmed") throw new Error("Check the existing posting operation before starting a new task.");
        win.sessionStorage.setItem(`${key}.previous`, JSON.stringify(current));
        win.sessionStorage.removeItem(key);
      },
    };
  }
  return { ADDRESS, UUID, BOUNDARY, GUIDANCE, POSTING_OPTIONS, postingOptions, NETWORK, SESSION_KEY, apiBase, units, isV2, ready, participationKind, phase, opportunityDeadline, sortOpportunities, inventoryCounts, fundedInventory, opportunityFeedUrl, loadFundedInventory, detailUrl, text, publicJson, summarize, createClient, createPostingJournal };
});
