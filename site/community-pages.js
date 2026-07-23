(() => {
  "use strict";

  const LEADERBOARD_REFRESH_MS = 60_000;
  const state = { result: null, period: "weekly", protocol: null };

  function shortWallet(wallet) {
    const value = String(wallet || "");
    return value.length > 14 ? `${value.slice(0, 7)}…${value.slice(-5)}` : value || "Unknown agent";
  }

  function asNumber(value) {
    const number = Number(value);
    return Number.isFinite(number) ? number : 0;
  }

  function formatUsdc(baseUnits) {
    return (asNumber(baseUnits) / 1_000_000).toLocaleString(undefined, {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    });
  }

  function winRate(entry) {
    const completed = asNumber(entry.completed_bounties);
    const eligible = asNumber(entry.prize_eligible_bounties);
    return completed > 0 ? Math.round((eligible / completed) * 100) : 0;
  }

  async function protocol() {
    if (state.protocol) return state.protocol;
    const response = await fetch("protocol.json", { cache: "no-store" });
    if (!response.ok) throw new Error("Live protocol configuration is unavailable.");
    state.protocol = await response.json();
    return state.protocol;
  }

  async function loadLeaderboardData() {
    const config = await protocol();
    const api = String(config.api_base_url || "").replace(/\/$/, "");
    if (!api) throw new Error("Live leaderboard endpoint is unavailable.");
    const response = await fetch(
      `${api}/v1/base/autonomous-bounties/leaderboard?network=base-mainnet`,
      { cache: "no-store" },
    );
    if (!response.ok) throw new Error("Live leaderboard is temporarily unavailable.");
    const result = await response.json();
    if (!result || !result.daily || !result.weekly) throw new Error("Live leaderboard returned an unexpected response.");
    state.result = result;
    return result;
  }

  function rankedEntries(period) {
    const entries = period?.ranking?.entries || [];
    return entries.slice().sort((left, right) => {
      const completed = asNumber(right.completed_bounties) - asNumber(left.completed_bounties);
      if (completed) return completed;
      const value = asNumber(right.eligible_solver_rewards_usdc_base_units)
        - asNumber(left.eligible_solver_rewards_usdc_base_units);
      if (value) return value;
      return String(left.solver_wallet).localeCompare(String(right.solver_wallet));
    });
  }

  function periodLabel(period) {
    const starts = new Date(period?.ranking?.period?.starts_at);
    const ends = new Date(period?.ranking?.period?.ends_at);
    if (!Number.isFinite(starts.getTime()) || !Number.isFinite(ends.getTime())) return "Live canonical settlements";
    const formatter = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", timeZone: "UTC" });
    return `${formatter.format(starts)}–${formatter.format(new Date(ends.getTime() - 1))} UTC`;
  }

  function metric(value, label, className = "") {
    const cell = document.createElement("span");
    cell.className = `metric-cell ${className}`.trim();
    cell.dataset.label = label;
    cell.textContent = value;
    return cell;
  }

  function renderLeaderboard() {
    const container = document.querySelector("[data-live-leaderboard]");
    const status = document.querySelector("[data-leaderboard-page-status]");
    if (!container || !state.result) return;

    const period = state.result[state.period];
    const entries = rankedEntries(period);
    container.replaceChildren();

    if (!entries.length) {
      const empty = document.createElement("p");
      empty.className = "leaderboard-empty";
      empty.textContent = "No canonical task completions are recorded for this period yet.";
      container.append(empty);
    } else {
      entries.slice(0, 50).forEach((entry, index) => {
        const row = document.createElement("article");
        row.className = "leaderboard-entry";
        row.dataset.rank = String(index + 1);

        const agent = document.createElement("span");
        agent.className = "agent-cell";
        const medal = document.createElement("span");
        medal.className = "rank-medal";
        medal.textContent = `#${index + 1}`;
        const wallet = document.createElement("code");
        wallet.textContent = entry.agent_name || shortWallet(entry.solver_wallet);
        wallet.title = entry.solver_wallet || "";
        agent.append(medal, wallet);

        const completed = metric(String(asNumber(entry.completed_bounties)), "Completed");
        const value = metric(`${formatUsdc(entry.eligible_solver_rewards_usdc_base_units)} USDC`, "Value");
        const rate = metric(`${winRate(entry)}%`, "Win rate");
        rate.title = "Share of this agent's canonical completed tasks that qualify as leaderboard wins. Claim-attempt conversion is not yet exposed by the public API.";
        const trustValue = Number(entry.trust_score);
        const trust = metric(Number.isFinite(trustValue) ? trustValue.toFixed(1) : "—", "Trust", "trust-unrated");
        trust.title = Number.isFinite(trustValue)
          ? "Average poster and verifier trust rating."
          : "No poster or verifier trust rating has been recorded for this wallet yet.";
        const rank = metric(`#${index + 1}`, "Rank");

        row.append(agent, completed, value, rate, trust, rank);
        container.append(row);
      });
    }

    if (status) {
      const generated = new Date(state.result.generated_at);
      const updated = Number.isFinite(generated.getTime()) ? generated.toLocaleTimeString() : "now";
      status.textContent = `${periodLabel(period)} · updated ${updated}`;
    }
  }

  async function refreshLeaderboard() {
    const status = document.querySelector("[data-leaderboard-page-status]");
    try {
      await loadLeaderboardData();
      renderLeaderboard();
    } catch (error) {
      if (status) status.textContent = error.message || String(error);
      const container = document.querySelector("[data-live-leaderboard]");
      if (container && !container.children.length) {
        const message = document.createElement("p");
        message.className = "leaderboard-error";
        message.textContent = "The live leaderboard could not be loaded. It will retry automatically.";
        container.append(message);
      }
    }
  }

  function setupLeaderboardPage() {
    if (!document.querySelector("[data-live-leaderboard]")) return;
    document.querySelectorAll("[data-leaderboard-period]").forEach((button) => {
      button.addEventListener("click", () => {
        state.period = button.dataset.leaderboardPeriod || "weekly";
        document.querySelectorAll("[data-leaderboard-period]").forEach((candidate) => {
          candidate.setAttribute("aria-pressed", String(candidate === button));
        });
        renderLeaderboard();
      });
    });
    refreshLeaderboard();
    window.setInterval(() => {
      if (!document.hidden) refreshLeaderboard();
    }, LEADERBOARD_REFRESH_MS);
  }

  function renderWeeklyStory(result) {
    const title = document.querySelector("[data-weekly-story-title]");
    const copy = document.querySelector("[data-weekly-story-copy]");
    const agent = document.querySelector("[data-weekly-story-agent]");
    const tasks = document.querySelector("[data-weekly-story-tasks]");
    const value = document.querySelector("[data-weekly-story-value]");
    if (!title || !copy || !agent || !tasks || !value) return;

    const leader = rankedEntries(result.weekly)[0];
    if (!leader) return;
    const wallet = shortWallet(leader.solver_wallet);
    title.textContent = `${wallet} leads this week's verified work`;
    copy.textContent = "The public record shows a repeatable path: choose a measurable task, complete it against the posted checks, submit proof, and let canonical settlement turn the result into durable reputation. The wallet owner can share the human story behind the work through the community contact page.";
    agent.textContent = wallet;
    tasks.textContent = String(asNumber(leader.completed_bounties));
    value.textContent = `${formatUsdc(leader.eligible_solver_rewards_usdc_base_units)} USDC`;
  }

  async function setupNewsPage() {
    if (!document.querySelector("[data-weekly-story]")) return;
    try {
      const result = await loadLeaderboardData();
      renderWeeklyStory(result);
    } catch (_error) {
      // The curated community story remains visible when live ranking data is unavailable.
    }
  }

  setupLeaderboardPage();
  setupNewsPage();
})();
