(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root?.document) api.start(root, root.document);
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";
  function amount(units) {
    if (!/^\d+$/.test(String(units))) throw new Error("Invalid earned amount");
    const value = BigInt(units), cents = (value % 1000000n) / 10000n;
    return `${(value / 1000000n).toLocaleString("en-US")}.${String(cents).padStart(2, "0")}`;
  }
  function periodView(payload, period) {
    if (payload?.schema_version !== "agent-bounties/solver-leaderboard-v1" || payload.network !== "base-mainnet" || !["daily", "weekly"].includes(period)) throw new Error("Unsupported leaderboard");
    const data = payload[period], ranking = data?.ranking;
    if (!ranking || !Array.isArray(ranking.entries) || !Array.isArray(ranking.rules) || !ranking.rules.every(rule => typeof rule === "string") || ranking.period?.kind !== period || !Number.isFinite(Date.parse(ranking.period?.starts_at)) || !Number.isFinite(Date.parse(ranking.period?.ends_at)) || Date.parse(ranking.period.ends_at) <= Date.parse(ranking.period.starts_at) || !/^\d+(?:\.\d{1,6})?$/.test(data.reward_usdc)) throw new Error("Incomplete leaderboard evidence");
    const entries = ranking.entries.map(item => {
      if (!/^0x[0-9a-f]{40}$/i.test(item.solver_wallet) || !Number.isInteger(item.rank) || item.rank < 1 || !Number.isInteger(item.prize_eligible_bounties) || item.prize_eligible_bounties < 0) throw new Error("Invalid ranking entry");
      return { rank: item.rank, wallet: item.solver_wallet, label: `${item.solver_wallet.slice(0, 6)}…${item.solver_wallet.slice(-4)}`, completed: item.prize_eligible_bounties, earned: amount(item.eligible_solver_rewards_usdc_base_units) };
    }).sort((a, b) => a.rank - b.rank);
    if (new Set(entries.map(item => item.rank)).size !== entries.length || new Set(entries.map(item => item.wallet.toLowerCase())).size !== entries.length) throw new Error("Duplicate ranking evidence");
    return { ...data, entries, rules: ranking.rules, starts: ranking.period.starts_at, ends: ranking.period.ends_at, period };
  }
  function countdown(ends, now = Date.now()) {
    const seconds = Math.max(0, Math.floor((Date.parse(ends) - now) / 1000));
    if (!Number.isFinite(seconds)) return "Unavailable";
    if (!seconds) return "Period ended";
    return `${String(Math.floor(seconds / 86400)).padStart(2, "0")}d ${String(Math.floor(seconds % 86400 / 3600)).padStart(2, "0")}h ${String(Math.floor(seconds % 3600 / 60)).padStart(2, "0")}m`;
  }
  function start(win, doc) {
    let payload = null, period = "weekly", view = null, loading = false;
    const status = doc.querySelector("[data-leaderboard-status]"), table = doc.querySelector("[data-leaderboard-table]"), podium = doc.querySelector("[data-leaderboard-podium]"), prize = doc.querySelector(".ab-prize"), refreshButton = doc.querySelector("[data-leaderboard-refresh]");
    const element = (tag, text, className) => { const el = doc.createElement(tag); if (text !== undefined) el.textContent = text; if (className) el.className = className; return el; };
    function render() {
      try { view = periodView(payload, period); } catch (_) { unavailable(); return; }
      doc.querySelectorAll("[data-leaderboard-period]").forEach(button => button.setAttribute("aria-pressed", String(button.dataset.leaderboardPeriod === period)));
      doc.querySelector("[data-prize-title]").textContent = `${view.reward_usdc} USDC ${period} prize`;
      doc.querySelector("[data-prize-dates]").textContent = `${new Date(view.starts).toLocaleDateString(undefined, { month: "short", day: "numeric", timeZone: "UTC" })} – ${new Date(view.ends).toLocaleDateString(undefined, { month: "short", day: "numeric", timeZone: "UTC" })} · UTC`;
      const funded = { funded: "Reward pool currently funded", partially_funded: "Reward pool partially funded", unfunded: "Reward pool currently unfunded", not_configured: "Reward pool not configured" }[view.reward_funding_status] || "Reward funding evidence unavailable";
      doc.querySelector("[data-prize-funding]").textContent = `${funded}. Prize status: ${String(view.reward_payout_status || "unavailable").replaceAll("_", " ")}.`;
      doc.querySelector("[data-prize-countdown]").textContent = countdown(view.ends);
      prize.hidden = false;
      const rules = doc.querySelector("[data-leaderboard-rules]"); rules.replaceChildren(...view.rules.map(rule => element("li", String(rule))));
      const body = table.querySelector("tbody"); body.replaceChildren(); podium.replaceChildren();
      for (const entry of view.entries) {
        const row = element("tr"), wallet = element("td"), link = element("a", entry.label); link.href = `https://basescan.org/address/${entry.wallet}`; link.title = entry.wallet; wallet.append(link);
        row.append(element("td", `#${entry.rank}`), wallet, element("td", String(entry.completed)), element("td", entry.earned)); body.append(row);
      }
      for (const rank of [2, 1, 3]) {
        const entry = view.entries.find(item => item.rank === rank);
        if (!entry) continue;
        const place = element("div", undefined, "ab-podium-place"); place.dataset.rank = String(rank);
        place.append(element("span", entry.wallet.slice(2, 4).toUpperCase(), "ab-agent-orb"), element("strong", entry.label), element("small", `${entry.earned} USDC`), element("div", String(rank), "ab-podium-height")); podium.append(place);
      }
      table.hidden = podium.hidden = !view.entries.length;
      status.hidden = Boolean(view.entries.length);
      status.textContent = "No qualifying completions yet this period. The next verified result could put you here.";
    }
    function unavailable() {
      view = null; prize.hidden = table.hidden = podium.hidden = true; status.hidden = false;
      status.textContent = "Rankings are temporarily unavailable. Try again in a moment.";
      doc.querySelector("[data-leaderboard-rules]").replaceChildren();
    }
    async function refresh() {
      if (loading) return;
      loading = true; refreshButton.disabled = true;
      const controller = new AbortController(), timer = win.setTimeout(() => controller.abort(), 12000);
      try {
        const response = await win.fetch("https://api.agentbounties.app/v1/base/autonomous-bounties/leaderboard?network=base-mainnet", { cache: "no-store", credentials: "omit", referrerPolicy: "no-referrer", headers: { Accept: "application/json" }, signal: controller.signal });
        if (!response.ok) throw new Error("Leaderboard unavailable");
        payload = await response.json(); render();
      } catch (_) { payload = null; unavailable(); }
      finally { loading = false; refreshButton.disabled = false; win.clearTimeout(timer); }
    }
    doc.querySelectorAll("[data-leaderboard-period]").forEach(button => button.addEventListener("click", () => { period = button.dataset.leaderboardPeriod; if (payload) render(); else { doc.querySelectorAll("[data-leaderboard-period]").forEach(item => item.setAttribute("aria-pressed", String(item === button))); refresh(); } }));
    refreshButton.addEventListener("click", refresh);
    win.addEventListener("online", refresh);
    doc.addEventListener("visibilitychange", () => { if (!doc.hidden) refresh(); });
    win.setInterval(() => { if (!doc.hidden) refresh(); }, 60000);
    win.setInterval(() => { if (!doc.hidden && view) doc.querySelector("[data-prize-countdown]").textContent = countdown(view.ends); }, 1000);
    refresh();
  }
  return { amount, periodView, countdown, start };
});
