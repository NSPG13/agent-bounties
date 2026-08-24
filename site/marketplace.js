(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.AgentBountiesMarketplace = api;
  if (root && root.document) api.startBoard(root, root.document);
})(typeof window !== "undefined" ? window : globalThis, function () {
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

  function isV2(item) {
    return String(item?.opportunity_id || "").startsWith("open-competition-v2:")
      || String(item?.next_action?.action || "").includes("open_competition_v2");
  }

  function amountsAgree(item) {
    const funded = amountNumber(item?.funded_amount);
    const target = amountNumber(item?.funding_target);
    return funded !== null && target !== null && funded >= target && target > 0;
  }

  function isReadyToEarn(item) {
    if (!item || item.source_type !== "canonical_base") return false;
    const shared = item.work_state === "claimable"
      && item.payment_state === "escrowed"
      && item.payment_committed === true
      && item.verification_ready === true
      && amountNumber(item.reward) > 0
      && amountsAgree(item);
    if (!shared) return false;
    if (isV2(item)) {
      return item.source_status === "active"
        && ["best_score", "first_proven"].includes(item.competition_mode)
        && Boolean(item.evidence_requirements?.program_profile)
        && Boolean(item.evidence_requirements?.verification_policy_hash);
    }
    return item.source_status === "claimable" && Boolean(item.terms_hash);
  }

  function scoringWindow(item) {
    const window = item?.evidence_requirements?.scoring_window;
    const startsAt = Date.parse(window?.starts_at || "");
    const endsAt = Date.parse(window?.ends_at || "");
    return Number.isFinite(startsAt) && Number.isFinite(endsAt) && startsAt < endsAt
      ? { startsAt, endsAt, startsIso: window.starts_at, endsIso: window.ends_at }
      : null;
  }

  function timingState(item, nowMs = Date.now()) {
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

  function detailUrl(item) {
    if (isV2(item)) {
      const network = encodeURIComponent(item.network || NETWORK);
      return `competition.html?bountyContract=${encodeURIComponent(item.source_id)}&network=${network}`;
    }
    return item.public_url || item.next_action?.url || "#";
  }

  function text(value) {
    return String(value ?? "").replace(/[&<>'"]/g, function (character) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;" }[character];
    });
  }

  function renderOpportunity(item, index, nowMs) {
    const timing = timingState(item, nowMs);
    const reward = formatUsdc(item.reward);
    const margin = item.cash_economics ? formatUsdc(item.cash_economics.gross_cash_margin) : null;
    const entries = Number.isInteger(item.entry_count) ? `${item.entry_count} accepted ${item.entry_count === 1 ? "entry" : "entries"}` : "Open participation";
    const categories = Array.isArray(item.categories) ? item.categories.slice(0, 3) : [];
    return `<article class="opportunity-row" data-phase="${timing.phase}" style="animation-delay:${Math.min(index * 45, 360)}ms">
      <div class="opportunity-timing" data-phase="${timing.phase}"><strong>${text(timing.label)}</strong><time>${text(timing.detail)}</time></div>
      <div class="opportunity-main"><h2>${text(item.title)}</h2><p>${text(item.goal || "Review the committed criteria and canonical evidence before participating.")}</p><div class="opportunity-meta"><span>${text(entries)}</span>${categories.map((category) => `<span>${text(category)}</span>`).join("")}</div></div>
      <div class="opportunity-action"><span class="opportunity-reward">${text(reward.replace(" USDC", ""))} <small>USDC prize</small></span>${margin ? `<span class="opportunity-margin">${text(margin)} published margin if you win<br>before task capital and labor</span>` : ""}<a class="market-button market-button-primary" href="${text(detailUrl(item))}" data-analytics-event="funded_bounty_click" data-analytics-opportunity-id="${text(item.opportunity_id)}" data-analytics-bounty-contract="${text(item.source_id)}">Review and participate</a></div>
    </article>`;
  }

  function filterItems(items, search, timing, nowMs) {
    const needle = String(search || "").trim().toLowerCase();
    return items.filter((item) => {
      const phase = timingState(item, nowMs).phase;
      if (timing === "now" && phase === "upcoming") return false;
      if (timing === "upcoming" && phase !== "upcoming") return false;
      if (!needle) return true;
      return [item.title, item.goal, ...(item.categories || []), ...(item.skills || [])]
        .join(" ").toLowerCase().includes(needle);
    });
  }

  async function loadOpportunities(win) {
    const response = await win.fetch(opportunityFeedUrl(win.location), { credentials: "omit", referrerPolicy: "no-referrer", headers: { Accept: "application/json" } });
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
    let items = [];
    let generatedAt = null;

    const render = () => {
      const nowMs = Date.now();
      const visible = filterItems(items, search?.value, timing?.value || "all", nowMs);
      list.innerHTML = visible.length ? visible.map((item, index) => renderOpportunity(item, index, nowMs)).join("") : '<p class="market-empty">No funded opportunity matches this view.</p>';
      list.setAttribute("aria-busy", "false");
      const nowCount = items.filter((item) => timingState(item, nowMs).phase !== "upcoming").length;
      const futureCount = items.length - nowCount;
      if (summary) summary.textContent = `${items.length} funded opportunities · ${nowCount} actionable now${futureCount ? ` · ${futureCount} starts later` : ""}${generatedAt ? ` · refreshed ${new Date(generatedAt).toLocaleTimeString()}` : ""}`;
    };

    search?.addEventListener("input", render);
    timing?.addEventListener("change", render);
    loadOpportunities(win).then(({ payload, items: ready }) => {
      items = ready;
      generatedAt = payload.generated_at;
      render();
      win.agentBountiesAnalytics?.track("market_view");
    }).catch((error) => {
      list.setAttribute("aria-busy", "false");
      list.innerHTML = `<p class="market-empty">Live funded inventory is unavailable. ${text(error.message)} No stale opportunity is shown.</p>`;
      if (summary) summary.textContent = "Canonical inventory unavailable";
    });
    win.setInterval(render, 60_000);
  }

  return { amountNumber, apiBase, detailUrl, filterItems, formatUsdc, isReadyToEarn, isV2, loadOpportunities, opportunityFeedUrl, scoringWindow, startBoard, timingState, windowLabel };
});
