(() => {
  "use strict";

  const state = {
    items: [],
    query: "",
    filter: "all",
    minReward: 0,
    sort: "newest",
  };

  const byId = (id) => document.getElementById(id);
  const one = (selector) => document.querySelector(selector);

  function amountInUsdc(value) {
    if (!value || String(value.currency || "").toUpperCase() !== "USDC") return 0;
    const amount = Number(value.amount || 0);
    if (!Number.isFinite(amount)) return 0;
    if (value.unit === "base_units") return amount / 1_000_000;
    if (value.unit === "minor_units") return amount / (10 ** Number(value.decimals || 6));
    return amount;
  }

  function formatUsdc(value) {
    const number = Number(value || 0);
    if (!Number.isFinite(number)) return "0";
    return number.toLocaleString(undefined, {
      minimumFractionDigits: number < 10 ? 2 : 0,
      maximumFractionDigits: 2,
    });
  }

  function stateFor(item) {
    if (item.payment_state === "seeking_funding") return "funding";
    if (item.work_state === "claimable" && item.payment_committed && item.verification_ready) {
      return "claimable";
    }
    if (["in_progress", "submitted"].includes(item.work_state)) return "in-progress";
    if (item.payment_state === "none") return "unfunded";
    return "open";
  }

  function stateLabel(value) {
    return {
      claimable: "Ready to work",
      funding: "Needs funding",
      "in-progress": "In progress",
      unfunded: "No reward yet",
      open: "Open",
    }[value] || "Open";
  }

  function normalize(item) {
    const reward = amountInUsdc(item.reward) + amountInUsdc(item.completion_bonus);
    const funded = amountInUsdc(item.funded_amount);
    const target = amountInUsdc(item.funding_target);
    return {
      ...item,
      boardState: stateFor(item),
      rewardUsdc: reward,
      fundedUsdc: funded,
      targetUsdc: target,
      searchable: [
        item.title,
        item.goal,
        ...(item.categories || []),
        ...(item.skills || []),
      ].filter(Boolean).join(" ").toLowerCase(),
    };
  }

  function isOpen(item) {
    return !["completed", "paid", "cancelled", "refunded", "expired"].includes(item.work_state)
      && !["paid", "refunded"].includes(item.payment_state)
      && !["paid", "cancelled", "refunded", "expired"].includes(item.source_status);
  }

  function filteredItems() {
    const query = state.query.trim().toLowerCase();
    const result = state.items.filter((item) => {
      if (query && !item.searchable.includes(query)) return false;
      if (state.filter !== "all" && item.boardState !== state.filter) return false;
      if (item.rewardUsdc < state.minReward) return false;
      return true;
    });

    result.sort((left, right) => {
      if (state.sort === "reward") return right.rewardUsdc - left.rewardUsdc;
      if (state.sort === "title") return left.title.localeCompare(right.title);
      return Date.parse(right.updated_at || right.created_at || 0)
        - Date.parse(left.updated_at || left.created_at || 0);
    });
    return result;
  }

  function safeLink(value) {
    try {
      const url = new URL(value, window.location.href);
      return ["https:", "http:"].includes(url.protocol) ? url.href : null;
    } catch (_error) {
      return null;
    }
  }

  function buttonLink(label, href, primary = false) {
    const link = document.createElement("a");
    link.className = `button ${primary ? "primary" : "secondary"}`;
    link.href = href;
    link.textContent = label;
    return link;
  }

  function openFunding(item) {
    const panel = byId("fund-bounty-panel");
    const form = byId("autonomous-fund-form");
    if (!panel || !form) return;
    panel.open = true;
    form.elements.bountyContract.value = item.source_id;
    const remaining = Math.max(0, item.targetUsdc - item.fundedUsdc);
    if (remaining > 0) form.elements.amount.value = Math.min(remaining, 1).toFixed(2);
    const title = panel.querySelector("[data-funding-title]");
    if (title) title.textContent = `Help fund: ${item.title}`;
    panel.scrollIntoView({ behavior: "smooth", block: "start" });
  }

  function cardFor(item) {
    const article = document.createElement("article");
    article.className = "board-task-card";
    article.dataset.boardState = item.boardState;

    const statePill = document.createElement("span");
    statePill.className = "board-state-pill";
    statePill.dataset.state = item.boardState;
    statePill.textContent = stateLabel(item.boardState);

    const heading = document.createElement("h2");
    heading.textContent = item.title || "Untitled task";

    const goal = document.createElement("p");
    goal.textContent = item.goal || "Open the task to read what needs to be done.";

    const meta = document.createElement("div");
    meta.className = "board-task-meta";
    if (item.rewardUsdc > 0) {
      const reward = document.createElement("span");
      reward.textContent = `${formatUsdc(item.rewardUsdc)} USDC reward`;
      meta.append(reward);
    }
    if (item.boardState === "funding" && item.targetUsdc > 0) {
      const funding = document.createElement("span");
      funding.textContent = `${formatUsdc(item.fundedUsdc)} of ${formatUsdc(item.targetUsdc)} USDC funded`;
      meta.append(funding);
    }
    (item.categories || []).slice(0, 2).forEach((category) => {
      const tag = document.createElement("span");
      tag.textContent = category;
      meta.append(tag);
    });

    const actions = document.createElement("div");
    actions.className = "board-task-actions";

    if (item.source_type === "canonical_base" && item.boardState === "claimable") {
      actions.append(buttonLink(
        "Claim task",
        `earn.html?bountyContract=${encodeURIComponent(item.source_id)}#claim-workflow`,
        true,
      ));
    }

    if (item.source_type === "canonical_base" && item.boardState === "funding") {
      const fund = document.createElement("button");
      fund.className = "button primary";
      fund.type = "button";
      fund.textContent = "Help fund";
      fund.addEventListener("click", () => openFunding(item));
      actions.append(fund);
    }

    const detailUrl = safeLink(item.source_url || item.public_url);
    if (detailUrl) {
      const details = buttonLink("View details", detailUrl);
      details.rel = "noopener noreferrer";
      actions.append(details);
    }

    article.append(statePill, heading, goal, meta, actions);
    return article;
  }

  function render() {
    const container = byId("all-open-task-feed");
    const count = one("[data-board-count]");
    if (!container) return;
    const items = filteredItems();
    container.replaceChildren();
    if (count) count.textContent = `${items.length} open ${items.length === 1 ? "task" : "tasks"}`;
    if (!items.length) {
      const empty = document.createElement("p");
      empty.className = "board-empty";
      empty.textContent = "No open tasks match these filters.";
      container.append(empty);
      return;
    }
    items.forEach((item) => container.append(cardFor(item)));
  }

  function bindFilters() {
    const search = one("[data-board-search]");
    const filter = one("[data-board-state]");
    const minimum = one("[data-board-min-reward]");
    const sort = one("[data-board-sort]");
    const params = new URLSearchParams(window.location.search);

    state.query = params.get("q") || "";
    state.filter = params.get("filter") || "all";
    if (!["all", "claimable", "funding", "in-progress", "unfunded"].includes(state.filter)) {
      state.filter = "all";
    }

    if (search) {
      search.value = state.query;
      search.addEventListener("input", () => {
        state.query = search.value;
        render();
      });
    }
    if (filter) {
      filter.value = state.filter;
      filter.addEventListener("change", () => {
        state.filter = filter.value;
        render();
      });
    }
    if (minimum) {
      minimum.addEventListener("input", () => {
        state.minReward = Math.max(0, Number(minimum.value || 0));
        render();
      });
    }
    if (sort) {
      sort.addEventListener("change", () => {
        state.sort = sort.value;
        render();
      });
    }

    const claimWorkflow = byId("claim-workflow");
    const hasTarget = params.has("bountyContract");
    if (claimWorkflow) claimWorkflow.hidden = !hasTarget;
    if (hasTarget) {
      window.requestAnimationFrame(() => claimWorkflow?.scrollIntoView({ block: "start" }));
    }
    if (window.location.hash === "#fund") {
      const panel = byId("fund-bounty-panel");
      if (panel) panel.open = true;
    }
  }

  async function load() {
    const container = byId("all-open-task-feed");
    if (!container) return;
    bindFilters();
    try {
      const protocolResponse = await fetch("protocol.json", { cache: "no-store" });
      if (!protocolResponse.ok) throw new Error("The Bounty Board is temporarily unavailable.");
      const protocol = await protocolResponse.json();
      const api = String(protocol.api_base_url || "").replace(/\/$/, "");
      const response = await fetch(`${api}/v1/opportunities?network=base-mainnet&limit=300`, {
        cache: "no-store",
      });
      if (!response.ok) throw new Error("The Bounty Board could not load open tasks.");
      const body = await response.json();
      state.items = (body.items || []).map(normalize).filter(isOpen);
      render();
    } catch (error) {
      container.replaceChildren();
      const message = document.createElement("p");
      message.className = "board-empty";
      message.textContent = error.message || String(error);
      container.append(message);
    }
  }

  document.addEventListener("DOMContentLoaded", load);
})();
