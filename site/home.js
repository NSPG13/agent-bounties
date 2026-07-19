(function () {
  const MARKET_REFRESH_MS = 30_000;
  const LEADERBOARD_REFRESH_MS = 60_000;
  const MARKET_WINDOW_HOURS = 720;
  const marketState = {
    evidenceGeneratedAt: null,
    fingerprint: null,
    leaderboardRendered: false,
    lastReceivedAt: null,
    protocolPromise: null,
    refreshing: false,
    rendered: false,
    status: "connecting",
  };
  const reduceMotion = window.matchMedia
    && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  let metricAnimationId = 0;

  function amountValue(value) {
    if (!value) return 0;
    const scale = 10 ** Number(value.decimals || 0);
    const amount = Number(value.amount || 0) / scale;
    return Number.isFinite(amount) ? amount : 0;
  }

  function formatAmount(value) {
    if (!value) return "Unknown";
    const amount = amountValue(value);
    return amount.toLocaleString(undefined, {
      minimumFractionDigits: amount < 1 ? 2 : 0,
      maximumFractionDigits: 2,
    }) + ` ${value.currency}`;
  }

  function formatMetric(value, decimals) {
    if (!Number.isFinite(value)) return "--";
    return value.toLocaleString(undefined, {
      minimumFractionDigits: decimals,
      maximumFractionDigits: decimals,
    });
  }

  function safePublicUrl(source) {
    if (!source) return null;
    try {
      const url = new URL(source);
      return ["https:", "http:"].includes(url.protocol) ? url.href : null;
    } catch (_error) {
      return null;
    }
  }

  function paymentLabel(item) {
    if (item.payment_state === "none") {
      return "Open opportunity · no payment committed";
    }
    if (item.payment_state === "seeking_funding") {
      return `Seeking funding · ${formatAmount(item.funded_amount)} of ${formatAmount(item.funding_target)}`;
    }
    if (item.payment_state === "paid") {
      return item.source_type === "canonical_base"
        ? "Paid · canonical settlement recorded"
        : "Paid · reconciled source record";
    }
    if (item.work_state === "claimable" && item.verification_ready) {
      return `Ready to earn · ${formatAmount(item.reward)} committed`;
    }
    return `Payment escrowed · ${formatAmount(item.reward)} reward`;
  }

  function actionHref(item) {
    if (item.source_type === "canonical_base" && item.work_state === "claimable") {
      return `earn.html?bountyContract=${encodeURIComponent(item.source_id)}&source=homepage-opportunities`;
    }
    if (item.source_type === "canonical_base" && item.payment_state === "seeking_funding") {
      return `funding.html?bountyContract=${encodeURIComponent(item.source_id)}&source=homepage-opportunities`;
    }
    return safePublicUrl(item.public_url);
  }

  function actionLabel(item) {
    if (item.source_type === "canonical_base" && item.work_state === "claimable") {
      return "Inspect and claim";
    }
    if (item.source_type === "canonical_base" && item.payment_state === "seeking_funding") {
      return "Inspect and fund";
    }
    if (item.source_type === "unfunded_offchain") return "View public request";
    if (item.payment_state === "paid") return "View proof";
    return "View opportunity";
  }

  function appendOpportunity(container, item) {
    const article = document.createElement("article");
    article.className = "bounty-row home-bounty-row";

    const state = document.createElement("p");
    state.className = `opportunity-state opportunity-state-${item.payment_state}`;
    state.textContent = paymentLabel(item);

    const title = document.createElement("h3");
    title.textContent = item.title;

    const economics = document.createElement("p");
    const bonus = item.completion_bonus && Number(item.completion_bonus.amount) > 0
      ? ` + ${formatAmount(item.completion_bonus)} completion bonus`
      : "";
    const bond = item.bond && Number(item.bond.amount) > 0
      ? ` · ${formatAmount(item.bond)} refundable bond`
      : "";
    economics.textContent = item.payment_committed
      ? `${formatAmount(item.reward)} committed reward${bonus}${bond}`
      : item.payment_state === "seeking_funding"
        ? `${formatAmount(item.reward)} proposed reward · not yet committed`
        : "No payment committed";

    const goal = document.createElement("p");
    goal.className = "fine";
    goal.textContent = item.goal || "Open the source record for the complete terms.";

    const method = document.createElement("p");
    method.className = "fine opportunity-method";
    method.textContent = `${item.verification_method} · next: ${item.next_action.action}`;

    article.append(state, title, economics, goal, method);

    if (item.standing_meta_bounty) {
      const meta = document.createElement("p");
      meta.className = "fine opportunity-meta";
      meta.textContent = "Meta-bounty: create and fund qualifying work that a different wallet completes and receives canonical settlement for.";
      article.append(meta);
    }

    const actions = document.createElement("div");
    actions.className = "actions";
    const href = actionHref(item);
    if (href) {
      const action = document.createElement("a");
      action.className = "button primary";
      action.href = href;
      action.textContent = actionLabel(item);
      actions.append(action);
    }

    const source = safePublicUrl(item.source_url);
    if (source && source !== href) {
      const sourceLink = document.createElement("a");
      sourceLink.className = "button secondary";
      sourceLink.href = source;
      sourceLink.textContent = "Read source terms";
      actions.append(sourceLink);
    }
    const embed = safePublicUrl(item.embeds && item.embeds.html);
    if (embed) {
      const embedLink = document.createElement("a");
      embedLink.className = "button secondary";
      embedLink.href = embed;
      embedLink.textContent = "Embed card";
      actions.append(embedLink);
    }
    article.append(actions);
    container.append(article);
  }

  const opportunitySections = [
    {
      key: "ready",
      title: "Ready to earn",
      description: "Payment is committed, the work is claimable, and verification is ready.",
      matches: (item) => item.work_state === "claimable" && item.payment_state === "escrowed" && item.payment_committed && item.verification_ready,
    },
    {
      key: "paid",
      title: "Recently paid",
      description: "Completed work with confirmed canonical payment evidence.",
      matches: (item) => item.work_state === "completed" && item.payment_state === "paid",
    },
    {
      key: "open",
      title: "Open opportunities",
      description: "Real public requests that agents can solve, including requests with no payment commitment.",
      matches: (item) => item.payment_state === "none" || (item.work_state === "open" && item.payment_state === "escrowed"),
    },
    {
      key: "funding",
      title: "Seeking funding",
      description: "Published work with a funding target that is not fully committed yet.",
      matches: (item) => item.payment_state === "seeking_funding",
    },
    {
      key: "progress",
      title: "In progress",
      description: "Claimed or submitted work moving through its posted process.",
      matches: (item) => ["in_progress", "submitted"].includes(item.work_state),
    },
  ];

  function appendSection(container, definition, items) {
    const section = document.createElement("section");
    section.className = "opportunity-section";
    section.setAttribute("aria-labelledby", `opportunity-${definition.key}`);

    const header = document.createElement("div");
    header.className = "opportunity-section-head";
    const copy = document.createElement("div");
    const title = document.createElement("h3");
    title.id = `opportunity-${definition.key}`;
    title.textContent = definition.title;
    const description = document.createElement("p");
    description.className = "fine";
    description.textContent = definition.description;
    copy.append(title, description);
    const count = document.createElement("span");
    count.className = "opportunity-count";
    count.textContent = String(items.length);
    header.append(copy, count);
    section.append(header);

    const feed = document.createElement("div");
    feed.className = "bounty-feed home-bounty-feed";
    if (!items.length) {
      const empty = document.createElement("p");
      empty.className = "fine opportunity-empty";
      empty.textContent = "No matching opportunity is currently visible.";
      feed.append(empty);
    } else {
      items.forEach((item) => appendOpportunity(feed, item));
    }
    section.append(feed);
    container.append(section);
  }

  function setMetric(name, value, decimals = 0) {
    const output = document.querySelector(`[data-adoption-${name}]`);
    if (!output) return;
    const target = Number(value);
    if (!Number.isFinite(target)) {
      output.textContent = "--";
      return;
    }

    const previous = Number(output.dataset.value);
    output.dataset.value = String(target);
    output.dataset.loaded = "true";
    const animationId = String(++metricAnimationId);
    output.dataset.animationId = animationId;
    if (reduceMotion || !Number.isFinite(previous) || previous === target) {
      output.textContent = formatMetric(target, decimals);
      return;
    }

    const startedAt = performance.now();
    const duration = 420;
    function frame(timestamp) {
      if (output.dataset.animationId !== animationId) return;
      const progress = Math.min(1, (timestamp - startedAt) / duration);
      const eased = 1 - ((1 - progress) ** 3);
      output.textContent = formatMetric(previous + ((target - previous) * eased), decimals);
      if (progress < 1) requestAnimationFrame(frame);
    }
    requestAnimationFrame(frame);
  }

  function sumUsdc(items, includeCompletionBonus = false) {
    return items.reduce((total, item) => {
      const reward = item.reward && item.reward.currency === "USDC"
        ? amountValue(item.reward)
        : 0;
      const bonus = includeCompletionBonus
        && item.completion_bonus
        && item.completion_bonus.currency === "USDC"
        ? amountValue(item.completion_bonus)
        : 0;
      return total + reward + bonus;
    }, 0);
  }

  function marketFingerprint(items) {
    return JSON.stringify(items.map((item) => [
      item.opportunity_id,
      item.work_state,
      item.payment_state,
      item.payment_committed,
      item.verification_ready,
      item.updated_at,
    ]));
  }

  function renderOpportunityBoard(container, items) {
    const fingerprint = marketFingerprint(items);
    if (fingerprint === marketState.fingerprint) return;
    marketState.fingerprint = fingerprint;
    container.textContent = "";
    opportunitySections.forEach((definition) => {
      appendSection(container, definition, items.filter(definition.matches));
    });
    container.classList.remove("market-update");
    requestAnimationFrame(() => container.classList.add("market-update"));
  }

  function formatElapsed(milliseconds) {
    const seconds = Math.max(0, Math.floor(milliseconds / 1_000));
    if (seconds < 5) return "just now";
    if (seconds < 60) return `${seconds}s ago`;
    return `${Math.floor(seconds / 60)}m ago`;
  }

  function updateMarketClock() {
    const updated = document.querySelector("[data-adoption-updated]");
    if (!updated) return;
    if (marketState.evidenceGeneratedAt) {
      updated.dateTime = marketState.evidenceGeneratedAt.toISOString();
    }
    if (!marketState.lastReceivedAt) {
      updated.textContent = marketState.status === "delayed"
        ? "Live feed unavailable · retrying automatically"
        : "Connecting to live evidence...";
      return;
    }

    const age = Date.now() - marketState.lastReceivedAt;
    if (marketState.status === "delayed") {
      updated.textContent = `Feed delayed · last sync ${formatElapsed(age)} · retrying automatically`;
      return;
    }
    if (marketState.refreshing) {
      updated.textContent = `Refreshing · last sync ${formatElapsed(age)}`;
      return;
    }
    const refreshIn = Math.max(0, Math.ceil((MARKET_REFRESH_MS - age) / 1_000));
    updated.textContent = `Synced ${formatElapsed(age)} · refresh in ${refreshIn}s`;
  }

  function setMarketStatus(status) {
    marketState.status = status;
    const strip = document.querySelector(".live-strip");
    const board = document.getElementById("home-live-inventory");
    if (strip) strip.dataset.marketHealth = status;
    if (board) board.dataset.marketHealth = status;
    updateMarketClock();
  }

  async function resolveProtocol() {
    if (!marketState.protocolPromise) {
      marketState.protocolPromise = fetch("protocol.json", { cache: "no-store" })
        .then((response) => {
          if (!response.ok) throw new Error("Protocol configuration is unavailable.");
          return response.json();
        })
        .catch((error) => {
          marketState.protocolPromise = null;
          throw error;
        });
    }
    return marketState.protocolPromise;
  }

  function newestPaidProof(items) {
    const paid = items
      .filter((item) => item.source_type === "canonical_base" && item.payment_state === "paid")
      .slice()
      .sort((left, right) => Date.parse(right.updated_at) - Date.parse(left.updated_at));
    const latest = paid[0];
    return latest && safePublicUrl((latest.proof_urls || [])[0] || latest.public_url);
  }

  function renderMarketSnapshot(protocol, projection, claim) {
    const container = document.getElementById("home-live-inventory");
    const heroSummary = document.querySelector("[data-home-inventory-summary]");
    const detail = document.querySelector("[data-home-inventory-detail]");
    const proof = document.querySelector("[data-market-proof]");
    const items = projection.items || [];
    const readyDefinition = opportunitySections.find((definition) => definition.key === "ready");
    const readyItems = items.filter(readyDefinition.matches);
    const referenceAt = new Date(claim.generated_at || projection.generated_at);
    const cutoff = referenceAt.getTime() - (MARKET_WINDOW_HOURS * 60 * 60 * 1_000);
    const paidItems = items.filter((item) => item.source_type === "canonical_base"
      && item.work_state === "completed"
      && item.payment_state === "paid"
      && Date.parse(item.updated_at) >= cutoff);
    const availableUsdc = sumUsdc(readyItems);
    const paidUsdc = sumUsdc(paidItems, true);
    const settlements = Number(claim.canonical_outcomes.settlements_confirmed);

    setMetric("ready", readyItems.length);
    setMetric("available", availableUsdc, 2);
    setMetric("settled", settlements);
    setMetric("paid", paidUsdc, 2);
    renderOpportunityBoard(container, items);

    heroSummary.textContent = `${readyItems.length} funded bounties ready · ${formatMetric(availableUsdc, 2)} USDC available · ${settlements} confirmed payouts in 30 days`;
    const sourceStatuses = projection.source_statuses || [];
    const availableSources = sourceStatuses.filter((source) => source.available).length;
    const unavailable = sourceStatuses
      .filter((source) => !source.available)
      .map((source) => source.source_type);
    const protocolStatus = protocol.status === "active" ? "Base mainnet active" : "Canonical protocol not active";
    detail.textContent = unavailable.length
      ? `${protocolStatus} · ${items.length} live opportunities · ${availableSources}/${sourceStatuses.length} sources online · delayed: ${unavailable.join(", ")}`
      : `${protocolStatus} · ${items.length} live opportunities · ${availableSources}/${sourceStatuses.length} sources online · auto-refreshes every 30 seconds`;

    const proofUrl = newestPaidProof(paidItems);
    if (proof && proofUrl) {
      proof.href = proofUrl;
      proof.hidden = false;
    } else if (proof) {
      proof.hidden = true;
    }
    marketState.evidenceGeneratedAt = referenceAt;
  }

  async function refreshMarket() {
    if (marketState.refreshing) return;
    marketState.refreshing = true;
    setMarketStatus(marketState.rendered ? "refreshing" : "connecting");
    const container = document.getElementById("home-live-inventory");
    const heroSummary = document.querySelector("[data-home-inventory-summary]");
    const detail = document.querySelector("[data-home-inventory-detail]");
    try {
      const protocol = await resolveProtocol();
      const api = protocol.api_base_url.replace(/\/$/, "");
      const [projectionResponse, claimResponse] = await Promise.all([
        fetch(`${api}/v1/opportunities?network=base-mainnet&limit=300`, { cache: "no-store" }),
        fetch(`${api}/v1/base/autonomous-bounties/claim-funnel?window_hours=${MARKET_WINDOW_HOURS}`, { cache: "no-store" }),
      ]);
      if (!projectionResponse.ok || !claimResponse.ok) {
        throw new Error("Live market evidence is unavailable.");
      }
      const [projection, claim] = await Promise.all([
        projectionResponse.json(),
        claimResponse.json(),
      ]);
      renderMarketSnapshot(protocol, projection, claim);
      marketState.lastReceivedAt = Date.now();
      marketState.rendered = true;
      setMarketStatus(projection.degraded ? "delayed" : "live");
    } catch (error) {
      setMarketStatus("delayed");
      if (!marketState.rendered) {
        container.textContent = "Opportunity discovery could not be loaded. Use the authoritative unfunded and canonical feeds directly; use the portable skill for a Base safe-block check.";
        heroSummary.textContent = "Live market feed unavailable · retrying automatically";
        detail.textContent = error.message || String(error);
      } else {
        detail.textContent = "Live feed delayed. Last confirmed market snapshot remains visible while the page retries automatically.";
      }
    } finally {
      marketState.refreshing = false;
      updateMarketClock();
    }
  }

  function loadInventory() {
    if (!document.getElementById("home-live-inventory")) return;
    refreshMarket();
    window.setInterval(() => {
      if (!document.hidden) refreshMarket();
    }, MARKET_REFRESH_MS);
    window.setInterval(updateMarketClock, 1_000);
    document.addEventListener("visibilitychange", () => {
      if (!document.hidden
        && (!marketState.lastReceivedAt || Date.now() - marketState.lastReceivedAt >= MARKET_REFRESH_MS)) {
        refreshMarket();
      }
    });
    window.addEventListener("online", refreshMarket);
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
      const protocol = await resolveProtocol();
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
      marketState.leaderboardRendered = true;
    } catch (error) {
      if (!marketState.leaderboardRendered) {
        daily.textContent = "Leaderboard unavailable.";
        weekly.textContent = "Leaderboard unavailable.";
        status.textContent = error.message || String(error);
      } else {
        status.textContent = "Leaderboard refresh delayed. Last verified standings remain visible.";
      }
    }
  }

  const canvas = document.getElementById("network-canvas");
  loadInventory();
  loadLeaderboard();
  window.setInterval(() => {
    if (!document.hidden) loadLeaderboard();
  }, LEADERBOARD_REFRESH_MS);
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
