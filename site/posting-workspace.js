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
      const split = helper.proposedSplit(budget.value);
      const matching = draft && helper.cents(draft.solver_reward_usdc) + helper.cents(draft.verifier_reward_usdc) === helper.cents(budget.value);
      budgetSummary.textContent = matching ? (draft.review_mode === "creator" ? `Your proposal: ${draft.solver_reward_usdc} USDC to the solver + ${draft.verifier_reward_usdc} USDC creator-review reserve. The reserve is paid back to you after either signed verdict. You confirm the verdict.` : `Your proposal: ${draft.solver_reward_usdc} USDC to the solver + ${draft.verifier_reward_usdc} USDC verifier reward.`) : (split ? `For creator review, suggested split: ${split.solver} USDC to the solver + ${split.reserve} USDC creator-review reserve (90% / 10%, rounded to cents). The reserve is paid back to you after either signed verdict. Your AI will propose the review method; you approve the final amounts.` : "Enter the combined solver and review budget. Both rewards must be positive.");
    }
    if (warningList) {
      warningList.replaceChildren();
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
    status.textContent = journey.draft ? "Draft saved on this device. Review the proposal below." : "Brief saved on this device. Your connected AI can use these answers.";
    restoring = false;
    render();
  }
  function save() {
    if (restoring) return;
    const current = client.load() || client.start({ role: "post" });
    const result = render();
    const brief = { goal: goal.value.trim(), budget_usdc: budget.value, deadline_at: result.iso, deadline_local: deadline.value, timezone: timezone.value.trim() };
    // Compare decisions, not display formatting, so restoring an old brief
    // does not invalidate approval simply because display fields were added.
    const before = current.brief || {};
    const changed = brief.goal !== (before.goal || current.draft?.goal || current.goal || "")
      || Number(brief.budget_usdc) !== Number(before.budget_usdc || (current.draft ? Number(current.draft.solver_reward_usdc) + Number(current.draft.verifier_reward_usdc) : ""))
      || (Date.parse(brief.deadline_at) || null) !== (Date.parse(before.deadline_at || current.draft?.delivery_deadline) || null);
    if (changed && current.draft) window.AgentBountiesComposer?.invalidate();
    client.save({ ...current, role: "post", goal: brief.goal, brief, draft_stale: current.draft_stale || Boolean(changed && current.draft), updated_at: new Date().toISOString() });
    status.textContent = result.error || (changed && current.draft ? "Brief updated. Your AI must update the proposal before you can approve funding." : "Brief saved. Continue with your AI in the same conversation.");
  }
  for (const input of [goal, budget, deadline, timezone]) input.addEventListener("input", save);
  form.addEventListener("submit", (event) => {
    event.preventDefault(); event.stopImmediatePropagation(); save();
    if (!form.reportValidity()) return;
    const journey = client.load();
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
  window.setInterval(() => { if (!document.hidden) render(); }, 30000);
  document.addEventListener("visibilitychange", render);
  // Canonical completion belongs to the shared posting operation tracker.
})();
