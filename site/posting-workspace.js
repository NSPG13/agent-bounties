(() => {
  "use strict";
  const flow = window.AgentBountiesWorkflow;
  const client = flow.createClient(window);
  const form = document.querySelector("#bounty-composer-form");
  const goal = document.querySelector("#bounty-composer-input");
  const budget = document.querySelector("[data-brief-budget]");
  const deadline = document.querySelector("[data-brief-deadline]");
  const status = document.querySelector("[data-brief-status]");
  const timezone = document.querySelector("[data-brief-timezone]");
  const helper = window.AgentBountiesPostingBrief;
  const mode = document.querySelector("[data-brief-mode]");
  const manualFields = document.querySelector("[data-manual-fields]");
  const title = document.querySelector("[data-brief-title]");
  const criteria = document.querySelector("[data-brief-criteria]");
  const reserve = document.querySelector("[data-brief-reserve]");
  const consent = document.querySelector("[data-manual-review-consent]");
  const params = new URLSearchParams(window.location.search);
  const protectedHandoff = params.has("parentBounty") || ["ai-app", "chatgpt-app"].includes(params.get("from"));
  function setMode() {
    const manual = mode.value === "manual";
    manualFields.hidden = !manual;
    manualFields.disabled = !manual;
    form.querySelector("[data-composer-submit]").textContent = manual ? "Prepare for review →" : "Continue with my AI →";
  }
  const saved = client.load();
  mode.value = protectedHandoff ? "ai" : params.get("mode") === "manual" ? "manual" : saved?.brief?.preparation_mode || (saved?.draft ? "ai" : "manual");
  mode.disabled = protectedHandoff;
  setMode();
  const deadlineSummary = document.querySelector("[data-brief-deadline-summary]");
  const budgetSummary = document.querySelector("[data-brief-budget-summary]");
  const warningList = document.querySelector("[data-brief-warnings]");
  const detectedTimezone = Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  timezone.value = detectedTimezone;
  const zoneList = document.querySelector("#brief-timezones");
  if (zoneList) {
    const zones = typeof Intl.supportedValuesOf === "function" ? Intl.supportedValuesOf("timeZone") : ["America/Mexico_City", "America/Chicago", "Europe/London", "Asia/Shanghai"];
    for (const zone of new Set([detectedTimezone, "UTC", "-06:00", ...zones])) {
      const option = document.createElement("option"); option.value = zone; zoneList.append(option);
    }
  }
  let restoring = false;
  const actions = document.querySelector(".bounty-card-actions");
  if (actions) {
    const measureActions = () => document.documentElement.style.setProperty("--posting-actions-height", `${Math.ceil(actions.getBoundingClientRect().height)}px`);
    if (typeof ResizeObserver === "function") new ResizeObserver(measureActions).observe(actions);
    window.addEventListener("resize", measureActions);
    measureActions();
  }
  // Pricing is an explicit planning assumption; settled micro-bounties are not
  // represented as market-clearing prices for unrelated professional work.
  if (budgetSummary) {
    const estimator = document.createElement("details");
    const title = document.createElement("summary"); title.textContent = "Estimate a starting budget";
    const inputs = [ ["Estimated work hours", "hours", "2"], ["USDC per hour", "rate", "20"], ["Review reserve in USDC", "reserve", "1"] ].map(([label, name, value]) => {
      const row = document.createElement("label"); row.textContent = label;
      const field = document.createElement("input"); field.type = "number"; field.min = "0.01"; field.step = "0.01"; field.value = value; field.name = `budget-estimate-${name}`;
      row.append(field); return { row, field };
    });
    const result = document.createElement("p"); result.setAttribute("aria-live", "polite");
    const note = document.createElement("p"); note.textContent = "This is hours × your chosen rate, plus the review reserve. It is a planning estimate, not a prediction of bids. Consider specialist access, tools and external expenses.";
    const comparable = document.createElement("a"); comparable.href = "/metrics.html"; comparable.textContent = "Compare dated, confirmed payouts";
    const update = () => { const estimate = helper.estimateBudget(...inputs.map(({ field }) => field.value)); result.textContent = estimate ? `${estimate.total} USDC total: ${estimate.solver} for the work + ${estimate.reserve} review reserve. Update your brief if you want this budget.` : "Enter positive hours, rate and review reserve within the supported range."; };
    for (const { field } of inputs) field.addEventListener("input", update);
    estimator.append(title, ...inputs.map(({ row }) => row), result, note, comparable); budgetSummary.after(estimator); update();
  }
  function render() {
    const result = helper.resolveDeadline(deadline.value, timezone.value.trim());
    const journey = client.load();
    const savedDeadline = journey?.brief?.deadline_at || journey?.draft?.delivery_deadline;
    // datetime-local shows minutes. Preserve an already committed second or
    // millisecond when this is only an edit to another field.
    if (result.iso && Number.isFinite(Date.parse(savedDeadline)) && helper.wallTime(Date.parse(savedDeadline), timezone.value.trim()) === deadline.value) result.iso = savedDeadline;
    deadline.setCustomValidity(result.error || "");
    if (deadlineSummary) deadlineSummary.textContent = result.error || (result.iso ? `${deadline.value.replace("T", " ")} (${timezone.value.trim()}) · ${result.iso} · ${helper.countdown(result.iso)}` : "Choose the exact calendar deadline; the clock continues during wallet setup.");
    const draft = journey?.draft_stale ? null : journey?.draft;
    if (budgetSummary) {
      const staged = journey?.draft;
      budgetSummary.textContent = staged
        ? `Proposal split: ${staged.solver_reward_usdc} USDC for the worker + ${staged.verifier_reward_usdc} USDC ${staged.review_mode === "creator" ? "creator-review reserve" : "verifier reward"}.${journey.draft_stale ? " Your brief changed; prepare an updated proposal before approving." : " These are the amounts on the review card."}`
        : "Enter the combined worker and review budget. The exact split will appear on the review card.";
    }
    if (warningList) {
      warningList.replaceChildren();
      const changes = window.AgentBountiesPostingSession?.create(window).approvalChanges() || [];
      if (changes.length) { const item = document.createElement("li"); item.textContent = `Changed since approval: ${changes.join("; ")}. Review the updated card and approve these changes.`; warningList.append(item); }
      for (const warning of helper.warnings({ goal: goal.value, budget: budget.value, deadline: result.iso, draft })) {
        const item = document.createElement("li"); item.textContent = warning; warningList.append(item);
      }
      warningList.hidden = !warningList.childElementCount;
    }
    return result;
  }
  function restore(journey) {
    if (!journey || journey.role !== "post") { render(); return; }
    restoring = true;
    const brief = journey.brief || {};
    const savedDeadline = brief.deadline_at || journey.draft?.delivery_deadline || "";
    if (document.activeElement !== goal) goal.value = brief.goal || journey.draft?.goal || journey.goal || "";
    if (document.activeElement !== budget) budget.value = brief.budget_usdc || (journey.draft ? String(Number(journey.draft.solver_reward_usdc) + Number(journey.draft.verifier_reward_usdc)) : "");
    if (document.activeElement !== timezone) timezone.value = brief.timezone || savedDeadline.match(/([+-]\d\d:\d\d)$/)?.[1] || detectedTimezone;
    if (document.activeElement !== deadline) {
      try {
        const local = helper.resolveDeadline(brief.deadline_local || "", timezone.value);
        // A restaged draft may update only deadline_at. Do not let an older
        // display field overwrite that newly agreed instant on the next edit.
        const localMatches = !savedDeadline || Date.parse(local.iso) === Date.parse(savedDeadline);
        deadline.value = brief.deadline_local && localMatches ? brief.deadline_local : (savedDeadline ? helper.wallTime(Date.parse(savedDeadline), timezone.value) : "");
      }
      catch (_) { deadline.value = brief.deadline_local || ""; }
    }
    if (document.activeElement !== title) title.value = brief.title ?? journey.draft?.title ?? "";
    if (document.activeElement !== criteria) criteria.value = brief.criteria ?? journey.draft?.acceptance_criteria?.join("\n") ?? "";
    if (document.activeElement !== reserve) reserve.value = brief.review_reward_usdc ?? journey.draft?.verifier_reward_usdc ?? "0.10";
    if (document.activeElement !== consent) consent.checked = brief.review_consent === true;
    if (brief.preparation_mode && !protectedHandoff) mode.value = brief.preparation_mode;
    setMode();
    status.textContent = journey.draft ? "Draft saved on this device. Review the proposal below." : "Brief saved on this device. Complete the details to prepare your proposal.";
    restoring = false;
    render();
  }
  function save() {
    if (restoring) return;
    const current = client.load() || client.start({ role: "post" });
    const result = render();
    const brief = { preparation_mode: mode.value, title: title.value, criteria: criteria.value, review_reward_usdc: reserve.value, review_consent: consent.checked, goal: goal.value.trim(), budget_usdc: budget.value, deadline_at: result.iso, deadline_local: deadline.value, timezone: timezone.value.trim() };
    // Compare decisions, not display formatting, so restoring an old brief
    // does not invalidate approval simply because display fields were added.
    const before = current.brief || {};
    const manualChanged = mode.value === "manual" && (brief.title !== (before.title ?? current.draft?.title ?? "")
      || brief.criteria !== (before.criteria ?? current.draft?.acceptance_criteria?.join("\n") ?? "")
      || Number(brief.review_reward_usdc) !== Number(before.review_reward_usdc ?? current.draft?.verifier_reward_usdc ?? "0.10")
      || brief.review_consent !== (before.review_consent === true));
    const changed = manualChanged || brief.goal !== (before.goal || current.draft?.goal || current.goal || "")
      || Number(brief.budget_usdc) !== Number(before.budget_usdc || (current.draft ? Number(current.draft.solver_reward_usdc) + Number(current.draft.verifier_reward_usdc) : ""))
      || (Date.parse(brief.deadline_at) || null) !== (Date.parse(before.deadline_at || current.draft?.delivery_deadline) || null);
    if (changed && current.draft) window.AgentBountiesComposer?.invalidate();
    client.save({ ...current, role: "post", goal: brief.goal, brief, draft_stale: current.draft_stale || Boolean(changed && current.draft), updated_at: new Date().toISOString() });
    status.textContent = result.error || (changed && current.draft ? "Brief updated. Prepare the updated proposal before approving funding." : "Brief saved. Prepare your proposal when the details are ready.");
  }
  mode.addEventListener("change", () => {
    const url = new URL(window.location.href); url.searchParams.set("mode", mode.value);
    window.history?.replaceState?.(null, "", url);
    setMode(); save();
  });
  for (const input of [goal, budget, deadline, timezone, title, criteria, reserve, consent]) input.addEventListener("input", save);
  form.addEventListener("submit", async (event) => {
    event.preventDefault(); event.stopImmediatePropagation(); save();
    if (!form.reportValidity()) return;
    const journey = client.load();
    if (mode.value === "manual") {
      const submit = form.querySelector("[data-composer-submit]");
      submit.disabled = true;
      try {
        if (journey?.meta_child || journey?.draft?.meta_child || protectedHandoff) throw new Error("This linked bounty requires its existing reviewed policy. Continue with that proposal.");
        const draft = helper.manualDraft(journey.brief);
        await window.AgentBountiesComposer.stage(draft);
        status.textContent = "Proposal prepared from your form. Review the exact terms and costs below before approving.";
        document.querySelector("#bounty-preview").scrollIntoView({ block: "start", behavior: "smooth" });
      } catch (error) { status.textContent = error.message || "Could not prepare the proposal. Your brief is still saved."; }
      finally { submit.disabled = false; }
      return;
    }
    window.AgentBountyAI.show(goal.value.trim(), { draft: journey?.draft, brief: journey?.brief });
    document.querySelector("[data-ai-options]").open = !document.documentElement.dataset.agentConnected;
  }, true);
  window.addEventListener("agent-bounties:journey", (event) => {
    restore(event.detail);
  });
  window.addEventListener("agent-bounties:site-tool-used", () => {
    document.documentElement.dataset.agentConnected = "true";
    document.querySelector("[data-ai-options]").open = false;
    status.textContent = "Your AI is connected to this workspace. Continue in your current conversation.";
  });
  window.addEventListener("agent-bounties:request-ai-handoff", () => { document.querySelector("[data-ai-options]").open = !document.documentElement.dataset.agentConnected; });
  document.querySelector("[data-export-draft]").addEventListener("click", () => {
    const journey = client.load();
    const blob = new Blob([JSON.stringify(journey?.draft || journey?.brief || {}, null, 2)], { type: "application/json" });
    const url = URL.createObjectURL(blob), link = document.createElement("a");
    link.href = url; link.download = "agent-bounties-draft.json"; link.click(); URL.revokeObjectURL(url);
  });
  restore(client.load());
  if (params.get("mode") === "manual" && !protectedHandoff) { mode.value = "manual"; setMode(); }
  window.setInterval(() => { if (!document.hidden) render(); }, 30000);
  document.addEventListener("visibilitychange", render);
  // Canonical completion belongs to the shared posting operation tracker.
})();
