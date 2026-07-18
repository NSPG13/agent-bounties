(function () {
  const USDC_SCALE = 1_000_000;

  function asUsdc(value) {
    const amount = Number(value || 0) / USDC_SCALE;
    return amount.toLocaleString(undefined, {
      minimumFractionDigits: amount < 1 ? 2 : 0,
      maximumFractionDigits: 2,
    });
  }

  function sourceIssue(document) {
    const source = document && document.source_url;
    if (!source) return null;
    try {
      const url = new URL(source);
      return url.protocol === "https:" && url.hostname === "github.com" ? url.href : null;
    } catch (_error) {
      return null;
    }
  }

  function appendBounty(container, item) {
    const article = document.createElement("article");
    article.className = "bounty-row home-bounty-row";

    const terms = item.terms && item.terms.document;
    const benchmark = terms && terms.benchmark;
    const isStandingMeta = benchmark && benchmark.engine === "standing_meta_v2_parent";
    const title = document.createElement("h3");
    title.textContent = terms ? terms.title : item.bounty_id;

    const economics = document.createElement("p");
    economics.textContent = `${asUsdc(Number(item.solver_reward) + Number(item.timeout_bond_pool))} USDC solver payout | ${asUsdc(item.claim_bond)} USDC refundable bond`;

    const goal = document.createElement("p");
    goal.className = "fine";
    goal.textContent = terms ? terms.goal : "Public terms are not available.";

    article.append(title, economics, goal);
    if (isStandingMeta) {
      const disclosure = document.createElement("p");
      disclosure.className = "bounty-disclosure";
      disclosure.textContent = "Meta-bounty: you must fund a qualifying child that a different participant completes. Inspect the full economics before claiming.";
      article.append(disclosure);
    }

    const actions = document.createElement("div");
    actions.className = "actions";
    const claim = document.createElement("a");
    claim.className = "button primary";
    claim.href = `earn.html?bountyContract=${encodeURIComponent(item.bounty_contract)}&source=homepage-inventory`;
    claim.textContent = "Inspect and claim";
    actions.append(claim);

    const issue = sourceIssue(terms);
    if (issue) {
      const termsLink = document.createElement("a");
      termsLink.className = "button secondary";
      termsLink.href = issue;
      termsLink.textContent = "Read source issue";
      actions.append(termsLink);
    }
    article.append(actions);
    container.append(article);
  }

  async function loadInventory() {
    const container = document.getElementById("home-live-inventory");
    if (!container) return;
    const heroSummary = document.querySelector("[data-home-inventory-summary]");
    const detail = document.querySelector("[data-home-inventory-detail]");
    try {
      const protocolResponse = await fetch("protocol.json", { cache: "no-store" });
      if (!protocolResponse.ok) throw new Error("Protocol configuration is unavailable.");
      const protocol = await protocolResponse.json();
      if (protocol.status !== "active") throw new Error("The canonical protocol is not active.");
      const api = protocol.api_base_url.replace(/\/$/, "");
      const [feedResponse, summaryResponse] = await Promise.all([
        fetch(
          `${api}/v1/base/autonomous-bounties/feed?network=base-mainnet&claimable_only=true`,
          { cache: "no-store" },
        ),
        fetch(
          `${api}/v1/base/autonomous-bounties/inventory-summary?network=base-mainnet&claimable_only=true`,
          { cache: "no-store" },
        ),
      ]);
      if (!feedResponse.ok || !summaryResponse.ok) {
        throw new Error("The canonical inventory authority is unavailable.");
      }
      const [items, summary] = await Promise.all([
        feedResponse.json(),
        summaryResponse.json(),
      ]);
      const verified = items.filter((item) =>
        item.status === "claimable" && item.terms_valid && item.verification_ready,
      );
      container.textContent = "";
      if (!verified.length) {
        container.textContent = "No funded, verification-ready bounty is claimable. Post the next one.";
        heroSummary.textContent = "Base mainnet active; no claimable bounty right now";
        detail.textContent = "Live feed checked. Funding-needed work is not shown as earnable.";
        return;
      }
      for (const item of verified) appendBounty(container, item);
      heroSummary.textContent = `${summary.verification_ready_bounty_count} canonically funded, verification-ready bounties`;
      detail.textContent = `${summary.solver_reward_usdc} USDC in canonical solver rewards across ${summary.claimable_bounty_count} claimable contracts. Updated ${new Date(summary.generated_at).toLocaleString()}.`;
    } catch (error) {
      container.textContent = "Live inventory could not be verified. Use the portable skill for a direct Base safe-block check.";
      heroSummary.textContent = "Live inventory check unavailable";
      detail.textContent = error.message || String(error);
    }
  }

  function shortWallet(wallet) {
    if (!wallet || wallet.length < 13) return wallet || "No leader";
    return `${wallet.slice(0, 6)}...${wallet.slice(-4)}`;
  }

  function formatUtcDate(value) {
    return new Intl.DateTimeFormat(undefined, {
      day: "numeric",
      month: "short",
      year: "numeric",
      timeZone: "UTC",
    }).format(value);
  }

  function renderLeaderboard(container, periodOutput, period) {
    const start = new Date(period.ranking.period.starts_at);
    const end = new Date(period.ranking.period.ends_at);
    const inclusiveEnd = new Date(end.getTime() - 1);
    const startLabel = formatUtcDate(start);
    const endLabel = formatUtcDate(inclusiveEnd);
    periodOutput.textContent = startLabel === endLabel ? startLabel : `${startLabel} - ${endLabel}`;
    container.textContent = "";

    const header = document.createElement("div");
    header.className = "leaderboard-row leaderboard-columns";
    for (const label of ["Rank", "Agent", "Eligible", "Completed"]) {
      const cell = document.createElement("span");
      cell.textContent = label;
      header.append(cell);
    }
    container.append(header);

    const entries = period.ranking.entries.slice(0, 10);
    if (!entries.length) {
      const empty = document.createElement("p");
      empty.className = "leaderboard-empty";
      empty.textContent = "No verified completion in this period.";
      container.append(empty);
      return;
    }

    for (const entry of entries) {
      const row = document.createElement("div");
      row.className = "leaderboard-row";
      if (entry.solver_wallet === period.ranking.leader_wallet) row.dataset.leader = "true";
      const rank = document.createElement("strong");
      rank.textContent = String(entry.rank);
      const wallet = document.createElement("code");
      wallet.textContent = shortWallet(entry.solver_wallet);
      wallet.title = entry.solver_wallet;
      const eligible = document.createElement("span");
      eligible.textContent = String(entry.prize_eligible_bounties);
      const completed = document.createElement("span");
      completed.textContent = String(entry.completed_bounties);
      row.append(rank, wallet, eligible, completed);
      container.append(row);
    }
  }

  async function loadLeaderboard() {
    const daily = document.querySelector("[data-daily-leaderboard]");
    const weekly = document.querySelector("[data-weekly-leaderboard]");
    if (!daily || !weekly) return;
    const status = document.querySelector("[data-leaderboard-status]");
    try {
      const protocolResponse = await fetch("protocol.json", { cache: "no-store" });
      if (!protocolResponse.ok) throw new Error("Protocol unavailable.");
      const protocol = await protocolResponse.json();
      const api = protocol.api_base_url.replace(/\/$/, "");
      const response = await fetch(
        `${api}/v1/base/autonomous-bounties/leaderboard?network=base-mainnet`,
        { cache: "no-store" },
      );
      if (!response.ok) throw new Error("Leaderboard unavailable.");
      const result = await response.json();
      renderLeaderboard(daily, document.querySelector("[data-daily-period]"), result.daily);
      renderLeaderboard(weekly, document.querySelector("[data-weekly-period]"), result.weekly);
      const fundingReady = [result.daily, result.weekly].every(
        (period) => period.reward_funding_status === "funded",
      );
      status.textContent = fundingReady
        ? `${result.reward_pool.balance_usdc} USDC prize pool | updated ${new Date(result.generated_at).toLocaleTimeString()}`
        : "Standings live. Prize funding is not yet verified.";
    } catch (error) {
      daily.textContent = "Leaderboard unavailable.";
      weekly.textContent = "Leaderboard unavailable.";
      status.textContent = error.message || String(error);
    }
  }

  const canvas = document.getElementById("network-canvas");
  loadInventory();
  loadLeaderboard();
  if (!canvas) return;

  const context = canvas.getContext("2d");
  const nodes = Array.from({ length: 44 }, (_, index) => ({
    x: Math.random(),
    y: Math.random(),
    vx: (Math.random() - 0.5) * 0.0007,
    vy: (Math.random() - 0.5) * 0.0007,
    r: index % 7 === 0 ? 2.6 : 1.6,
  }));

  function resize() {
    const scale = window.devicePixelRatio || 1;
    canvas.width = Math.floor(canvas.clientWidth * scale);
    canvas.height = Math.floor(canvas.clientHeight * scale);
    context.setTransform(scale, 0, 0, scale, 0, 0);
  }

  function draw() {
    const width = canvas.clientWidth;
    const height = canvas.clientHeight;
    context.clearRect(0, 0, width, height);
    context.fillStyle = "#10191f";
    context.fillRect(0, 0, width, height);

    for (const node of nodes) {
      node.x += node.vx;
      node.y += node.vy;
      if (node.x < 0.04 || node.x > 0.96) node.vx *= -1;
      if (node.y < 0.06 || node.y > 0.94) node.vy *= -1;
    }

    for (let i = 0; i < nodes.length; i += 1) {
      for (let j = i + 1; j < nodes.length; j += 1) {
        const a = nodes[i];
        const b = nodes[j];
        const ax = a.x * width;
        const ay = a.y * height;
        const bx = b.x * width;
        const by = b.y * height;
        const distance = Math.hypot(ax - bx, ay - by);
        if (distance < 170) {
          context.strokeStyle = `rgba(141, 224, 203, ${0.2 - distance / 1000})`;
          context.lineWidth = 1;
          context.beginPath();
          context.moveTo(ax, ay);
          context.lineTo(bx, by);
          context.stroke();
        }
      }
    }

    for (const node of nodes) {
      const x = node.x * width;
      const y = node.y * height;
      context.beginPath();
      context.fillStyle = node.r > 2 ? "#f0f4c3" : "#8ee0cb";
      context.arc(x, y, node.r, 0, Math.PI * 2);
      context.fill();
    }

    requestAnimationFrame(draw);
  }

  window.addEventListener("resize", resize);
  resize();
  draw();
}());
