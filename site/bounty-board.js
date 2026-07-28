(() => {
  "use strict";

  const state = {
    items: [],
    query: "",
    minReward: 0,
    sort: "newest",
    refreshing: false,
    lastSync: 0,
  };
  const REFRESH_MS = 15_000;
  const STREAM_STALE_MS = 35_000;

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

  function normalize(item) {
    const reward = amountInUsdc(item.reward) + amountInUsdc(item.completion_bonus);
    return {
      ...item,
      boardState: "claimable",
      rewardUsdc: reward,
      searchable: [
        item.title,
        item.goal,
        ...(item.categories || []),
        ...(item.skills || []),
      ].filter(Boolean).join(" ").toLowerCase(),
    };
  }

  function isReadyToEarn(item) {
    return item.source_type === "canonical_base"
      && item.source_status === "claimable"
      && item.work_state === "claimable"
      && item.payment_state === "escrowed"
      && item.payment_committed === true
      && item.verification_ready === true
      && Boolean(item.terms_hash)
      && amountInUsdc(item.funded_amount) >= amountInUsdc(item.funding_target)
      && amountInUsdc(item.reward) > 0;
  }

  function filteredItems() {
    const query = state.query.trim().toLowerCase();
    const result = state.items.filter((item) => {
      if (query && !item.searchable.includes(query)) return false;
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

  function cardFor(item) {
    const article = document.createElement("article");
    article.className = "board-task-card";
    article.dataset.boardState = item.boardState;

    const statePill = document.createElement("span");
    statePill.className = "board-state-pill";
    statePill.dataset.state = item.boardState;
    statePill.textContent = "Ready to work";

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
    (item.categories || []).slice(0, 2).forEach((category) => {
      const tag = document.createElement("span");
      tag.textContent = category;
      meta.append(tag);
    });

    const actions = document.createElement("div");
    actions.className = "board-task-actions";

    const claim = buttonLink(
      "Claim task",
      `earn.html?bountyContract=${encodeURIComponent(item.source_id)}&source=bounty-board#claim-workflow`,
      true,
    );
    claim.dataset.analyticsEvent = "funded_bounty_click";
    claim.dataset.analyticsOpportunityId = item.opportunity_id;
    claim.dataset.analyticsBountyContract = item.source_id;
    actions.append(claim);

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
    const updated = one("[data-board-updated]");
    if (!container) return;
    const items = filteredItems();
    container.replaceChildren();
    if (count) {
      count.textContent = `${items.length} funded ${items.length === 1 ? "bounty" : "bounties"} ready to claim`;
    }
    if (updated) {
      updated.textContent = `Live sync ${new Date(state.lastSync).toLocaleTimeString()} - refreshes every 15 seconds`;
    }
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
    const minimum = one("[data-board-min-reward]");
    const sort = one("[data-board-sort]");
    const params = new URLSearchParams(window.location.search);

    state.query = params.get("q") || "";

    if (search) {
      search.value = state.query;
      search.addEventListener("input", () => {
        state.query = search.value;
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
    const targetContract = params.get("bountyContract") || "";
    const hasTarget = /^0x[0-9a-fA-F]{40}$/.test(targetContract);
    const fundingMode = window.location.hash === "#fund";
    if (claimWorkflow) claimWorkflow.hidden = !hasTarget || fundingMode;
    if (hasTarget && !fundingMode) {
      window.requestAnimationFrame(() => claimWorkflow?.scrollIntoView({ block: "start" }));
    }
    if (fundingMode) {
      const panel = byId("fund-bounty-panel");
      const form = byId("autonomous-fund-form");
      if (panel) panel.open = true;
      if (form && hasTarget) form.elements.bountyContract.value = targetContract;
      window.requestAnimationFrame(() => panel?.scrollIntoView({ block: "start" }));
    }
  }

  function applyProjection(body) {
    if (body.applied_view !== "ready_to_earn" || body.degraded) {
      throw new Error("Live earning inventory is not authoritative.");
    }
    const items = body.items || [];
    if (items.some((item) => !isReadyToEarn(item))) {
      throw new Error("Live inventory failed its claimability gate.");
    }
    state.items = items.map(normalize);
    state.lastSync = Date.now();
    render();
  }

  function showUnavailable(error) {
    const container = byId("all-open-task-feed");
    if (!container) return;
    state.items = [];
    container.replaceChildren();
    const message = document.createElement("p");
    message.className = "board-empty";
    message.textContent = `${error.message || String(error)} Retrying automatically; no stale bounty is shown.`;
    container.append(message);
    const count = one("[data-board-count]");
    const updated = one("[data-board-updated]");
    if (count) count.textContent = "Live earning inventory unavailable";
    if (updated) updated.textContent = "Reconnecting automatically";
  }

  async function refresh() {
    const container = byId("all-open-task-feed");
    if (!container || state.refreshing) return;
    state.refreshing = true;
    try {
      const protocolResponse = await fetch("protocol.json", { cache: "no-store" });
      if (!protocolResponse.ok) throw new Error("The Bounty Board is temporarily unavailable.");
      const protocol = await protocolResponse.json();
      const api = String(protocol.api_base_url || "").replace(/\/$/, "");
      const url = `${api}/v1/opportunities?network=base-mainnet&view=ready_to_earn&source_type=canonical_base&limit=300&live=${Date.now()}`;
      const response = await fetch(url, {
        cache: "no-store",
        headers: { "Cache-Control": "no-cache" },
      });
      if (!response.ok) throw new Error("Live earning inventory is unavailable.");
      applyProjection(await response.json());
    } catch (error) {
      showUnavailable(error);
    } finally {
      state.refreshing = false;
    }
  }

  async function connectStream() {
    const protocolResponse = await fetch("protocol.json", { cache: "no-store" });
    if (!protocolResponse.ok) throw new Error("The Bounty Board is temporarily unavailable.");
    const protocol = await protocolResponse.json();
    const api = String(protocol.api_base_url || "").replace(/\/$/, "");
    const url = `${api}/v1/opportunities/stream?network=base-mainnet&view=ready_to_earn&source_type=canonical_base&limit=300&live=${Date.now()}`;
    const stream = new EventSource(url);
    stream.addEventListener("inventory", (event) => {
      try {
        applyProjection(JSON.parse(event.data));
      } catch (error) {
        showUnavailable(error);
      }
    });
    stream.addEventListener("projection_error", () => {
      showUnavailable(new Error("Canonical inventory is unavailable."));
    });
    stream.onerror = () => {
      if (Date.now() - state.lastSync >= STREAM_STALE_MS) {
        showUnavailable(new Error("Live inventory stream disconnected."));
      }
    };
  }

  function load() {
    if (!byId("all-open-task-feed")) return;
    bindFilters();
    refresh();
    connectStream().catch(showUnavailable);
    window.setInterval(() => {
      if (!document.hidden && Date.now() - state.lastSync >= STREAM_STALE_MS) refresh();
    }, REFRESH_MS);
    document.addEventListener("visibilitychange", () => {
      if (!document.hidden && Date.now() - state.lastSync >= REFRESH_MS) refresh();
    });
    window.addEventListener("online", refresh);
  }

  document.addEventListener("DOMContentLoaded", load);
})();
