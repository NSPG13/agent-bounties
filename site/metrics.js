(function (root, factory) {
  "use strict";
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.AgentBountiesMetrics = api;
  if (root && root.document) api.start(root, root.document);
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";

  const PLATFORM_URL = "https://api.agentbounties.app/v1/metrics/platform";
  const GITHUB_URL = "generated/github-participation.json";
  const ACQUISITION_URL = "https://api.agentbounties.app/v1/analytics/site";
  const PLATFORM_DELAY_MS = 5 * 60 * 1000;
  const GITHUB_DELAY_MS = 2 * 60 * 60 * 1000;
  const ACQUISITION_DELAY_MS = 15 * 60 * 1000;
  const PERIOD_HOURS = Object.freeze({ "7d": 168, "28d": 672, "90d": 2160 });

  function finiteNumber(value, fallback = 0) {
    const number = Number(value);
    return Number.isFinite(number) ? number : fallback;
  }

  function formatInteger(value) {
    return new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 }).format(
      Math.max(0, finiteNumber(value)),
    );
  }

  function formatUsdc(value, options = {}) {
    const amount = Math.max(0, finiteNumber(value));
    const maximumFractionDigits = options.compact ? 1 : 2;
    return `${new Intl.NumberFormat("en-US", {
      notation: options.compact && amount >= 1000 ? "compact" : "standard",
      minimumFractionDigits: 0,
      maximumFractionDigits,
    }).format(amount)} USDC`;
  }

  function weeklyGrowth(current, previous) {
    const now = Math.max(0, finiteNumber(current));
    const before = Math.max(0, finiteNumber(previous));
    if (before === 0 && now === 0) return "0%";
    if (before === 0 && now > 0) return "New";
    const growth = ((now - before) / before) * 100;
    const rounded = Math.round(growth * 10) / 10;
    return `${rounded > 0 ? "+" : ""}${rounded.toLocaleString("en-US", {
      maximumFractionDigits: 1,
    })}%`;
  }

  function sourceStatus(source, now, delayedAfterMs) {
    if (!source || source.error) return "unavailable";
    const declared = String(source.status || "ready").toLowerCase();
    if (declared === "unavailable") return "unavailable";
    const generatedAt = Date.parse(source.generated_at || "");
    if (!Number.isFinite(generatedAt)) return "partial";
    if (now - generatedAt > delayedAfterMs) return "delayed";
    return declared === "partial" ? "partial" : "ready";
  }

  function combinedStatus(platformStatus, githubStatus) {
    if (platformStatus === "unavailable") return "unavailable";
    if (platformStatus === "partial" || githubStatus === "partial" || githubStatus === "unavailable") return "partial";
    if (platformStatus === "delayed" || githubStatus === "delayed") return "delayed";
    return "ready";
  }

  function roleMap(roles) {
    return new Map(
      (Array.isArray(roles) ? roles : []).map((item) => [
        String(item.role || ""),
        Math.max(0, finiteNumber(item.active_identities)),
      ]),
    );
  }

  function mergeDaily(platformDaily, githubDaily) {
    const days = new Map();
    (Array.isArray(platformDaily) ? platformDaily : []).forEach((item) => {
      days.set(String(item.day), {
        day: String(item.day),
        active_identities: Math.max(0, finiteNumber(item.active_identities)),
        payout_usdc: Math.max(0, finiteNumber(item.payout?.usdc)),
        settled_rounds: Math.max(0, finiteNumber(item.settled_rounds)),
      });
    });
    (Array.isArray(githubDaily) ? githubDaily : []).forEach((item) => {
      const key = String(item.day);
      const existing = days.get(key) || {
        day: key,
        active_identities: 0,
        payout_usdc: 0,
        settled_rounds: 0,
      };
      existing.active_identities += Math.max(0, finiteNumber(item.active_identities));
      days.set(key, existing);
    });
    return [...days.values()].sort((left, right) => left.day.localeCompare(right.day));
  }

  function mergeMetrics(platform, github, period, now = Date.now()) {
    const platformStatus = sourceStatus(
      platform
        ? { generated_at: platform.generated_at, status: platform.coverage?.status }
        : null,
      now,
      PLATFORM_DELAY_MS,
    );
    const githubPeriod = github?.periods?.[period] || null;
    const githubStatus = sourceStatus(
      github
        ? { generated_at: github.generated_at, status: github.coverage?.status }
        : null,
      now,
      GITHUB_DELAY_MS,
    );
    const platformIdentities = platform?.platform_active_identities || {};
    const githubSelected = finiteNumber(githubPeriod?.active_identities);
    const githubPrevious = finiteNumber(githubPeriod?.previous_active_identities);
    const githubFirstMonth = finiteNumber(github?.first_month?.active_identities);
    const githubLifetime = finiteNumber(github?.periods?.lifetime?.active_identities);
    const activeComplete = !["unavailable", "partial"].includes(platformStatus)
      && !["unavailable", "partial"].includes(githubStatus);
    const platformRoles = roleMap(platformIdentities.roles);
    const githubRoles = roleMap(githubPeriod?.roles);
    const roles = [
      ["Posters", finiteNumber(platformRoles.get("posters")) + finiteNumber(githubRoles.get("issue_posters"))],
      ["PR contributors", finiteNumber(githubRoles.get("pull_request_contributors"))],
      ["Funders", finiteNumber(platformRoles.get("funders"))],
      ["Solvers", finiteNumber(platformRoles.get("solvers"))],
      ["Verifiers", finiteNumber(platformRoles.get("verifiers"))],
      ["Commenters", finiteNumber(platformRoles.get("commenters")) + finiteNumber(githubRoles.get("commenters"))],
      ["Reviewers", finiteNumber(githubRoles.get("reviewers"))],
    ].map(([role, activeIdentities]) => ({ role, active_identities: activeIdentities }));
    return {
      status: combinedStatus(platformStatus, githubStatus),
      platform_status: platformStatus,
      github_status: githubStatus,
      active_complete: activeComplete,
      active_identities: finiteNumber(platformIdentities.selected) + githubSelected,
      previous_active_identities: finiteNumber(platformIdentities.previous) + githubPrevious,
      latest_week:
        finiteNumber(platformIdentities.latest_week) +
        finiteNumber(github?.weekly?.latest_active_identities),
      previous_week:
        finiteNumber(platformIdentities.previous_week) +
        finiteNumber(github?.weekly?.previous_active_identities),
      first_month_identities: finiteNumber(platformIdentities.first_month) + githubFirstMonth,
      lifetime_identities: finiteNumber(platformIdentities.lifetime) + githubLifetime,
      roles,
      daily: mergeDaily(platform?.daily, githubPeriod?.daily),
    };
  }

  function acquisitionWindowHours(period, now = Date.now()) {
    if (PERIOD_HOURS[period]) return PERIOD_HOURS[period];
    const launch = Date.parse("2026-07-08T20:22:19Z");
    return Math.max(1, Math.min(8760, Math.ceil((now - launch) / 3_600_000)));
  }

  function escapeXml(value) {
    return String(value)
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;")
      .replaceAll("'", "&apos;");
  }

  function chartSvg(points, valueKey, kind) {
    const selected = Array.isArray(points) ? points : [];
    if (selected.length < 2) return "";
    const width = 680;
    const height = 270;
    const inset = { top: 16, right: 16, bottom: 34, left: 38 };
    const innerWidth = width - inset.left - inset.right;
    const innerHeight = height - inset.top - inset.bottom;
    const values = selected.map((point) => Math.max(0, finiteNumber(point[valueKey])));
    const maximum = Math.max(1, ...values);
    const x = (index) => inset.left + (index / Math.max(1, selected.length - 1)) * innerWidth;
    const y = (value) => inset.top + innerHeight - (value / maximum) * innerHeight;
    const line = values.map((value, index) => `${index ? "L" : "M"}${x(index).toFixed(2)} ${y(value).toFixed(2)}`).join(" ");
    const area = `${line} L${x(selected.length - 1).toFixed(2)} ${(inset.top + innerHeight).toFixed(2)} L${x(0).toFixed(2)} ${(inset.top + innerHeight).toFixed(2)} Z`;
    const ticks = [0, 0.5, 1].map((ratio) => {
      const tickY = inset.top + innerHeight * (1 - ratio);
      const tickValue = maximum * ratio;
      return `<line class="chart-grid-line" x1="${inset.left}" x2="${width - inset.right}" y1="${tickY}" y2="${tickY}"/><text class="chart-axis-label" x="0" y="${tickY + 4}">${escapeXml(kind === "payout" ? new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 1 }).format(tickValue) : Math.round(tickValue))}</text>`;
    }).join("");
    const markerIndexes = [...new Set([0, Math.floor((selected.length - 1) / 2), selected.length - 1])];
    const labels = markerIndexes.map((index) => `<text class="chart-axis-label" text-anchor="${index === 0 ? "start" : index === selected.length - 1 ? "end" : "middle"}" x="${x(index)}" y="${height - 6}">${escapeXml(selected[index].day.slice(5))}</text>`).join("");
    const dots = selected.map((point, index) => `<circle class="chart-dot ${kind === "payout" ? "payout" : ""}" cx="${x(index)}" cy="${y(values[index])}" r="3" tabindex="0"><title>${escapeXml(point.day)}: ${escapeXml(kind === "payout" ? formatUsdc(values[index]) : `${formatInteger(values[index])} active identities`)}</title></circle>`).join("");
    const gradient = kind === "payout"
      ? '<linearGradient id="payout-area" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#79a8ff" stop-opacity=".24"/><stop offset="1" stop-color="#79a8ff" stop-opacity="0"/></linearGradient>'
      : '<linearGradient id="identity-area" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#b9ef37" stop-opacity=".22"/><stop offset="1" stop-color="#b9ef37" stop-opacity="0"/></linearGradient>';
    return `<svg viewBox="0 0 ${width} ${height}" role="img" aria-label="${kind === "payout" ? "Daily confirmed payout in USDC" : "Daily external active identities"}"><defs>${gradient}</defs>${ticks}<path class="chart-area ${kind === "payout" ? "payout" : ""}" d="${area}"/><path class="chart-line ${kind === "payout" ? "payout" : ""}" d="${line}"/>${dots}${labels}</svg>`;
  }

  function start(win, doc) {
    const state = {
      period: "7d",
      platform: null,
      github: null,
      acquisition: null,
      errors: {},
      timers: [],
    };

    const one = (selector) => doc.querySelector(selector);
    const setText = (selector, value) => {
      const node = one(selector);
      if (node) node.textContent = value;
    };
    const requestJson = async (url) => {
      const response = await win.fetch(url, { cache: "no-store", headers: { accept: "application/json" } });
      if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
      return response.json();
    };

    function amount(response) {
      return formatUsdc(response?.usdc);
    }

    function renderChart(selector, points, valueKey, kind) {
      const stage = one(selector);
      if (!stage) return;
      const svg = chartSvg(points, valueKey, kind);
      if (svg) {
        stage.innerHTML = svg;
      } else {
        stage.replaceChildren();
        const empty = doc.createElement("p");
        empty.className = "chart-empty";
        empty.textContent = points?.length === 1 ? "One reporting day is available; a trend needs at least two." : "No daily records are available for this period.";
        stage.appendChild(empty);
      }
    }

    function renderRoles(roles) {
      const target = one("[data-role-breakdown]");
      if (!target) return;
      target.replaceChildren();
      const maximum = Math.max(1, ...roles.map((role) => role.active_identities));
      roles.forEach((role) => {
        const row = doc.createElement("div");
        row.className = "role-row";
        const label = doc.createElement("span");
        label.textContent = role.role;
        const track = doc.createElement("span");
        track.className = "role-track";
        const fill = doc.createElement("i");
        fill.style.setProperty("--role-width", `${(role.active_identities / maximum) * 100}%`);
        track.appendChild(fill);
        const value = doc.createElement("strong");
        value.textContent = formatInteger(role.active_identities);
        row.append(label, track, value);
        target.appendChild(row);
      });
    }

    function renderSourceLedger(merged, acquisitionStatus) {
      const target = one("[data-source-ledger]");
      if (!target) return;
      target.replaceChildren();
      const sources = [
        ["Marketplace events", merged.platform_status, state.platform?.generated_at, "Confirmed canonical Base events with verified block time."],
        ["GitHub participation", merged.github_status, state.github?.generated_at, "Hourly aggregate of external issues, pull requests, comments, and reviews."],
        ["Browser acquisition", acquisitionStatus, state.acquisition?.generated_at, "First-party browser/device IDs; never counted as users or identities."],
      ];
      sources.forEach(([name, status, generatedAt, description]) => {
        const item = doc.createElement("article");
        item.className = "source-item";
        const header = doc.createElement("header");
        const title = doc.createElement("h3");
        title.textContent = name;
        const stateNode = doc.createElement("span");
        stateNode.className = "source-state";
        stateNode.dataset.status = status;
        stateNode.textContent = status;
        const body = doc.createElement("p");
        const time = generatedAt ? new Date(generatedAt).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" }) : "No source timestamp";
        body.textContent = `${description} Last source update: ${time}.`;
        header.append(title, stateNode);
        item.append(header, body);
        target.appendChild(item);
      });
    }

    function render() {
      const now = Date.now();
      const merged = mergeMetrics(state.platform, state.github, state.period, now);
      const platform = state.platform;
      const payout = platform?.marketplace_payout_volume;
      const cohort = platform?.mature_claim_to_settlement;
      const inventory = platform?.current_inventory;
      const acquisitionStatus = sourceStatus(
        state.acquisition ? { generated_at: state.acquisition.generated_at, status: "ready" } : null,
        now,
        ACQUISITION_DELAY_MS,
      );
      const status = one("[data-overall-status]");
      if (status) {
        status.dataset.status = merged.status;
        status.lastChild.textContent = merged.status === "ready" ? "Live data" : merged.status[0].toUpperCase() + merged.status.slice(1);
      }
      const timestamps = [platform?.generated_at, state.github?.generated_at]
        .map((value) => Date.parse(value || ""))
        .filter(Number.isFinite);
      setText("[data-last-update]", timestamps.length ? new Date(Math.min(...timestamps)).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" }) : "not available");

      const activeSuffix = merged.active_complete ? "" : "*";
      setText("[data-active-identities]", platform ? `${formatInteger(merged.active_identities)}${activeSuffix}` : "—");
      setText("[data-active-growth]", platform ? weeklyGrowth(merged.latest_week, merged.previous_week) : "—");
      setText("[data-active-context]", merged.active_complete
        ? "Distinct participating identities across separate GitHub, wallet, and comment-author namespaces."
        : "Partial identity count: one or more required participation sources are unavailable.");
      setText("[data-payout-volume]", payout ? amount(payout.selected) : "—");
      setText("[data-settled-rounds]", payout ? formatInteger(payout.selected_settled_rounds) : "—");
      setText("[data-settlement-rate]", cohort?.settlement_rate == null ? (cohort ? "Not yet available" : "—") : `${(cohort.settlement_rate * 100).toLocaleString("en-US", { maximumFractionDigits: 1 })}%`);
      setText("[data-mature-cohort]", cohort ? formatInteger(cohort.mature_claimed_rounds) : "—");
      setText("[data-immature-context]", cohort ? `${formatInteger(cohort.immature_claimed_rounds)} immature claimed round${cohort.immature_claimed_rounds === 1 ? "" : "s"} shown separately.` : "Recent claims stay outside the rate until they mature or reach a terminal event.");

      renderChart("[data-identity-chart]", merged.daily, "active_identities", "identities");
      renderChart("[data-payout-chart]", merged.daily, "payout_usdc", "payout");
      setText("[data-identity-chart-subtitle]", merged.active_complete ? "Selected period" : "Partial source coverage");
      renderRoles(merged.roles);
      setText("[data-solver-pay]", payout ? amount(payout.selected_solver_pay) : "—");
      setText("[data-verifier-pay]", payout ? amount(payout.selected_verifier_pay) : "—");
      setText("[data-completion-bonus]", payout ? amount(payout.selected_completion_bonus) : "—");

      const inventoryReady = inventory?.status === "ready";
      setText("[data-inventory-ready]", inventoryReady ? formatInteger(inventory.ready_to_earn_opportunities) : "—");
      setText("[data-inventory-autonomous]", inventoryReady ? formatInteger(inventory.autonomous_claimable_bounties) : "—");
      setText("[data-inventory-competitions]", inventoryReady ? formatInteger(inventory.open_competitions_ready_to_earn) : "—");
      setText("[data-inventory-verification]", inventoryReady ? formatInteger(inventory.verification_ready_bounties) : "—");
      setText("[data-inventory-standing]", inventoryReady ? formatInteger(inventory.standing_meta_bounties) : "—");
      setText("[data-inventory-funded]", inventoryReady ? amount(inventory.funded) : "—");
      setText("[data-inventory-solvers]", inventoryReady ? amount(inventory.solver_rewards) : "—");
      setText("[data-inventory-verifiers]", inventoryReady ? amount(inventory.verifier_rewards) : "—");
      setText("[data-inventory-status]", inventoryReady ? `Both canonical inventory protocols checked at ${new Date(inventory.generated_at).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" })}.` : "Inventory is partial or unavailable; historical values remain visible and combined inventory is not replaced with a lower total or zero.");

      setText("[data-first-month-identities]", platform ? `${formatInteger(merged.first_month_identities)}${activeSuffix}` : "—");
      setText("[data-lifetime-identities]", platform ? `${formatInteger(merged.lifetime_identities)}${activeSuffix}` : "—");
      setText("[data-first-month-payout]", payout ? amount(payout.first_month) : "—");
      setText("[data-lifetime-payout]", payout ? amount(payout.lifetime) : "—");
      setText("[data-first-month-settled]", payout ? formatInteger(payout.first_month_settled_rounds) : "—");
      setText("[data-lifetime-settled]", payout ? formatInteger(payout.lifetime_settled_rounds) : "—");

      setText("[data-browser-identities]", state.acquisition ? formatInteger(state.acquisition.overview?.unique_visitors) : "—");
      setText("[data-browser-context]", state.acquisition
        ? `${formatInteger(state.acquisition.overview?.sessions)} browser sessions in the matching lookback. Device/browser IDs are not people.`
        : "Acquisition source unavailable. Browser IDs are never estimated or counted as active identities.");
      setText("[data-platform-revenue]", platform ? amount(platform.platform_revenue) : "0 USDC");
      renderSourceLedger(merged, acquisitionStatus);

      const notices = [];
      if (merged.status === "partial") notices.push("Identity totals are partial because a required participation source is unavailable or incomplete.");
      if (merged.status === "delayed") notices.push("One or more required sources are delayed; values remain visible with their source timestamps.");
      if (merged.status === "unavailable") notices.push("Marketplace metrics are temporarily unavailable; no missing value has been replaced with zero.");
      if (merged.status === "ready") notices.push(`Live aggregate for ${state.period === "lifetime" ? "lifetime since launch" : state.period}. Roles are not additive.`);
      setText("[data-dashboard-notice]", notices.join(" "));
    }

    async function refreshPlatform() {
      try {
        state.platform = await requestJson(`${PLATFORM_URL}?period=${encodeURIComponent(state.period)}`);
        delete state.errors.platform;
      } catch (error) {
        state.errors.platform = error;
        state.platform = null;
      }
      render();
    }

    async function refreshSupporting() {
      const hours = acquisitionWindowHours(state.period);
      const [githubResult, acquisitionResult] = await Promise.allSettled([
        requestJson(`${GITHUB_URL}?v=${Date.now()}`),
        requestJson(`${ACQUISITION_URL}?window_hours=${hours}`),
      ]);
      if (githubResult.status === "fulfilled") {
        state.github = githubResult.value;
        delete state.errors.github;
      } else {
        state.github = null;
        state.errors.github = githubResult.reason;
      }
      if (acquisitionResult.status === "fulfilled") {
        state.acquisition = acquisitionResult.value;
        delete state.errors.acquisition;
      } else {
        state.acquisition = null;
        state.errors.acquisition = acquisitionResult.reason;
      }
      render();
    }

    doc.querySelectorAll("[data-period]").forEach((button) => {
      button.addEventListener("click", () => {
        const next = button.dataset.period;
        if (!PERIOD_HOURS[next] && next !== "lifetime") return;
        state.period = next;
        doc.querySelectorAll("[data-period]").forEach((candidate) => {
          candidate.setAttribute("aria-pressed", String(candidate === button));
        });
        state.platform = null;
        state.acquisition = null;
        render();
        void Promise.all([refreshPlatform(), refreshSupporting()]);
      });
    });

    function schedule() {
      state.timers.push(win.setInterval(() => {
        if (!doc.hidden) void refreshPlatform();
      }, 60_000));
      state.timers.push(win.setInterval(() => {
        if (!doc.hidden) void refreshSupporting();
      }, 300_000));
    }

    doc.addEventListener("visibilitychange", () => {
      if (!doc.hidden) void Promise.all([refreshPlatform(), refreshSupporting()]);
    });
    render();
    void Promise.all([refreshPlatform(), refreshSupporting()]);
    schedule();
    return state;
  }

  return {
    acquisitionWindowHours,
    chartSvg,
    combinedStatus,
    mergeDaily,
    mergeMetrics,
    sourceStatus,
    start,
    weeklyGrowth,
  };
});
