(function (root, factory) {
  "use strict";
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.AgentBountiesPayoutProof = api;
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";
  const API = "https://api.agentbounties.app";
  const PATH = "/v1/metrics/platform/payouts";
  const MAX_ROWS = 5000, MAX_PAGES = 25, MAX_BYTES = 262144;
  const FIELDS = ["solver_base_units", "verifier_base_units", "keeper_base_units", "bonus_base_units", "total_base_units"];
  const fail = (message) => { throw new Error(message); };
  const uint = (value) => typeof value === "string" && /^(0|[1-9]\d{0,38})$/.test(value);
  const hash = (value) => typeof value === "string" && /^sha256:[a-f0-9]{64}$/.test(value);
  const hex = (value, length) => typeof value === "string" && new RegExp(`^0x[0-9a-fA-F]{${length}}$`).test(value);
  const count = (value) => Number.isSafeInteger(value) && value >= 0;
  const sameTime = (left, right) => typeof left === "string" && typeof right === "string" && left === right;
  function scope(platform) {
    const meta = platform?.payout_proof, window = platform?.window;
    if (platform?.network !== "base-mainnet" || meta?.status !== "available" || !hash(meta.snapshot) || !hash(meta.policy_hash)
      || !["7d", "28d", "90d", "lifetime"].includes(window?.period)
      || !Number.isFinite(Date.parse(window?.started_at)) || !Number.isFinite(Date.parse(window?.ended_at))
      || Date.parse(window.started_at) > Date.parse(window.ended_at)) fail("Historical proof metadata unavailable");
    return { meta, window };
  }
  function proofUrl(platform, cursor) {
    const { meta, window } = scope(platform);
    const url = new URL(PATH, API);
    url.searchParams.set("period", window.period);
    url.searchParams.set("as_of", window.ended_at);
    url.searchParams.set("snapshot", meta.snapshot);
    url.searchParams.set("limit", "200");
    if (cursor) url.searchParams.set("cursor", cursor);
    return url.href;
  }
  function summary(rows) {
    const sums = Object.fromEntries(FIELDS.map((field) => [field, 0n]));
    let settlements = 0;
    for (const row of rows) {
      for (const field of FIELDS) sums[field] += BigInt(row[field]);
      settlements += row.is_settlement ? 1 : 0;
    }
    return { ...Object.fromEntries(FIELDS.map((field) => [field, String(sums[field])])), payout_events: rows.length, settlement_events: settlements };
  }
  function inspect(platform, pages) {
    const { meta, window } = scope(platform);
    if (!Array.isArray(pages) || pages.length < 1 || pages.length > MAX_PAGES) fail("Proof pages unavailable or over limit");
    const rows = [], seen = new Set();
    let total, excluded, cursor = null;
    for (let index = 0; index < pages.length; index += 1) {
      const page = pages[index];
      if (page?.schema_version !== "agent-bounties/platform-payout-proof-v1" || page.network !== "base-mainnet"
        || page.period !== window.period || !sameTime(page.started_at, window.started_at) || !sameTime(page.ended_at, window.ended_at)
        || page.snapshot !== meta.snapshot || page.policy_hash !== meta.policy_hash || page.coverage !== "indexed_history_only"
        || !count(page.total_rows) || page.total_rows > MAX_ROWS || page.offset !== rows.length
        || !Array.isArray(page.records) || page.records.length > 200 || page.excluded_rows_enumerated !== false
        || !Array.isArray(page.excluded_bounty_contracts) || page.excluded_bounty_contracts.some((contract) => !hex(contract, 40))) fail("Proof scope, bounds or policy mismatch");
      const policy = JSON.stringify(page.excluded_bounty_contracts);
      if (index && (total !== page.total_rows || policy !== excluded)) fail("Proof selection changed; restart required");
      total = page.total_rows; excluded = policy;
      const excludedSet = new Set(page.excluded_bounty_contracts.map((contract) => contract.toLowerCase()));
      const apiUrl = proofUrl(platform, cursor);
      for (const row of page.records) {
        const key = `${String(row?.tx_hash).toLowerCase()}:${row?.log_index}`;
        const time = Date.parse(row?.occurred_at);
        const settlement = (row?.kind === "bounty_settled" && ["autonomous-v1", "open-competition-v1"].includes(row?.protocol))
          || (row?.kind === "competition_settled" && row?.protocol === "open-competition-v2");
        const rejection = (row?.kind === "submission_rejected" && row?.protocol === "autonomous-v1")
          || (row?.kind === "competition_submission_rejected" && row?.protocol === "open-competition-v1");
        if ((!settlement && !rejection) || row.is_settlement !== settlement || row.network !== "base-mainnet"
          || !hex(row.contract_address, 40) || !hex(row.tx_hash, 64) || !hex(row.bounty_id, 64)
          || !(hex(row.factory_contract, 40) || (row.protocol === "autonomous-v1" && row.factory_contract === null))
          || !uint(row.block_number) || !uint(row.log_index) || !FIELDS.every((field) => uint(row[field]))
          || !Number.isFinite(time) || time < Date.parse(window.started_at) || time >= Date.parse(window.ended_at)
          || seen.has(key) || excludedSet.has(row.contract_address.toLowerCase())) fail("Invalid or duplicate historical payout record");
        const sum = FIELDS.slice(0, 4).reduce((value, field) => value + BigInt(row[field]), 0n);
        if (sum !== BigInt(row.total_base_units)) fail("Payout components do not sum");
        seen.add(key);
        rows.push({ ...row, event_label: settlement ? "Settlement" : "Rejected submission payout", api_url: apiUrl,
          explorer_url: `https://basescan.org/tx/${row.tx_hash}#eventlog` });
      }
      const last = index === pages.length - 1;
      if (rows.length > total || (last && (page.complete !== true || page.next_cursor !== null || rows.length !== total))
        || (!last && (page.complete !== false || !page.records.length || typeof page.next_cursor !== "string"
          || !/^v1:[1-9]\d{0,3}:sha256:[a-f0-9]{64}$/.test(page.next_cursor)
          || Number(page.next_cursor.split(":")[1]) !== rows.length))) fail("Incomplete or inconsistent proof pagination");
      cursor = page.next_cursor;
    }
    const totals = summary(rows), payout = platform.marketplace_payout_volume;
    const expected = [payout?.selected, payout?.selected_solver_pay, payout?.selected_verifier_pay, payout?.selected_keeper_pay, payout?.selected_completion_bonus];
    const mapped = ["total_base_units", "solver_base_units", "verifier_base_units", "keeper_base_units", "bonus_base_units"];
    if (!expected.every((value) => uint(value?.usdc_base_units)) || !count(payout?.selected_settled_rounds)) fail("Headline amounts unavailable");
    let matches = mapped.every((field, index) => totals[field] === expected[index].usdc_base_units)
      && totals.settlement_events === payout.selected_settled_rounds;
    const days = new Map();
    for (const row of rows) {
      const day = new Date(row.occurred_at).toISOString().slice(0, 10);
      const value = days.get(day) || { amount: 0n, settlements: 0 };
      value.amount += BigInt(row.total_base_units); value.settlements += row.is_settlement ? 1 : 0;
      days.set(day, value);
    }
    if (!Array.isArray(platform.daily)) fail("Daily payout series unavailable");
    const checked = new Set();
    for (const day of platform.daily) {
      if (!/^\d{4}-\d{2}-\d{2}$/.test(day?.day) || checked.has(day.day) || !uint(day.payout?.usdc_base_units) || !count(day.settled_rounds)) fail("Invalid daily payout series");
      checked.add(day.day);
      const actual = days.get(day.day) || { amount: 0n, settlements: 0 };
      matches = matches && actual.amount.toString() === day.payout.usdc_base_units && actual.settlements === day.settled_rounds;
    }
    matches = matches && [...days.keys()].every((day) => checked.has(day));
    return { status: matches ? "ready" : "partial", rows, summary: totals, excluded_rows: [], excluded_summary: null,
      excluded_contracts: JSON.parse(excluded), generated_at: pages[0].generated_at, snapshot: meta.snapshot,
      chain_freshness_verified: false, reason: matches ? "Exact selected headline components, settlements and daily totals match." : "Historical proof does not match the selected headline/day totals." };
  }
  function audit(platform, pages) {
    try { return inspect(platform, pages); } catch (error) {
      return { status: "unavailable", reason: error.message, rows: [], excluded_rows: [], summary: summary([]), excluded_summary: null, generated_at: null };
    }
  }
  function formatAmount(value) {
    if (!uint(value)) return "—";
    const amount = BigInt(value), fraction = (amount % 1000000n).toString().padStart(6, "0").replace(/0+$/, "");
    return `${(amount / 1000000n).toLocaleString("en-US")}${fraction ? `.${fraction}` : ""} USDC`;
  }
  async function requestJson(fetchImpl, url) {
    const target = new URL(url);
    if (target.origin !== API || ![PATH, "/v1/metrics/platform"].includes(target.pathname)) fail("Unapproved proof URL");
    const controller = new AbortController(), timer = setTimeout(() => controller.abort(), 12000);
    let reader;
    try {
      const response = await fetchImpl(target.href, { credentials: "omit", redirect: "error", signal: controller.signal, headers: { accept: "application/json" } });
      if (!response.ok) fail(response.status === 409 ? "Proof selection changed; restart from the aggregate" : `Proof request failed: HTTP ${response.status}`);
      if (!response.body?.getReader) fail("Bounded response streaming unavailable");
      reader = response.body.getReader();
      const decoder = new TextDecoder("utf-8", { fatal: true });
      let bytes = 0, text = "";
      while (true) {
        const { done, value } = await reader.read(); if (done) break;
        bytes += value.byteLength;
        if (bytes > MAX_BYTES) fail("Proof response exceeds byte limit");
        text += decoder.decode(value, { stream: true });
      }
      text += decoder.decode();
      return JSON.parse(text);
    } finally {
      clearTimeout(timer);
      if (reader) { try { await reader.cancel(); } catch (_) { /* preserve primary failure */ } }
    }
  }
  async function loadPages(platform, fetchImpl) {
    scope(platform);
    const pages = [], cursors = new Set();
    let cursor = null;
    for (let index = 0; index < MAX_PAGES; index += 1) {
      const page = await requestJson(fetchImpl, proofUrl(platform, cursor));
      pages.push(page);
      if (page.next_cursor === null) {
        const result = audit(platform, pages); if (result.status === "unavailable") fail(result.reason);
        return pages;
      }
      if (typeof page.next_cursor !== "string" || page.next_cursor.length > 128 || cursors.has(page.next_cursor)) fail("Invalid continuation");
      cursors.add(page.next_cursor); cursor = page.next_cursor;
    }
    fail("Proof page limit exceeded");
  }
  async function read(fetchImpl, period = "lifetime") {
    if (!["7d", "28d", "90d", "lifetime"].includes(period)) fail("Invalid period");
    const platform = await requestJson(fetchImpl, `${API}/v1/metrics/platform?period=${period}`);
    const pages = await loadPages(platform, fetchImpl);
    return { platform, pages, audit: audit(platform, pages) };
  }
  return { audit, formatAmount, loadPages, read, requestJson, proofUrl };
});
