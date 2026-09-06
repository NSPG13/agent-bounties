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
  const BOUNDARY = "Only confirmed canonical BountySettled or CompetitionSettledV2 events prove payment. A draft, approval, signature or transaction hash does not.";
  const GUIDANCE = {
    style: "Use ordinary language. Ask only for missing outcome, budget, deadline or work preferences. Suggest sensible defaults together; do not ask one technical question at a time.",
    autonomy: "Inspect, compare, draft, revise, check readiness, prepare evidence and poll unchanged requests without asking permission again. Do the work in the current assistant; never send the person to another chat to copy technical instructions.",
    consent: "Show one concise review of the exact public terms or submission, total cost, refundable bond, losing exposure and deadline. The person approves the commitment and accepts legal terms in the first-party page, then confirms native wallet requests. Never click their consent controls or accept consent through a tool boolean. Ask again only when the commitment, amount, recipient, wallet, scope or expiry changes.",
    progress: "Report meaningful changes only. Preserve the journey and stable request keys across navigation/retries. A pending transaction means wait, never ask to sign it again.",
    verification: "Prepare the exact committed verifier and evidence yourself. Never invent a benchmark or claim payment from an AI assessment. Explain unsupported work honestly and help adapt it to a supported verifier before funding.",
  };
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
  function phase(item, now = Date.now()) {
    const scoring = item?.evidence_requirements?.scoring_window;
    if (!scoring) return "now";
    const start = Date.parse(scoring.starts_at), end = Date.parse(scoring.ends_at);
    if (!Number.isFinite(start) || !Number.isFinite(end) || start >= end) return "unavailable";
    if (now < start) return "upcoming";
    if (now < end) return "now";
    return "ended";
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
      const fingerprint = JSON.stringify(body);
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
    const save = (value) => { win.sessionStorage.setItem(key, JSON.stringify(value)); return value; };
    return {
      load,
      prepare(plan) {
        if (load()) throw new Error("A posting wallet step is already recorded. Check its canonical status before starting another.");
        return save({ bounty_contract: plan.predicted_bounty_contract, bounty_id: plan.bounty_id, phase: "prepared", transactions: [] });
      },
      checkpoint(phase, hash) {
        const current = load();
        if (!current) throw new Error("Posting recovery state is unavailable; no new wallet request can be sent.");
        return save({ ...current, phase, authorizationIssued: current.authorizationIssued || phase === "authorized", transactions: hash ? [...current.transactions, hash] : current.transactions });
      },
      reject(error) {
        const current = load();
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
  return { ADDRESS, UUID, BOUNDARY, GUIDANCE, NETWORK, SESSION_KEY, apiBase, units, isV2, ready, phase, detailUrl, text, publicJson, summarize, createClient, createPostingJournal };
});
