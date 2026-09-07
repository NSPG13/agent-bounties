(() => {
  "use strict";

  const API = "https://api.agentbounties.app";
  const MAX_AI_ROUNDS = 3;
  const MAX_PLAN_TASKS = 6;
  const MAX_TASK_DAYS = 30;
  const MIN_TOTAL_USDC = 2.01;
  const MIN_SOLVER_USDC_BASE_UNITS = 2_000_000n;
  const metaChild = typeof module === "object" && module.exports ? require("./meta-child.js") : window.AgentBountiesMetaChild;
  const MAX_TOTAL_USDC = 9_000_000_000;
  const REGRESSION_ENGINE = "sandboxed_regression_v1";
  const REGRESSION_VERIFIERS = [
    "0xbe6292b9e465f549e2363b918d6dd9187038431e",
  ];
  const RECONCILED_REGRESSION_BENCHMARK_DIGESTS = new Set([
    "sha256:b61a96a7d07ca01337ea3576de734f5b62ccab966a6d0da42a8736cfc0287ce6",
    "sha256:b9b0d026347a2922f913e9a8ed3651dd74e7eba930598981a169da3bf42e7c3f",
    "sha256:30bb17e3e3916747144c7087f49fb1ce41ddaf1aec4d717f878d2840203895a2",
    "sha256:6c7a300bcdd84f125bf9811297d72f3717d5ebd65f326c5e23687f44ba553043",
    "sha256:94eff483d0fbba47037a1dedaae1e9339e23f218eb29ea3182fbc256e7e1c587",
    "sha256:63e28323ea17da7ef0fb79e447256540e28f9c7525a8657707aea1598ce05bff",
    "sha256:73fc58dcd45e551344f8889095b7d3a71546170ba7f05fb1876aaf6aa796ac3d",
    "sha256:3bfb647d41539693c9598a01d9f9f7953a285dfb7c1986a190560a8745f64731",
    "sha256:a14e53feada2f49b646d340a494c822ec3112a2a6c468ce1cdb21fd7ee23a3d7",
  ]);
  const RECONCILED_REGRESSION_BENCHMARK_COMMIT = "fa946859a3379b8c9128183e20dedb3b8319a646";
  const RECONCILED_REGRESSION_BENCHMARK_SOURCES = new Map([
    ["sha256:b61a96a7d07ca01337ea3576de734f5b62ccab966a6d0da42a8736cfc0287ce6", "benchmarks/direct-growth-v2/a2a-agent-card"],
    ["sha256:b9b0d026347a2922f913e9a8ed3651dd74e7eba930598981a169da3bf42e7c3f", "benchmarks/direct-growth-v2/hermes-integration"],
    ["sha256:30bb17e3e3916747144c7087f49fb1ce41ddaf1aec4d717f878d2840203895a2", "benchmarks/direct-growth-v2/openhands-integration"],
    ["sha256:6c7a300bcdd84f125bf9811297d72f3717d5ebd65f326c5e23687f44ba553043", "benchmarks/direct-growth-v2/mini-swe-agent-environment"],
    ["sha256:94eff483d0fbba47037a1dedaae1e9339e23f218eb29ea3182fbc256e7e1c587", "benchmarks/direct-inventory-v1/rpc-failover"],
    ["sha256:63e28323ea17da7ef0fb79e447256540e28f9c7525a8657707aea1598ce05bff", "benchmarks/direct-inventory-v1/inventory-breakdown"],
    ["sha256:73fc58dcd45e551344f8889095b7d3a71546170ba7f05fb1876aaf6aa796ac3d", "benchmarks/direct-inventory-v1/wallet-liquidity"],
    ["sha256:3bfb647d41539693c9598a01d9f9f7953a285dfb7c1986a190560a8745f64731", "benchmarks/direct-inventory-v1/replenishment-plan"],
    ["sha256:a14e53feada2f49b646d340a494c822ec3112a2a6c468ce1cdb21fd7ee23a3d7", "benchmarks/direct-inventory-v1/stalled-work"],
  ]);
  const RUNNER_MANIFEST_FIELDS = [
    "schema_version", "image", "command", "workdir", "benchmark_digest",
    "timeout_seconds", "cpu_millis", "memory_bytes", "pids_limit",
    "max_output_bytes", "tmpfs_bytes", "max_source_bytes", "max_source_files",
    "max_benchmark_bytes", "max_benchmark_files", "platform", "test_seed",
  ];
  const RUNNER_BOUNDS = {
    timeout_seconds: [1, 900],
    cpu_millis: [100, 4_000],
    memory_bytes: [67_108_864, 4_294_967_296],
    pids_limit: [16, 512],
    max_output_bytes: [1_024, 16_777_216],
    tmpfs_bytes: [67_108_864, 4_294_967_296],
    max_source_bytes: [1, 2_147_483_648],
    max_source_files: [1, 100_000],
    max_benchmark_bytes: [1, 536_870_912],
    max_benchmark_files: [1, 50_000],
    test_seed: [0, Number.MAX_SAFE_INTEGER],
  };
  const VISUAL_EXTENSION = "x-agent-bounties-draft-visual";
  const ALLOWED_SCENES = new Set([
    "infrastructure", "digital", "nature", "health", "research", "education", "coordination", "general",
  ]);
  const ALLOWED_ELEMENTS = new Set([
    "sun", "road", "lamp", "building", "tree", "water", "screen", "document", "network", "tool", "person", "check", "bridge", "book", "chart",
  ]);

  if (typeof module === "object" && module.exports) {
    module.exports = Object.freeze({
      parseDistributionAttribution,
      parsePreparedRewardSplit,
      rewardSplitForTotal,
      verificationReadiness,
    });
    return;
  }

  const ui = {
    form: document.getElementById("bounty-composer-form"),
    input: document.getElementById("bounty-composer-input"),
    label: document.querySelector("[data-composer-label]"),
    prompt: document.querySelector("[data-assistant-prompt]"),
    submit: document.querySelector("[data-composer-submit]"),
    mic: document.querySelector("[data-dictate]"),
    hint: document.querySelector("[data-composer-hint]"),
    status: document.querySelector("[data-composer-status]"),
    progress: Array.from(document.querySelectorAll("[data-progress-step]")),
    preview: document.getElementById("bounty-preview"),
    art: document.getElementById("bounty-card-art"),
    imageStatus: document.querySelector("[data-image-status]"),
    badge: document.querySelector("[data-card-badge]"),
    title: document.querySelector("[data-card-title]"),
    goal: document.querySelector("[data-card-goal]"),
    criteria: document.querySelector("[data-card-criteria]"),
    reward: document.querySelector("[data-card-reward]"),
    deadline: document.querySelector("[data-card-deadline]"),
    checks: document.querySelector("[data-card-checks]"),
    confidence: document.querySelector("[data-card-confidence]"),
    risks: document.querySelector("[data-card-risks]"),
    verifierSummary: document.querySelector("[data-card-verifier-summary]"),
    verifier: document.querySelector("[data-card-verifier]"),
    missionContext: document.querySelector("[data-mission-context]"),
    missionTitle: document.querySelector("[data-mission-title]"),
    missionSummary: document.querySelector("[data-mission-summary]"),
    missionHorizon: document.querySelector("[data-mission-horizon]"),
    missionTasks: document.querySelector("[data-mission-tasks]"),
    approve: document.querySelector("[data-approve-card]"),
    revise: document.querySelector("[data-revise-card]"),
    share: document.querySelector("[data-share-card]"),
    fund: document.querySelector("[data-open-funding]"),
    dialog: document.getElementById("funding-dialog"),
    closeDialog: document.querySelector("[data-close-funding]"),
    cryptoMethod: document.querySelector("[data-payment-method='crypto']"),
    walletPanel: document.querySelector("[data-wallet-panel]"),
    walletOptions: document.querySelector("[data-wallet-options]"),
    walletMessage: document.querySelector("[data-wallet-message]"),
    readiness: document.querySelector("[data-wallet-readiness]"),
    account: document.querySelector("[data-wallet-account]"),
    usdcBalance: document.querySelector("[data-wallet-usdc]"),
    ethBalance: document.querySelector("[data-wallet-eth]"),
    requiredUsdc: document.querySelector("[data-wallet-required]"),
    fundingHelp: document.querySelector("[data-funding-help]"),
    missingUsdc: document.querySelector("[data-missing-usdc]"),
    onramps: [...document.querySelectorAll("[data-onramp-link]")],
    watchUsdc: document.querySelector("[data-watch-usdc]"),
    copyUsdc: document.querySelector("[data-copy-usdc]"),
    recheck: document.querySelector("[data-recheck-balance]"),
    fundNow: document.querySelector("[data-fund-now]"),
    paymentStatus: document.querySelector("[data-payment-status]"),
    headerTitle: document.querySelector(".chat-app-title"),
    aiHandoff: document.querySelector("[data-ai-handoff]"),
  };

  if (!ui.form || !ui.input || !ui.preview || !ui.dialog || !ui.art) return;

  const state = {
    phase: "describe",
    originalRequest: "",
    context: [],
    initialDraft: null,
    draft: null,
    aiRounds: 0,
    questions: [],
    currentQuestion: null,
    askedQuestions: new Set(),
    scope: null,
    horizon: null,
    missionPlan: null,
    selectedTaskId: null,
    taskWindowDays: null,
    fundingUsdc: null,
    metaParent: null,
    preparedRewards: null,
    visualSpec: null,
    visualSource: "fallback",
    visualCache: new Map(),
    bountyImage: null,
    imageReady: false,
    approved: false,
    protocol: null,
    providers: [],
    provider: null,
    account: null,
    balances: null,
    bountyContract: null,
    bountyId: null,
    speech: null,
    handoffReview: false,
    distributionAttribution: null,
    observedWalletState: null,
  };

  function track(eventName, details) {
    window.agentBountiesAnalytics?.track(eventName, details);
  }

  const postingJournal = window.AgentBountiesWorkflow.createPostingJournal(window);
  let postingBusy = false;

  function enableAiHandoffReview() {
    state.handoffReview = true;
    document.body.classList.add("chatgpt-handoff-review");
    ui.form.dataset.handoffReview = "true";
    const inputWrap = ui.form.querySelector(".chat-composer-input-wrap");
    if (inputWrap) inputWrap.hidden = true;
    ui.input.disabled = true;
    ui.submit.disabled = true;
    ui.mic.hidden = true;
    ui.revise.hidden = true;
    if (ui.aiHandoff) ui.aiHandoff.hidden = true;
    if (ui.headerTitle) ui.headerTitle.textContent = "Review bounty";
    setProgress("review");
  }

  const announcedProviders = [];
  window.addEventListener("eip6963:announceProvider", (event) => {
    const detail = event && event.detail;
    if (!detail || !detail.provider || typeof detail.provider.request !== "function") return;
    if (!announcedProviders.some((item) => item.provider === detail.provider)) announcedProviders.push(detail);
  });

  function setStatus(message, tone = "") {
    ui.status.textContent = message || "";
    ui.status.dataset.tone = tone;
  }

  function setPaymentStatus(message, tone = "") {
    ui.paymentStatus.textContent = message || "";
    ui.paymentStatus.dataset.tone = tone;
  }

  function setProgress(active) {
    const order = ["describe", "clarify", "review", "fund"];
    const activeIndex = order.indexOf(active);
    ui.progress.forEach((item) => {
      const index = order.indexOf(item.dataset.progressStep);
      item.dataset.active = String(index === activeIndex);
      item.dataset.complete = String(index >= 0 && index < activeIndex);
    });
  }

  function setComposer({ phase, prompt, label, placeholder, button, hint = "" }) {
    state.phase = phase;
    ui.prompt.textContent = prompt;
    ui.label.textContent = label;
    ui.input.placeholder = placeholder;
    ui.submit.textContent = button;
    ui.hint.textContent = hint;
    ui.input.value = "";
    ui.input.disabled = false;
    ui.submit.disabled = false;
    setProgress(phase === "describe" ? "describe" : "clarify");
    requestAnimationFrame(() => ui.input.focus({ preventScroll: true }));
  }

  function normalizeQuestion(value) {
    return String(value || "").trim().toLowerCase().replace(/\s+/g, " ");
  }

  function safeTextList(values, maximum = 12) {
    return (Array.isArray(values) ? values : [])
      .map((value) => String(value || "").trim())
      .filter(Boolean)
      .slice(0, maximum);
  }

  function randomBytes32() {
    const bytes = new Uint8Array(32);
    crypto.getRandomValues(bytes);
    return `0x${Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
  }

  function formatUsdc(value, maximum = 6) {
    const number = Number(value);
    if (!Number.isFinite(number)) return "0";
    return number.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: maximum });
  }

  function usdcBaseUnits(value) {
    const number = Number(value);
    if (!Number.isFinite(number) || number < 0 || number > MAX_TOTAL_USDC) {
      throw new Error("Enter a valid USDC amount.");
    }
    return BigInt(Math.round(number * 1_000_000));
  }

  function splitReward(total) {
    const totalUnits = usdcBaseUnits(total);
    if (totalUnits < 2_010_000n) {
      throw new Error("Public bounties require at least 2 USDC for the solver plus a verifier reserve.");
    }
    const proportional = totalUnits / 50n;
    const verifier = proportional < 10_000n ? 10_000n : proportional;
    let cappedVerifier = verifier > totalUnits / 5n ? totalUnits / 5n : verifier;
    if (totalUnits - cappedVerifier < MIN_SOLVER_USDC_BASE_UNITS) {
      cappedVerifier = totalUnits - MIN_SOLVER_USDC_BASE_UNITS;
    }
    if (cappedVerifier % 2n !== 0n) cappedVerifier -= 1n;
    const solver = totalUnits - cappedVerifier;
    if (solver < MIN_SOLVER_USDC_BASE_UNITS || cappedVerifier < 10_000n) {
      throw new Error("The solver reward must remain at least 2 USDC and the verifier reserve at least 0.01 USDC.");
    }
    return { total: totalUnits, solver, verifier: cappedVerifier };
  }

  function parsePreparedRewardSplit(solverValue, verifierValue, parent = null) {
    const decimalUsdc = /^\d+(?:\.\d{1,6})?$/;
    if (!decimalUsdc.test(String(solverValue || "").trim()) || !decimalUsdc.test(String(verifierValue || "").trim())) {
      throw new Error("Solver and verifier rewards must be decimal USDC amounts with up to six places.");
    }
    const solver = usdcBaseUnits(solverValue);
    const verifier = usdcBaseUnits(verifierValue);
    const total = solver + verifier;
    if (solver <= 0n || verifier <= 0n) throw new Error("Solver and verifier rewards must both be positive.");
    if (parent) return metaChild.rewards(solver, verifier, parent);
    if (solver < MIN_SOLVER_USDC_BASE_UNITS) {
      throw new Error("Public bounties require at least 2 USDC for the solver.");
    }
    if (verifier < 10_000n) {
      throw new Error("Public bounties require at least 0.01 USDC for the verifier.");
    }
    if (total < usdcBaseUnits(MIN_TOTAL_USDC) || total > usdcBaseUnits(MAX_TOTAL_USDC)) {
      throw new Error("The combined reward is invalid.");
    }
    return { total, solver, verifier };
  }

  function currentRewardSplit() {
    return rewardSplitForTotal(state.fundingUsdc, state.preparedRewards, state.metaParent);
  }

  function rewardSplitForTotal(total, preparedRewards = null, parent = null) {
    const totalUnits = usdcBaseUnits(total);
    if (parent) {
      if (totalUnits !== 1000000n) throw new Error("This parent's qualifying child must total exactly 1 USDC.");
      return metaChild.rewards(preparedRewards?.solver ?? 990000n, preparedRewards?.verifier ?? 10000n, parent);
    }
    if (preparedRewards && preparedRewards.total === totalUnits) return preparedRewards;
    return splitReward(total);
  }

  function parseFunding(value) {
    const cleaned = String(value || "").replace(/,/g, "");
    const match = cleaned.match(/(?:^|[^0-9])(\d+(?:\.\d{1,6})?)(?:\s*(?:base\s*)?usdc|\s*dollars?|\s*usd)?(?:$|[^0-9])/i);
    if (!match) return null;
    const amount = Number(match[1]);
    if (state.metaParent) return amount === 1 ? amount : null;
    return Number.isFinite(amount) && amount >= MIN_TOTAL_USDC && amount <= MAX_TOTAL_USDC ? amount : null;
  }

  function parseDistributionAttribution(params) {
    const acquisition = params.get("acquisition");
    const handoff = params.get("handoff");
    if (!acquisition && !handoff) return null;
    if (!acquisition || !handoff) {
      throw new Error("The attributed handoff must include both acquisition and handoff identifiers.");
    }
    if (!/^aba1_[0-9a-f]{64}\.[0-9a-f]{64}$/.test(acquisition)
      || !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(handoff)) {
      throw new Error("The attributed handoff identifiers are malformed. Return to the originating agent and prepare the bounty again.");
    }
    return Object.freeze({ acquisition, handoff: handoff.toLowerCase() });
  }

  function parseHorizon(value) {
    const raw = String(value || "").trim();
    if (/\b(ongoing|continuous|continuing|indefinite|no fixed end|no end date|open[- ]ended|work toward)\b/i.test(raw)) {
      return { kind: "ongoing", label: "Ongoing", date: null, days: null };
    }
    const duration = raw.match(/(?:in\s+)?(\d+)\s*(hour|hours|day|days|week|weeks|month|months|year|years)\b/i);
    let date = null;
    if (duration) {
      const count = Number(duration[1]);
      const unit = duration[2].toLowerCase();
      const hours = unit.startsWith("year")
        ? count * 365 * 24
        : unit.startsWith("month")
          ? count * 30 * 24
          : unit.startsWith("week")
            ? count * 7 * 24
            : unit.startsWith("day")
              ? count * 24
              : count;
      if (!Number.isFinite(hours) || hours < 1 || hours > 100 * 365 * 24) return null;
      date = new Date(Date.now() + hours * 3_600_000);
    } else {
      const iso = raw.match(/\b(20\d{2}-\d{2}-\d{2})(?:[T\s](\d{1,2}:\d{2}))?\b/);
      if (iso) date = new Date(`${iso[1]}T${iso[2] || "23:59"}:00`);
      else {
        const parsed = Date.parse(raw);
        if (Number.isFinite(parsed)) date = new Date(parsed);
      }
    }
    if (!date || !Number.isFinite(date.getTime()) || date.getTime() <= Date.now()) return null;
    const days = Math.max(1, Math.ceil((date.getTime() - Date.now()) / 86_400_000));
    if (days > 100 * 365) return null;
    return {
      kind: "date",
      date,
      days,
      label: date.toLocaleString(undefined, { year: "numeric", month: "short", day: "numeric" }),
    };
  }

  function parseScope(value) {
    const raw = String(value || "").toLowerCase();
    if (/\b(mission|ongoing|long[- ]term|larger effort|multiple goals|several goals|many tasks|work toward|continuous)\b/.test(raw)) return "mission";
    if (/\b(single|one result|one task|one-time|bounded|specific deliverable|finish once)\b/.test(raw)) return "single";
    return null;
  }

  function parseTaskWindow(value) {
    const match = String(value || "").match(/(\d+)\s*(hour|hours|day|days|week|weeks)?/i);
    if (!match) return null;
    const count = Number(match[1]);
    const unit = String(match[2] || "days").toLowerCase();
    const days = unit.startsWith("hour") ? Math.ceil(count / 24) : unit.startsWith("week") ? count * 7 : count;
    return Number.isInteger(days) && days >= 1 && days <= MAX_TASK_DAYS ? days : null;
  }

  function visualInstruction() {
    return `The evidence_schema must remain a valid JSON Schema object and may include this non-verification annotation: "${VISUAL_EXTENSION}". The annotation must be an object with: version 1; scene chosen from infrastructure, digital, nature, health, research, education, coordination, general; palette containing 2-5 six-digit hex colours; and elements containing 3-10 objects. Each element must have kind chosen from sun, road, lamp, building, tree, water, screen, document, network, tool, person, check, bridge, book, chart; x and y integers from 0 to 100; and size a number from 0.4 to 2.0. Use it to depict the achieved result without words, logos, brands, payment symbols, or claims that the work is already complete. This annotation is only for a bounded pre-publication illustration and will be removed before the evidence schema is published.`;
  }

  async function requestJson(url, options = {}) {
    const acceptance = window.AgentBountiesLegal && window.AgentBountiesLegal.latestReceipt();
    const response = await fetch(url, {
      ...options,
      headers: {
        "content-type": "application/json",
        ...(acceptance ? { "x-agent-bounties-legal-acceptance": acceptance.acceptance_id } : {}),
        ...(options.headers || {}),
      },
    });
    const text = await response.text();
    let body = null;
    if (text) {
      try { body = JSON.parse(text); } catch (_error) { body = text; }
    }
    if (!response.ok) {
      const message = typeof body === "string"
        ? body
        : body && (body.message || body.error)
          ? body.message || body.error
          : `Request failed (${response.status}).`;
      throw new Error(message);
    }
    return body;
  }

  function draftContext(extra = "") {
    return [
      "Create one public digital-work bounty draft. Make the result specific, measurable, attainable, and independently inspectable. Ask only questions whose answers materially change the public terms.",
      visualInstruction(),
      ...state.context,
      extra,
    ].filter(Boolean).join("\n").slice(0, 18_000);
  }

  async function generateInitialDraft() {
    ui.submit.disabled = true;
    ui.input.disabled = true;
    setStatus("The AI is turning your words into clear, measurable terms…", "pending");
    try {
      const readiness = await requestJson(`${API}/v1/cloud-agent/readiness`, { cache: "no-store" });
      if (!readiness.available || !readiness.public_drafts) throw new Error("The AI composer is temporarily unavailable. Nothing was posted or funded.");
      const draft = await requestJson(`${API}/v1/cloud-agent/bounty-drafts`, {
        method: "POST",
        body: JSON.stringify({
          objective: state.originalRequest,
          context: draftContext(),
          constraints: [],
          source_url: null,
          idempotency_key: `web-card:${randomBytes32().slice(2)}`,
        }),
      });
      state.initialDraft = normalizeDraft(draft);
      state.draft = state.initialDraft;
      state.aiRounds += 1;
      queueQuestions(state.draft.questions);
      advanceConversation();
    } catch (error) {
      setStatus(error.message || String(error), "error");
      ui.input.disabled = false;
      ui.submit.disabled = false;
    }
  }

  function normalizeDraft(draft) {
    return {
      ...draft,
      acceptance_criteria: safeTextList(draft.acceptance_criteria, 20),
      questions: safeTextList(draft.questions, 10),
      risk_flags: safeTextList(draft.risk_flags, 10),
    };
  }

  function requestUserOwnedAi(intent, context = null) {
    if (state.metaParent) context = { ...context, meta_child: metaChild.normalize(state.metaParent) };
    window.dispatchEvent(new CustomEvent("agent-bounties:request-ai-handoff", {
      detail: { intent, context },
    }));
  }

  async function importPreparedDraft(value) {
    let prepared = window.AgentBountyAI?.parseDraft
      ? window.AgentBountyAI.parseDraft(value)
      : value;
    if (!prepared || typeof prepared !== "object") throw new Error("The prepared bounty draft is invalid.");

    prepared = window.AgentBountiesCreatorReview.prepare(prepared);
    const days = Number(prepared.task_window_days || MAX_TASK_DAYS);
    const parentContext = prepared.meta_child || (new URLSearchParams(window.location.search).has("parentBounty") ? state.metaParent : null);
    const metaParent = parentContext ? await metaChild.resolve(parentContext, window.AgentBountiesWorkflow.createClient(window)) : null;
    const preparedRewards = parsePreparedRewardSplit(prepared.solver_reward_usdc, prepared.verifier_reward_usdc, metaParent);
    const total = Number(preparedRewards.total) / 1_000_000;
    if (!Number.isInteger(days) || days < 1 || days > MAX_TASK_DAYS) throw new Error(`The task window must be from 1 to ${MAX_TASK_DAYS} days.`);
    const bountyImage = normalizeBountyImage(prepared.image);
    if (prepared.image_required === true && !bountyImage) {
      throw new Error("The prepared bounty must include the exact image generated and approved in ChatGPT.");
    }

    const deadline = prepared.delivery_deadline ? new Date(prepared.delivery_deadline) : new Date(Date.now() + days * 86_400_000);
    state.deliveryDeadline = prepared.delivery_deadline || null;
    state.phase = "review";
    state.originalRequest = prepared.goal;
    state.reviewStale = false;
    state.context = [];
    state.initialDraft = normalizeDraft({
      title: prepared.title,
      goal: prepared.goal,
      acceptance_criteria: prepared.acceptance_criteria,
      questions: [],
      risk_flags: [],
      benchmark: prepared.benchmark || { type: "creator_review" },
      evidence_schema: prepared.evidence_schema || { type: "object", additionalProperties: true },
    });
    state.draft = state.initialDraft;
    state.aiRounds = 0;
    state.questions = [];
    state.currentQuestion = null;
    state.askedQuestions.clear();
    state.scope = "single";
    state.horizon = {
      kind: "date",
      date: deadline,
      days,
      label: prepared.delivery_deadline ? `${deadline.toLocaleString()} (${Intl.DateTimeFormat().resolvedOptions().timeZone})` : `${days} days after claim`,
    };
    state.missionPlan = null;
    state.selectedTaskId = null;
    state.taskWindowDays = days;
    state.fundingUsdc = total;
    state.preparedRewards = preparedRewards;
    state.metaParent = metaParent;
    state.bountyImage = bountyImage;
    state.approved = false;
    ui.input.value = prepared.goal;
    ui.prompt.textContent = state.handoffReview
      ? (state.bountyImage
          ? "Review the exact image and bounty terms you approved in your AI conversation. Agent Bounties cannot edit or replace them."
          : "Review the exact bounty terms you approved in your AI conversation. This card uses a deterministic content-derived visual.")
      : "Review the bounty card your AI prepared. You can approve it or ask your AI for a revision.";
    await renderPreview();
    const client = window.AgentBountiesWorkflow.createClient(window);
    const journey = client.load() || client.start({ role: "post" });
    client.save({ ...journey, role: "post", goal: prepared.goal, draft: prepared, draft_stale: false, brief: { ...journey.brief, goal: prepared.goal, budget_usdc: String(total), deadline_at: prepared.delivery_deadline || journey.brief?.deadline_at || null } });
  }

  function queueQuestions(questions) {
    for (const question of questions || []) {
      const normalized = normalizeQuestion(question);
      if (normalized && !state.askedQuestions.has(normalized)) state.questions.push(question);
    }
  }

  function askNextQuestion() {
    const question = state.questions.shift();
    if (!question) return false;
    state.currentQuestion = question;
    state.askedQuestions.add(normalizeQuestion(question));
    setComposer({
      phase: "ai_question",
      prompt: question,
      label: "Your answer",
      placeholder: "Answer naturally. You can type or dictate.",
      button: "Continue",
      hint: "The AI will use this answer to improve the bounty terms.",
    });
    setStatus("Nothing is posted until you approve the final card.");
    return true;
  }

  async function regenerateAfterAnswers() {
    if (state.aiRounds >= MAX_AI_ROUNDS) {
      advanceConversation();
      return;
    }
    await generateInitialDraft();
  }

  function askScope() {
    setComposer({
      phase: "scope",
      prompt: "Is this one result to finish, or a larger or ongoing mission that should be broken into tasks?",
      label: "Scope",
      placeholder: "Example: one result, or an ongoing mission",
      button: "Continue",
      hint: "Long-running and open-ended missions are allowed. Their individual tasks still need definite results and completion checks.",
    });
  }

  function askHorizon() {
    setComposer({
      phase: "horizon",
      prompt: state.scope === "mission"
        ? "Does the mission have an overall target date, or is it ongoing?"
        : "When should this result be completed?",
      label: state.scope === "mission" ? "Mission horizon" : "Completion deadline",
      placeholder: state.scope === "mission" ? "Example: ongoing, in 18 months, or 2028-06-30" : "Example: in 14 days or 2026-08-15",
      button: "Continue",
      hint: state.scope === "mission"
        ? "The mission may be long-term or ongoing. The AI will break it into bounded tasks."
        : "A single on-chain task currently has a maximum 30-day work window. Longer efforts are converted into a mission with staged tasks.",
    });
  }

  async function buildMissionPlan() {
    ui.input.disabled = true;
    ui.submit.disabled = true;
    setStatus("The AI is breaking the mission into definite, verifiable tasks…", "pending");
    try {
      const plan = await requestJson(`${API}/v1/cloud-agent/objective-plans`, {
        method: "POST",
        body: JSON.stringify({
          objective: state.originalRequest,
          context: [
            `Clarified draft: ${state.initialDraft.goal}`,
            `Mission horizon: ${state.horizon.label}`,
            ...state.context,
            "Break this mission into independent public digital tasks. Each task must have a definite inspectable artifact and binary acceptance criteria. No individual task should require more than 30 days once claimed.",
          ].join("\n").slice(0, 16_000),
          constraints: ["Each task must be bounded, measurable, and independently fundable."],
          max_tasks: MAX_PLAN_TASKS,
          solver_budget_usdc: null,
          source_url: null,
          idempotency_key: `web-mission:${randomBytes32().slice(2)}`,
        }),
      });
      state.missionPlan = plan;
      state.selectedTaskId = plan.tasks[0].task_id;
      state.draft = draftFromTask(plan.tasks[0]);
      askTaskWindow();
    } catch (error) {
      setStatus(error.message || String(error), "error");
      ui.input.disabled = false;
      ui.submit.disabled = false;
    }
  }

  function draftFromTask(task) {
    const properties = {};
    for (const field of task.evidence_schema?.required || task.evidence_fields || []) {
      properties[field] = { type: "string", minLength: 1 };
    }
    const evidenceSchema = task.evidence_schema || {
      type: "object",
      additionalProperties: false,
      required: Object.keys(properties),
      properties,
    };
    return {
      title: task.title,
      goal: task.goal,
      acceptance_criteria: safeTextList(task.acceptance_criteria, 20),
      benchmark: {
        verifier: task.verifier,
        mission_task_id: task.task_id,
        depends_on: task.depends_on || [],
      },
      evidence_schema: evidenceSchema,
      questions: [],
      risk_flags: safeTextList(state.missionPlan?.risk_flags, 10),
    };
  }

  function askTaskWindow() {
    setComposer({
      phase: "task_window",
      prompt: "How many days should the first funded task have once someone claims it?",
      label: "Task work window",
      placeholder: "Example: 14 days",
      button: "Continue",
      hint: `The mission may be much longer, but each current bounty task must be staged into a 1–${MAX_TASK_DAYS} day work window.`,
    });
  }

  function askFunding() {
    setComposer({
      phase: "funding",
      prompt: state.scope === "mission"
        ? "How much Base USDC do you want to fund for the selected task?"
        : "How much Base USDC do you want to deposit as the total bounty reward?",
      label: "Total bounty funding",
      placeholder: "Example: 25 USDC",
      button: "Create bounty card",
      hint: "The total includes the solver reward and a verifier reserve. Minimum: 2.01 USDC total, including at least 2 USDC for the solver.",
    });
  }

  function advanceConversation() {
    ui.input.disabled = false;
    ui.submit.disabled = false;
    if (askNextQuestion()) return;
    if (!state.scope) {
      askScope();
      return;
    }
    if (!state.horizon) {
      askHorizon();
      return;
    }
    if (state.scope === "mission" && !state.missionPlan) {
      buildMissionPlan();
      return;
    }
    if (state.scope === "mission" && !state.taskWindowDays) {
      askTaskWindow();
      return;
    }
    if (state.scope === "single" && !state.taskWindowDays) {
      state.taskWindowDays = Math.max(1, Math.min(MAX_TASK_DAYS, state.horizon.days || MAX_TASK_DAYS));
    }
    if (state.fundingUsdc == null) {
      askFunding();
      return;
    }
    renderPreview();
  }

  async function handleComposerSubmit(event) {
    event.preventDefault();
    const value = ui.input.value.trim();
    if (!value) {
      setStatus("Please answer before continuing.", "error");
      return;
    }
    if (state.phase === "describe") {
      state.originalRequest = value;
      state.context = [];
      state.aiRounds = 0;
      state.questions = [];
      state.askedQuestions.clear();
      state.scope = null;
      state.horizon = null;
      state.missionPlan = null;
      state.taskWindowDays = null;
      state.fundingUsdc = parseFunding(value);
      state.preparedRewards = null;
      requestUserOwnedAi(value);
      ui.input.value = "";
      return;
    }
    if (state.phase === "ai_question") {
      state.context.push(`Question: ${state.currentQuestion}\nAnswer: ${value}`);
      state.currentQuestion = null;
      if (state.questions.length) askNextQuestion();
      else await regenerateAfterAnswers();
      return;
    }
    if (state.phase === "scope") {
      const scope = parseScope(value);
      if (!scope) {
        setStatus("Please say whether this is one result or a larger or ongoing mission.", "error");
        return;
      }
      state.scope = scope;
      state.context.push(`Scope chosen by the user: ${scope}.`);
      setStatus("");
      advanceConversation();
      return;
    }
    if (state.phase === "horizon") {
      const horizon = parseHorizon(value);
      if (!horizon) {
        setStatus("Enter a future date, a duration, or say that the mission is ongoing.", "error");
        return;
      }
      state.horizon = horizon;
      if (state.scope === "single" && (horizon.kind === "ongoing" || horizon.days > MAX_TASK_DAYS)) {
        state.scope = "mission";
        state.context.push("The requested horizon exceeds one bounty work window, so the effort must be staged as a mission with bounded tasks.");
        setStatus("This is longer than one current bounty work window, so the AI will split it into staged tasks. The overall mission keeps its full horizon.", "pending");
      } else setStatus("");
      advanceConversation();
      return;
    }
    if (state.phase === "task_window") {
      const days = parseTaskWindow(value);
      if (!days) {
        setStatus(`Choose a work window from 1 to ${MAX_TASK_DAYS} days. Longer work should be split into another milestone task.`, "error");
        return;
      }
      state.taskWindowDays = days;
      setStatus("");
      advanceConversation();
      return;
    }
    if (state.phase === "funding") {
      const amount = parseFunding(value);
      if (amount == null) {
        setStatus(state.metaParent ? "This parent's qualifying child requires exactly 1 USDC total, including verifiers." : `Enter an amount from ${MIN_TOTAL_USDC.toFixed(2)} to ${MAX_TOTAL_USDC.toLocaleString()} USDC.`, "error");
        return;
      }
      state.fundingUsdc = amount;
      state.preparedRewards = null;
      setStatus("");
      renderPreview();
      return;
    }
    if (state.phase === "revise") {
      const rewards = currentRewardSplit();
      requestUserOwnedAi(value, {
        draft: state.draft,
        solver_reward_usdc: formatUsdc(Number(rewards.solver) / 1_000_000),
        verifier_reward_usdc: formatUsdc(Number(rewards.verifier) / 1_000_000),
        task_window_days: state.taskWindowDays,
      });
      ui.input.value = "";
      return;
    }
  }

  function selectedTask() {
    return state.missionPlan?.tasks.find((task) => task.task_id === state.selectedTaskId) || null;
  }

  function renderMissionPlan() {
    const isMission = state.scope === "mission" && state.missionPlan;
    ui.missionContext.hidden = !isMission;
    if (!isMission) return;
    ui.missionTitle.textContent = state.missionPlan.title;
    ui.missionSummary.textContent = state.missionPlan.success_definition;
    ui.missionHorizon.textContent = `Mission horizon: ${state.horizon.label}`;
    ui.missionTasks.replaceChildren();
    for (const task of state.missionPlan.tasks) {
      const label = document.createElement("label");
      label.className = "mission-task-option";
      const input = document.createElement("input");
      input.type = "radio";
      input.name = "mission-task";
      input.value = task.task_id;
      input.checked = task.task_id === state.selectedTaskId;
      const copy = document.createElement("span");
      const title = document.createElement("strong");
      title.textContent = task.title;
      const detail = document.createElement("small");
      detail.textContent = `${task.acceptance_criteria.length} completion checks${task.depends_on?.length ? ` · depends on ${task.depends_on.join(", ")}` : " · can start independently"}`;
      copy.append(title, detail);
      label.append(input, copy);
      input.addEventListener("change", () => selectMissionTask(task.task_id));
      ui.missionTasks.append(label);
    }
  }

  async function selectMissionTask(taskId) {
    if (taskId === state.selectedTaskId) return;
    const task = state.missionPlan.tasks.find((candidate) => candidate.task_id === taskId);
    if (!task) return;
    state.selectedTaskId = taskId;
    state.draft = draftFromTask(task);
    state.approved = false;
    ui.fund.disabled = true;
    ui.approve.dataset.approved = "false";
    ui.approve.textContent = "Approve bounty card";
    renderCardText();
    renderMissionPlan();
    await renderAiVisualForCurrentDraft();
  }

  function riskSummary() {
    const risks = state.draft.risk_flags || [];
    return risks.length ? `${risks.length} item${risks.length === 1 ? "" : "s"} to review` : "Review required";
  }

  function renderCardText() {
    const rewards = currentRewardSplit();
    ui.title.textContent = state.draft.title;
    ui.goal.textContent = state.draft.goal;
    ui.reward.textContent = `${formatUsdc(state.fundingUsdc)} USDC (${formatUsdc(Number(rewards.solver) / 1_000_000)} solver + ${formatUsdc(Number(rewards.verifier) / 1_000_000)} verifier)`;
    if (state.metaParent) ui.reward.textContent += ` · Qualifying child for: ${state.metaParent.title}`;
    ui.deadline.textContent = state.scope === "mission"
      ? `${state.taskWindowDays} days for this task · ${state.horizon.label} mission`
      : state.horizon.label;
    ui.checks.textContent = `${state.draft.acceptance_criteria.length} check${state.draft.acceptance_criteria.length === 1 ? "" : "s"}`;
    ui.confidence.textContent = riskSummary();
    ui.criteria.replaceChildren();
    for (const criterion of state.draft.acceptance_criteria) {
      const item = document.createElement("li");
      item.textContent = criterion;
      ui.criteria.append(item);
    }
    renderVerifierTerms();
    ui.risks.replaceChildren();
    const risks = state.draft.risk_flags || [];
    if (!risks.length) {
      const item = document.createElement("li");
      item.textContent = "No material blocker was identified by the drafting AI. The creator still accepts feasibility and verification risk.";
      ui.risks.append(item);
    } else {
      for (const risk of risks) {
        const item = document.createElement("li");
        item.textContent = risk;
        ui.risks.append(item);
      }
    }
    ui.badge.textContent = state.scope === "mission" ? "Mission task draft · not posted" : "Draft · not posted";
  }

  function renderVerifierTerms() {
    if (!ui.verifierSummary || !ui.verifier) return;
    const benchmark = missionBenchmark(state.draft?.benchmark || {});
    if (benchmark.engine === "creator_review_v1") {
      const reward = currentRewardSplit();
      ui.verifierSummary.textContent = `You review the delivered files and confirm the verdict in your wallet. The ${formatUsdc(Number(reward.verifier) / 1_000_000)} USDC verifier reward is paid to you on pass or fail. This is your review; no independent automated verifier is involved.`;
      ui.verifier.replaceChildren();
      for (const [label, value] of [["Reviewer", "You, using the wallet that funds this bounty"], ["Due", state.horizon.label], ["Evidence", "Public deliverable URL and SHA-256 digest"], ["Review window", "48 hours after submission"], ["Wallet cost", "Base gas is additional; this route does not promise sponsorship"]]) {
        const dt = document.createElement("dt"), dd = document.createElement("dd"); dt.textContent = label; dd.textContent = value; ui.verifier.append(dt, dd);
      }
      return;
    }
    const source = benchmark.source || {};
    const runner = benchmark.runner_manifest || {};
    const requiredEvidence = state.draft?.evidence_schema?.required || [];
    const readiness = verificationReadiness(benchmark, state.draft?.evidence_schema);
    ui.verifierSummary.textContent = readiness.blocked
      ? "This exact benchmark digest has not been independently reconciled. Choose a reviewed benchmark before connecting a wallet."
      : readiness.executable
        ? "These exact public inputs and direct command decide whether the verifier may sign. Confirm every value before connecting a wallet."
        : "No complete executable verifier is attached. This draft cannot be funded until one is precommitted and reviewed.";
    ui.verifier.replaceChildren();
    const rows = [
      ...(state.metaParent ? [["Parent bounty", state.metaParent.parent_bounty_contract], ["Total child funding", "1 USDC including two verifier rewards"], ["Intended child solver", state.metaParent.intended_child_solver || "The agent must identify a different participant before funding"], ["Parent claim timing", "Both participant registrations and child terms must be confirmed at an earlier timestamp"], ["Gas", "Direct wallet calls require Base ETH; wallet shows the fee"]] : []),
      ["Engine", benchmark.engine || "Not supplied"],
      ["Source", source.repository || "Not supplied"],
      ["Commit", source.commit || "Not supplied"],
      ["Benchmark path", source.subdirectory || "Not supplied"],
      ["Runner schema", runner.schema_version || "Not supplied"],
      ["Container image", runner.image || "Not supplied"],
      ["Direct command", Array.isArray(runner.command) ? JSON.stringify(runner.command) : "Not supplied"],
      ["Working directory", runner.workdir || "Not supplied"],
      ["Benchmark digest", runner.benchmark_digest || "Not supplied"],
      ["Timeout (seconds)", runner.timeout_seconds ?? "Not supplied"],
      ["CPU limit (millicores)", runner.cpu_millis ?? "Not supplied"],
      ["Memory limit (bytes)", runner.memory_bytes ?? "Not supplied"],
      ["Process limit", runner.pids_limit ?? "Not supplied"],
      ["Output limit (bytes)", runner.max_output_bytes ?? "Not supplied"],
      ["Temporary storage (bytes)", runner.tmpfs_bytes ?? "Not supplied"],
      ["Source size limit (bytes)", runner.max_source_bytes ?? "Not supplied"],
      ["Source file limit", runner.max_source_files ?? "Not supplied"],
      ["Benchmark size limit (bytes)", runner.max_benchmark_bytes ?? "Not supplied"],
      ["Benchmark file limit", runner.max_benchmark_files ?? "Not supplied"],
      ["Platform", runner.platform || "Not supplied"],
      ["Test seed", runner.test_seed ?? "Not supplied"],
      ["Required evidence", Array.isArray(requiredEvidence) && requiredEvidence.length ? requiredEvidence.join(", ") : "None declared"],
    ];
    for (const [label, value] of rows) {
      const term = document.createElement("dt");
      term.textContent = label;
      const detail = document.createElement("dd");
      const code = document.createElement("code");
      code.textContent = String(value);
      detail.append(code);
      ui.verifier.append(term, detail);
    }
  }

  async function renderPreview() {
    if (!state.draft || state.fundingUsdc == null || !state.horizon || !state.taskWindowDays) return;
    currentRewardSplit();
    state.phase = "review";
    state.approved = false;
    state.imageReady = false;
    setProgress("review");
    renderCardText();
    renderMissionPlan();
    ui.approve.disabled = true;
    ui.approve.dataset.approved = "false";
    ui.approve.textContent = "Preparing image…";
    ui.fund.disabled = true;
    ui.preview.hidden = false;
    if (!state.handoffReview) {
      ui.preview.scrollIntoView({ behavior: "smooth", block: "start" });
    }
    setStatus("Rendering a bounded pre-publication illustration from the prepared draft. Nothing has been posted or funded.", "pending");
    if (state.handoffReview && state.bountyImage) {
      ui.approve.textContent = "Loading approved image…";
      setStatus(
        "Loading the exact image approved in your AI conversation. Agent Bounties will not generate or substitute an image.",
        "pending",
      );
    }
    await renderAiVisualForCurrentDraft();
    ui.approve.disabled = false;
    ui.approve.textContent = "Approve bounty card";
    setStatus("Review the result, completion checks, time window, reward, and creator-verification disclosure. Nothing has been posted or funded.");
    if (state.handoffReview) {
      ui.approve.textContent = "Confirm bounty";
      setStatus(state.bountyImage
        ? "Review the exact approved image, terms, reward, and verification disclosure. Nothing has been posted or funded."
        : "Review the terms, reward, content-derived visual, and verification disclosure. Nothing has been posted or funded.");
    }
    try { supportedVerificationPolicy(); }
    catch (error) {
      ui.approve.disabled = true;
      ui.fund.disabled = true;
      ui.approve.textContent = "Verification setup needed";
      ui.confidence.textContent = "Your AI needs to finish the verification setup";
      setStatus("Your draft is saved. Ask your AI to finish its executable verification setup before you approve funding.", "pending");
    }
  }

  function validHex(value) {
    return /^#[0-9a-fA-F]{6}$/.test(String(value || ""));
  }

  function validateVisualSpec(value) {
    if (!value || typeof value !== "object" || value.version !== 1 || !ALLOWED_SCENES.has(value.scene)) return null;
    const palette = safeTextList(value.palette, 5).filter(validHex);
    if (palette.length < 2) return null;
    const elements = (Array.isArray(value.elements) ? value.elements : []).slice(0, 10).map((element) => ({
      kind: String(element?.kind || ""),
      x: Number(element?.x),
      y: Number(element?.y),
      size: Number(element?.size),
    })).filter((element) => ALLOWED_ELEMENTS.has(element.kind)
      && Number.isInteger(element.x) && element.x >= 0 && element.x <= 100
      && Number.isInteger(element.y) && element.y >= 0 && element.y <= 100
      && Number.isFinite(element.size) && element.size >= 0.4 && element.size <= 2);
    if (elements.length < 3) return null;
    return { version: 1, scene: value.scene, palette, elements };
  }

  function extractVisualSpec(draft) {
    return validateVisualSpec(draft?.evidence_schema?.[VISUAL_EXTENSION]);
  }

  function fallbackVisualSpec(text) {
    const value = String(text || "").toLowerCase();
    const scene = /street|light|road|bridge|building|infrastructure/.test(value)
      ? "infrastructure"
      : /water|tree|climate|nature|environment/.test(value)
        ? "nature"
        : /health|medical|care|blood/.test(value)
          ? "health"
          : /school|learn|education|course|book/.test(value)
            ? "education"
            : /research|report|study|analysis|data/.test(value)
              ? "research"
              : /website|app|software|code|platform/.test(value)
                ? "digital"
                : "coordination";
    const elementsByScene = {
      infrastructure: ["road", "lamp", "lamp", "building", "check"],
      nature: ["water", "tree", "tree", "sun", "check"],
      health: ["person", "health", "chart", "check", "sun"].map((kind) => kind === "health" ? "check" : kind),
      education: ["book", "person", "screen", "check", "sun"],
      research: ["document", "chart", "document", "check", "network"],
      digital: ["screen", "network", "tool", "check", "document"],
      coordination: ["person", "network", "person", "tool", "check"],
    };
    return {
      version: 1,
      scene,
      palette: ["#06140d", "#1b5132", "#c9f548", "#7cefd1"],
      elements: elementsByScene[scene].map((kind, index) => ({ kind, x: 14 + index * 18, y: 62 - (index % 2) * 22, size: 0.8 + (index % 3) * 0.2 })),
    };
  }

  async function requestVisualSpecForTask(task) {
    const cached = state.visualCache.get(task.task_id);
    if (cached) return cached;
    try {
      const visualDraft = await requestJson(`${API}/v1/cloud-agent/bounty-drafts`, {
        method: "POST",
        body: JSON.stringify({
          objective: task.goal,
          context: draftContext([
            `Mission: ${state.missionPlan.title}`,
            `Selected task: ${task.title}`,
            `Fixed acceptance criteria: ${task.acceptance_criteria.join(" | ")}`,
            "Do not ask questions. Preserve the selected task. The only extra work is the bounded visual annotation.",
          ].join("\n")),
          constraints: task.acceptance_criteria,
          source_url: null,
          idempotency_key: `web-task-visual:${task.task_id}:${randomBytes32().slice(2)}`,
        }),
      });
      const spec = extractVisualSpec(visualDraft);
      if (spec) state.visualCache.set(task.task_id, spec);
      return spec;
    } catch (_error) {
      return null;
    }
  }

  async function renderAiVisualForCurrentDraft() {
    if (state.bountyImage) {
      ui.imageStatus.textContent = "Loading your approved ChatGPT image…";
      ui.imageStatus.dataset.tone = "ai";
      await renderApprovedChatgptImage(ui.art, state.bountyImage);
      state.visualSpec = null;
      state.visualSource = "chatgpt_user_generated";
      state.imageReady = true;
      ui.imageStatus.textContent = "Approved image from your ChatGPT conversation";
      return;
    }
    ui.imageStatus.textContent = "Preparing visual…";
    ui.imageStatus.dataset.tone = "";
    let spec = extractVisualSpec(state.draft);
    if (!spec && state.scope === "mission") {
      const task = selectedTask();
      if (task) spec = await requestVisualSpecForTask(task);
    }
    if (spec) {
      state.visualSpec = spec;
      state.visualSource = "ai";
      ui.imageStatus.textContent = "AI-prepared draft visual";
      ui.imageStatus.dataset.tone = "ai";
    } else {
      state.visualSpec = fallbackVisualSpec(`${state.draft.title} ${state.draft.goal}`);
      state.visualSource = "fallback";
      ui.imageStatus.textContent = "Bounded fallback visual";
      ui.imageStatus.dataset.tone = "";
    }
    renderVisual(ui.art, state.visualSpec);
    state.imageReady = true;
  }

  function normalizeBountyImage(value) {
    if (!value || typeof value !== "object") return null;
    const source = String(value.source || "");
    const prompt = String(value.prompt || "").trim();
    const altText = String(value.alt_text || "").trim();
    const assetUrl = String(value.asset_url || "").trim();
    const sha256 = String(value.sha256 || "");
    const mimeType = String(value.mime_type || "");
    let parsed;
    try {
      parsed = new URL(assetUrl);
    } catch (_error) {
      return null;
    }
    if (source !== "chatgpt_user_generated"
      || !prompt || prompt.length > 4000
      || !altText || altText.length > 500
      || parsed.protocol !== "https:"
      || !/^[0-9a-f]{64}$/.test(sha256)
      || !["image/png", "image/jpeg", "image/webp"].includes(mimeType)) return null;
    return {
      source,
      prompt,
      alt_text: altText,
      asset_url: parsed.toString(),
      sha256,
      mime_type: mimeType,
    };
  }

  async function renderApprovedChatgptImage(canvas, imageReference) {
    const image = new Image();
    image.crossOrigin = "anonymous";
    image.decoding = "async";
    image.alt = imageReference.alt_text;
    const loaded = new Promise((resolve, reject) => {
      image.addEventListener("load", resolve, { once: true });
      image.addEventListener("error", () => reject(new Error(
        "The approved ChatGPT image could not be loaded. Nothing was replaced or published."
      )), { once: true });
    });
    image.src = imageReference.asset_url;
    await loaded;
    canvas.width = 1200;
    canvas.height = 675;
    canvas.setAttribute("aria-label", imageReference.alt_text);
    const context = canvas.getContext("2d");
    const scale = Math.max(canvas.width / image.naturalWidth, canvas.height / image.naturalHeight);
    const width = image.naturalWidth * scale;
    const height = image.naturalHeight * scale;
    context.clearRect(0, 0, canvas.width, canvas.height);
    context.drawImage(
      image,
      (canvas.width - width) / 2,
      (canvas.height - height) / 2,
      width,
      height,
    );
  }

  function renderVisual(canvas, spec) {
    const width = 1200;
    const height = 675;
    canvas.width = width;
    canvas.height = height;
    const context = canvas.getContext("2d");
    const palette = spec.palette;
    const background = context.createLinearGradient(0, 0, width, height);
    background.addColorStop(0, palette[0]);
    background.addColorStop(1, palette[1]);
    context.fillStyle = background;
    context.fillRect(0, 0, width, height);

    const glow = context.createRadialGradient(width * .72, height * .2, 10, width * .72, height * .2, 420);
    glow.addColorStop(0, `${palette[2]}66`);
    glow.addColorStop(1, `${palette[2]}00`);
    context.fillStyle = glow;
    context.fillRect(0, 0, width, height);

    context.lineCap = "round";
    context.lineJoin = "round";
    for (const element of spec.elements) drawElement(context, element, palette, width, height);

    context.fillStyle = "rgba(2,11,8,.22)";
    context.fillRect(0, 0, width, height);
    context.fillStyle = "rgba(255,255,255,.92)";
    context.font = "750 28px system-ui, sans-serif";
    context.fillText("AI outcome visual", 52, 610);
    context.fillStyle = "rgba(255,255,255,.62)";
    context.font = "500 20px system-ui, sans-serif";
    context.fillText("Bounded pre-publication draft · not completion evidence", 52, 642);
  }

  function drawElement(context, element, palette, width, height) {
    const x = element.x / 100 * width;
    const y = element.y / 100 * height;
    const size = 70 * element.size;
    const accent = palette[2] || "#c9f548";
    const secondary = palette[3] || "#7cefd1";
    context.save();
    context.translate(x, y);
    context.strokeStyle = accent;
    context.fillStyle = secondary;
    context.lineWidth = Math.max(4, size * .09);
    switch (element.kind) {
      case "sun":
        context.fillStyle = accent;
        context.beginPath(); context.arc(0, 0, size * .45, 0, Math.PI * 2); context.fill();
        break;
      case "road":
        context.fillStyle = "rgba(1,8,5,.72)";
        context.beginPath(); context.moveTo(-size, size); context.lineTo(-size * .28, -size); context.lineTo(size * .28, -size); context.lineTo(size, size); context.closePath(); context.fill();
        context.strokeStyle = accent; context.setLineDash([size * .18, size * .18]); context.beginPath(); context.moveTo(0, size); context.lineTo(0, -size); context.stroke(); context.setLineDash([]);
        break;
      case "lamp":
        context.strokeStyle = secondary; context.beginPath(); context.moveTo(0, size); context.lineTo(0, -size * .55); context.quadraticCurveTo(0, -size, size * .45, -size); context.stroke();
        context.fillStyle = accent; context.beginPath(); context.arc(size * .45, -size, size * .16, 0, Math.PI * 2); context.fill();
        break;
      case "building":
        context.fillStyle = "rgba(255,255,255,.14)"; context.fillRect(-size * .65, -size, size * 1.3, size * 2);
        context.fillStyle = accent; for (let row = -1; row <= 1; row += 1) for (let column = -1; column <= 1; column += 1) context.fillRect(column * size * .32 - size * .1, row * size * .45 - size * .1, size * .18, size * .18);
        break;
      case "tree":
        context.strokeStyle = secondary; context.beginPath(); context.moveTo(0, size); context.lineTo(0, -size * .15); context.stroke();
        context.fillStyle = accent; context.beginPath(); context.arc(0, -size * .45, size * .55, 0, Math.PI * 2); context.fill();
        break;
      case "water":
        context.strokeStyle = secondary; for (let row = -1; row <= 1; row += 1) { context.beginPath(); context.moveTo(-size, row * size * .35); context.bezierCurveTo(-size * .5, row * size * .35 - size * .25, 0, row * size * .35 + size * .25, size, row * size * .35); context.stroke(); }
        break;
      case "screen":
        context.fillStyle = "rgba(1,8,5,.66)"; context.strokeStyle = secondary; context.roundRect(-size, -size * .65, size * 2, size * 1.3, size * .15); context.fill(); context.stroke();
        context.fillStyle = accent; context.fillRect(-size * .7, -size * .3, size * 1.1, size * .13); context.fillRect(-size * .7, 0, size * 1.4, size * .13);
        break;
      case "document":
        context.fillStyle = "rgba(246,248,232,.86)"; context.roundRect(-size * .65, -size, size * 1.3, size * 2, size * .12); context.fill();
        context.fillStyle = palette[1]; for (let row = 0; row < 4; row += 1) context.fillRect(-size * .42, -size * .55 + row * size * .38, size * (.8 - row * .08), size * .1);
        break;
      case "network":
        context.strokeStyle = secondary; for (let index = 0; index < 5; index += 1) { const angle = index / 5 * Math.PI * 2; const px = Math.cos(angle) * size * .75; const py = Math.sin(angle) * size * .75; context.beginPath(); context.moveTo(0,0); context.lineTo(px,py); context.stroke(); context.fillStyle = accent; context.beginPath(); context.arc(px,py,size*.12,0,Math.PI*2); context.fill(); } context.beginPath(); context.arc(0,0,size*.18,0,Math.PI*2); context.fill();
        break;
      case "tool":
        context.strokeStyle = accent; context.lineWidth = size * .18; context.beginPath(); context.moveTo(-size * .65, size * .65); context.lineTo(size * .45, -size * .45); context.stroke(); context.beginPath(); context.arc(size * .55, -size * .55, size * .35, Math.PI * .2, Math.PI * 1.3); context.stroke();
        break;
      case "person":
        context.fillStyle = accent; context.beginPath(); context.arc(0, -size * .55, size * .24, 0, Math.PI * 2); context.fill(); context.strokeStyle = secondary; context.beginPath(); context.moveTo(0,-size*.25); context.lineTo(0,size*.5); context.moveTo(0,0); context.lineTo(-size*.5,size*.2); context.moveTo(0,0); context.lineTo(size*.5,size*.2); context.moveTo(0,size*.5); context.lineTo(-size*.4,size); context.moveTo(0,size*.5); context.lineTo(size*.4,size); context.stroke();
        break;
      case "check":
        context.fillStyle = accent; context.beginPath(); context.arc(0,0,size*.8,0,Math.PI*2); context.fill(); context.strokeStyle = palette[0]; context.lineWidth = size*.18; context.beginPath(); context.moveTo(-size*.35,0); context.lineTo(-size*.05,size*.3); context.lineTo(size*.45,-size*.35); context.stroke();
        break;
      case "bridge":
        context.strokeStyle = secondary; context.beginPath(); context.moveTo(-size, size * .45); context.lineTo(size, size * .45); context.moveTo(-size*.75,size*.45); context.quadraticCurveTo(0,-size*.8,size*.75,size*.45); context.stroke();
        break;
      case "book":
        context.fillStyle = "rgba(246,248,232,.84)"; context.beginPath(); context.moveTo(0,size*.7); context.quadraticCurveTo(-size*.45,size*.25,-size,-size*.6); context.lineTo(-size,-size*.9); context.quadraticCurveTo(-size*.35,-size*.5,0,0); context.quadraticCurveTo(size*.35,-size*.5,size,-size*.9); context.lineTo(size,-size*.6); context.quadraticCurveTo(size*.45,size*.25,0,size*.7); context.fill(); context.strokeStyle = accent; context.beginPath(); context.moveTo(0,0); context.lineTo(0,size*.7); context.stroke();
        break;
      case "chart":
        context.strokeStyle = secondary; context.beginPath(); context.moveTo(-size,-size); context.lineTo(-size,size); context.lineTo(size,size); context.stroke(); context.fillStyle = accent; for (let index=0; index<4; index+=1) context.fillRect(-size*.7+index*size*.45,size*.65-(index+1)*size*.32,size*.25,(index+1)*size*.32);
        break;
      default:
        break;
    }
    context.restore();
  }

  function stripVisualExtension(schema) {
    const clone = JSON.parse(JSON.stringify(schema || { type: "object", additionalProperties: true }));
    if (clone && typeof clone === "object") delete clone[VISUAL_EXTENSION];
    return clone;
  }

  function canonicalJsonValue(value) {
    if (Array.isArray(value)) return value.map(canonicalJsonValue);
    if (value && typeof value === "object") {
      return Object.keys(value).sort().reduce((result, key) => {
        result[key] = canonicalJsonValue(value[key]);
        return result;
      }, {});
    }
    return value;
  }

  function missionBenchmark(base) {
    const benchmark = JSON.parse(JSON.stringify(base || {}));
    if (state.scope === "mission" && state.missionPlan) {
      benchmark.x_agent_bounties_mission = {
        title: state.missionPlan.title,
        success_definition: state.missionPlan.success_definition,
        horizon: state.horizon.label,
        selected_task_id: state.selectedTaskId,
        task_window_days: state.taskWindowDays,
        planned_task_ids: state.missionPlan.tasks.map((task) => task.task_id),
      };
    }
    return benchmark;
  }

  function verificationReadiness(benchmark, evidenceSchema) {
    const source = benchmark?.source;
    const runner = benchmark?.runner_manifest;
    const sourceParts = typeof source?.subdirectory === "string"
      ? source.subdirectory.split("/")
      : [];
    const sourceReady = source?.kind === "github_commit"
      && /^[A-Za-z0-9._-]+\/[A-Za-z0-9._-]+$/.test(source.repository || "")
      && /^[0-9a-f]{40}$/.test(source.commit || "")
      && typeof source.subdirectory === "string"
      && source.subdirectory.length > 0
      && !source.subdirectory.startsWith("/")
      && !source.subdirectory.endsWith("/")
      && !source.subdirectory.includes("\\")
      && sourceParts.every((part) => part && part !== "." && part !== "..");
    const command = runner?.command;
    const executable = Array.isArray(command) && typeof command[0] === "string"
      ? command[0].split(/[\\/]/).pop().toLowerCase()
      : "";
    const commandReady = Array.isArray(command)
      && command.length >= 1
      && command.length <= 64
      && command.every((argument) => typeof argument === "string"
        && argument.length >= 1
        && argument.length <= 4_096
        && !/[\0\r\n]/.test(argument))
      && command.reduce((total, argument) => total + argument.length, 0) <= 16_384
      && !new Set(["sh", "bash", "dash", "zsh", "cmd", "cmd.exe", "powershell", "pwsh"]).has(executable);
    const boundsReady = Object.entries(RUNNER_BOUNDS).every(([field, [minimum, maximum]]) =>
      Number.isSafeInteger(runner?.[field])
      && runner[field] >= minimum
      && runner[field] <= maximum);
    const image = String(runner?.image || "");
    const imageParts = image.split("@sha256:");
    const imageName = imageParts[0] || "";
    const imageReady = imageParts.length === 2
      && !image.startsWith("-")
      && (image.match(/@/g) || []).length === 1
      && imageName.length > 0
      && /^[a-z0-9./:_-]+$/.test(imageName)
      && !imageName.includes("..")
      && /^[0-9a-f]{64}$/.test(imageParts[1]);
    const runnerReady = runner && Object.keys(runner).length === RUNNER_MANIFEST_FIELDS.length
      && RUNNER_MANIFEST_FIELDS.every((field) => Object.hasOwn(runner, field))
      && runner.schema_version === "agent-bounties/regression-sandbox-v1"
      && imageReady
      && commandReady
      && runner.workdir === "/workspace"
      && /^sha256:[0-9a-f]{64}$/.test(runner.benchmark_digest || "")
      && boundsReady
      && runner.tmpfs_bytes <= runner.memory_bytes
      && new Set(["linux/amd64", "linux/arm64"]).has(runner.platform);
    const approvedSubdirectory = RECONCILED_REGRESSION_BENCHMARK_SOURCES.get(runner?.benchmark_digest);
    const approvedSource = typeof approvedSubdirectory === "string"
      && String(source?.repository || "").toLowerCase() === "nspg13/agent-bounties"
      && (String(source?.commit || "").toLowerCase() === RECONCILED_REGRESSION_BENCHMARK_COMMIT
        || String(source?.commit || "").toLowerCase() === "aa28ec742efd4063260653510ba324e291267515"
          && approvedSubdirectory === "benchmarks/direct-growth-v2/openhands-integration")
      && source?.subdirectory === approvedSubdirectory;
    const blocked = !RECONCILED_REGRESSION_BENCHMARK_DIGESTS.has(runner?.benchmark_digest)
      || !approvedSource;
    const requiredEvidence = evidenceSchema?.required;
    const sourceSnapshotDigest = evidenceSchema?.properties?.source_snapshot_digest;
    const evidenceReady = evidenceSchema?.type === "object"
      && Array.isArray(requiredEvidence)
      && requiredEvidence.includes("source_snapshot_digest")
      && sourceSnapshotDigest?.type === "string"
      && sourceSnapshotDigest.pattern === "^sha256:[0-9a-f]{64}$";
    return {
      blocked,
      executable: benchmark?.engine === REGRESSION_ENGINE
        && sourceReady
        && runnerReady
        && evidenceReady
        && !blocked,
    };
  }

  function supportedVerificationPolicy() {
    if (state.reviewStale) throw new Error("The saved brief changed. Update the proposal from its current outcome, budget and deadline before approval.");
    const benchmark = missionBenchmark(state.draft?.benchmark || {});
    if (benchmark.engine === "creator_review_v1") {
      if (state.metaParent || !window.AgentBountiesCreatorReview.ready(benchmark, state.draft?.evidence_schema)) throw new Error("Creator review needs a future agreed deadline and public artifact evidence. It cannot be used for a qualifying meta child.");
      return window.AgentBountiesCreatorReview.policy(state.account);
    }
    if (state.deliveryDeadline) throw new Error("This automated benchmark does not enforce the requested calendar deadline. Propose creator review or agree a relative work window; do not silently change the deadline.");
    const readiness = verificationReadiness(benchmark, state.draft?.evidence_schema);
    if (readiness.blocked) {
      throw new Error(
        "This exact benchmark digest cannot be funded until it is independently reconciled and approved. Choose a reviewed benchmark.",
      );
    }
    if (!readiness.executable) {
      throw new Error(
        "This draft has no executable verifier, so it cannot be funded. Add the exact public benchmark source, complete sandbox runner manifest, and required source_snapshot_digest evidence schema, then retry.",
      );
    }
    if (state.metaParent) {
      currentRewardSplit();
      if (!state.metaParent.intended_child_solver) throw new Error("The agent must identify a distinct child solver and preserve both pre-claim participant registrations before funding this child.");
    }
    return {
      mechanism: "signed_quorum",
      engine: REGRESSION_ENGINE,
      verifier_module: null,
      verifier_reward_recipient: null,
      verifiers: state.metaParent ? metaChild.VERIFIERS : REGRESSION_VERIFIERS,
      threshold: state.metaParent ? 2 : REGRESSION_VERIFIERS.length,
      ai_provider: null,
      ai_model: null,
      ai_model_version: null,
      system_prompt: null,
      rubric: "The pinned verifier agent runs the precommitted sandboxed regression and requires every acceptance criterion to pass.",
      decoding_parameters: {},
      public_disclosure: "The precommitted verifier quorum runs the exact coding benchmark and must produce the required matching signatures.",
    };
  }

  function wrapCanvasText(context, text, maxWidth, maxLines) {
    const words = String(text || "").split(/\s+/).filter(Boolean);
    const lines = [];
    let line = "";
    for (const word of words) {
      const next = line ? `${line} ${word}` : word;
      if (context.measureText(next).width > maxWidth && line) {
        lines.push(line);
        line = word;
        if (lines.length >= maxLines) break;
      } else line = next;
    }
    if (line && lines.length < maxLines) lines.push(line);
    if (lines.length === maxLines && lines.join(" ").split(/\s+/).length < words.length) lines[lines.length - 1] = `${lines[lines.length - 1].replace(/[. ]+$/, "")}…`;
    return lines;
  }

  async function shareBountyCard() {
    if (!state.draft || !state.imageReady) return;
    ui.share.disabled = true;
    try {
      const canvas = document.createElement("canvas");
      canvas.width = 1200;
      canvas.height = 1500;
      const context = canvas.getContext("2d");
      context.fillStyle = "#03110b"; context.fillRect(0,0,1200,1500);
      context.drawImage(ui.art, 0, 0, 1200, 675);
      const gradient = context.createLinearGradient(0,620,0,1500); gradient.addColorStop(0,"rgba(6,31,18,.95)"); gradient.addColorStop(1,"#020b08"); context.fillStyle = gradient; context.fillRect(0,610,1200,890);
      context.fillStyle = "#c9f548"; context.font = "800 29px system-ui"; context.fillText(state.scope === "mission" ? "AGENT BOUNTIES · MISSION TASK" : "AGENT BOUNTIES · BOUNTY CARD", 68, 710);
      context.fillStyle = "#f4f6ef"; context.font = "850 68px system-ui";
      let y = 806; for (const line of wrapCanvasText(context,state.draft.title,1060,3)) { context.fillText(line,68,y); y += 76; }
      context.fillStyle = "#bdc8c0"; context.font = "440 31px system-ui"; y += 10; for (const line of wrapCanvasText(context,state.draft.goal,1060,5)) { context.fillText(line,68,y); y += 43; }
      const statsY = Math.max(y + 32, 1180);
      const stats = [["TOTAL REWARD",`${formatUsdc(state.fundingUsdc)} USDC`],["TASK WINDOW",`${state.taskWindowDays} days`],["MISSION HORIZON",state.scope === "mission" ? state.horizon.label : "Single result"]];
      stats.forEach(([label,value],index) => { const x=68+index*355; context.fillStyle="rgba(201,245,72,.12)"; context.fillRect(x,statsY,325,126); context.fillStyle="#8d9a91"; context.font="800 19px system-ui"; context.fillText(label,x+20,statsY+36); context.fillStyle="#f4f6ef"; context.font="750 27px system-ui"; let lineY=statsY+76; for(const line of wrapCanvasText(context,value,285,2)){context.fillText(line,x+20,lineY);lineY+=31;} });
      context.fillStyle="#e8c15a"; context.font="650 23px system-ui"; context.fillText("DRAFT · NOT POSTED OR FUNDED · CREATOR VERIFIES COMPLETION",68,1435);
      const blob = await new Promise((resolve) => canvas.toBlob(resolve,"image/png"));
      if (!blob) throw new Error("The share image could not be created.");
      const file = new File([blob],"agent-bounties-card.png",{type:"image/png"});
      const data = { title: state.draft.title, text: `${state.draft.title} — ${formatUsdc(state.fundingUsdc)} USDC bounty draft on Agent Bounties.`, ...(state.bountyContract ? {url:`https://agentbounties.app/earn.html?bountyContract=${encodeURIComponent(state.bountyContract)}`} : {}) };
      if (navigator.canShare && navigator.canShare({files:[file]})) await navigator.share({...data,files:[file]});
      else { const url=URL.createObjectURL(file); const link=document.createElement("a"); link.href=url; link.download=file.name; link.click(); setTimeout(()=>URL.revokeObjectURL(url),1500); if(state.bountyContract&&navigator.clipboard) await navigator.clipboard.writeText(data.url); setStatus(state.bountyContract?"Card downloaded and the funded bounty link was copied.":"Draft card downloaded. It makes no funding claim.","success"); }
    } catch (error) { if (error?.name !== "AbortError") setStatus(error.message || String(error),"error"); }
    finally { ui.share.disabled=false; }
  }

  function approveCard() {
    if (!state.imageReady) return;
    try { supportedVerificationPolicy(); } catch (error) { setStatus(error.message, "error"); return; }
    state.approved = true;
    ui.approve.dataset.approved = "true";
    ui.approve.textContent = state.handoffReview ? "Confirmed ✓" : "Approved ✓";
    ui.fund.disabled = false;
    // The card confirmation leads straight to wallet review; no duplicate chat approval.
    openFunding().catch((error) => setPaymentStatus(error.message, "error"));
    setProgress("fund");
    setStatus(state.bountyImage
      ? "Card approved with the exact supplied image. Funding still requires a separate wallet review and signature."
      : "Card approved with a deterministic content-derived visual. Funding still requires a separate wallet review and signature.", "success");
  }

  function reviseCard() {
    state.approved = false;
    ui.approve.dataset.approved = "false";
    ui.approve.textContent = "Approve bounty card";
    ui.fund.disabled = true;
    const savedGoal = ui.input.value;
    setComposer({ phase:"revise", prompt:"Edit your brief, then continue with your AI in this conversation.", label:"What do you want delivered?", placeholder:"Describe the result you need.", button:"Save brief", hint:"Your AI uses the saved brief to update the proposal." });
    ui.input.value = savedGoal || state.draft?.goal || state.originalRequest;
    ui.form.scrollIntoView({behavior:"smooth",block:"start"});
    ui.input.focus();
    setStatus("Edit the saved brief above and ask your AI to update the proposal in your current conversation.", "pending");
  }

  async function openFunding() {
    if (!state.approved) return;
    const attribution = state.distributionAttribution;
    if (attribution) {
      ui.fund.disabled = true;
      setStatus("Recording the attributed wallet-review boundary…", "pending");
      try {
        const acknowledgement = await requestJson(`${API}/v1/distribution/handoffs/wallet-review`, {
          method: "POST",
          headers: {
            "x-agent-bounties-acquisition-id": attribution.acquisition,
            "x-agent-bounties-handoff-id": attribution.handoff,
          },
        });
        if (acknowledgement?.schema_version !== "agent-bounties/distribution-wallet-review-v1"
          || acknowledgement.recorded !== true) {
          throw new Error("The attributed wallet-review boundary was not durably acknowledged.");
        }
        if (!state.approved || state.distributionAttribution !== attribution) {
          throw new Error("The bounty changed before wallet review. Review and approve it again.");
        }
      } catch (error) {
        ui.fund.disabled = !state.approved;
        setStatus(`${error.message || String(error)} No wallet interface was opened.`, "error");
        return;
      }
      ui.fund.disabled = false;
    }
    const onrampUrl = new URL("onramp.html", window.location.href);
    onrampUrl.searchParams.set("purpose", "post");
    onrampUrl.searchParams.set("amount", formatUsdc(state.fundingUsdc));
    onrampUrl.searchParams.set("return", window.location.href);
    for (const link of ui.onramps) link.href = onrampUrl.href;
    ui.dialog.showModal();
    setPaymentStatus("Choose a detected wallet or open the Base USDC handoff in a separate tab.");
    ui.walletPanel.hidden = false;
    ui.readiness.hidden = true;
    ui.fundNow.disabled = true;
    chooseCryptoWallet().catch((error) => setPaymentStatus(error.message || String(error), "error"));
  }

  function providerName(item) {
    if (item.info?.name) return item.info.name;
    if (item.provider.isMetaMask) return "MetaMask";
    if (item.provider.isCoinbaseWallet) return "Coinbase Wallet";
    if (item.provider.isBraveWallet) return "Brave Wallet";
    return "Browser wallet";
  }

  async function discoverWallets() {
    window.dispatchEvent(new Event("eip6963:requestProvider"));
    await new Promise((resolve) => setTimeout(resolve,350));
    const candidates=[...announcedProviders];
    const injected=window.ethereum&&Array.isArray(window.ethereum.providers)?window.ethereum.providers:(window.ethereum?[window.ethereum]:[]);
    for(const provider of injected) if(provider&&typeof provider.request==="function"&&!candidates.some((item)=>item.provider===provider)) candidates.push({provider,info:{}});
    state.providers=candidates;
    return candidates;
  }

  async function chooseCryptoWallet() {
    ui.cryptoMethod.dataset.active="true";
    ui.walletPanel.hidden=false;
    ui.walletOptions.textContent="";
    ui.walletMessage.textContent="Looking for wallets on this device…";
    const providers=await discoverWallets();
    if(!providers.length){ui.walletMessage.textContent="No compatible browser wallet was detected. Use the Base USDC handoff below to create or fund a wallet, then reopen this review in a browser where that wallet can sign. Never enter a recovery phrase on this website.";track("wallet_missing_detected");return;}
    ui.walletMessage.textContent=providers.length===1?"One wallet is available.":`${providers.length} wallets are available. Choose which one to use.`;
    for(const item of providers){const button=document.createElement("button");button.type="button";button.className="wallet-option";const name=document.createElement("strong");name.textContent=providerName(item);const note=document.createElement("small");note.textContent="Connect and check Base USDC";button.append(name,note);button.addEventListener("click",()=>connectWallet(item));ui.walletOptions.append(button);}
  }

  async function loadProtocol() {
    if(state.protocol)return state.protocol;
    const response=await fetch("protocol.json",{cache:"no-store"});
    if(!response.ok)throw new Error("Protocol configuration is unavailable.");
    const protocol=await response.json();
    if(protocol.status!=="active"||!/^0x[0-9a-fA-F]{40}$/.test(protocol.factory||""))throw new Error("The Base protocol is not active. No transaction was requested.");
    state.protocol=protocol;return protocol;
  }

  async function switchToBase(provider,protocol){const current=await provider.request({method:"eth_chainId"});if(String(current).toLowerCase()===String(protocol.chain_id_hex).toLowerCase())return;try{await provider.request({method:"wallet_switchEthereumChain",params:[{chainId:protocol.chain_id_hex}]});}catch(error){if(error&&error.code===4902){await provider.request({method:"wallet_addEthereumChain",params:[{chainId:protocol.chain_id_hex,chainName:"Base",nativeCurrency:{name:"Ether",symbol:"ETH",decimals:18},rpcUrls:["https://mainnet.base.org"],blockExplorerUrls:[protocol.explorer_url]}]});}else throw error;}}

  async function connectWallet(item){state.provider=item.provider;setPaymentStatus(`Connecting ${providerName(item)}…`,"pending");try{const protocol=await loadProtocol();const accounts=await state.provider.request({method:"eth_requestAccounts"});if(!accounts||!accounts[0])throw new Error("The wallet did not return an account.");state.account=accounts[0];track("wallet_connected");await switchToBase(state.provider,protocol);await refreshWalletReadiness();}catch(error){setPaymentStatus(error.message||String(error),"error");}}

  function addressWord(address){return String(address).toLowerCase().replace(/^0x/,"").padStart(64,"0");}

  async function refreshWalletReadiness(){if(!state.provider||!state.account)return;const protocol=await loadProtocol();const required=usdcBaseUnits(state.fundingUsdc);setPaymentStatus("Checking Base USDC and gas readiness…","pending");const[usdcRaw,ethRaw]=await Promise.all([state.provider.request({method:"eth_call",params:[{to:protocol.native_usdc,data:`0x70a08231${addressWord(state.account)}`},"latest"]}),state.provider.request({method:"eth_getBalance",params:[state.account,"latest"]})]);const usdc=BigInt(usdcRaw||"0x0");const eth=BigInt(ethRaw||"0x0");state.balances={usdc,eth,required};const usdcReady=usdc>=required;const ethReady=eth>0n;const observed=usdcReady?"funded":"unfunded";if(state.observedWalletState!==observed){state.observedWalletState=observed;track(usdcReady?"wallet_funded_observed":"wallet_unfunded_detected");}ui.account.textContent=`${state.account.slice(0,8)}…${state.account.slice(-6)}`;ui.usdcBalance.textContent=`${formatUsdc(Number(usdc)/1_000_000)} USDC`;ui.ethBalance.textContent=`${(Number(eth)/1e18).toFixed(6)} ETH`;ui.requiredUsdc.textContent=`${formatUsdc(state.fundingUsdc)} USDC`;ui.readiness.hidden=false;ui.fundingHelp.hidden=usdcReady&&ethReady;const missing=required>usdc?required-usdc:0n;ui.missingUsdc.textContent=`${formatUsdc(Number(missing)/1_000_000)} USDC`;ui.fundNow.disabled=!(usdcReady&&ethReady);if(usdcReady&&ethReady)setPaymentStatus("Wallet ready. Review the exact amount, legal terms, and wallet request before signing.","success");else if(!usdcReady&&!ethReady)setPaymentStatus("This wallet needs more Base USDC and a small amount of Base ETH for gas.","error");else if(!usdcReady)setPaymentStatus("This wallet does not yet hold enough USDC on Base.","error");else setPaymentStatus("The USDC is available, but the wallet needs a small amount of Base ETH for gas.","error");}

  async function watchUsdcAsset(){try{const protocol=await loadProtocol();await state.provider.request({method:"wallet_watchAsset",params:{type:"ERC20",options:{address:protocol.native_usdc,symbol:"USDC",decimals:6}}});setPaymentStatus("Base USDC was offered to the wallet. This does not buy or transfer tokens.","success");}catch(error){setPaymentStatus(error.message||String(error),"error");}}
  async function copyUsdcAddress(){const protocol=await loadProtocol();await navigator.clipboard.writeText(protocol.native_usdc);setPaymentStatus("Base USDC contract address copied. Verify the network and address inside your wallet before acquiring tokens.","success");}

  function signatureParts(signature){const value=String(signature).replace(/^0x/,"");if(value.length!==130)throw new Error("The wallet returned an invalid signature.");return{r:`0x${value.slice(0,64)}`,s:`0x${value.slice(64,128)}`,v:Number.parseInt(value.slice(128,130),16)};}
  async function sendTransaction(transaction){if(!transaction||!transaction.to||!transaction.data||Number(transaction.value_wei||0)!==0)throw new Error("The planned transaction is invalid.");postingJournal.checkpoint("sending");const hash=await state.provider.request({method:"eth_sendTransaction",params:[{from:state.account,to:transaction.to,data:transaction.data,value:"0x0"}]});if(!/^0x[0-9a-fA-F]{64}$/.test(hash))throw new Error("The wallet response is uncertain. Check the recorded posting before retrying.");postingJournal.checkpoint("submitted",hash);return hash;}
  async function waitReceipt(hash,timeoutMs=150000){const started=Date.now();while(Date.now()-started<timeoutMs){const receipt=await state.provider.request({method:"eth_getTransactionReceipt",params:[hash]});if(receipt){if(receipt.status!=="0x1")throw new Error(`The Base transaction reverted: ${hash}`);return receipt;}await new Promise((resolve)=>setTimeout(resolve,1600));}throw new Error("The transaction is still pending. Check the wallet or Base explorer before trying again.");}
  async function isContractAccount(){const code=await state.provider.request({method:"eth_getCode",params:[state.account,"latest"]});return code&&code!=="0x"&&code!=="0x0";}
  async function sendWalletCalls(calls,protocol){
    postingJournal.checkpoint("sending");
    try {
      const batch=await state.provider.request({method:"wallet_sendCalls",params:[{version:"2.0.0",chainId:protocol.chain_id_hex,from:state.account,calls:calls.map((call)=>({to:call.to,data:call.data,value:"0x0"}))}]});
      postingJournal.checkpoint("batch_submitted",batch);return batch;
    } catch(error) {
      // Only explicit lack of method support permits a fallback. A lost reply can conceal a submitted batch.
      if(![-32601,4200].includes(error.code))throw error;
      postingJournal.checkpoint("prepared");
      let last=null;for(const call of calls){last=await sendTransaction(call);await waitReceipt(last);}return last;
    }
  }

  function contractTerms(protocol,rewards){const now=Math.floor(Date.now()/1000);return{protocol_version:protocol.protocol_version,creator_wallet:state.account,network:protocol.network,settlement_token:protocol.native_usdc,solver_reward:{amount:Number(rewards.solver),currency:"usdc"},verifier_reward:{amount:Number(rewards.verifier),currency:"usdc"},claim_bond:{amount:Number(rewards.verifier),currency:"usdc"},initial_funding:{amount:Number(rewards.total),currency:"usdc"},funding_deadline:state.deliveryDeadline?Math.floor(Date.parse(state.deliveryDeadline)/1000):now+30*86400,claim_window_seconds:state.taskWindowDays*86400,verification_window_seconds:48*3600,creation_nonce:randomBytes32()};}

  function termsDocument(committed){const document={schema_version:"agent-bounties/terms-v1",contract_terms:committed,title:state.draft.title,goal:state.draft.goal,acceptance_criteria:state.draft.acceptance_criteria,benchmark:canonicalJsonValue(missionBenchmark(state.draft.benchmark||{})),evidence_schema:canonicalJsonValue(stripVisualExtension(state.draft.evidence_schema)),verification_policy:supportedVerificationPolicy(),source_url:null,discovery_source:state.bountyImage?"ai_assistant_approved_image_handoff":"ai_assistant_terms_handoff"};if(state.bountyImage)document.image=canonicalJsonValue(state.bountyImage);return document;}

  function createPayload(terms,committed){return{creator:state.account,solver_reward:committed.solver_reward,verifier_reward:committed.verifier_reward,terms_hash:terms.terms_hash,policy_hash:terms.policy_hash,acceptance_criteria_hash:terms.acceptance_criteria_hash,benchmark_hash:terms.benchmark_hash,evidence_schema_hash:terms.evidence_schema_hash,funding_deadline:committed.funding_deadline,claim_window_seconds:committed.claim_window_seconds,verification_window_seconds:committed.verification_window_seconds,verification_mode:"signed_quorum",verifier_module:null,verifier_reward_recipient:null,verifiers:supportedVerificationPolicy().verifiers,threshold:supportedVerificationPolicy().threshold,initial_funding:committed.initial_funding,creation_nonce:committed.creation_nonce};}

  function validateCreationPlan(plan,protocol,create){if(!plan||!/^0x[0-9a-fA-F]{40}$/.test(plan.predicted_bounty_contract||""))throw new Error("The creation plan did not return a valid bounty address.");if(Number(plan.network&&plan.network.chain_id)!==Number(protocol.chain_id))throw new Error("The creation plan targets the wrong network.");if(String(plan.factory_contract||"").toLowerCase()!==String(protocol.factory).toLowerCase())throw new Error("The creation plan does not use the canonical factory.");const target=Number(create.solver_reward.amount)+Number(create.verifier_reward.amount);if(Number(create.initial_funding.amount)!==target)throw new Error("The creation plan is not fully funded.");}

  async function pollCreation(api,bountyId,timeoutMs=100000){const started=Date.now();while(Date.now()-started<timeoutMs){const events=await requestJson(`${api}/v1/base/autonomous-bounties/events?network=base-mainnet&bounty_id=${encodeURIComponent(bountyId)}`,{cache:"no-store"});const created=events.some((event)=>event.kind==="canonical_bounty_created");const funded=events.some((event)=>event.kind==="funding_added");const claimable=events.some((event)=>event.kind==="bounty_became_claimable");if(created&&funded&&claimable)return events;await new Promise((resolve)=>setTimeout(resolve,2500));}return null;}

  async function fetchFeedItem(api,contract){try{const items=await requestJson(`${api}/v1/base/autonomous-bounties/feed?network=base-mainnet&claimable_only=false`,{cache:"no-store"});return items.find((item)=>String(item.bounty_contract).toLowerCase()===String(contract).toLowerCase())||null;}catch(_error){return null;}}

  async function fundApprovedBounty() {
    if (postingBusy) return;
    if (postingJournal.load()) {
      setPaymentStatus("A posting wallet step is already recorded. Your AI can check its canonical status; do not create or fund it again.", "pending");
      return;
    }
    if (!state.approved || !state.provider || !state.account || !state.balances) return;
    postingBusy = true;
    for (const field of ui.form.querySelectorAll("input, textarea, button")) field.disabled = true;
    const approvedDraft = state.draft;
    const approvedAccount = state.account;
    track("canonical_post_started");
    ui.fundNow.disabled = true;
    setPaymentStatus("Preparing the exact canonical Base USDC funding request…", "pending");
    try {
      await refreshWalletReadiness();
      if (state.balances.usdc < state.balances.required || state.balances.eth === 0n) throw new Error("The wallet is not ready to fund this bounty.");
      if (!window.AgentBountiesLegal) throw new Error("The legal agreement could not be loaded. Reload before using the wallet.");
      const agreement = await window.AgentBountiesLegal.requireAcceptance({ action: "post_bounty", walletAddress: state.account, scope: ui.dialog });
      if (!agreement.durable) throw new Error("The agreement could not be recorded. Retry when the service is available; no transaction was sent.");
      const protocol = await loadProtocol();
      const api = String(protocol.api_base_url).replace(/\/$/, "");
      const rewards = currentRewardSplit();
      let create, plan, childPlan = null;
      if (state.metaParent) {
        const parent = await metaChild.resolve(state.metaParent, window.AgentBountiesWorkflow.createClient(window));
        if (parent.terms_hash !== state.metaParent.terms_hash) throw new Error("The parent terms changed. Review the child again.");
        const input = metaChild.request(state.draft, parent, state.account, rewards, state.taskWindowDays, randomBytes32());
        const retryKey = "agent-bounties:meta-child-preparation:v1";
        const fingerprint = metaChild.canonical({ ...input, creation_nonce: null });
        const previous = JSON.parse(window.sessionStorage.getItem(retryKey) || "null");
        if (previous?.fingerprint === fingerprint) input.creation_nonce = previous.nonce;
        else window.sessionStorage.setItem(retryKey, JSON.stringify({ fingerprint, nonce: input.creation_nonce }));
        // This endpoint publishes hosted terms. It is invoked only after the
        // person's trusted funding action and durable publication consent above.
        childPlan = await requestJson(`${api}/v1/base/autonomous-bounties/standing-meta-v2-child-preparation`, {
          method: "POST", body: JSON.stringify(input),
        });
        metaChild.validatePlan(childPlan, input, rewards, window.AgentBountiesEvm);
        if (childPlan.hosted_terms_published !== true) throw new Error("The exact child terms were not published; no wallet action was sent.");
        create = childPlan.child_create;
        plan = childPlan.child_creation;
      } else {
        const committed = contractTerms(protocol, rewards);
        const document = termsDocument(committed);
        const terms = await requestJson(`${api}/v1/base/autonomous-bounties/terms`, {
          method: "POST",
          headers: state.distributionAttribution ? {
            "x-agent-bounties-acquisition-id": state.distributionAttribution.acquisition,
            "x-agent-bounties-handoff-id": state.distributionAttribution.handoff,
          } : {},
          body: JSON.stringify({ creator_wallet: state.account, document }),
        });
        create = createPayload(terms, committed);
        plan = await requestJson(`${api}/v1/base/autonomous-bounties/creation-plan`, {
          method: "POST", body: JSON.stringify({ network: "base-mainnet", create }),
        });
      }
      validateCreationPlan(plan, protocol, create);
      if (!state.approved || state.draft !== approvedDraft) throw new Error("The bounty changed during preparation. Review the revised commitment before funding.");
      if (state.account !== approvedAccount || String((await state.provider.request({ method: "eth_accounts" }))[0]).toLowerCase() !== String(approvedAccount).toLowerCase() || String(await state.provider.request({ method: "eth_chainId" })).toLowerCase() !== "0x2105") throw new Error("The wallet or network changed. Reopen the same review before funding.");
      postingJournal.prepare(plan);
      setPaymentStatus([
        "Review the wallet request carefully.",
        `Exact total funding: ${formatUsdc(state.fundingUsdc)} Base USDC.`,
        `Solver reward: ${formatUsdc(Number(rewards.solver) / 1_000_000)} Base USDC.`,
        `Verifier reward and solver bond: ${formatUsdc(Number(rewards.verifier) / 1_000_000)} Base USDC.`,
        `Predicted bounty: ${plan.predicted_bounty_contract}`,
        "A signature or transaction hash is not funding evidence.",
      ].join("\n"), "pending");
      let transactionHash = null;
      if (childPlan) {
        setPaymentStatus("Create the reviewed 1 USDC child: publish its exact on-chain terms, approve only 1 USDC, then create and fully fund it. Parent claiming is a later step. These direct wallet calls require Base ETH for gas.", "pending");
        await sendWalletCalls(childPlan.pre_claim_wallet_calls, protocol);
      } else if (!(await isContractAccount()) && plan.eip3009_authorization) {
        postingJournal.checkpoint("signing");
        const signature = await state.provider.request({ method: "eth_signTypedData_v4", params: [state.account, JSON.stringify(plan.eip3009_authorization)] });
        postingJournal.checkpoint("authorized");
        const authorized = await requestJson(`${api}/v1/base/autonomous-bounties/authorized-creation-plan`, {
          method: "POST", body: JSON.stringify({ network: "base-mainnet", create, signature: signatureParts(signature), relayer: state.account }),
        });
        if (!authorized.relay_transaction || String(authorized.relay_transaction.to).toLowerCase() !== String(protocol.factory).toLowerCase()) throw new Error("The authorized transaction does not target the canonical factory.");
        transactionHash = await sendTransaction(authorized.relay_transaction);
        await waitReceipt(transactionHash);
      } else {
        await sendWalletCalls(plan.wallet_calls, protocol);
        // Batch identifiers are not transaction hashes. The journal retains either form for recovery.
      }
      state.bountyContract = plan.predicted_bounty_contract;
      state.bountyId = plan.bounty_id;
      setPaymentStatus("Wallet step returned. Waiting for canonical FundingAdded and BountyBecameClaimable evidence…", "pending");
      const events = await pollCreation(api, plan.bounty_id);
      if (!events) {
        setPaymentStatus([
          "The wallet step returned, but canonical funding evidence is still pending.",
          transactionHash ? `Transaction: ${protocol.explorer_url}/tx/${transactionHash}` : "The recorded wallet operation is awaiting reconciliation.",
          "Do not describe the bounty as funded until FundingAdded and BountyBecameClaimable are confirmed.",
        ].join("\n"), "pending");
        return;
      }
      const item = await fetchFeedItem(api, state.bountyContract);
      if (item?.terms_valid && item.verification_ready) {
        const link = document.querySelector("[data-posted-bounty]");
        if (link) { link.href = `participate.html?bountyContract=${encodeURIComponent(state.bountyContract)}&network=base-mainnet`; link.hidden = false; }
      }
      ui.badge.textContent = item?.verification_ready ? "Funded · ready to earn" : "Funded · verifier readiness pending";
      setPaymentStatus([
        item?.verification_ready ? "Bounty funded and ready for public earning." : "Canonical funding is confirmed. The committed verifier must be ready before this bounty appears in ready-to-earn inventory.",
        `Contract: ${state.bountyContract}`,
        "Solver payment will be proven only by BountySettled.",
      ].join("\n"), "success");
      ui.fundNow.textContent = "Funded ✓";
      ui.fundNow.disabled = true;
      postingJournal.checkpoint("funding_confirmed");
      if (childPlan) setPaymentStatus("The child is canonically funded. Before claiming the parent, confirm both distinct participant registrations and the child TermsPublished event, then wait for a strictly later Base timestamp. The child must later settle to the distinct solver before the parent can pay.", "success");
      track("canonical_post_confirmed", { bounty_contract: state.bountyContract });
    } catch (error) {
      postingJournal.reject(error);
      setPaymentStatus(error.message || String(error), "error");
      ui.fundNow.disabled = Boolean(postingJournal.load());
    } finally {
      postingBusy = false;
      if (!postingJournal.load()) for (const field of ui.form.querySelectorAll("input, textarea, button")) field.disabled = false;
    }
  }

  function configureSpeech(){const Recognition=window.SpeechRecognition||window.webkitSpeechRecognition;if(!Recognition){ui.mic.hidden=true;ui.hint.textContent="Type naturally. Your words are not posted until you approve the final card.";return;}const recognition=new Recognition();recognition.continuous=false;recognition.interimResults=true;recognition.lang=document.documentElement.lang||navigator.language||"en-US";let original="";recognition.addEventListener("start",()=>{original=ui.input.value.trim();ui.mic.dataset.listening="true";setStatus("Listening…","pending");});recognition.addEventListener("result",(event)=>{let transcript="";for(let index=event.resultIndex;index<event.results.length;index+=1)transcript+=event.results[index][0].transcript;ui.input.value=[original,transcript.trim()].filter(Boolean).join(original?" ":"");});recognition.addEventListener("end",()=>{ui.mic.dataset.listening="false";setStatus("Review the dictated text, then continue.");});recognition.addEventListener("error",(event)=>{ui.mic.dataset.listening="false";setStatus(event.error==="not-allowed"?"Microphone permission was not granted. You can still type.":"Dictation stopped. You can continue typing.","error");});ui.mic.addEventListener("click",()=>{if(ui.mic.dataset.listening==="true")recognition.stop();else recognition.start();});state.speech=recognition;}

  let metaContextPromise = null;
  function prepareMetaParent() {
    if (metaContextPromise) return metaContextPromise;
    const params = new URLSearchParams(window.location.search);
    const address = params.get("parentBounty");
    if (!address) return Promise.resolve(null);
    metaContextPromise = (async () => {
      if (params.has("parentCompetition")) throw new Error("Choose one parent bounty or competition for this draft.");
      const client = window.AgentBountiesWorkflow.createClient(window);
      const journey = client.load() || client.start({ role: "earn" });
      const existing = journey.meta_child?.parent_bounty_contract === address.toLowerCase() ? journey.meta_child : {};
      const parent = await metaChild.resolve({ ...existing, parent_bounty_contract: address }, client);
      state.metaParent = parent;
      state.fundingUsdc = 1;
      state.preparedRewards = metaChild.rewards(990000n, 10000n, parent);
      client.save({ ...journey, meta_child: metaChild.normalize(parent) });
      const note = document.createElement("p"); note.dataset.parentContext = ""; note.setAttribute("role", "status");
      note.textContent = `Preparing a 1 USDC child for “${parent.title}”. The total includes both verifier rewards. A different participant must complete this child. Publish its terms and register both participants before claiming the parent. Your wallet shows any Base gas fee.`;
      document.querySelector("main")?.prepend(note);
      return parent;
    })();
    return metaContextPromise;
  }

  async function prefillFromQuery() {
    await prepareMetaParent();
    const params = new URLSearchParams(window.location.search);
    try {
      state.distributionAttribution = parseDistributionAttribution(params);
    } catch (error) {
      setStatus(error.message || String(error), "error");
      return;
    }
    const fromAiApp = ["ai-app", "chatgpt-app"].includes(params.get("from"));
    if (fromAiApp) { enableAiHandoffReview(); track("canonical_post_handoff_viewed"); }
    const title = params.get("title");
    const goal = params.get("goal");
    const criteria = params.getAll("criterion");
    const solver = params.get("solverReward");
    const verifier = params.get("verifierReward");

    if (title && goal && criteria.length && solver && verifier) {
      try {
        const parseHandoffJson = (name) => {
          const encoded = params.get(name);
          if (!encoded) return null;
          if (encoded.length > 20000) throw new Error(`${name} exceeds the browser handoff limit.`);
          const value = JSON.parse(encoded);
          if (!value || typeof value !== "object" || Array.isArray(value)) {
            throw new Error(`${name} must be one JSON object.`);
          }
          return value;
        };
        await importPreparedDraft({
          title,
          goal,
          acceptance_criteria: criteria,
          solver_reward_usdc: solver,
          verifier_reward_usdc: verifier,
          task_window_days: Number(params.get("taskWindowDays") || MAX_TASK_DAYS),
          source_url: params.get("sourceUrl"),
          crowdfund: params.get("crowdfund") === "true",
          discovery_source: params.get("discoverySource") || "AI assistant via MCP",
          benchmark: parseHandoffJson("benchmark"),
          evidence_schema: parseHandoffJson("evidenceSchema"),
          image_required: Boolean(params.get("imageUrl")),
          image: params.get("imageUrl") ? {
              source: "chatgpt_user_generated",
              asset_url: params.get("imageUrl"),
              sha256: params.get("imageSha256"),
              mime_type: params.get("imageMimeType"),
              prompt: params.get("imagePrompt"),
              alt_text: params.get("imageAlt"),
            } : null,
        });
      } catch (error) {
        setStatus(error.message || String(error), "error");
      }
    } else {
      const supplied = goal || params.get("draftObjective") || params.get("objective");
      if (supplied) ui.input.value = supplied;
    }

    const draftId = params.get("socialDraft");
    if (draftId && /^[0-9a-f-]{36}$/i.test(draftId)) {
      try {
        const response = await requestJson(`${API}/v1/social/mention-drafts/${draftId}`);
        const draft = response && response.draft;
        if (draft && draft.state === "review_required_not_published") {
          ui.input.value = [draft.draft_objective, draft.goal, ...(draft.acceptance_criteria || [])].filter(Boolean).join("\n");
          const importedSolver = Number(draft.solver_reward && draft.solver_reward.amount || 0);
          const importedVerifier = Number(draft.verifier_reward && draft.verifier_reward.amount || 0);
          if (Number.isSafeInteger(importedSolver) && importedSolver > 0 && Number.isSafeInteger(importedVerifier) && importedVerifier > 0) {
            state.preparedRewards = {
              solver: BigInt(importedSolver),
              verifier: BigInt(importedVerifier),
              total: BigInt(importedSolver + importedVerifier),
            };
            state.fundingUsdc = Number(state.preparedRewards.total) / 1_000_000;
          }
          setStatus("Draft imported. It has not been posted or funded. Describe any changes, then continue.", "pending");
        }
      } catch (error) {
        setStatus(error.message || String(error), "error");
      }
    }
  }

  ui.form.addEventListener("submit",handleComposerSubmit);
  ui.approve.addEventListener("click", (event) => { if (event.isTrusted) approveCard(); });
  ui.revise.addEventListener("click",reviseCard);
  ui.share.addEventListener("click",shareBountyCard);
  ui.fund.addEventListener("click",openFunding);
  ui.closeDialog.addEventListener("click",()=>ui.dialog.close());
  ui.cryptoMethod.addEventListener("click",chooseCryptoWallet);
  ui.watchUsdc.addEventListener("click",watchUsdcAsset);
  ui.copyUsdc.addEventListener("click",copyUsdcAddress);
  ui.recheck.addEventListener("click",()=>refreshWalletReadiness().catch((error)=>setPaymentStatus(error.message||String(error),"error")));
  ui.fundNow.addEventListener("click",(event)=>{
    if (!event.isTrusted) return;
    try {
      supportedVerificationPolicy();
      fundApprovedBounty();
    } catch (error) {
      ui.fundNow.disabled = true;
      setPaymentStatus(error.message || String(error), "error");
    }
  });
  ui.dialog.addEventListener("click",(event)=>{if(event.target===ui.dialog)ui.dialog.close();});
  window.addEventListener("agent-bounties:prepared-draft", (event) => {
    importPreparedDraft(event.detail).catch((error) => setStatus(error.message || String(error), "error"));
  });

  // No approval or signing methods are exposed to the agent registry.
  let staging = Promise.resolve();
  let stagedFingerprint = null;
  window.AgentBountiesComposer = Object.freeze({
    prepareMetaParent,
    invalidate() {
      if (!state.draft || postingBusy || postingJournal.load()) return;
      state.reviewStale = true; state.approved = false; stagedFingerprint = null;
      ui.approve.dataset.approved = "false"; ui.approve.disabled = true; ui.fund.disabled = true; ui.fundNow.disabled = true;
    },
    stage(value) {
      staging = staging.catch(() => {}).then(() => {
        if (postingBusy || state.bountyContract || postingJournal.load()) throw new Error("A posting operation is in progress or recorded. Check its canonical status before preparing another draft.");
        const fingerprint = JSON.stringify(value);
        if (stagedFingerprint === fingerprint && state.draft) return;
        return importPreparedDraft(value).then((result) => { stagedFingerprint = fingerprint; return result; });
      });
      return staging;
    },
    review() {
      let blocker = null;
      if (state.draft) { try { supportedVerificationPolicy(); } catch (error) { blocker = error.message; } }
      return {
        status: state.bountyContract ? "created_check_canonical_funding" : state.draft ? "staged" : "no_staged_bounty",
        explicitly_approved: state.approved === true,
        funding_ready: Boolean(state.draft && !blocker),
        blocker,
        bounty_contract: state.bountyContract || postingJournal.load()?.bounty_contract || null,
        posting_operation: postingJournal.load(),
        meta_child: state.metaParent ? { parent_bounty_contract: state.metaParent.parent_bounty_contract, total_usdc: "1.00", intended_child_solver: state.metaParent.intended_child_solver, verifier_threshold: 2, terms_before_parent_claim: true, gas_sponsored: false } : null,
        review_mode: state.draft?.benchmark?.engine === "creator_review_v1" ? "creator" : "automated",
        delivery_deadline: state.deliveryDeadline || null,
        saved_brief: window.AgentBountiesWorkflow.createClient(window).load()?.brief || null,
        next_action: !state.draft || state.reviewStale ? "Prepare or update the proposal using saved_brief. Preserve its outcome, budget and deadline. Ask only for missing business decisions."
          : blocker ? "For non-software work, propose review_mode=creator with the agreed delivery_deadline and no automated benchmark, then stage it for the person’s review. Meta children still require their automated verifier. Never invent benchmark details."
          : "The person reviews the terms once, then uses the wallet confirmation. No separate approval in chat is needed.",
      };
    },
  });

  configureSpeech();
  setProgress("describe");
  prefillFromQuery().catch((error) => setStatus(error.message || String(error), "error"));
})();
