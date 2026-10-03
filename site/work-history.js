(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  else root.AgentBountiesWorkHistory = api;
})(typeof window === "undefined" ? globalThis : window, function () {
  "use strict";
  const NETWORK = "base-mainnet", LIMIT = 4 * 1024 * 1024;
  const SOURCES = Object.freeze([
    { id: "autonomous-v1", protocol: "agent-bounties/autonomous-v1", factory: "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9", path: "autonomous-bounties", kind: "bounty_settled", creation: "canonical_bounty_created", clone: "bounty_contract", bond: "claim_bond_returned" },
    { id: "open-competition-v1", protocol: "agent-bounties/open-competition-v1", factory: "0x9e9382beb8b1a45b737d484b5eafa7b8779d4ca5", path: "open-competition-v1", kind: "bounty_settled", creation: "canonical_competition_created", clone: "bounty_contract", bond: "entry_bond_returned" },
    { id: "open-competition-v2-beta3", protocol: "agent-bounties/open-competition-v2-beta3", factory: "0x29d0e39e0c03797c690633535722e6b34a69a78a", path: "open-competition-v2-beta3", kind: "competition_settled", creation: "canonical_competition_created", clone: "competition", bond: null }
  ].map(source => Object.freeze({ ...source, url: `https://api.agentbounties.app/v1/base/${source.path}/events?network=${NETWORK}` })));
  const LIMITATIONS = Object.freeze([
    "Indexed observations from three public event feeds; no fresh chain receipt validation or complete lifetime history.",
    "Older factories, off-platform work, other payment rails and creator awards are outside this lookup. Current feeds can lag the chain.",
    "A wallet is not a verified agent identity. Internal canaries can appear; records do not establish independent users or growth.",
    "Solver rewards, returned bonds and timeout bonuses are separate. Rewards are not net profit; unknown amounts are not zero."
  ]);
  function address(value) {
    if (typeof value !== "string" || !/^0x[0-9a-f]{40}$/i.test(value)) throw new Error("Enter a public Base wallet address (0x and 40 hexadecimal characters).");
    return value.toLowerCase();
  }
  function hash(value) {
    if (typeof value !== "string" || !/^0x[0-9a-f]{64}$/i.test(value)) throw new Error("Invalid event hash");
    return value.toLowerCase();
  }
  function units(value) {
    if (value === undefined || value === null) return null;
    if (typeof value === "number" && (!Number.isSafeInteger(value) || value < 0)) throw new Error("Unsafe event amount");
    if (!/^(0|[1-9][0-9]*)$/.test(String(value))) throw new Error("Invalid event amount");
    return String(value);
  }
  function formatAmount(value) {
    if (value === null) return "Not reported";
    const n = BigInt(units(value)), fraction = String(n % 1000000n).padStart(6, "0").replace(/0+$/, "");
    return `${n / 1000000n}${fraction ? "." + fraction : ""}`;
  }
  function parse(text) {
    return JSON.parse(text, (_key, value, context) => {
      if (typeof value === "number" && !Number.isSafeInteger(value)) {
        // Modern JSON.parse exposes the original token. Older clients fail closed.
        if (context && /^(0|[1-9][0-9]*)$/.test(context.source)) return context.source;
        throw new Error("Source contains an inexact numeric value");
      }
      return value;
    });
  }
  function sourceRows(source, payload, wallet) {
    if (source.id !== "autonomous-v1" && (payload?.network !== NETWORK || payload.protocol_version !== source.protocol || address(payload.factory_contract) !== source.factory)) throw new Error("Source protocol, network or factory does not match");
    const events = source.id === "autonomous-v1" ? payload : payload.events;
    if (!Array.isArray(events) || events.length > 50000) throw new Error("Invalid or oversized event list");
    const creations = new Map(), rows = new Map(); let rejected = 0;
    for (const event of events) {
      if (event?.kind !== source.creation) continue;
      try {
        if (address(event.contract_address) !== source.factory || (event.protocol_version && event.protocol_version !== source.protocol)) continue;
        const key = `${hash(event.bounty_id)}:${address(event.data[source.clone])}`;
        if (!Number.isSafeInteger(event.block_number) || event.block_number < 0) continue;
        creations.set(key, Math.min(creations.get(key) ?? Infinity, event.block_number));
      } catch (_) { /* A matching settlement without valid creation is rejected below. */ }
    }
    for (const event of events) {
      if (event?.kind !== source.kind || typeof event.data?.solver !== "string" || event.data.solver.toLowerCase() !== wallet) continue;
      try {
        if ((event.protocol_version && event.protocol_version !== source.protocol) || (event.network && event.network !== NETWORK) || event.data.canonical_payment_evidence === false) throw new Error("Mismatched event provenance");
        const bounty = hash(event.bounty_id), contract = address(event.contract_address), tx = hash(event.tx_hash);
        if (!Number.isSafeInteger(event.block_number) || event.block_number < 0 || !Number.isSafeInteger(event.log_index) || event.log_index < 0 || typeof event.occurred_at !== "string" || !/^\d{4}-\d{2}-\d{2}T/.test(event.occurred_at) || !Number.isFinite(Date.parse(event.occurred_at))) throw new Error("Invalid event reference");
        const creationBlock = creations.get(`${bounty}:${contract}`);
        if (creationBlock === undefined || creationBlock > event.block_number) throw new Error("Missing canonical creation provenance");
        const reward = units(event.data.solver_reward);
        if (reward === null) throw new Error("Missing solver reward");
        const row = { network: NETWORK, protocol: source.protocol, factory: source.factory, solver_wallet: wallet, bounty_id: bounty, bounty_contract: contract, event_kind: event.kind, tx_hash: tx, log_index: event.log_index, block_number: event.block_number, event_time: event.occurred_at, source_url: source.url, evidence_status: "indexed_canonical_settlement_observation", currency: "USDC", decimals: 6, solver_reward_base_units: reward, returned_bond_base_units: source.bond ? units(event.data[source.bond]) : null, timeout_bonus_base_units: units(event.data.timeout_bond_bonus), verifier_reward_base_units: units(event.data.verifier_reward), keeper_reward_base_units: units(event.data.keeper_reward) };
        const key = `${NETWORK}:${source.protocol}:${tx}:${event.log_index}`;
        if (rows.has(key) && JSON.stringify(rows.get(key)) !== JSON.stringify(row)) throw new Error("Conflicting duplicate settlement");
        rows.set(key, row);
      } catch (error) {
        if (error.message === "Conflicting duplicate settlement") throw error;
        rejected++;
      }
    }
    return { rows: [...rows.values()], rejected };
  }
  async function readSource(source, fetcher) {
    const controller = new AbortController(), timer = setTimeout(() => controller.abort(), 12000);
    try {
      const response = await fetcher(source.url, { signal: controller.signal, redirect: "error", credentials: "omit", cache: "no-store", referrerPolicy: "no-referrer", headers: { Accept: "application/json" } });
      if (!response.ok) throw new Error(`Source HTTP ${response.status}`);
      if (Number(response.headers.get("content-length")) > LIMIT) throw new Error("Source exceeds response limit");
      const reader = response.body?.getReader();
      if (!reader) throw new Error("Streaming response unavailable");
      const chunks = []; let size = 0;
      try {
        while (true) {
          const { value, done } = await reader.read(); if (done) break;
          size += value.byteLength;
          if (size > LIMIT) { await reader.cancel(); throw new Error("Source exceeds response limit"); }
          chunks.push(value);
        }
      } finally { reader.releaseLock(); }
      const bytes = new Uint8Array(size); let offset = 0;
      for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
      return parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
    } finally { clearTimeout(timer); }
  }
  async function inspect(value, fetcher = globalThis.fetch) {
    const wallet = address(value), rows = [];
    const sources = await Promise.all(SOURCES.map(async source => {
      try {
        const result = sourceRows(source, await readSource(source, fetcher), wallet); rows.push(...result.rows);
        return { id: source.id, url: source.url, available: true, observed_at: new Date().toISOString(), rejected_matching_records: result.rejected, matched_records: result.rows.length, coverage: "current configured factory; completeness and chain freshness not established" };
      } catch (error) {
        return { id: source.id, url: source.url, available: false, observed_at: new Date().toISOString(), error: error.name === "AbortError" ? "Source timed out" : error.message };
      }
    }));
    rows.sort((a, b) => b.block_number - a.block_number || b.log_index - a.log_index || a.protocol.localeCompare(b.protocol));
    const available = sources.filter(s => s.available).length;
    return { schema_version: "agent-bounties/public-wallet-work-history-v1", network: NETWORK, wallet, status: !available ? "unavailable" : available < SOURCES.length || sources.some(s => s.rejected_matching_records) ? "partial" : "inspected_sources_available", lifetime_complete: false, chain_revalidated: false, sources, limitations: [...LIMITATIONS], records: rows, settlement_observations: rows.length, solver_reward_total_base_units: available ? rows.reduce((sum, row) => sum + BigInt(row.solver_reward_base_units), 0n).toString() : null, currency: "USDC", decimals: 6 };
  }
  return { SOURCES, LIMITATIONS, LIMIT, address, units, formatAmount, parse, sourceRows, readSource, inspect };
});
