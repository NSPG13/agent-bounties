(() => {
  "use strict";
  const flow = window.AgentBountiesWorkflow;
  const client = flow.createClient(window);
  const form = document.querySelector("#bounty-composer-form");
  const goal = document.querySelector("#bounty-composer-input");
  const budget = document.querySelector("[data-brief-budget]");
  const deadline = document.querySelector("[data-brief-deadline]");
  const status = document.querySelector("[data-brief-status]");
  const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  document.querySelector("[data-brief-timezone]").textContent = timezone;
  let restoring = false;
  // Keep the wallet utility in the header, clear of proposal actions.
  const launcher = document.querySelector(".ab-phone-launcher");
  if (launcher) document.querySelector("[data-posting-wallet]").append(launcher);
  const actions = document.querySelector(".bounty-card-actions");
  if (actions) {
    const measureActions = () => document.documentElement.style.setProperty("--posting-actions-height", `${Math.ceil(actions.getBoundingClientRect().height)}px`);
    if (typeof ResizeObserver === "function") new ResizeObserver(measureActions).observe(actions);
    window.addEventListener("resize", measureActions);
    measureActions();
  }
  function localDate(value) {
    const date = new Date(value);
    return Number.isFinite(date.getTime()) ? new Date(date.getTime() - date.getTimezoneOffset() * 60000).toISOString().slice(0, 16) : "";
  }
  function restore(journey) {
    if (!journey || journey.role !== "post") return;
    restoring = true;
    const brief = journey.brief || {};
    if (document.activeElement !== goal) goal.value = brief.goal || journey.draft?.goal || journey.goal || "";
    if (document.activeElement !== budget) budget.value = brief.budget_usdc || (journey.draft ? String(Number(journey.draft.solver_reward_usdc) + Number(journey.draft.verifier_reward_usdc)) : "");
    if (document.activeElement !== deadline) deadline.value = localDate(brief.deadline_at || journey.draft?.delivery_deadline || "");
    status.textContent = journey.draft ? "Draft saved in this browser. Review it below; it has not been posted." : "Brief saved. Your connected AI can use these answers.";
    restoring = false;
  }
  function save() {
    if (restoring) return;
    const current = client.load() || client.start({ role: "post" });
    const brief = { goal: goal.value.trim(), budget_usdc: budget.value, deadline_at: deadline.value ? new Date(deadline.value).toISOString() : null, timezone };
    const changed = JSON.stringify(brief) !== JSON.stringify(current.brief);
    if (changed && current.draft) window.AgentBountiesComposer?.invalidate();
    client.save({ ...current, role: "post", goal: brief.goal, brief, draft_stale: current.draft_stale || Boolean(changed && current.draft), updated_at: new Date().toISOString() });
    status.textContent = changed && current.draft ? "Brief updated. Your AI must update the proposal before you can approve funding." : "Brief saved. Continue with your AI in the same conversation.";
  }
  for (const input of [goal, budget, deadline]) input.addEventListener("input", save);
  form.addEventListener("submit", (event) => {
    event.preventDefault(); event.stopImmediatePropagation(); save();
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
  const operation = flow.createPostingJournal(window).load();
  if (operation?.bounty_contract) {
    client.request("/v1/base/autonomous-bounties/feed?network=base-mainnet&claimable_only=false").then((feed) => {
      const item = feed.find((entry) => entry.bounty_id === operation.bounty_id && entry.bounty_contract.toLowerCase() === operation.bounty_contract.toLowerCase());
      if (!item?.terms_valid || !["claimable", "claimed", "submitted", "paid"].includes(item.status) || BigInt(item.funded_amount) < BigInt(item.target_amount)) {
        status.textContent = "A posting step is recorded; canonical funding is not confirmed. Ask your AI to check it before trying again."; return;
      }
      status.textContent = `Your bounty is funded. Current state: ${item.status}. `;
      const link = document.createElement("a"); link.href = `participate.html?bountyContract=${encodeURIComponent(item.bounty_contract)}&network=base-mainnet`; link.textContent = "Open your bounty"; status.append(link);
    }).catch(() => { status.textContent = "A posting step is recorded. Funding status is temporarily unavailable; keep the same operation."; });
  }
})();
