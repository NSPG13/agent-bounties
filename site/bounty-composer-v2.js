(() => {
  "use strict";

  const API = "https://api.agentbounties.app";
  const MAX_AI_ROUNDS = 3;
  const MAX_PLAN_TASKS = 6;
  const MAX_TASK_DAYS = 30;
  const MIN_TOTAL_USDC = 0.02;
  const MAX_TOTAL_USDC = 9_000_000_000;
  const VISUAL_EXTENSION = "x-agent-bounties-draft-visual";
  const ALLOWED_SCENES = new Set([
    "infrastructure", "digital", "nature", "health", "research", "education", "coordination", "general",
  ]);
  const ALLOWED_ELEMENTS = new Set([
    "sun", "road", "lamp", "building", "tree", "water", "screen", "document", "network", "tool", "person", "check", "bridge", "book", "chart",
  ]);

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
    watchUsdc: document.querySelector("[data-watch-usdc]"),
    copyUsdc: document.querySelector("[data-copy-usdc]"),
    recheck: document.querySelector("[data-recheck-balance]"),
    fundNow: document.querySelector("[data-fund-now]"),
    paymentStatus: document.querySelector("[data-payment-status]"),
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
    visualSpec: null,
    visualSource: "fallback",
    visualCache: new Map(),
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
  };

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
    if (totalUnits < 20_000n) throw new Error("The current protocol requires at least 0.02 USDC in total.");
    const proportional = totalUnits / 50n;
    const verifier = proportional < 10_000n ? 10_000n : proportional;
    const cappedVerifier = verifier > totalUnits / 5n ? totalUnits / 5n : verifier;
    const solver = totalUnits - cappedVerifier;
    if (solver < 10_000n) throw new Error("The solver reward must remain at least 0.01 USDC.");
    return { total: totalUnits, solver, verifier: cappedVerifier };
  }

  function parseFunding(value) {
    const cleaned = String(value || "").replace(/,/g, "");
    const match = cleaned.match(/(?:^|[^0-9])(\d+(?:\.\d{1,6})?)(?:\s*(?:base\s*)?usdc|\s*dollars?|\s*usd)?(?:$|[^0-9])/i);
    if (!match) return null;
    const amount = Number(match[1]);
    return Number.isFinite(amount) && amount >= MIN_TOTAL_USDC && amount <= MAX_TOTAL_USDC ? amount : null;
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
      hint: "The total includes the solver reward and a small verifier reserve. Minimum: 0.02 USDC.",
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
      await generateInitialDraft();
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
        setStatus(`Enter an amount from ${MIN_TOTAL_USDC.toFixed(2)} to ${MAX_TOTAL_USDC.toLocaleString()} USDC.`, "error");
        return;
      }
      state.fundingUsdc = amount;
      setStatus("");
      renderPreview();
      return;
    }
    if (state.phase === "revise") {
      state.context.push(`Requested revision: ${value}`);
      state.originalRequest = `${state.draft.goal}\nRevision requested by the user: ${value}`;
      state.aiRounds = 0;
      state.questions = [];
      state.askedQuestions.clear();
      state.initialDraft = null;
      state.draft = null;
      state.missionPlan = null;
      state.selectedTaskId = null;
      state.visualCache.clear();
      state.approved = false;
      await generateInitialDraft();
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
    return risks.length ? `${risks.length} item${risks.length === 1 ? "" : "s"} to review` : "No AI blocker flagged";
  }

  function renderCardText() {
    ui.title.textContent = state.draft.title;
    ui.goal.textContent = state.draft.goal;
    ui.reward.textContent = `${formatUsdc(state.fundingUsdc)} USDC`;
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

  async function renderPreview() {
    if (!state.draft || state.fundingUsdc == null || !state.horizon || !state.taskWindowDays) return;
    splitReward(state.fundingUsdc);
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
    ui.preview.scrollIntoView({ behavior: "smooth", block: "start" });
    setStatus("Generating a bounded pre-publication illustration from the AI draft. Nothing has been posted or funded.", "pending");
    await renderAiVisualForCurrentDraft();
    ui.approve.disabled = false;
    ui.approve.textContent = "Approve bounty card";
    setStatus("Review the result, tasks, completion checks, horizon, reward, and creator-verification disclosure. Nothing has been posted or funded.");
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
    ui.imageStatus.textContent = "Generating AI visual…";
    ui.imageStatus.dataset.tone = "";
    let spec = extractVisualSpec(state.draft);
    if (!spec && state.scope === "mission") {
      const task = selectedTask();
      if (task) spec = await requestVisualSpecForTask(task);
    }
    if (spec) {
      state.visualSpec = spec;
      state.visualSource = "ai";
      ui.imageStatus.textContent = "AI-generated draft visual";
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
    state.approved = true;
    ui.approve.dataset.approved = "true";
    ui.approve.textContent = "Approved ✓";
    ui.fund.disabled = false;
    setProgress("fund");
    setStatus("Card approved. Funding still requires a separate wallet review and signature.", "success");
  }

  function reviseCard() {
    state.approved = false;
    ui.approve.dataset.approved = "false";
    ui.approve.textContent = "Approve bounty card";
    ui.fund.disabled = true;
    setComposer({ phase:"revise", prompt:"What should the AI change about this bounty or mission plan?", label:"Revision request", placeholder:"Example: Split the research into its own task and extend the mission horizon to one year.", button:"Update card", hint:"Changing the card removes approval until you review it again." });
    document.querySelector(".composer-shell")?.scrollIntoView({behavior:"smooth",block:"center"});
  }

  function openFunding() {
    if (!state.approved) return;
    ui.dialog.showModal();
    setPaymentStatus("Choose a payment method. Only a crypto wallet is available today.");
    ui.walletPanel.hidden = true;
    ui.readiness.hidden = true;
    ui.fundNow.disabled = true;
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
    if(!providers.length){ui.walletMessage.textContent="No compatible browser wallet was detected. Install or open a Base-compatible wallet, then try again. Never enter a recovery phrase on this website.";return;}
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

  async function connectWallet(item){state.provider=item.provider;setPaymentStatus(`Connecting ${providerName(item)}…`,"pending");try{const protocol=await loadProtocol();const accounts=await state.provider.request({method:"eth_requestAccounts"});if(!accounts||!accounts[0])throw new Error("The wallet did not return an account.");state.account=accounts[0];await switchToBase(state.provider,protocol);await refreshWalletReadiness();}catch(error){setPaymentStatus(error.message||String(error),"error");}}

  function addressWord(address){return String(address).toLowerCase().replace(/^0x/,"").padStart(64,"0");}

  async function refreshWalletReadiness(){if(!state.provider||!state.account)return;const protocol=await loadProtocol();const required=usdcBaseUnits(state.fundingUsdc);setPaymentStatus("Checking Base USDC and gas readiness…","pending");const[usdcRaw,ethRaw]=await Promise.all([state.provider.request({method:"eth_call",params:[{to:protocol.native_usdc,data:`0x70a08231${addressWord(state.account)}`},"latest"]}),state.provider.request({method:"eth_getBalance",params:[state.account,"latest"]})]);const usdc=BigInt(usdcRaw||"0x0");const eth=BigInt(ethRaw||"0x0");state.balances={usdc,eth,required};const usdcReady=usdc>=required;const ethReady=eth>0n;ui.account.textContent=`${state.account.slice(0,8)}…${state.account.slice(-6)}`;ui.usdcBalance.textContent=`${formatUsdc(Number(usdc)/1_000_000)} USDC`;ui.ethBalance.textContent=`${(Number(eth)/1e18).toFixed(6)} ETH`;ui.requiredUsdc.textContent=`${formatUsdc(state.fundingUsdc)} USDC`;ui.readiness.hidden=false;ui.fundingHelp.hidden=usdcReady&&ethReady;const missing=required>usdc?required-usdc:0n;ui.missingUsdc.textContent=`${formatUsdc(Number(missing)/1_000_000)} USDC`;ui.fundNow.disabled=!(usdcReady&&ethReady);if(usdcReady&&ethReady)setPaymentStatus("Wallet ready. Review the exact amount, legal terms, and wallet request before signing.","success");else if(!usdcReady&&!ethReady)setPaymentStatus("This wallet needs more Base USDC and a small amount of Base ETH for gas.","error");else if(!usdcReady)setPaymentStatus("This wallet does not yet hold enough USDC on Base.","error");else setPaymentStatus("The USDC is available, but the wallet needs a small amount of Base ETH for gas.","error");}

  async function watchUsdcAsset(){try{const protocol=await loadProtocol();await state.provider.request({method:"wallet_watchAsset",params:{type:"ERC20",options:{address:protocol.native_usdc,symbol:"USDC",decimals:6}}});setPaymentStatus("Base USDC was offered to the wallet. This does not buy or transfer tokens.","success");}catch(error){setPaymentStatus(error.message||String(error),"error");}}
  async function copyUsdcAddress(){const protocol=await loadProtocol();await navigator.clipboard.writeText(protocol.native_usdc);setPaymentStatus("Base USDC contract address copied. Verify the network and address inside your wallet before acquiring tokens.","success");}

  function signatureParts(signature){const value=String(signature).replace(/^0x/,"");if(value.length!==130)throw new Error("The wallet returned an invalid signature.");return{r:`0x${value.slice(0,64)}`,s:`0x${value.slice(64,128)}`,v:Number.parseInt(value.slice(128,130),16)};}
  async function sendTransaction(transaction){if(!transaction||!transaction.to||!transaction.data||Number(transaction.value_wei||0)!==0)throw new Error("The planned transaction is invalid.");return state.provider.request({method:"eth_sendTransaction",params:[{from:state.account,to:transaction.to,data:transaction.data,value:"0x0"}]});}
  async function waitReceipt(hash,timeoutMs=150000){const started=Date.now();while(Date.now()-started<timeoutMs){const receipt=await state.provider.request({method:"eth_getTransactionReceipt",params:[hash]});if(receipt){if(receipt.status!=="0x1")throw new Error(`The Base transaction reverted: ${hash}`);return receipt;}await new Promise((resolve)=>setTimeout(resolve,1600));}throw new Error("The transaction is still pending. Check the wallet or Base explorer before trying again.");}
  async function isContractAccount(){const code=await state.provider.request({method:"eth_getCode",params:[state.account,"latest"]});return code&&code!=="0x"&&code!=="0x0";}
  async function sendWalletCalls(calls,protocol){try{return await state.provider.request({method:"wallet_sendCalls",params:[{version:"2.0.0",chainId:protocol.chain_id_hex,from:state.account,calls:calls.map((call)=>({to:call.to,data:call.data,value:"0x0"}))}]});}catch(_error){let last=null;for(const call of calls){last=await sendTransaction(call);await waitReceipt(last);}return last;}}

  function contractTerms(protocol,rewards){const now=Math.floor(Date.now()/1000);return{protocol_version:protocol.protocol_version,creator_wallet:state.account,network:protocol.network,settlement_token:protocol.native_usdc,solver_reward:{amount:Number(rewards.solver),currency:"usdc"},verifier_reward:{amount:Number(rewards.verifier),currency:"usdc"},claim_bond:{amount:Number(rewards.verifier),currency:"usdc"},initial_funding:{amount:Number(rewards.total),currency:"usdc"},funding_deadline:now+30*86400,claim_window_seconds:state.taskWindowDays*86400,verification_window_seconds:48*3600,creation_nonce:randomBytes32()};}

  function termsDocument(committed){return{schema_version:"agent-bounties/terms-v1",contract_terms:committed,title:state.draft.title,goal:state.draft.goal,acceptance_criteria:state.draft.acceptance_criteria,benchmark:canonicalJsonValue(missionBenchmark(state.draft.benchmark||{type:"creator_review"})),evidence_schema:canonicalJsonValue(stripVisualExtension(state.draft.evidence_schema)),verification_policy:{mechanism:"signed_quorum",verifier_module:null,verifier_reward_recipient:null,verifiers:[state.account],threshold:1,ai_provider:null,ai_model:null,ai_model_version:null,system_prompt:null,rubric:null,decoding_parameters:{},public_disclosure:"The bounty creator is the single subjective verifier and decides pass or fail against the published acceptance criteria."},source_url:null,discovery_source:"web_ai_bounty_card_composer_v2"};}

  function createPayload(terms,committed){return{creator:state.account,solver_reward:committed.solver_reward,verifier_reward:committed.verifier_reward,terms_hash:terms.terms_hash,policy_hash:terms.policy_hash,acceptance_criteria_hash:terms.acceptance_criteria_hash,benchmark_hash:terms.benchmark_hash,evidence_schema_hash:terms.evidence_schema_hash,funding_deadline:committed.funding_deadline,claim_window_seconds:committed.claim_window_seconds,verification_window_seconds:committed.verification_window_seconds,verification_mode:"signed_quorum",verifier_module:null,verifier_reward_recipient:null,verifiers:[state.account],threshold:1,initial_funding:committed.initial_funding,creation_nonce:committed.creation_nonce};}

  function validateCreationPlan(plan,protocol,create){if(!plan||!/^0x[0-9a-fA-F]{40}$/.test(plan.predicted_bounty_contract||""))throw new Error("The creation plan did not return a valid bounty address.");if(Number(plan.network&&plan.network.chain_id)!==Number(protocol.chain_id))throw new Error("The creation plan targets the wrong network.");if(String(plan.factory_contract||"").toLowerCase()!==String(protocol.factory).toLowerCase())throw new Error("The creation plan does not use the canonical factory.");const target=Number(create.solver_reward.amount)+Number(create.verifier_reward.amount);if(Number(create.initial_funding.amount)!==target)throw new Error("The creation plan is not fully funded.");}

  async function pollCreation(api,bountyId,timeoutMs=100000){const started=Date.now();while(Date.now()-started<timeoutMs){const events=await requestJson(`${api}/v1/base/autonomous-bounties/events?network=base-mainnet&bounty_id=${encodeURIComponent(bountyId)}`,{cache:"no-store"});const created=events.some((event)=>event.kind==="canonical_bounty_created");const funded=events.some((event)=>event.kind==="funding_added");const claimable=events.some((event)=>event.kind==="bounty_became_claimable");if(created&&funded&&claimable)return events;await new Promise((resolve)=>setTimeout(resolve,2500));}return null;}

  async function fetchFeedItem(api,contract){try{const items=await requestJson(`${api}/v1/base/autonomous-bounties/feed?network=base-mainnet&claimable_only=false`,{cache:"no-store"});return items.find((item)=>String(item.bounty_contract).toLowerCase()===String(contract).toLowerCase())||null;}catch(_error){return null;}}

  async function fundApprovedBounty(){if(!state.approved||!state.provider||!state.account||!state.balances)return;ui.fundNow.disabled=true;setPaymentStatus("Preparing the exact canonical Base USDC funding request…","pending");try{await refreshWalletReadiness();if(state.balances.usdc<state.balances.required||state.balances.eth===0n)throw new Error("The wallet is not ready to fund this bounty.");if(!window.AgentBountiesLegal)throw new Error("The legal agreement could not be loaded. Reload before using the wallet.");await window.AgentBountiesLegal.requireAcceptance({action:"post_bounty",walletAddress:state.account,scope:ui.dialog});const protocol=await loadProtocol();const api=String(protocol.api_base_url).replace(/\/$/,"");const rewards=splitReward(state.fundingUsdc);const committed=contractTerms(protocol,rewards);const document=termsDocument(committed);const terms=await requestJson(`${api}/v1/base/autonomous-bounties/terms`,{method:"POST",body:JSON.stringify({creator_wallet:state.account,document})});const create=createPayload(terms,committed);const plan=await requestJson(`${api}/v1/base/autonomous-bounties/creation-plan`,{method:"POST",body:JSON.stringify({network:"base-mainnet",create})});validateCreationPlan(plan,protocol,create);setPaymentStatus(["Review the wallet request carefully.",`Exact total funding: ${formatUsdc(state.fundingUsdc)} Base USDC.`,`Predicted bounty: ${plan.predicted_bounty_contract}`,"A signature or transaction hash is not funding evidence."].join("\n"),"pending");let transactionHash=null;if(!(await isContractAccount())&&plan.eip3009_authorization){const signature=await state.provider.request({method:"eth_signTypedData_v4",params:[state.account,JSON.stringify(plan.eip3009_authorization)]});const authorized=await requestJson(`${api}/v1/base/autonomous-bounties/authorized-creation-plan`,{method:"POST",body:JSON.stringify({network:"base-mainnet",create,signature:signatureParts(signature),relayer:state.account})});if(!authorized.relay_transaction||String(authorized.relay_transaction.to).toLowerCase()!==String(protocol.factory).toLowerCase())throw new Error("The authorized transaction does not target the canonical factory.");transactionHash=await sendTransaction(authorized.relay_transaction);await waitReceipt(transactionHash);}else{const result=await sendWalletCalls(plan.wallet_calls,protocol);if(typeof result==="string"&&result.startsWith("0x"))transactionHash=result;}state.bountyContract=plan.predicted_bounty_contract;state.bountyId=plan.bounty_id;setPaymentStatus("Transaction confirmed. Waiting for canonical FundingAdded and BountyBecameClaimable evidence…","pending");const events=await pollCreation(api,plan.bounty_id);if(!events){setPaymentStatus(["The transaction was confirmed, but canonical funding evidence is still pending.",transactionHash?`Transaction: ${protocol.explorer_url}/tx/${transactionHash}`:"Wallet batch submitted.","Do not describe the bounty as funded until FundingAdded and BountyBecameClaimable are confirmed."].join("\n"),"pending");return;}const item=await fetchFeedItem(api,state.bountyContract);if(item?.verification_ready){ui.badge.textContent="Funded · ready to earn";setPaymentStatus(["Bounty funded and ready for public earning.",`Contract: ${state.bountyContract}`,"Solver payment will be proven only by BountySettled."].join("\n"),"success");}else{ui.badge.textContent="Funded · verifier setup required";setPaymentStatus(["Canonical funding is confirmed.",`Contract: ${state.bountyContract}`,"The creator is the committed verifier. The bounty will not appear in the default ready-to-earn inventory until verifier availability is represented by the protocol.","Solver payment will be proven only by BountySettled."].join("\n"),"success");}ui.fundNow.textContent="Funded ✓";ui.fundNow.disabled=true;if(window.agentBountiesTrack)window.agentBountiesTrack("canonical_post_confirmed",{bounty_contract:state.bountyContract});}catch(error){setPaymentStatus(error.message||String(error),"error");ui.fundNow.disabled=false;}}

  function configureSpeech(){const Recognition=window.SpeechRecognition||window.webkitSpeechRecognition;if(!Recognition){ui.mic.hidden=true;ui.hint.textContent="Type naturally. Your words are not posted until you approve the final card.";return;}const recognition=new Recognition();recognition.continuous=false;recognition.interimResults=true;recognition.lang=document.documentElement.lang||navigator.language||"en-US";let original="";recognition.addEventListener("start",()=>{original=ui.input.value.trim();ui.mic.dataset.listening="true";setStatus("Listening…","pending");});recognition.addEventListener("result",(event)=>{let transcript="";for(let index=event.resultIndex;index<event.results.length;index+=1)transcript+=event.results[index][0].transcript;ui.input.value=[original,transcript.trim()].filter(Boolean).join(original?" ":"");});recognition.addEventListener("end",()=>{ui.mic.dataset.listening="false";setStatus("Review the dictated text, then continue.");});recognition.addEventListener("error",(event)=>{ui.mic.dataset.listening="false";setStatus(event.error==="not-allowed"?"Microphone permission was not granted. You can still type.":"Dictation stopped. You can continue typing.","error");});ui.mic.addEventListener("click",()=>{if(ui.mic.dataset.listening==="true")recognition.stop();else recognition.start();});state.speech=recognition;}

  async function prefillFromQuery(){const params=new URLSearchParams(window.location.search);const supplied=params.get("goal")||params.get("draftObjective")||params.get("objective");const title=params.get("title");const goal=params.get("goal");const criteria=params.getAll("criterion");if(supplied||title||goal||criteria.length){ui.input.value=[supplied,title&&`Title: ${title}`,goal&&`Result: ${goal}`,criteria.length&&`Completion checks: ${criteria.join("; ")}`].filter(Boolean).join("\n");state.fundingUsdc=parseFunding(params.get("solverReward"));}const draftId=params.get("socialDraft");if(draftId&&/^[0-9a-f-]{36}$/i.test(draftId)){try{const response=await requestJson(`${API}/v1/social/mention-drafts/${draftId}`);const draft=response&&response.draft;if(draft&&draft.state==="review_required_not_published"){ui.input.value=[draft.draft_objective,draft.goal,...(draft.acceptance_criteria||[])].filter(Boolean).join("\n");const solver=Number(draft.solver_reward&&draft.solver_reward.amount||0);const verifier=Number(draft.verifier_reward&&draft.verifier_reward.amount||0);if(Number.isSafeInteger(solver)&&Number.isSafeInteger(verifier))state.fundingUsdc=(solver+verifier)/1_000_000;setStatus("Draft imported. It has not been posted or funded. Describe any changes, then continue.","pending");}}catch(error){setStatus(error.message||String(error),"error");}}}

  ui.form.addEventListener("submit",handleComposerSubmit);
  ui.approve.addEventListener("click",approveCard);
  ui.revise.addEventListener("click",reviseCard);
  ui.share.addEventListener("click",shareBountyCard);
  ui.fund.addEventListener("click",openFunding);
  ui.closeDialog.addEventListener("click",()=>ui.dialog.close());
  ui.cryptoMethod.addEventListener("click",chooseCryptoWallet);
  ui.watchUsdc.addEventListener("click",watchUsdcAsset);
  ui.copyUsdc.addEventListener("click",copyUsdcAddress);
  ui.recheck.addEventListener("click",()=>refreshWalletReadiness().catch((error)=>setPaymentStatus(error.message||String(error),"error")));
  ui.fundNow.addEventListener("click",fundApprovedBounty);
  ui.dialog.addEventListener("click",(event)=>{if(event.target===ui.dialog)ui.dialog.close();});

  configureSpeech();
  setProgress("describe");
  prefillFromQuery();
})();