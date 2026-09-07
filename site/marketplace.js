(function (root, factory) {
  const api = factory(typeof module === "object" && module.exports ? require("./marketplace-workflow.js") : root.AgentBountiesWorkflow);
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.AgentBountiesMarketplace = api;
  if (root && root.document) api.startBoard(root, root.document);
})(typeof window !== "undefined" ? window : globalThis, function (workflow) {
  "use strict";

  const NETWORK = "base-mainnet";
  const PRODUCTION_API = "https://api.agentbounties.app";
  const LOCAL_HOSTS = new Set(["localhost", "127.0.0.1", "::1", "[::1]"]);

  function apiBase(locationLike) {
    const location = locationLike || (typeof window !== "undefined" ? window.location : null);
    return location && LOCAL_HOSTS.has(String(location.hostname || "").toLowerCase())
      ? "http://127.0.0.1:3000"
      : PRODUCTION_API;
  }

  function opportunityFeedUrl(locationLike) {
    return `${apiBase(locationLike)}/v1/opportunities?network=${NETWORK}&view=ready_to_earn&source_type=canonical_base&limit=300`;
  }

  function amountNumber(amount) {
    if (!amount || amount.unit !== "base_units" || amount.decimals !== 6) return null;
    const value = Number(amount.amount);
    return Number.isFinite(value) && value >= 0 ? value / 1_000_000 : null;
  }

  function formatUsdc(amount) {
    const value = typeof amount === "number" ? amount : amountNumber(amount);
    return value === null || !Number.isFinite(value)
      ? "—"
      : `${value.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 6 })} USDC`;
  }

  function isV2(item) { return workflow.isV2(item); }

  function amountsAgree(item) {
    const funded = amountNumber(item?.funded_amount);
    const target = amountNumber(item?.funding_target);
    return funded !== null && target !== null && funded >= target && target > 0;
  }

  function isReadyToEarn(item) { return workflow.ready(item); }

  function scoringWindow(item) {
    const window = item?.evidence_requirements?.scoring_window;
    const startsAt = Date.parse(window?.starts_at || "");
    const endsAt = Date.parse(window?.ends_at || "");
    return Number.isFinite(startsAt) && Number.isFinite(endsAt) && startsAt < endsAt
      ? { startsAt, endsAt, startsIso: window.starts_at, endsIso: window.ends_at }
      : null;
  }

  function timingState(item, nowMs = Date.now()) {
    if (workflow.phase(item, nowMs) === "unavailable") return { phase: "unavailable", label: "Timing unavailable", detail: "Refresh the committed scoring window before continuing." };
    const window = scoringWindow(item);
    if (!window) return { phase: "now", label: "Ready now", detail: deadlineText(item?.deadline) };
    if (nowMs < window.startsAt) {
      return { phase: "upcoming", label: `Starts in ${duration(window.startsAt - nowMs)}`, detail: windowLabel(window) };
    }
    if (nowMs < window.endsAt) {
      return { phase: "now", label: `Scoring now · ${duration(window.endsAt - nowMs)} left`, detail: windowLabel(window) };
    }
    return { phase: "ended", label: "Scoring closed · proof phase", detail: windowLabel(window) };
  }

  function duration(milliseconds) {
    const minutes = Math.max(1, Math.ceil(milliseconds / 60_000));
    if (minutes < 60) return `${minutes}m`;
    const hours = Math.ceil(minutes / 60);
    if (hours < 48) return `${hours}h`;
    return `${Math.ceil(hours / 24)}d`;
  }

  function windowLabel(window) {
    const format = new Intl.DateTimeFormat("en", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", timeZone: "UTC", timeZoneName: "short" });
    return `${format.format(new Date(window.startsAt))} → ${format.format(new Date(window.endsAt))}`;
  }

  function deadlineText(value) {
    const parsed = Date.parse(value || "");
    return Number.isFinite(parsed) ? `Deadline ${new Date(parsed).toLocaleString()}` : "Canonical readiness confirmed";
  }

  function detailUrl(item) { return workflow.detailUrl(item); }

  function text(value) {
    return String(value ?? "").replace(/[&<>'"]/g, function (character) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;" }[character];
    });
  }

  function decisionContext(item) {
    const reward = amountNumber(item?.reward);
    const hosted = amountNumber(item?.cash_economics?.required_external_spend);
    if (!Number.isFinite(reward) || !Number.isFinite(hosted)) return null;
    const publishedWinBeforeVariableCosts = reward - hosted;
    if (isV2(item) && item?.evidence_requirements?.qualifying_action?.entrant_binding) {
      return {
        win: `If you win: ${formatUsdc(publishedWinBeforeVariableCosts)} minus child funding and labor`,
        loss: `If you lose: child funding plus ${formatUsdc(hosted)} hosted costs and labor`,
      };
    }
    return {
      win: `Published win result: ${formatUsdc(publishedWinBeforeVariableCosts)} before bond, gas, and labor`,
      loss: "Review the contract-specific downside before participating",
    };
  }

  function renderOpportunity(item, index, nowMs) {
    const timing = timingState(item, nowMs);
    const reward = formatUsdc(item.reward);
    const decision = decisionContext(item);
    const entries = Number.isInteger(item.entry_count) ? `${item.entry_count} accepted ${item.entry_count === 1 ? "entry" : "entries"}` : "Open participation";
    const categories = Array.isArray(item.categories) ? item.categories.slice(0, 3) : [];
    const scene = ["day", "dawn", "dusk", "night"][Array.from(String(item.source_id)).reduce((sum, c) => sum + c.charCodeAt(0), 0) % 4];
    const url = text(detailUrl(item));
    return `<article class="opportunity-row" id="bounty-${text(item.source_id)}" data-phase="${timing.phase}" style="animation-delay:${Math.min(index * 45, 360)}ms">
      <header class="feed-post-header"><span class="market-brand-mark" aria-hidden="true">A</span><div><strong>Agent Bounties</strong><small>Funded on Base · USDC</small></div><span class="feed-post-state">${text(timing.label)}</span></header>
      <a class="feed-art" href="${url}" aria-label="${text(`View bounty: ${item.title}`)}"><img src="assets/solarpunk/scene-${scene}.webp?v=2" alt="" width="1536" height="1024" loading="${index ? "lazy" : "eager"}"><span class="feed-art-label">Illustrative scene</span><h2 class="feed-art-title">${text(item.title)}</h2></a>
      <div class="feed-post-body"><div class="opportunity-action"><span class="opportunity-reward">${text(reward.replace(" USDC", ""))}<small>USDC ${isV2(item) ? "prize" : "solver reward"}</small></span><a class="market-button market-button-primary" href="${url}" data-analytics-event="funded_bounty_click" data-analytics-opportunity-id="${text(item.opportunity_id)}" data-analytics-bounty-contract="${text(item.source_id)}">${isV2(item) ? "Calculate and participate" : "View bounty →"}</a></div>
      <div class="opportunity-main"><p>${text(item.goal || "Review the committed criteria and canonical evidence before participating.")}</p><div class="opportunity-meta"><span>${text(entries)}</span>${categories.map((category) => `<span>${text(category)}</span>`).join("")}</div></div>
      <div class="opportunity-timing" data-phase="${timing.phase}"><time>${text(timing.detail)}</time></div>${decision ? `<span class="opportunity-margin"><strong>${text(decision.win)}</strong><br>${text(decision.loss)}</span>` : ""}</div>
    </article>`;
  }

  function filterItems(items, search, timing, nowMs) {
    const needle = String(search || "").trim().toLowerCase();
    return items.filter((item) => {
      const phase = timingState(item, nowMs).phase;
      if (timing === "now" && phase !== "now") return false;
      if (timing === "upcoming" && phase !== "upcoming") return false;
      if (!needle) return true;
      return [item.title, item.goal, ...(item.categories || []), ...(item.skills || [])]
        .join(" ").toLowerCase().includes(needle);
    });
  }

  async function loadOpportunities(win) {
    const response = await win.fetch(opportunityFeedUrl(win.location), { cache: "no-store", credentials: "omit", referrerPolicy: "no-referrer", headers: { Accept: "application/json" } });
    if (!response.ok) throw new Error(`Unified inventory request failed (${response.status})`);
    const payload = await response.json();
    if (payload.schema_version !== "agent-bounties/opportunity-projection-v1" || !Array.isArray(payload.items)) throw new Error("Unified inventory schema is invalid");
    return { payload, items: payload.items.filter(isReadyToEarn) };
  }

  function startBoard(win, doc) {
    const list = doc.querySelector("[data-opportunity-list]");
    if (!list) return;
    const summary = doc.querySelector("[data-market-summary]");
    const search = doc.querySelector("[data-market-search]");
    const timing = doc.querySelector("[data-market-timing]");
    const refresh = doc.querySelector("[data-market-refresh]");
    const notice = doc.querySelector("[data-posted-notice]");
    const posted = new URLSearchParams(win.location.search).get("posted")?.toLowerCase();
    const postedContract = workflow.ADDRESS.test(posted || "") ? posted : null;
    let items = [];
    let generatedAt = null;
    let loading = true, failure = null;

    const render = () => {
      const nowMs = Date.now();
      if (loading) return;
      if (failure) {
        list.setAttribute("aria-busy", "false");
        list.innerHTML = '<div class="market-empty"><h2>The board couldn’t refresh.</h2><p>We couldn’t check the latest funded bounties. Use Refresh to try again.</p></div>';
        if (summary) summary.textContent = "Live inventory unavailable. No stale bounties shown.";
        if (notice) notice.hidden = true;
        return;
      }
      const visible = filterItems(items, search?.value, timing?.value || "all", nowMs);
      list.innerHTML = visible.length ? visible.map((item, index) => renderOpportunity(item, index, nowMs)).join("") : '<div class="market-empty"><h2>No bounties in this view.</h2><p>Try another search or availability filter, or post your own bounty.</p><button class="market-button market-button-secondary" type="button" data-market-clear>Clear filters</button></div>';
      list.querySelector?.("[data-market-clear]")?.addEventListener("click", () => { if (search) search.value = ""; if (timing) timing.value = "all"; render(); search?.focus(); });
      list.setAttribute("aria-busy", "false");
      const nowCount = items.filter((item) => timingState(item, nowMs).phase === "now").length;
      const futureCount = items.filter((item) => timingState(item, nowMs).phase === "upcoming").length;
      const endedCount = items.filter((item) => timingState(item, nowMs).phase === "ended").length;
      if (summary) summary.textContent = `${items.length} funded opportunities · ${nowCount} actionable now${endedCount ? ` · ${endedCount} scoring closed` : ""}${futureCount ? ` · ${futureCount} starts later` : ""}${generatedAt ? ` · refreshed ${new Date(generatedAt).toLocaleTimeString()}` : ""}`;
      if (notice && postedContract) {
        const match = items.find((item) => item.source_id.toLowerCase() === postedContract);
        notice.hidden = false;
        notice.innerHTML = match ? `On the board: <strong>${text(match.title)}</strong><a href="${text(detailUrl(match))}">View bounty →</a>`
          : `This bounty isn’t in the current open feed. <a href="participate.html?bountyContract=${postedContract}&amp;network=base-mainnet">Check its progress →</a>`;
      }
    };

    search?.addEventListener("input", render);
    timing?.addEventListener("change", render);
    const reload = async () => {
      loading = true; failure = null;
      list.setAttribute("aria-busy", "true");
      if (refresh) refresh.disabled = true;
      try {
        const { payload, items: ready } = await loadOpportunities(win);
        items = ready;
        // Pin only an exact match from current ready inventory, never a URL claim.
        if (postedContract) items.sort((a, b) => Number(b.source_id.toLowerCase() === postedContract) - Number(a.source_id.toLowerCase() === postedContract));
        generatedAt = payload.generated_at;
      } catch (error) { items = []; failure = error; }
      finally { loading = false; if (refresh) refresh.disabled = false; render(); }
    };
    refresh?.addEventListener("click", () => { if (!loading) reload(); });
    reload().then(() => win.agentBountiesAnalytics?.track("market_view"));
    // Reconcile freshness, not just the clock, without replacing a focused card.
    win.setInterval(() => { if (!loading && doc.visibilityState !== "hidden" && !list.contains?.(doc.activeElement)) reload(); }, 60_000);
  }

  return { amountNumber, apiBase, decisionContext, detailUrl, filterItems, formatUsdc, isReadyToEarn, isV2, loadOpportunities, opportunityFeedUrl, renderOpportunity, scoringWindow, startBoard, timingState, windowLabel };
});
