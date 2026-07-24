import { App } from "@modelcontextprotocol/ext-apps/app-with-deps";

(() => {
  "use strict";

  let current = null;
  let connected = false;
  const byId = (id) => document.getElementById(id);
  const app = new App(
    { name: "Agent Bounties bounty review", version: "1.0.0" },
    {},
    { autoResize: true },
  );

  function render(value) {
    if (!value || typeof value !== "object") return;
    current = value;
    byId("title").textContent = value.title || "Review task";
    byId("goal").textContent = value.goal || "No measurable objective was supplied.";
    byId("criteria").replaceChildren(...(Array.isArray(value.acceptance_criteria)
      ? value.acceptance_criteria.map((criterion) => {
          const item = document.createElement("li");
          item.textContent = String(criterion);
          return item;
        })
      : []));
    byId("solver").textContent = `${value.solver_reward_usdc || "—"} USDC`;
    byId("verifier").textContent = `${value.verifier_reward_usdc || "—"} USDC`;
    byId("window").textContent = `${value.task_window_days || 30} days`;
    byId("initial").textContent = `${value.initial_funding_usdc ?? "—"} USDC`;
    byId("continue").disabled = !value.post_url;
    byId("status").textContent = value.crowdfund
      ? "Unfunded posting selected: no USDC will be deposited now and no payment is promised yet."
      : `Full funding is currently selected (${value.target_usdc || "—"} USDC). On the next page, choose “Post with 0 USDC now” to publish without committing a reward.`;
  }

  app.ontoolresult = (params) => render(params.structuredContent);
  app.onhostcontextchanged = (context) => {
    if (context?.theme) document.documentElement.dataset.theme = context.theme;
  };

  render(window.openai?.toolOutput);
  window.addEventListener("openai:set_globals", (event) => {
    render(event.detail?.globals?.toolOutput ?? window.openai?.toolOutput);
  }, { passive: true });

  byId("continue").addEventListener("click", async () => {
    if (!current?.post_url) return;
    byId("status").textContent = "Opening the secure wallet handoff…";
    try {
      if (connected) {
        const result = await app.openLink({ url: current.post_url });
        if (!result?.isError) return;
      }
      if (window.openai?.openExternal) {
        await window.openai.openExternal({ href: current.post_url });
        return;
      }
      window.open(current.post_url, "_blank", "noopener,noreferrer");
    } catch (_error) {
      byId("status").textContent = `Open this secure review URL: ${current.post_url}`;
    }
  });

  void app.connect().then(() => {
    connected = true;
    const context = app.getHostContext();
    if (context?.theme) document.documentElement.dataset.theme = context.theme;
  }).catch(() => {
    if (!window.openai) {
      byId("status").textContent = "The interactive card could not connect. The same draft and secure review URL remain available in the conversation.";
    }
  });
})();
