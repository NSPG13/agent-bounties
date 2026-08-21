(function (root, factory) {
  "use strict";
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.AgentBountiesMetrics = api;
  if (root && root.document) api.start(root, root.document);
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";

  const PLATFORM_URL = "https://api.agentbounties.app/v1/metrics/platform";
  const AUTONOMOUS_EVENTS_URL = "https://api.agentbounties.app/v1/base/autonomous-bounties/events?network=base-mainnet";
  const COMPETITION_EVENTS_URL = "https://api.agentbounties.app/v1/base/open-competition-v1/events?network=base-mainnet";
  const COMPETITION_V2_EVENTS_URL = "https://api.agentbounties.app/v1/base/open-competition-v2-beta3/events?network=base-mainnet";
  const BASESCAN_TX_URL = "https://basescan.org/tx/";
  const GITHUB_URL = "generated/github-participation.json";
  const ACQUISITION_URL = "https://api.agentbounties.app/v1/analytics/site";
  const PLATFORM_DELAY_MS = 5 * 60 * 1000;
  const GITHUB_DELAY_MS = 2 * 60 * 60 * 1000;
  const ACQUISITION_DELAY_MS = 15 * 60 * 1000;
  const PERIOD_HOURS = Object.freeze({ "7d": 168, "28d": 672, "90d": 2160 });
  const USDC_SCALE = 1_000_000;
  const INTERFACE_DEFINITIONS = Object.freeze([
    { key: "api:not_applicable", interface: "api", label: "REST API", detail: "Direct HTTP and official SDK requests", tone: "api" },
    { key: "cli:not_applicable", interface: "cli", label: "CLI", detail: "Rust command-line workflows", tone: "cli" },
    { key: "mcp:modern", interface: "mcp", label: "MCP 2026-07-28", detail: "Stateless discovery and self-contained requests", tone: "modern" },
    { key: "mcp:legacy", interface: "mcp", label: "Legacy MCP", detail: "Initialization-era compatible clients", tone: "legacy" },
    { key: "mcp:http_adapter", interface: "mcp", label: "MCP HTTP adapter", detail: "Direct /tools/* compatibility calls", tone: "adapter" },
  ]);

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

  function formatPercent(value) {
    const percent = Math.max(0, Math.min(1, finiteNumber(value))) * 100;
    return `${percent.toLocaleString("en-US", { maximumFractionDigits: 1 })}%`;
  }

  function interfaceUsageSummary(response) {
    const emptyRows = () => INTERFACE_DEFINITIONS.map((definition) => ({
      ...definition,
      request_count: 0,
      successful_request_count: 0,
      success_rate: null,
      share: 0,
      first_observed_at: null,
      last_observed_at: null,
    }));
    if (!response || !Array.isArray(response.interfaces)) {
      return {
        status: "unavailable",
        rows: emptyRows(),
        request_count: 0,
        successful_request_count: 0,
        success_rate: null,
        mcp_request_count: 0,
        mcp_share: null,
        first_observed_at: null,
        last_observed_at: null,
      };
    }
    const observed = new Map();
    response.interfaces.forEach((item) => {
      const key = `${String(item?.interface || "")}:${String(item?.protocol_era || "")}`;
      if (!INTERFACE_DEFINITIONS.some((definition) => definition.key === key)) return;
      const current = observed.get(key) || {
        request_count: 0,
        successful_request_count: 0,
        first_observed_at: null,
        last_observed_at: null,
      };
      const requestCount = Math.max(0, finiteNumber(item?.request_count));
      const successfulCount = Math.min(requestCount, Math.max(0, finiteNumber(item?.successful_request_count)));
      const firstObserved = Date.parse(item?.first_observed_at || "");
      const lastObserved = Date.parse(item?.last_observed_at || "");
      current.request_count += requestCount;
      current.successful_request_count += successfulCount;
      if (Number.isFinite(firstObserved) && (!current.first_observed_at || firstObserved < Date.parse(current.first_observed_at))) {
        current.first_observed_at = new Date(firstObserved).toISOString();
      }
      if (Number.isFinite(lastObserved) && (!current.last_observed_at || lastObserved > Date.parse(current.last_observed_at))) {
        current.last_observed_at = new Date(lastObserved).toISOString();
      }
      observed.set(key, current);
    });
    const requestCount = [...observed.values()].reduce((sum, row) => sum + row.request_count, 0);
    const successfulRequestCount = [...observed.values()].reduce((sum, row) => sum + row.successful_request_count, 0);
    const rows = INTERFACE_DEFINITIONS.map((definition) => {
      const row = observed.get(definition.key) || {
        request_count: 0,
        successful_request_count: 0,
        first_observed_at: null,
        last_observed_at: null,
      };
      return {
        ...definition,
        ...row,
        success_rate: row.request_count ? row.successful_request_count / row.request_count : null,
        share: requestCount ? row.request_count / requestCount : 0,
      };
    });
    const mcpRequestCount = rows
      .filter((row) => row.interface === "mcp")
      .reduce((sum, row) => sum + row.request_count, 0);
    const timestamps = rows.flatMap((row) => [
      Date.parse(row.first_observed_at || ""),
      Date.parse(row.last_observed_at || ""),
    ]).filter(Number.isFinite);
    return {
      status: "ready",
      rows,
      request_count: requestCount,
      successful_request_count: successfulRequestCount,
      success_rate: requestCount ? successfulRequestCount / requestCount : null,
      mcp_request_count: mcpRequestCount,
      mcp_share: requestCount ? mcpRequestCount / requestCount : null,
      first_observed_at: timestamps.length ? new Date(Math.min(...timestamps)).toISOString() : null,
      last_observed_at: timestamps.length ? new Date(Math.max(...timestamps)).toISOString() : null,
    };
  }

  function nonNegativeBaseUnits(value) {
    const amount = Number(value ?? 0);
    return Number.isSafeInteger(amount) && amount > 0 ? amount : 0;
  }

  function canonicalPayoutRows(autonomousResponse, competitionResponse, competitionV2Response, window) {
    const startedAt = Date.parse(window?.started_at || "");
    const endedAt = Date.parse(window?.ended_at || "");
    if (!Number.isFinite(startedAt) || !Number.isFinite(endedAt)) return [];
    const sources = [
      {
        protocol: "Exclusive claim",
        protocolKey: "autonomous-v1",
        events: Array.isArray(autonomousResponse) ? autonomousResponse : [],
        eventUrl: AUTONOMOUS_EVENTS_URL,
        settledKind: "bounty_settled",
        settledSecondaryAmount: "verifier_reward",
        settledSecondaryRole: "verifier",
        rejectedKind: "submission_rejected",
        rejectedAmount: "verifier_reward",
      },
      {
        protocol: "Open competition",
        protocolKey: "open-competition-v1",
        events: Array.isArray(competitionResponse?.events) ? competitionResponse.events : [],
        eventUrl: COMPETITION_EVENTS_URL,
        settledKind: "bounty_settled",
        settledSecondaryAmount: "verifier_reward",
        settledSecondaryRole: "verifier",
        rejectedKind: "competition_submission_rejected",
        rejectedAmount: "bond_paid_to_verifier",
      },
      {
        protocol: "Open competition",
        protocolKey: "open-competition-v2",
        events: Array.isArray(competitionV2Response?.events) ? competitionV2Response.events : [],
        eventUrl: COMPETITION_V2_EVENTS_URL,
        settledKind: "competition_settled",
        settledSecondaryAmount: "keeper_reward",
        settledSecondaryRole: "keeper",
        rejectedKind: null,
        rejectedAmount: null,
      },
    ];
    const rows = [];
    sources.forEach((source) => {
      source.events.forEach((event) => {
        const occurredAt = Date.parse(event?.occurred_at || "");
        const settled = event?.kind === source.settledKind;
        const rejected = event?.kind === source.rejectedKind;
        if ((!settled && !rejected) || !Number.isFinite(occurredAt)
          || occurredAt < startedAt || occurredAt >= endedAt) return;
        const solver = settled ? nonNegativeBaseUnits(event.data?.solver_reward) : 0;
        const secondary = settled
          ? nonNegativeBaseUnits(event.data?.[source.settledSecondaryAmount])
          : nonNegativeBaseUnits(event.data?.[source.rejectedAmount]);
        const verifier = source.settledSecondaryRole === "keeper" ? 0 : secondary;
        const keeper = source.settledSecondaryRole === "keeper" ? secondary : 0;
        const bonus = settled ? nonNegativeBaseUnits(event.data?.timeout_bond_bonus) : 0;
        rows.push({
          protocol: source.protocol,
          protocol_key: source.protocolKey,
          kind: event.kind,
          event_label: settled ? "Settlement" : "Rejected submission payout",
          is_settlement: settled,
          contract_address: String(event.contract_address || ""),
          bounty_id: String(event.bounty_id || ""),
          round: event.data?.round ?? event.data?.submission_sequence ?? null,
          tx_hash: String(event.tx_hash || ""),
          block_number: finiteNumber(event.block_number),
          log_index: finiteNumber(event.log_index),
          occurred_at: String(event.occurred_at || ""),
          solver_base_units: solver,
          verifier_base_units: verifier,
          keeper_base_units: keeper,
          bonus_base_units: bonus,
          total_base_units: solver + verifier + keeper + bonus,
          api_url: `${source.eventUrl}&bounty_id=${encodeURIComponent(String(event.bounty_id || ""))}`,
          explorer_url: `${BASESCAN_TX_URL}${encodeURIComponent(String(event.tx_hash || ""))}#eventlog`,
        });
      });
    });
    return rows.sort((left, right) => (
      right.block_number - left.block_number || right.log_index - left.log_index
    ));
  }

  function payoutAuditSummary(rows) {
    return (Array.isArray(rows) ? rows : []).reduce((summary, row) => {
      summary.payout_events += 1;
      summary.settlement_events += row.is_settlement ? 1 : 0;
      summary.solver_base_units += nonNegativeBaseUnits(row.solver_base_units);
      summary.verifier_base_units += nonNegativeBaseUnits(row.verifier_base_units);
      summary.keeper_base_units += nonNegativeBaseUnits(row.keeper_base_units);
      summary.bonus_base_units += nonNegativeBaseUnits(row.bonus_base_units);
      summary.total_base_units += nonNegativeBaseUnits(row.total_base_units);
      return summary;
    }, {
      payout_events: 0,
      settlement_events: 0,
      solver_base_units: 0,
      verifier_base_units: 0,
      keeper_base_units: 0,
      bonus_base_units: 0,
      total_base_units: 0,
    });
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

  function dashboardStatus(coreStatus, repositoryStatus) {
    if (coreStatus === "unavailable") return "unavailable";
    if (coreStatus === "partial" || repositoryStatus === "partial" || repositoryStatus === "unavailable") return "partial";
    if (coreStatus === "delayed" || repositoryStatus === "delayed") return "delayed";
    return "ready";
  }

  function ratioMultiple(current, baseline) {
    const denominator = Math.max(0, finiteNumber(baseline));
    if (!denominator) return null;
    return Math.max(0, finiteNumber(current)) / denominator;
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
      period: "lifetime",
      platform: null,
      autonomousEvents: null,
      competitionEvents: null,
      competitionV2Events: null,
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

    function shortHash(value) {
      const text = String(value || "");
      return text.length > 18 ? `${text.slice(0, 10)}…${text.slice(-6)}` : text || "—";
    }

    function payoutAuditSnapshot(platform) {
      if (!platform || !state.autonomousEvents || !state.competitionEvents || !state.competitionV2Events) {
        return { status: "unavailable", rows: [], summary: payoutAuditSummary([]), generated_at: null };
      }
      const rows = canonicalPayoutRows(
        state.autonomousEvents,
        state.competitionEvents,
        state.competitionV2Events,
        platform.window,
      );
      const summary = payoutAuditSummary(rows);
      const expectedTotal = Number(platform.marketplace_payout_volume?.selected?.usdc_base_units);
      const expectedSettlements = Number(platform.marketplace_payout_volume?.selected_settled_rounds);
      const reconciled = Number.isSafeInteger(expectedTotal)
        && Number.isSafeInteger(expectedSettlements)
        && summary.total_base_units === expectedTotal
        && summary.settlement_events === expectedSettlements;
      return {
        status: reconciled ? "ready" : "partial",
        rows,
        summary,
        generated_at: rows[0]?.occurred_at || platform.generated_at,
      };
    }

    function renderPayoutAudit(audit) {
      const status = one("[data-audit-status]");
      if (status) {
        status.dataset.status = audit.status;
        status.textContent = audit.status === "ready"
          ? "reconciled"
          : audit.status === "partial" ? "mismatch" : "unavailable";
      }
      setText("[data-audit-total]", audit.status === "unavailable"
        ? "—"
        : formatUsdc(audit.summary.total_base_units / USDC_SCALE));
      setText("[data-audit-settlements]", audit.status === "unavailable"
        ? "—"
        : formatInteger(audit.summary.settlement_events));
      setText("[data-audit-payout-events]", audit.status === "unavailable"
        ? "—"
        : formatInteger(audit.summary.payout_events));
      setText("[data-audit-copy]", audit.status === "ready"
        ? "The independent event sum exactly matches the headline payout and settlement count for this period."
        : audit.status === "partial"
          ? "The public event sum does not match the aggregate yet. Treat the headline as partial while indexing catches up."
          : "The raw canonical event streams are unavailable. No unverifiable replacement is shown.");

      const body = one("[data-audit-rows]");
      if (!body) return;
      body.replaceChildren();
      audit.rows.forEach((row) => {
        const tr = doc.createElement("tr");
        const when = doc.createElement("td");
        const time = doc.createElement("time");
        time.dateTime = row.occurred_at;
        time.textContent = new Date(row.occurred_at).toLocaleString(undefined, {
          day: "numeric",
          month: "short",
          year: "numeric",
          hour: "2-digit",
          minute: "2-digit",
          timeZone: "UTC",
          timeZoneName: "short",
        });
        when.appendChild(time);

        const event = doc.createElement("td");
        const eventName = doc.createElement("strong");
        eventName.textContent = row.event_label;
        const protocol = doc.createElement("span");
        protocol.textContent = `${row.protocol}${row.round == null ? "" : ` · round ${row.round}`}`;
        event.append(eventName, protocol);

        const contract = doc.createElement("td");
        const contractCode = doc.createElement("code");
        contractCode.textContent = shortHash(row.contract_address);
        contractCode.title = row.contract_address;
        const bountyCode = doc.createElement("span");
        bountyCode.textContent = `Bounty ${shortHash(row.bounty_id)}`;
        bountyCode.title = row.bounty_id;
        contract.append(contractCode, bountyCode);

        const payout = doc.createElement("td");
        payout.textContent = formatUsdc(row.total_base_units / USDC_SCALE);

        const proof = doc.createElement("td");
        const explorer = doc.createElement("a");
        explorer.href = row.explorer_url;
        explorer.target = "_blank";
        explorer.rel = "noopener noreferrer";
        explorer.textContent = "BaseScan";
        explorer.title = `Open transaction ${row.tx_hash} on BaseScan`;
        const raw = doc.createElement("a");
        raw.href = row.api_url;
        raw.target = "_blank";
        raw.rel = "noopener noreferrer";
        raw.textContent = "Raw events";
        raw.title = `Open canonical event records for ${row.bounty_id}`;
        proof.append(explorer, raw);
        tr.append(when, event, contract, payout, proof);
        body.appendChild(tr);
      });
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

    function renderInterfaceUsage(summary) {
      const available = summary.status !== "unavailable";
      const status = one("[data-interface-status]");
      if (status) {
        status.dataset.status = summary.status;
        status.textContent = summary.status === "ready"
          ? "live aggregate"
          : summary.status === "delayed" ? "delayed" : summary.status;
      }
      setText("[data-interface-total]", available ? formatInteger(summary.request_count) : "—");
      setText("[data-interface-success]", available
        ? `${formatInteger(summary.successful_request_count)}${summary.success_rate == null ? "" : ` · ${formatPercent(summary.success_rate)}`}`
        : "—");
      setText("[data-interface-mcp-share]", available
        ? (summary.mcp_share == null ? "No traffic yet" : formatPercent(summary.mcp_share))
        : "—");
      const windowLabel = state.period === "lifetime" ? "lifetime since interface tracking began" : state.period;
      setText("[data-interface-window]", summary.status === "unavailable"
        ? "The aggregate interface report is unavailable; no counts are estimated."
        : summary.last_observed_at
          ? `${windowLabel} · last observed ${new Date(summary.last_observed_at).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" })}`
          : `${windowLabel} · no attributed requests observed yet`);

      const target = one("[data-interface-rows]");
      if (!target) return;
      target.replaceChildren();
      summary.rows.forEach((row) => {
        const item = doc.createElement("article");
        item.className = "interface-row";
        item.dataset.tone = row.tone;

        const identity = doc.createElement("div");
        identity.className = "interface-identity";
        const label = doc.createElement("h3");
        label.textContent = row.label;
        const detail = doc.createElement("p");
        detail.textContent = row.detail;
        identity.append(label, detail);

        const volume = doc.createElement("div");
        volume.className = "interface-volume";
        const count = doc.createElement("strong");
        count.textContent = available ? formatInteger(row.request_count) : "—";
        const success = doc.createElement("span");
        success.textContent = !available
          ? "source unavailable"
          : row.success_rate == null
            ? "no requests in window"
            : `${formatInteger(row.successful_request_count)} HTTP 2xx · ${formatPercent(row.success_rate)}`;
        volume.append(count, success);

        const share = doc.createElement("div");
        share.className = "interface-share";
        const track = doc.createElement("span");
        track.className = "interface-track";
        track.setAttribute("role", "progressbar");
        track.setAttribute("aria-label", `${row.label} share of observed external requests`);
        track.setAttribute("aria-valuemin", "0");
        track.setAttribute("aria-valuemax", "100");
        track.setAttribute("aria-valuenow", String(Math.round(row.share * 100)));
        const fill = doc.createElement("i");
        fill.style.setProperty("--interface-width", `${row.share * 100}%`);
        track.appendChild(fill);
        const shareText = doc.createElement("span");
        shareText.textContent = summary.request_count ? `${formatPercent(row.share)} of requests` : "No share yet";
        share.append(track, shareText);

        item.append(identity, volume, share);
        target.appendChild(item);
      });
    }

    function renderSourceLedger(merged, acquisitionStatus, repositoryStatus, audit, interfaceUsage) {
      const target = one("[data-source-ledger]");
      if (!target) return;
      target.replaceChildren();
      const sources = [
        ["Marketplace events", merged.platform_status, state.platform?.generated_at, "Confirmed canonical Base events with verified block time."],
        ["Payout proof ledger", audit.status, audit.generated_at, "Every qualifying payout event is summed in the browser and linked to its raw record and Base transaction."],
        ["External interface usage", interfaceUsage.status, state.acquisition?.generated_at, "Hourly external API, CLI, and MCP request aggregates; verified operator traffic is omitted."],
        ["GitHub participation", merged.github_status, state.github?.generated_at, "Hourly aggregate of external issues, pull requests, comments, and reviews."],
        ["Repository acquisition", repositoryStatus, state.github?.repository_acquisition?.generated_at, "GitHub clone and page-view aggregates for its rolling 14-day traffic window."],
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
      const demandGrowth = platform?.demand_growth;
      const acquisitionStatus = sourceStatus(
        state.acquisition ? { generated_at: state.acquisition.generated_at, status: "ready" } : null,
        now,
        ACQUISITION_DELAY_MS,
      );
      const repositoryAcquisition = state.github?.repository_acquisition;
      const repositoryStatus = sourceStatus(
        repositoryAcquisition
          ? { generated_at: repositoryAcquisition.generated_at, status: repositoryAcquisition.coverage?.status }
          : null,
        now,
        GITHUB_DELAY_MS,
      );
      const audit = payoutAuditSnapshot(platform);
      const interfaceSummary = interfaceUsageSummary(state.acquisition);
      const interfaceUsage = interfaceSummary.status === "unavailable"
        ? interfaceSummary
        : { ...interfaceSummary, status: acquisitionStatus };
      const coreStatus = dashboardStatus(merged.status, repositoryStatus);
      const auditedStatus = dashboardStatus(coreStatus, audit.status);
      const overallStatus = dashboardStatus(auditedStatus, interfaceUsage.status);
      const status = one("[data-overall-status]");
      if (status) {
        status.dataset.status = overallStatus;
        status.lastChild.textContent = overallStatus === "ready" ? "Live data" : overallStatus[0].toUpperCase() + overallStatus.slice(1);
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
      renderInterfaceUsage(interfaceUsage);
      renderPayoutAudit(audit);

      const inventoryReady = inventory?.status === "ready";
      setText("[data-inventory-ready]", inventoryReady ? formatInteger(inventory.active_funded_opportunities) : "—");
      setText("[data-inventory-funded]", inventoryReady ? formatUsdc(inventory.available_funding_usdc) : "—");
      setText("[data-inventory-solvers]", inventoryReady ? formatUsdc(inventory.available_solver_rewards_usdc) : "—");
      setText("[data-inventory-verifiers]", inventoryReady ? formatUsdc(inventory.available_verifier_rewards_usdc) : "—");
      setText("[data-inventory-status]", inventoryReady ? `All required canonical inventory sources checked at ${new Date(inventory.generated_at).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" })}.` : "Inventory is partial or unavailable; historical values remain visible and the unified total is not replaced with a lower value or zero.");
      setText("[data-gmv-28d]", demandGrowth ? amount(demandGrowth.gmv_usdc_28d) : "—");
      setText(
        "[data-non-operator-gmv-share]",
        demandGrowth?.non_operator_funded_gmv_share_28d == null
          ? "—"
          : formatPercent(demandGrowth.non_operator_funded_gmv_share_28d),
      );
      setText(
        "[data-new-poster-funder-wallets]",
        demandGrowth ? formatInteger(demandGrowth.new_poster_funder_wallets_28d) : "—",
      );
      setText(
        "[data-repeat-poster-funder-rate]",
        demandGrowth?.repeat_poster_funder_rate_28d == null
          ? "—"
          : formatPercent(demandGrowth.repeat_poster_funder_rate_28d),
      );
      setText(
        "[data-demand-growth-status]",
        !demandGrowth
          ? "Demand-growth evidence is unavailable."
          : demandGrowth.funding_attribution_complete_28d
            ? "GMV uses confirmed canonical settlements; funding share uses canonically attributed contribution amounts."
            : "GMV is available, but the externally funded share is withheld because funding attribution is incomplete.",
      );

      setText("[data-first-month-identities]", platform ? `${formatInteger(merged.first_month_identities)}${activeSuffix}` : "—");
      setText("[data-lifetime-identities]", platform ? `${formatInteger(merged.lifetime_identities)}${activeSuffix}` : "—");
      setText("[data-first-month-payout]", payout ? amount(payout.first_month) : "—");
      setText("[data-lifetime-payout]", payout ? amount(payout.lifetime) : "—");
      setText("[data-first-month-settled]", payout ? formatInteger(payout.first_month_settled_rounds) : "—");
      setText("[data-lifetime-settled]", payout ? formatInteger(payout.lifetime_settled_rounds) : "—");

      const repositoryReady = repositoryStatus === "ready" || repositoryStatus === "delayed";
      const repositoryStatusNode = one("[data-repository-status]");
      if (repositoryStatusNode) {
        repositoryStatusNode.dataset.status = repositoryStatus;
        repositoryStatusNode.textContent = repositoryStatus;
      }
      setText("[data-clone-events]", repositoryReady ? formatInteger(repositoryAcquisition.clone_events) : "—");
      setText("[data-unique-cloners]", repositoryReady ? formatInteger(repositoryAcquisition.unique_cloners) : "—");
      setText("[data-page-views]", repositoryReady ? formatInteger(repositoryAcquisition.page_views) : "—");
      setText("[data-unique-visitors]", repositoryReady ? formatInteger(repositoryAcquisition.unique_visitors) : "—");
      if (repositoryReady) {
        const started = new Date(repositoryAcquisition.started_at);
        const ended = new Date(Date.parse(repositoryAcquisition.ended_at) - 1);
        const dateOptions = { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" };
        setText("[data-repository-window]", `${started.toLocaleDateString("en-US", dateOptions)} – ${ended.toLocaleDateString("en-US", dateOptions)} · GitHub rolling 14-day window`);
        const multiple = ratioMultiple(repositoryAcquisition.clone_events, 2448);
        setText("[data-repository-comparison]", multiple == null
          ? "Historical snapshots are overlapping rolling windows and are not summed."
          : `Current clone activity is ${multiple.toLocaleString("en-US", { maximumFractionDigits: 1 })}× the 11 July public snapshot. Historical snapshots are overlapping rolling windows and are not summed.`);
      } else {
        setText("[data-repository-window]", "GitHub repository traffic is unavailable; no lower value is substituted.");
        setText("[data-repository-comparison]", "The dated July snapshots remain visible, but they are not substituted for missing live traffic.");
      }

      setText("[data-browser-identities]", state.acquisition ? formatInteger(state.acquisition.overview?.unique_visitors) : "—");
      setText("[data-browser-context]", state.acquisition
        ? `${formatInteger(state.acquisition.overview?.sessions)} browser sessions in the matching lookback. Device/browser IDs are not people.`
        : "Acquisition source unavailable. Browser IDs are never estimated or counted as active identities.");
      setText("[data-platform-revenue]", platform ? amount(platform.platform_revenue) : "0 USDC");
      renderSourceLedger(merged, acquisitionStatus, repositoryStatus, audit, interfaceUsage);

      const notices = [];
      if (merged.status === "partial") notices.push("Identity totals are partial because a required participation source is unavailable or incomplete.");
      if (merged.status === "delayed") notices.push("One or more required sources are delayed; values remain visible with their source timestamps.");
      if (merged.status === "unavailable") notices.push("Marketplace metrics are temporarily unavailable; no missing value has been replaced with zero.");
      if (["partial", "unavailable"].includes(repositoryStatus)) notices.push("Repository acquisition is unavailable or incomplete; historical snapshots remain labeled and are not substituted as live data.");
      if (interfaceUsage.status === "unavailable") notices.push("External interface usage is unavailable; API, CLI, and MCP request counts are not estimated.");
      if (["partial", "delayed"].includes(interfaceUsage.status)) notices.push("External interface usage is delayed or incomplete; the last observed aggregate remains labeled with its timestamp.");
      if (audit.status === "partial") notices.push("The payout proof ledger does not yet reconcile to the aggregate; payout values are marked partial.");
      if (audit.status === "unavailable") notices.push("The public payout proof streams are unavailable, so payout auditability is temporarily partial.");
      if (overallStatus === "ready") notices.push(`Live aggregate for ${state.period === "lifetime" ? "lifetime since launch" : state.period}. Roles are not additive.`);
      setText("[data-dashboard-notice]", notices.join(" "));
    }

    async function refreshPlatform() {
      const [platformResult, autonomousResult, competitionResult, competitionV2Result] = await Promise.allSettled([
        requestJson(`${PLATFORM_URL}?period=${encodeURIComponent(state.period)}`),
        requestJson(AUTONOMOUS_EVENTS_URL),
        requestJson(COMPETITION_EVENTS_URL),
        requestJson(COMPETITION_V2_EVENTS_URL),
      ]);
      if (platformResult.status === "fulfilled") {
        state.platform = platformResult.value;
        delete state.errors.platform;
      } else {
        state.errors.platform = platformResult.reason;
        state.platform = null;
      }
      if (autonomousResult.status === "fulfilled") {
        state.autonomousEvents = autonomousResult.value;
        delete state.errors.autonomousEvents;
      } else {
        state.autonomousEvents = null;
        state.errors.autonomousEvents = autonomousResult.reason;
      }
      if (competitionResult.status === "fulfilled") {
        state.competitionEvents = competitionResult.value;
        delete state.errors.competitionEvents;
      } else {
        state.competitionEvents = null;
        state.errors.competitionEvents = competitionResult.reason;
      }
      if (competitionV2Result.status === "fulfilled") {
        state.competitionV2Events = competitionV2Result.value;
        delete state.errors.competitionV2Events;
      } else {
        state.competitionV2Events = null;
        state.errors.competitionV2Events = competitionV2Result.reason;
      }
      render();
    }

    async function refreshRepository() {
      try {
        state.github = await requestJson(`${GITHUB_URL}?v=${Date.now()}`);
        delete state.errors.github;
      } catch (error) {
        state.github = null;
        state.errors.github = error;
      }
      render();
    }

    async function refreshAcquisition() {
      const hours = acquisitionWindowHours(state.period);
      try {
        state.acquisition = await requestJson(`${ACQUISITION_URL}?window_hours=${hours}`);
        delete state.errors.acquisition;
      } catch (error) {
        state.acquisition = null;
        state.errors.acquisition = error;
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
        void Promise.all([refreshPlatform(), refreshRepository(), refreshAcquisition()]);
      });
    });

    function schedule() {
      state.timers.push(win.setInterval(() => {
        if (!doc.hidden) void refreshPlatform();
      }, 60_000));
      state.timers.push(win.setInterval(() => {
        if (!doc.hidden) void refreshAcquisition();
      }, 60_000));
      state.timers.push(win.setInterval(() => {
        if (!doc.hidden) void refreshRepository();
      }, 300_000));
    }

    doc.addEventListener("visibilitychange", () => {
      if (!doc.hidden) void Promise.all([refreshPlatform(), refreshRepository(), refreshAcquisition()]);
    });
    render();
    void Promise.all([refreshPlatform(), refreshRepository(), refreshAcquisition()]);
    schedule();
    return state;
  }

  return {
    acquisitionWindowHours,
    canonicalPayoutRows,
    chartSvg,
    combinedStatus,
    dashboardStatus,
    mergeDaily,
    mergeMetrics,
    payoutAuditSummary,
    interfaceUsageSummary,
    sourceStatus,
    start,
    ratioMultiple,
    weeklyGrowth,
  };
});
