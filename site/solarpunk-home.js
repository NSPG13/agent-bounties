(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.SolarpunkHome = api;
  if (root && root.document) api.start(root, root.document);
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";

  const PHASES = ["dawn", "day", "dusk", "night"];
  const LOCAL_HOSTS = new Set(["localhost", "127.0.0.1", "::1", "[::1]"]);
  const AUTH_PROVIDER_LABELS = {
    amazon: "Amazon",
    enterprise: "Company SSO",
    github: "GitHub",
    google: "Google",
    microsoft: "Microsoft",
  };
  const BOUNTY_POSTING_PROMPT = `You are helping me create and fund a bounty on Agent Bounties (https://agentbounties.app).

Initialize Agent Bounties posting mode:
1. Read https://agentbounties.app/.well-known/agent-bounties.json and https://agentbounties.app/llms.txt before choosing tools or endpoints.
2. Ask me for the desired outcome, deliverables, objective acceptance tests, deadline, and maximum USDC budget. Ask one concise question at a time.
3. Produce a clear draft and have me approve it before any public write, wallet signature, funding authorization, or transaction.
4. Use only official Agent Bounties MCP, API, and discovery routes you can verify. Do not invent platform state or endpoints.
5. Never ask for a seed phrase or private key. Show the exact bounded signature or payment request and explain its amount, network, recipient, expiry, and purpose before I approve.
6. Funding and payment are not confirmed by a plan, response, signature, authorization, broadcast, or transaction hash. Treat only confirmed canonical Base USDC evidence as proof.

Begin by asking: “What outcome do you want agents to deliver?”`;

  function clamp(value, minimum = 0, maximum = 1) {
    return Math.min(maximum, Math.max(minimum, value));
  }

  function smoothstep(value) {
    const x = clamp(value);
    return x * x * (3 - (2 * x));
  }

  function crossfade(from, to, progress) {
    const mix = smoothstep(progress);
    return { [from]: 1 - mix, [to]: mix };
  }

  function sceneBlend(minute) {
    const value = ((Number(minute) % 1440) + 1440) % 1440;
    let weights;
    if (value < 270) weights = { night: 1 };
    else if (value < 330) weights = crossfade("night", "dawn", (value - 270) / 60);
    else if (value < 420) weights = { dawn: 1 };
    else if (value < 480) weights = crossfade("dawn", "day", (value - 420) / 60);
    else if (value < 1020) weights = { day: 1 };
    else if (value < 1110) weights = crossfade("day", "dusk", (value - 1020) / 90);
    else if (value < 1200) weights = crossfade("dusk", "night", (value - 1110) / 90);
    else weights = { night: 1 };
    const complete = Object.fromEntries(PHASES.map((phase) => [phase, weights[phase] || 0]));
    const phase = PHASES.reduce((best, candidate) => complete[candidate] > complete[best] ? candidate : best, "night");
    return { minute: value, phase, weights: complete, nightStrength: complete.night };
  }

  function parseSceneTime(value) {
    const match = /^(\d{1,2}):(\d{2})$/.exec(String(value || "").trim());
    if (!match) return null;
    const hours = Number(match[1]);
    const minutes = Number(match[2]);
    return hours >= 0 && hours <= 23 && minutes >= 0 && minutes <= 59
      ? (hours * 60) + minutes
      : null;
  }

  function isLocalHost(hostname) {
    return LOCAL_HOSTS.has(String(hostname || "").toLowerCase());
  }

  function authApiPath(path, locationLike) {
    const suffix = String(path || "").startsWith("/") ? String(path || "") : `/${path || ""}`;
    const location = locationLike || (typeof window !== "undefined" ? window.location : null);
    return !location || isLocalHost(location.hostname)
      ? `/auth${suffix}`
      : `https://api.agentbounties.app/v1/site-auth${suffix}`;
  }

  function authProviderPath(provider, locationLike) {
    const key = String(provider || "").trim().toLowerCase();
    return Object.hasOwn(AUTH_PROVIDER_LABELS, key)
      ? authApiPath(`/login/${key}`, locationLike)
      : null;
  }

  function authResultMessage(result, provider, reason) {
    if (result === "success") {
      const label = AUTH_PROVIDER_LABELS[String(provider || "").toLowerCase()] || "your provider";
      return `Signed in with ${label}.`;
    }
    const messages = {
      access_denied: "Sign-in was cancelled before access was granted.",
      expired_state: "The sign-in request expired. Please try again.",
      invalid_state: "The sign-in response could not be verified. Please try again.",
      provider_exchange_failed: "The provider could not complete sign-in. Please try again.",
      provider_not_configured: "That sign-in provider is not configured on the authentication service.",
      account_service_unavailable: "The account service is temporarily unavailable. Please try again.",
    };
    return messages[reason] || "Sign-in could not be completed. Please try again.";
  }

  function bountyAssistantLinks(provider, prompt = BOUNTY_POSTING_PROMPT) {
    const key = String(provider || "").trim().toLowerCase();
    const encoded = encodeURIComponent(String(prompt || ""));
    const links = {
      gpt: {
        label: "GPT",
        desktopUrl: null,
        webUrl: `https://chatgpt.com/?prompt=${encoded}`,
        webPrefillsPrompt: true,
      },
      claude: {
        label: "Claude",
        desktopUrl: `claude://claude.ai/new?q=${encoded}`,
        webUrl: `https://claude.ai/new?q=${encoded}`,
        webPrefillsPrompt: true,
      },
      cursor: {
        label: "Cursor",
        desktopUrl: `cursor://anysphere.cursor-deeplink/prompt?text=${encoded}`,
        webUrl: `https://cursor.com/link/prompt?text=${encoded}`,
        webPrefillsPrompt: true,
      },
      custom: {
        label: "Custom",
        desktopUrl: null,
        webUrl: null,
        webPrefillsPrompt: false,
      },
    };
    return Object.hasOwn(links, key) ? links[key] : null;
  }

  function parseCompetitionPostingRequest(search) {
    const params = new URLSearchParams(String(search || ""));
    if (!params.has("parentCompetition")) return { requested: false, valid: false };
    const contract = String(params.get("parentCompetition") || "").trim().toLowerCase();
    const network = String(params.get("network") || "base-mainnet").trim().toLowerCase();
    return {
      requested: true,
      valid: /^0x[0-9a-f]{40}$/.test(contract) && network === "base-mainnet",
      contract,
      network,
    };
  }

  function competitionPostingItem(projection, request) {
    if (!request?.requested || !request.valid) throw new Error("invalid competition posting context");
    if (projection?.schema_version !== "agent-bounties/opportunity-projection-v1" || !Array.isArray(projection.items)) {
      throw new Error("invalid unified opportunity projection");
    }
    const item = projection.items.find((candidate) =>
      String(candidate?.source_id || "").toLowerCase() === request.contract
      && String(candidate?.network || "").toLowerCase() === request.network
      && String(candidate?.opportunity_id || "").startsWith("open-competition-v2:")
    );
    const window = item?.evidence_requirements?.scoring_window;
    const startsAt = Date.parse(String(window?.starts_at || ""));
    const endsAt = Date.parse(String(window?.ends_at || ""));
    if (!item
      || item.source_status !== "active"
      || item.work_state !== "claimable"
      || item.payment_state !== "escrowed"
      || item.payment_committed !== true
      || item.verification_ready !== true
      || item.competition_mode !== "best_score"
      || item.evidence_requirements?.program_profile !== "forward-canonical-gmv-attribution-metric-v2"
      || !Number.isFinite(startsAt)
      || !Number.isFinite(endsAt)
      || endsAt <= startsAt) {
      throw new Error("competition is not ready for a bound posting handoff");
    }
    return item;
  }

  function competitionChildBrief(item) {
    const window = item.evidence_requirements.scoring_window;
    const windowText = `${window.starts_at} through ${window.ends_at}`;
    return `Qualifying Agent Bounties demand brief

Parent competition: ${item.source_id}
Entering/funding wallet: 0xYOUR_BASE_WALLET
Scoring window: ${windowText}

Outcome requested:
[Describe one useful digital result that another agent can deliver.]

Objective acceptance tests:
1. [Binary or measurable test]
2. [Exact artifact/evidence schema]
3. [Deterministic verifier or precommitted quorum]

Funding:
- Network: Base mainnet
- Asset: native USDC
- Solver reward: [AMOUNT]
- Fully fund before another wallet claims or enters

Canonical completion requirement:
- A wallet different from the creator completes the child bounty.
- The child reaches confirmed canonical settlement inside ${windowText}.
- Funding attributable to 0xYOUR_BASE_WALLET is used in the parent score.

Safety:
- Do not use an operator/reserve wallet or excluded reward contract.
- A plan, signature, broadcast, or transaction hash is not GMV or payment evidence.
- Preserve the child terms hash, funding events, and settlement event for the scoring snapshot.`;
  }

  function competitionPostingPrompt(item) {
    const basePrompt = BOUNTY_POSTING_PROMPT.replace(/\n\nBegin by asking:[\s\S]*$/, "");
    return `${basePrompt}

This is a contract-bound Open Competition V2 child-bounty posting session.
- Parent competition: ${item.source_id}
- Network: ${item.network}
- Do not replace the parent contract, network, or UTC scoring window.
- Do not restart with a generic outcome question. Present the reviewed brief below, ask me to fill only its bracketed placeholders, and warn if the proposed child cannot settle inside the window.
- Before any wallet interaction, calculate the parent competition's complete win, loss, and expected economics using the child funding I select.

${competitionChildBrief(item)}`;
  }

  function shortWalletAddress(value) {
    const address = String(value || "").trim().toLowerCase();
    return /^0x[0-9a-f]{40}$/.test(address) ? `${address.slice(0, 8)}…${address.slice(-6)}` : "";
  }

  function utf8Hex(value) {
    return `0x${Array.from(new TextEncoder().encode(String(value))).map((byte) => byte.toString(16).padStart(2, "0")).join("")}`;
  }

  function walletLinkErrorMessage(error) {
    const reason = typeof error === "string" ? error : error?.reason || error?.code;
    const messages = {
      4001: "Wallet connection or signature was cancelled.",
      invalid_origin: "Wallet linking is available only from an approved Agent Bounties site.",
      invalid_wallet_address: "The wallet did not return a valid EVM address.",
      signature_verifier_unavailable: "The wallet signature verifier is unavailable.",
      wallet_challenge_invalid: "The ownership request expired. Please try linking again.",
      wallet_link_store_unavailable: "The verified wallet could not be saved right now.",
      wallet_linked_to_another_account: "That wallet is already linked to another account.",
      wallet_limit_reached: "This account has reached its linked-wallet limit.",
      wallet_signature_invalid: "The signature did not prove control of that wallet.",
    };
    return messages[reason] || "The wallet could not be linked. Please try again.";
  }

  function accountDashboardView(payload) {
    const normalizeWallets = (items) => {
      if (!Array.isArray(items)) return [];
      return items.slice(0, 8).map((item) => {
        const address = String(item?.address || "").trim().toLowerCase();
        if (!/^0x[0-9a-f]{40}$/.test(address)) throw new Error("invalid linked wallet");
        return {
          address,
          label: shortWalletAddress(address),
          linkedAt: String(item?.linked_at || "").slice(0, 32),
        };
      });
    };
    const unavailable = (reason) => ({
      available: false,
      participating: "—",
      completedPosts: "—",
      earned: "—",
      spent: "—",
      rank: "—",
      participatingCount: "— active",
      completedCount: "— settled",
      participatingItems: [],
      completedItems: [],
      wallets: normalizeWallets(payload?.wallets),
      message: reason === "marketplace_identity_unlinked"
        ? "Link and verify a wallet to load personal bounty, payment, and leaderboard statistics."
        : reason === "marketplace_evidence_unavailable"
          ? "Wallet ownership is verified, but canonical marketplace evidence is temporarily unavailable. No values have been estimated."
          : "Personal marketplace evidence is unavailable right now. No activity or money values have been estimated.",
    });
    if (!payload || payload.data_status !== "available") {
      return unavailable(payload?.reason);
    }

    try {
      const stats = payload.stats;
      if (!stats || typeof stats !== "object") throw new Error("missing stats");
      const participating = finiteNonNegative(stats.participating_bounties, "participating bounties");
      const completedPosts = finiteNonNegative(stats.completed_posted_bounties, "completed posts");
      const earned = finiteNonNegative(stats.earned_usdc, "earned USDC");
      const spent = finiteNonNegative(stats.spent_usdc, "spent USDC");
      if (![participating, completedPosts].every(Number.isInteger)) throw new Error("invalid bounty count");
      const rankValue = stats.leaderboard_rank;
      if (rankValue !== null && (!Number.isInteger(Number(rankValue)) || Number(rankValue) < 1)) {
        throw new Error("invalid leaderboard rank");
      }
      const activities = payload.activities;
      if (!activities || !Array.isArray(activities.participating) || !Array.isArray(activities.completed_posts)) {
        throw new Error("missing activity lists");
      }
      const normalizeItems = (items) => items.slice(0, 6).map((item) => {
        const title = String(item?.title || "").trim().slice(0, 160);
        const status = String(item?.status || "").trim().slice(0, 80);
        if (!title || !status) throw new Error("invalid activity item");
        return { title, status };
      });
      const integer = new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 });
      const money = new Intl.NumberFormat("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
      const wallets = normalizeWallets(payload.wallets);
      if (!wallets.length) throw new Error("linked wallet missing");
      return {
        available: true,
        participating: integer.format(participating),
        completedPosts: integer.format(completedPosts),
        earned: `${money.format(earned)} USDC`,
        spent: `${money.format(spent)} USDC`,
        rank: rankValue === null ? "Unranked" : `#${integer.format(Number(rankValue))}`,
        participatingCount: `${integer.format(participating)} active`,
        completedCount: `${integer.format(completedPosts)} settled`,
        participatingItems: normalizeItems(activities.participating),
        completedItems: normalizeItems(activities.completed_posts),
        wallets,
        message: "Linked-wallet values use confirmed canonical evidence. Earnings include solver rewards and timeout bonuses; spending is gross confirmed funding.",
      };
    } catch (error) {
      return unavailable("malformed_account_evidence");
    }
  }

  function sceneTimeOverride(search, hostname) {
    if (!isLocalHost(hostname)) return null;
    return parseSceneTime(new URLSearchParams(String(search || "")).get("sceneTime"));
  }

  function sceneInsectKind(minute) {
    return sceneBlend(minute).phase === "day" ? "dragonfly" : "firefly";
  }

  function hashSeed(value) {
    let hash = 2166136261;
    for (const character of String(value)) {
      hash ^= character.charCodeAt(0);
      hash = Math.imul(hash, 16777619);
    }
    return hash >>> 0;
  }

  function seededRandom(seed) {
    let state = typeof seed === "number" ? seed >>> 0 : hashSeed(seed);
    return function random() {
      state += 0x6D2B79F5;
      let output = state;
      output = Math.imul(output ^ (output >>> 15), output | 1);
      output ^= output + Math.imul(output ^ (output >>> 7), output | 61);
      return ((output ^ (output >>> 14)) >>> 0) / 4294967296;
    };
  }

  function flameMotion(seconds, phase, speed = 1) {
    const slow = Math.sin((seconds * speed) + phase);
    const curl = Math.sin((seconds * speed * 1.91) + (phase * 1.37));
    const lift = Math.sin((seconds * speed * .63) + (phase * .71));
    return {
      sway: clamp((slow * .72) + (curl * .28), -1, 1),
      lift: .89 + (lift * .11),
    };
  }

  function isReadyToEarn(item) {
    return Boolean(item)
      && item.source_type === "canonical_base"
      && item.work_state === "claimable"
      && item.payment_state === "escrowed"
      && item.payment_committed === true
      && item.verification_ready === true;
  }

  function finiteNonNegative(value, label) {
    const number = Number(value);
    if (!Number.isFinite(number) || number < 0) throw new Error(`${label} is unavailable`);
    return number;
  }

  function marketSnapshot(platform, projection, now = Date.now()) {
    if (!platform || !projection) throw new Error("Marketplace evidence is unavailable");
    if (projection.applied_view !== "ready_to_earn" || projection.degraded !== false || !Array.isArray(projection.items)) {
      throw new Error("Ready-to-earn evidence is incomplete");
    }
    const canonicalSource = Array.isArray(projection.source_statuses)
      ? projection.source_statuses.find((source) => source?.source_type === "canonical_base")
      : null;
    if (!canonicalSource || canonicalSource.available !== true || projection.items.some((item) => !isReadyToEarn(item))) {
      throw new Error("Canonical inventory evidence is incomplete");
    }

    const payout = finiteNonNegative(platform?.marketplace_payout_volume?.lifetime?.usdc, "Lifetime payout");
    const completed = finiteNonNegative(platform?.marketplace_payout_volume?.lifetime_settled_rounds, "Settled rounds");
    const weekStart = now - (7 * 24 * 60 * 60 * 1000);
    const addedThisWeek = projection.items.reduce((total, item) => {
      const created = Date.parse(item.created_at);
      return total + (Number.isFinite(created) && created >= weekStart && created <= now ? 1 : 0);
    }, 0);
    const completedThisWeek = (Array.isArray(platform.daily) ? platform.daily : []).reduce((total, item) => {
      const day = Date.parse(`${item?.day}T00:00:00Z`);
      if (!Number.isFinite(day) || day < weekStart || day > now) return total;
      return total + finiteNonNegative(item.settled_rounds, "Daily settled rounds");
    }, 0);
    return { payout, live: projection.items.length, completed, addedThisWeek, completedThisWeek };
  }

  function start(win, doc) {
    const root = doc.querySelector("[data-scene-root]");
    if (!root) return;
    const reducedMotion = win.matchMedia("(prefers-reduced-motion: reduce)");
    const query = new URLSearchParams(win.location.search);
    const forcedMinute = sceneTimeOverride(win.location.search, win.location.hostname);
    const today = new Date().toLocaleDateString("en-CA");
    let sessionSeed = win.sessionStorage.getItem("solarpunk-scene-seed");
    if (!sessionSeed) {
      sessionSeed = `${Date.now()}-${Math.random()}`;
      win.sessionStorage.setItem("solarpunk-scene-seed", sessionSeed);
    }
    const debugSeed = isLocalHost(win.location.hostname) ? query.get("sceneSeed") : null;
    const random = seededRandom(`${today}:${debugSeed || sessionSeed}`);
    const canvas = doc.querySelector("[data-scene-canvas]");
    const context = canvas?.getContext("2d", { alpha: true });
    let blend = sceneBlend(0);
    let fireflies = [];
    let dragonflies = [];
    let flameTongues = [];
    let frame = 0;
    let visible = !doc.hidden;
    let metricsTimer = 0;

    function currentMinute() {
      if (forcedMinute !== null) return forcedMinute;
      const now = new Date();
      return (now.getHours() * 60) + now.getMinutes() + (now.getSeconds() / 60);
    }

    function loadPlate(plate) {
      if (plate.dataset.loaded === "true") return;
      plate.querySelectorAll("source[data-srcset]").forEach((source) => {
        source.srcset = source.dataset.srcset;
      });
      const image = plate.querySelector("img[data-src]");
      if (image) image.src = image.dataset.src;
      plate.dataset.loaded = "true";
    }

    function updateSceneTime() {
      const firstPaint = doc.documentElement.dataset.sceneReady !== "true";
      blend = sceneBlend(currentMinute());
      const active = PHASES.filter((phase) => blend.weights[phase] > 0.001);
      doc.querySelectorAll("[data-scene-plate]").forEach((plate) => {
        const phase = plate.dataset.scenePlate;
        if (active.includes(phase)) loadPlate(plate);
        plate.style.opacity = String(blend.weights[phase]);
      });
      root.dataset.scenePhase = blend.phase;
      root.dataset.sceneInsects = sceneInsectKind(blend.minute);
      if (firstPaint) {
        win.requestAnimationFrame(() => {
          doc.documentElement.dataset.sceneReady = "true";
        });
      }
      root.style.setProperty("--night-strength", blend.nightStrength.toFixed(3));
    }

    function resizeCanvas() {
      if (!canvas || !context) return;
      const bounds = canvas.getBoundingClientRect();
      const density = Math.min(1.5, win.devicePixelRatio || 1);
      canvas.width = Math.max(1, Math.round(bounds.width * density));
      canvas.height = Math.max(1, Math.round(bounds.height * density));
      context.setTransform(density, 0, 0, density, 0, 0);
      const fireflyCount = win.innerWidth <= 720 ? 20 : 44;
      fireflies = Array.from({ length: fireflyCount }, () => ({
        x: random() * bounds.width,
        y: (.07 + (random() * .82)) * bounds.height,
        radius: .55 + (random() * 1.55),
        phase: random() * Math.PI * 2,
        speed: .18 + (random() * .5),
        drift: random() > .5 ? 1 : -1,
      }));
      const dragonflyCount = win.innerWidth <= 720 ? 5 : 10;
      dragonflies = Array.from({ length: dragonflyCount }, () => ({
        originX: random() * bounds.width,
        baseY: (.12 + (random() * .66)) * bounds.height,
        direction: random() > .5 ? 1 : -1,
        speed: 17 + (random() * 25),
        size: 1.55 + (random() * .85),
        phase: random() * Math.PI * 2,
        bob: 2 + (random() * 7),
        flutter: random() * Math.PI * 2,
      }));
      const tongueCount = win.innerWidth <= 720 ? 8 : 13;
      flameTongues = Array.from({ length: tongueCount }, (_, index) => {
        const layer = index % 3;
        return {
          offset: (random() - .5) * (win.innerWidth <= 720 ? 76 : 132),
          width: (12 + (random() * 19)) * (layer === 2 ? .62 : layer === 1 ? .82 : 1),
          height: (36 + (random() * 67)) * (layer === 2 ? .72 : layer === 1 ? .9 : 1),
          lean: (random() - .5) * .34,
          phase: random() * Math.PI * 2,
          speed: 1.35 + (random() * 1.3),
          layer,
        };
      }).sort((first, second) => first.layer - second.layer || second.height - first.height);
    }

    function drawProceduralFire(width, height, seconds) {
      const fireX = width * .5;
      const fireY = height * (win.innerWidth <= 720 ? .765 : .778);
      const scale = clamp(width / 1180, .72, 1.12);
      context.save();
      context.globalCompositeOperation = "lighter";

      const bloom = context.createRadialGradient(fireX, fireY - (13 * scale), 0, fireX, fireY, 118 * scale);
      bloom.addColorStop(0, "rgba(255, 190, 75, .24)");
      bloom.addColorStop(.32, "rgba(255, 100, 31, .11)");
      bloom.addColorStop(1, "rgba(255, 82, 20, 0)");
      context.fillStyle = bloom;
      context.beginPath();
      context.ellipse(fireX, fireY - (7 * scale), 128 * scale, 71 * scale, 0, 0, Math.PI * 2);
      context.fill();

      flameTongues.forEach((tongue) => {
        const motion = flameMotion(seconds, tongue.phase, tongue.speed);
        const baseX = fireX + (tongue.offset * scale);
        const flameHeight = tongue.height * motion.lift * scale;
        const flameWidth = tongue.width * scale;
        const tipX = baseX + (motion.sway * flameWidth * .58) + (tongue.lean * flameHeight);
        const topY = fireY - flameHeight;
        const gradient = context.createLinearGradient(baseX, fireY, tipX, topY);
        if (tongue.layer === 0) {
          gradient.addColorStop(0, "rgba(255, 189, 63, .36)");
          gradient.addColorStop(.48, "rgba(255, 92, 24, .30)");
          gradient.addColorStop(1, "rgba(255, 68, 18, .04)");
        } else if (tongue.layer === 1) {
          gradient.addColorStop(0, "rgba(255, 235, 146, .58)");
          gradient.addColorStop(.55, "rgba(255, 155, 42, .46)");
          gradient.addColorStop(1, "rgba(255, 101, 22, .06)");
        } else {
          gradient.addColorStop(0, "rgba(255, 250, 218, .76)");
          gradient.addColorStop(.62, "rgba(255, 208, 91, .48)");
          gradient.addColorStop(1, "rgba(255, 155, 43, .05)");
        }
        context.fillStyle = gradient;
        context.beginPath();
        context.moveTo(baseX - (flameWidth * .58), fireY);
        context.bezierCurveTo(
          baseX - (flameWidth * .68),
          fireY - (flameHeight * .32),
          tipX - (flameWidth * .34),
          topY + (flameHeight * .2),
          tipX,
          topY,
        );
        context.bezierCurveTo(
          tipX + (flameWidth * .42),
          topY + (flameHeight * .25),
          baseX + (flameWidth * .73),
          fireY - (flameHeight * .28),
          baseX + (flameWidth * .58),
          fireY,
        );
        context.closePath();
        context.fill();
      });

      const core = context.createRadialGradient(fireX, fireY - (8 * scale), 0, fireX, fireY, 62 * scale);
      core.addColorStop(0, "rgba(255, 255, 225, .58)");
      core.addColorStop(.34, "rgba(255, 208, 96, .34)");
      core.addColorStop(1, "rgba(255, 115, 27, 0)");
      context.fillStyle = core;
      context.beginPath();
      context.ellipse(fireX, fireY - (5 * scale), 68 * scale, 24 * scale, 0, 0, Math.PI * 2);
      context.fill();
      context.restore();
    }

    function drawFireflies(width, height, seconds) {
      context.save();
      context.globalCompositeOperation = "lighter";
      fireflies.forEach((firefly) => {
        firefly.phase += .008 * firefly.speed;
        firefly.x += Math.sin(firefly.phase) * .13 * firefly.drift;
        firefly.y += Math.cos(firefly.phase * .7) * .055;
        if (firefly.x < -5) firefly.x = width + 5;
        if (firefly.x > width + 5) firefly.x = -5;
        if (firefly.y < height * .06) firefly.y = height * .89;
        if (firefly.y > height * .9) firefly.y = height * .07;
        const pulse = .35 + (.65 * ((Math.sin(seconds * firefly.speed * 2 + firefly.phase) + 1) / 2));
        const alpha = (.22 + (.64 * blend.nightStrength)) * pulse;
        context.beginPath();
        context.fillStyle = `rgba(105, 255, 112, ${alpha})`;
        context.shadowBlur = 8;
        context.shadowColor = "#63ff74";
        context.arc(firefly.x, firefly.y, firefly.radius, 0, Math.PI * 2);
        context.fill();
      });
      context.restore();
    }

    function drawDragonfly(dragonfly, width, height, seconds) {
      const travel = width + 120;
      const travelled = dragonfly.originX + (seconds * dragonfly.speed * dragonfly.direction) + 60;
      const x = ((travelled % travel) + travel) % travel - 60;
      const wave = (seconds * .72) + dragonfly.phase;
      const y = clamp(
        dragonfly.baseY + (Math.sin(wave) * dragonfly.bob) + (Math.sin((wave * 1.83) + 1.4) * 1.8),
        height * .08,
        height * .84,
      );
      const incline = Math.cos(wave) * .06 * dragonfly.direction;
      const heading = dragonfly.direction > 0 ? incline : Math.PI - incline;
      const wingBeat = .62 + (.38 * Math.abs(Math.sin((seconds * 25) + dragonfly.flutter)));

      context.save();
      context.translate(x, y);
      context.rotate(heading);
      context.scale(dragonfly.size, dragonfly.size);
      context.lineCap = "round";
      context.shadowBlur = 3.4;
      context.shadowColor = "rgba(1, 20, 16, .92)";

      const wings = [
        { x: -.2, y: -4.1 * wingBeat, rx: 8.2, ry: 1.42, rotation: -.92 },
        { x: -.2, y: 4.1 * wingBeat, rx: 8.2, ry: 1.42, rotation: .92 },
        { x: -3.7, y: -3.45 * wingBeat, rx: 6.8, ry: 1.28, rotation: -1.15 },
        { x: -3.7, y: 3.45 * wingBeat, rx: 6.8, ry: 1.28, rotation: 1.15 },
      ];
      wings.forEach((wing) => {
        context.beginPath();
        context.fillStyle = "rgba(224, 255, 249, .62)";
        context.strokeStyle = "rgba(101, 244, 205, .98)";
        context.lineWidth = .82;
        context.ellipse(wing.x, wing.y, wing.rx, wing.ry, wing.rotation, 0, Math.PI * 2);
        context.fill();
        context.stroke();
      });

      context.shadowBlur = 2.2;
      const body = context.createLinearGradient(-10, 0, 5, 0);
      body.addColorStop(0, "#1c746c");
      body.addColorStop(.58, "#5cdda1");
      body.addColorStop(1, "#d7f56c");
      context.strokeStyle = "rgba(2, 42, 36, .95)";
      context.lineWidth = 3.1;
      context.beginPath();
      context.moveTo(-11.5, 0);
      context.quadraticCurveTo(-3, .3, 2.8, 0);
      context.stroke();
      context.strokeStyle = body;
      context.lineWidth = 2.05;
      context.beginPath();
      context.moveTo(-11.5, 0);
      context.quadraticCurveTo(-3, .3, 2.8, 0);
      context.stroke();
      context.strokeStyle = "rgba(10, 72, 66, .72)";
      context.lineWidth = .45;
      [-7.5, -4.8, -2.2].forEach((segment) => {
        context.beginPath();
        context.moveTo(segment, -1);
        context.lineTo(segment, 1);
        context.stroke();
      });
      context.fillStyle = "#55d59a";
      context.strokeStyle = "rgba(2, 42, 36, .95)";
      context.lineWidth = .8;
      context.beginPath();
      context.ellipse(1, 0, 2.45, 1.55, 0, 0, Math.PI * 2);
      context.fill();
      context.stroke();
      context.fillStyle = "#d5f56b";
      context.beginPath();
      context.arc(4.15, 0, 1.45, 0, Math.PI * 2);
      context.fill();
      context.stroke();
      context.fillStyle = "rgba(9, 56, 51, .88)";
      context.beginPath();
      context.arc(4.55, -.52, .31, 0, Math.PI * 2);
      context.arc(4.55, .52, .31, 0, Math.PI * 2);
      context.fill();
      context.restore();
    }

    function drawDragonflies(width, height, seconds) {
      context.save();
      context.globalCompositeOperation = "source-over";
      dragonflies.forEach((dragonfly) => drawDragonfly(dragonfly, width, height, seconds));
      context.restore();
    }

    function drawScene(time) {
      if (!visible || !context || !canvas) return;
      const width = canvas.clientWidth;
      const height = canvas.clientHeight;
      context.clearRect(0, 0, width, height);
      const seconds = time / 1000;

      if (blend.nightStrength > .08) {
        context.fillStyle = `rgba(220, 246, 235, ${.15 * blend.nightStrength})`;
        for (let index = 0; index < 18; index += 1) {
          const x = ((index * 173.7) % width);
          const y = 100 + ((index * 79.1) % Math.max(160, height * .42));
          context.fillRect(x, y, 1, 1);
        }
      }

      drawProceduralFire(width, height, seconds);

      if (sceneInsectKind(blend.minute) === "dragonfly") drawDragonflies(width, height, seconds);
      else drawFireflies(width, height, seconds);
      frame = win.requestAnimationFrame(drawScene);
    }

    function beginAnimation() {
      if (reducedMotion.matches || frame || !visible) return;
      frame = win.requestAnimationFrame(drawScene);
    }

    function stopAnimation() {
      if (frame) win.cancelAnimationFrame(frame);
      frame = 0;
    }

    function scheduleCharacters() {
      if (reducedMotion.matches || win.innerWidth <= 820) return;
      const characters = Array.from(doc.querySelectorAll("[data-character]"));
      if (!characters.length) return;
      const show = () => {
        if (!visible) return;
        const character = characters[Math.floor(random() * characters.length)];
        character.classList.add("is-active");
        win.setTimeout(() => character.classList.remove("is-active"), 9000 + Math.round(random() * 5500));
        win.setTimeout(show, 20000 + Math.round(random() * 25000));
      };
      win.setTimeout(show, 3500 + Math.round(random() * 5000));
    }

    function applyMetricState(snapshot) {
      const formatter = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 });
      doc.querySelector("[data-market-volume]").textContent = formatter.format(snapshot.payout);
      doc.querySelector("[data-live-bounties]").textContent = formatter.format(snapshot.live);
      doc.querySelector("[data-completed-bounties]").textContent = formatter.format(snapshot.completed);
      doc.querySelector("[data-live-weekly]").textContent = `+${formatter.format(snapshot.addedThisWeek)} created in the last 7 days`;
      doc.querySelector("[data-completed-weekly]").textContent = `+${formatter.format(snapshot.completedThisWeek)} settled in the last 7 days`;
      const status = doc.querySelector("[data-market-status]");
      status.textContent = "Canonical marketplace evidence is current.";
      status.dataset.state = "ready";
    }

    function applyUnavailableState(message = "Marketplace evidence is temporarily unavailable.") {
      ["[data-market-volume]", "[data-live-bounties]", "[data-completed-bounties]"].forEach((selector) => {
        doc.querySelector(selector).textContent = "—";
      });
      doc.querySelector("[data-live-weekly]").textContent = "Ready-to-earn inventory unavailable";
      doc.querySelector("[data-completed-weekly]").textContent = "Canonical settlement history unavailable";
      const status = doc.querySelector("[data-market-status]");
      status.textContent = message;
      status.dataset.state = "unavailable";
    }

    async function requestJson(url) {
      const response = await win.fetch(url, { cache: "no-store", headers: { accept: "application/json", "cache-control": "no-cache" } });
      if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
      return response.json();
    }

    async function refreshMetrics() {
      try {
        const protocol = await requestJson("protocol.json");
        const apiBase = String(protocol?.api_base_url || "").replace(/\/$/, "");
        if (!/^https:\/\//.test(apiBase)) throw new Error("API discovery is unavailable");
        const [platform, projection] = await Promise.all([
          requestJson(`${apiBase}/v1/metrics/platform?period=lifetime`),
          requestJson(`${apiBase}/v1/opportunities?network=base-mainnet&view=ready_to_earn&source_type=canonical_base&work_state=claimable&payment_state=escrowed&limit=300&live=${Date.now()}`),
        ]);
        applyMetricState(marketSnapshot(platform, projection));
      } catch (error) {
        applyUnavailableState();
      }
    }

    function restartMetricsTimer() {
      win.clearInterval(metricsTimer);
      metricsTimer = win.setInterval(refreshMetrics, 60000);
    }

    function setupAuthDialog() {
      const dialog = doc.querySelector("[data-auth-dialog]");
      const openButton = doc.querySelector("[data-auth-open]");
      if (!dialog || !openButton || typeof dialog.showModal !== "function") return;
      const closeButton = dialog.querySelector("[data-auth-close]");
      const form = dialog.querySelector("[data-auth-form]");
      const accountDashboard = dialog.querySelector("[data-account-dashboard]");
      const sessionAvatar = dialog.querySelector("[data-auth-avatar]");
      const sessionAvatarFallback = dialog.querySelector("[data-auth-avatar-fallback]");
      const sessionName = dialog.querySelector("[data-auth-name]");
      const sessionEmail = dialog.querySelector("[data-auth-email]");
      const sessionProvider = dialog.querySelector("[data-auth-provider-name]");
      const logoutButton = dialog.querySelector("[data-auth-logout]");
      const accountStats = dialog.querySelector("[data-account-stats]");
      const accountParticipating = dialog.querySelector("[data-account-participating]");
      const accountCompletedPosts = dialog.querySelector("[data-account-completed-posts]");
      const accountEarned = dialog.querySelector("[data-account-earned]");
      const accountSpent = dialog.querySelector("[data-account-spent]");
      const accountRank = dialog.querySelector("[data-account-rank]");
      const accountParticipatingCount = dialog.querySelector("[data-account-participating-count]");
      const accountCompletedCount = dialog.querySelector("[data-account-completed-count]");
      const accountParticipatingList = dialog.querySelector("[data-account-participating-list]");
      const accountCompletedList = dialog.querySelector("[data-account-completed-list]");
      const accountEvidence = dialog.querySelector("[data-account-evidence]");
      const walletLinkButton = dialog.querySelector("[data-wallet-link]");
      const walletList = dialog.querySelector("[data-wallet-list]");
      const walletStatus = dialog.querySelector("[data-wallet-status]");
      const heading = dialog.querySelector("#auth-title");
      const description = dialog.querySelector("#auth-description");
      const email = form?.elements.email;
      const name = form?.elements.name;
      const password = form?.elements.password;
      const passwordToggle = dialog.querySelector("[data-password-toggle]");
      const emailField = dialog.querySelector('[data-auth-field="email"]');
      const nameField = dialog.querySelector('[data-auth-field="name"]');
      const passwordField = dialog.querySelector('[data-auth-field="password"]');
      const recoveryButton = dialog.querySelector("[data-auth-recovery]");
      const registerButton = dialog.querySelector("[data-auth-register]");
      const backButton = dialog.querySelector("[data-auth-back]");
      const inbox = dialog.querySelector("[data-auth-inbox]");
      const submitButton = dialog.querySelector("[data-auth-submit]");
      const taskNote = dialog.querySelector("[data-auth-task-note]");
      const socialSections = Array.from(dialog.querySelectorAll("[data-auth-social]"));
      const switchLine = dialog.querySelector("[data-auth-switch]");
      const status = dialog.querySelector("[data-auth-status]");
      const providerButtons = Array.from(dialog.querySelectorAll("[data-auth-provider]"));
      let authServerReady = false;
      let passwordAuthReady = false;
      let providerAvailability = {};
      let currentUser = null;
      let accountLoadId = 0;
      let authView = "login";
      let pendingEmail = "";

      const setStatus = (message) => {
        if (status) status.textContent = message;
      };
      const setWalletStatus = (message) => {
        if (walletStatus) walletStatus.textContent = message;
      };
      const showDialog = () => {
        if (!dialog.open) dialog.showModal();
        openButton.setAttribute("aria-expanded", "true");
      };
      const closeDialog = () => {
        if (dialog.open) dialog.close();
      };

      const setBusy = (busy) => {
        if (submitButton) submitButton.disabled = busy;
        if (form) form.setAttribute("aria-busy", String(busy));
      };

      const setAuthView = (view, message = "") => {
        authView = view;
        if (form) form.dataset.authState = view;
        const login = view === "login";
        const registration = view === "registration";
        const reset = view === "reset";
        const registrationPassword = view === "registration-password";
        const resetPassword = view === "reset-password";
        const inboxView = view === "inbox";
        if (heading) {
          heading.textContent = login ? "Sign in"
            : registration ? "Create your account"
              : reset ? "Reset your password"
                : inboxView ? "Check your inbox"
                  : registrationPassword ? "Choose your password" : "Set a new password";
        }
        if (description) {
          description.textContent = login
            ? "Sign in to manage bounties, evidence, and collaboration."
            : registration
              ? "First, verify the mailbox you want connected to your account."
              : reset
                ? "We’ll send one private recovery link if this account can continue."
                : inboxView
                  ? "A private, single-use link is on its way."
                  : registrationPassword
                    ? "Your email is verified. Choose the name and passphrase for this account."
                    : "Your email is verified. This change signs out every other session.";
        }
        if (emailField) emailField.hidden = !(login || registration || reset);
        if (nameField) nameField.hidden = !registrationPassword;
        if (passwordField) passwordField.hidden = !(login || registrationPassword || resetPassword);
        if (recoveryButton) recoveryButton.hidden = !login;
        if (submitButton) {
          submitButton.hidden = inboxView;
          submitButton.textContent = login ? "Sign in"
            : registration ? "Send verification email"
              : reset ? "Send recovery email"
                : registrationPassword ? "Create account" : "Save new password";
        }
        if (inbox) inbox.hidden = !inboxView;
        socialSections.forEach((section) => { section.hidden = !login; });
        if (switchLine) switchLine.hidden = !login;
        if (taskNote) {
          taskNote.hidden = !(registrationPassword || resetPassword);
          taskNote.textContent = registrationPassword || resetPassword
            ? "Use 15–128 characters. Spaces and Unicode are welcome; common passphrases are refused."
            : "";
        }
        if (email) email.required = login || registration || reset;
        if (name) name.required = registrationPassword;
        if (password) {
          password.required = login || registrationPassword || resetPassword;
          password.autocomplete = login ? "current-password" : "new-password";
          if (!login) password.value = "";
        }
        setBusy(false);
        setStatus(message);
        win.requestAnimationFrame(() => {
          if (inboxView) backButton?.focus();
          else if (registrationPassword) name?.focus();
          else if (resetPassword) password?.focus();
          else email?.focus();
        });
      };

      const replaceActivityList = (list, items, emptyMessage) => {
        if (!list) return;
        list.replaceChildren();
        if (!items.length) {
          const empty = doc.createElement("li");
          empty.className = "account-empty";
          empty.textContent = emptyMessage;
          list.append(empty);
          return;
        }
        items.forEach((item) => {
          const row = doc.createElement("li");
          const title = doc.createElement("strong");
          const itemStatus = doc.createElement("span");
          title.textContent = item.title;
          itemStatus.textContent = item.status;
          row.append(title, itemStatus);
          list.append(row);
        });
      };

      const renderWallets = (wallets, loading = false) => {
        if (!walletList) return;
        walletList.replaceChildren();
        if (loading) {
          const item = doc.createElement("li");
          item.className = "wallet-empty";
          item.textContent = "Checking verified wallets…";
          walletList.append(item);
        } else if (!wallets.length) {
          const item = doc.createElement("li");
          item.className = "wallet-empty";
          item.textContent = "No verified wallet linked.";
          walletList.append(item);
        } else {
          wallets.forEach((wallet) => {
            const item = doc.createElement("li");
            const address = doc.createElement("code");
            const verified = doc.createElement("span");
            const remove = doc.createElement("button");
            address.textContent = wallet.label;
            address.title = wallet.address;
            verified.className = "wallet-verified";
            verified.textContent = "Verified";
            remove.className = "wallet-remove";
            remove.type = "button";
            remove.dataset.walletUnlink = wallet.address;
            remove.textContent = "Remove";
            remove.setAttribute("aria-label", `Remove linked wallet ${wallet.label}`);
            item.append(address, verified, remove);
            walletList.append(item);
          });
        }
        if (walletLinkButton) walletLinkButton.textContent = wallets.length ? "Link another" : "Link wallet";
      };

      const renderAccountDashboard = (payload) => {
        const view = accountDashboardView(payload);
        renderWallets(view.wallets);
        if (accountStats) accountStats.setAttribute("aria-busy", "false");
        if (accountParticipating) accountParticipating.textContent = view.participating;
        if (accountCompletedPosts) accountCompletedPosts.textContent = view.completedPosts;
        if (accountEarned) accountEarned.textContent = view.earned;
        if (accountSpent) accountSpent.textContent = view.spent;
        if (accountRank) accountRank.textContent = view.rank;
        if (accountParticipatingCount) accountParticipatingCount.textContent = view.participatingCount;
        if (accountCompletedCount) accountCompletedCount.textContent = view.completedCount;
        replaceActivityList(
          accountParticipatingList,
          view.participatingItems,
          view.available
            ? "No active bounty participation."
            : view.wallets.length ? "Canonical activity is temporarily unavailable." : "Link a verified wallet to load activity.",
        );
        replaceActivityList(
          accountCompletedList,
          view.completedItems,
          view.available
            ? "No completed posted bounties."
            : view.wallets.length ? "Settlement history is temporarily unavailable." : "Link a verified wallet to load settlements.",
        );
        if (accountEvidence) accountEvidence.textContent = view.message;
      };

      const renderAccountLoading = () => {
        if (accountStats) accountStats.setAttribute("aria-busy", "true");
        [accountParticipating, accountCompletedPosts, accountEarned, accountSpent, accountRank]
          .forEach((node) => { if (node) node.textContent = "—"; });
        if (accountParticipatingCount) accountParticipatingCount.textContent = "— active";
        if (accountCompletedCount) accountCompletedCount.textContent = "— settled";
        replaceActivityList(accountParticipatingList, [], "Checking marketplace activity…");
        replaceActivityList(accountCompletedList, [], "Checking settlement evidence…");
        renderWallets([], true);
        if (accountEvidence) accountEvidence.textContent = "Verifying marketplace activity…";
      };

      const renderProviderAvailability = () => {
        providerButtons.forEach((button) => {
          const key = String(button.dataset.authProvider || "").toLowerCase();
          const label = AUTH_PROVIDER_LABELS[key] || "Provider";
          const configured = Boolean(authServerReady && providerAvailability[key]);
          button.setAttribute("aria-disabled", String(!configured));
          button.title = configured
            ? `Continue with ${label}`
            : `${label} sign-in is not available right now`;
        });
      };

      const renderSession = (payload) => {
        providerAvailability = payload?.providers || providerAvailability;
        passwordAuthReady = Boolean(payload?.password);
        const user = payload?.authenticated
          ? { ...payload.user, provider: payload.sign_in_method, linkedMethods: payload.linked_methods || [] }
          : null;
        currentUser = user;
        dialog.dataset.view = user ? "account" : "login";
        if (form) form.hidden = Boolean(user);
        if (accountDashboard) accountDashboard.hidden = !user;
        if (heading) heading.textContent = user ? "Your activity" : "Sign in";
        if (description) {
          description.textContent = user
            ? "Your bounties, confirmed money flow, and platform standing in one private view."
            : "Sign in to manage bounties, evidence, and collaboration.";
        }
        openButton.textContent = user ? "Account" : "Login";
        openButton.title = user?.name ? `Signed in as ${user.name}` : "Sign in";
        if (!user) {
          accountLoadId += 1;
          renderWallets([]);
          setWalletStatus("");
          renderProviderAvailability();
          setAuthView(authView === "account" ? "login" : authView);
          return;
        }
        renderAccountLoading();
        if (sessionName) sessionName.textContent = user.name || "Signed in";
        if (sessionEmail) {
          sessionEmail.textContent = user.email || "Email not shared";
          sessionEmail.hidden = false;
        }
        if (sessionProvider) {
          const label = AUTH_PROVIDER_LABELS[user.provider] || user.provider || "OAuth";
          const linked = (user.linkedMethods || [])
            .map((method) => AUTH_PROVIDER_LABELS[method] || (method === "password" ? "Email" : method))
            .join(", ");
          sessionProvider.textContent = linked ? `Connected methods: ${linked}` : `Signed in with ${label}`;
        }
        if (sessionAvatar) {
          const avatar = String(user.avatar || "");
          const safeAvatar = /^https:\/\//i.test(avatar);
          sessionAvatar.hidden = !safeAvatar;
          if (safeAvatar) sessionAvatar.src = avatar;
          else sessionAvatar.removeAttribute("src");
        }
        if (sessionAvatarFallback) {
          const initials = String(user.name || user.email || "AB")
            .split(/\s+/)
            .filter(Boolean)
            .slice(0, 2)
            .map((part) => part[0])
            .join("")
            .toUpperCase();
          sessionAvatarFallback.textContent = initials || "AB";
        }
        renderProviderAvailability();
      };

      const postAccountJson = async (path, body) => {
        const response = await win.fetch(authApiPath(path, win.location), {
          method: "POST",
          credentials: "include",
          headers: { Accept: "application/json", "Content-Type": "application/json" },
          body: JSON.stringify(body),
        });
        const payload = await response.json().catch(() => ({}));
        if (!response.ok) throw { reason: payload.error || `http_${response.status}` };
        return payload;
      };

      const loadAccount = async () => {
        if (!currentUser) return null;
        const requestId = ++accountLoadId;
        renderAccountLoading();
        try {
          const response = await win.fetch(authApiPath("/account", win.location), {
            credentials: "include",
            headers: { Accept: "application/json" },
          });
          if (!response.ok) throw new Error(`account ${response.status}`);
          const payload = await response.json();
          if (requestId !== accountLoadId || !currentUser) return null;
          renderAccountDashboard(payload);
          return payload;
        } catch (error) {
          if (requestId === accountLoadId && currentUser) renderAccountDashboard(null);
          return null;
        }
      };

      const loadSession = async () => {
        try {
          const response = await win.fetch(authApiPath("/session", win.location), {
            credentials: "include",
            headers: { Accept: "application/json" },
          });
          if (!response.ok) throw new Error(`session ${response.status}`);
          const payload = await response.json();
          authServerReady = true;
          renderSession(payload);
          if (payload.authenticated) await loadAccount();
          return payload;
        } catch (error) {
          authServerReady = false;
          providerAvailability = {};
          renderProviderAvailability();
          return null;
        }
      };

      openButton.addEventListener("click", () => {
        setStatus("");
        setWalletStatus("");
        showDialog();
        if (currentUser) loadAccount();
        win.requestAnimationFrame(() => {
          if (currentUser) closeButton?.focus();
          else if (authView === "registration-password") name?.focus();
          else if (authView === "reset-password") password?.focus();
          else email?.focus();
        });
      });
      closeButton?.addEventListener("click", closeDialog);
      dialog.addEventListener("close", () => {
        openButton.setAttribute("aria-expanded", "false");
        openButton.focus();
      });
      dialog.addEventListener("click", (event) => {
        if (event.target !== dialog) return;
        const bounds = dialog.getBoundingClientRect();
        const inside = event.clientX >= bounds.left && event.clientX <= bounds.right
          && event.clientY >= bounds.top && event.clientY <= bounds.bottom;
        if (!inside) closeDialog();
      });
      passwordToggle?.addEventListener("click", () => {
        if (!password) return;
        const visiblePassword = password.type === "text";
        password.type = visiblePassword ? "password" : "text";
        passwordToggle.setAttribute("aria-pressed", String(!visiblePassword));
        passwordToggle.setAttribute("aria-label", visiblePassword ? "Show password" : "Hide password");
        password.focus();
      });
      form?.addEventListener("submit", async (event) => {
        event.preventDefault();
        if (!form.reportValidity()) return;
        if (!passwordAuthReady) {
          setStatus("Email and password sign-in is temporarily unavailable. You can still use a connected provider.");
          return;
        }
        setBusy(true);
        setStatus(authView === "login" ? "Checking your credentials…" : "Securing the next step…");
        try {
          if (authView === "login") {
            await postAccountJson("/password/login", { email: email.value, password: password.value });
            const payload = await loadSession();
            if (!payload?.authenticated) throw { reason: "session_unavailable" };
            setStatus("Signed in securely.");
            closeButton?.focus();
          } else if (authView === "registration" || authView === "reset") {
            pendingEmail = email.value.trim();
            const endpoint = authView === "registration" ? "/password/registration" : "/password/reset";
            const payload = await postAccountJson(endpoint, { email: pendingEmail });
            setAuthView("inbox", payload.message || "If this address can continue, an email is on its way.");
          } else if (authView === "registration-password" || authView === "reset-password") {
            const endpoint = authView === "registration-password"
              ? "/password/complete"
              : "/password/reset-complete";
            await postAccountJson(endpoint, { name: name?.value || "", password: password.value });
            const payload = await loadSession();
            if (!payload?.authenticated) throw { reason: "session_unavailable" };
            setStatus(authView === "reset-password" ? "Password replaced and other sessions revoked." : "Account created and signed in.");
            closeButton?.focus();
          }
        } catch (error) {
          const messages = {
            invalid_credentials: "Email or password is incorrect.",
            email_invalid: "Enter a valid email address.",
            name_invalid: "Enter the name you want shown on your account.",
            password_length_invalid: "Use a passphrase between 15 and 128 characters.",
            password_common: "That passphrase is too common. Choose a more distinctive one.",
            email_action_invalid: "This private link is invalid, expired, or has already been used.",
            password_auth_unavailable: "Email and password sign-in is temporarily unavailable.",
          };
          setStatus(messages[error?.reason] || "That step could not be completed. Please try again.");
        } finally {
          setBusy(false);
        }
      });
      registerButton?.addEventListener("click", () => setAuthView("registration"));
      recoveryButton?.addEventListener("click", () => setAuthView("reset"));
      backButton?.addEventListener("click", () => setAuthView("login"));
      walletLinkButton?.addEventListener("click", async () => {
        if (!currentUser) return;
        if (!win.ethereum || typeof win.ethereum.request !== "function") {
          setWalletStatus("No browser wallet was detected. Open this page in a browser with an EVM wallet extension.");
          return;
        }
        walletLinkButton.disabled = true;
        setWalletStatus("Choose the wallet address you want to link…");
        try {
          const accounts = await win.ethereum.request({ method: "eth_requestAccounts" });
          const address = String(Array.isArray(accounts) ? accounts[0] : "").trim();
          if (!/^0x[0-9a-fA-F]{40}$/.test(address)) throw { reason: "invalid_wallet_address" };
          const challenge = await postAccountJson("/wallet/challenge", { address });
          setWalletStatus("Review the ownership-only message in your wallet. It cannot move funds or approve tokens.");
          const signature = await win.ethereum.request({
            method: "personal_sign",
            params: [utf8Hex(challenge.message), address],
          });
          await postAccountJson("/wallet/verify", {
            challenge_id: challenge.challenge_id,
            address,
            signature,
          });
          await loadAccount();
          setWalletStatus(`${shortWalletAddress(address)} is verified and linked.`);
        } catch (error) {
          setWalletStatus(walletLinkErrorMessage(error));
        } finally {
          walletLinkButton.disabled = false;
        }
      });
      walletList?.addEventListener("click", async (event) => {
        const button = event.target.closest?.("[data-wallet-unlink]");
        if (!button || button.disabled || !currentUser) return;
        if (button.dataset.confirming !== "true") {
          button.dataset.confirming = "true";
          button.textContent = "Remove?";
          setWalletStatus("Select Remove? again to unlink this address. No onchain action will occur.");
          win.setTimeout(() => {
            if (!button.isConnected) return;
            button.dataset.confirming = "false";
            button.textContent = "Remove";
          }, 5000);
          return;
        }
        button.disabled = true;
        try {
          await postAccountJson("/wallet/unlink", { address: button.dataset.walletUnlink });
          await loadAccount();
          setWalletStatus("Wallet unlinked from this account. No onchain state changed.");
        } catch (error) {
          setWalletStatus(walletLinkErrorMessage(error));
          button.disabled = false;
        }
      });
      providerButtons.forEach((button) => {
        button.addEventListener("click", () => {
          const key = String(button.dataset.authProvider || "").toLowerCase();
          const label = AUTH_PROVIDER_LABELS[key] || "That provider";
          const path = authProviderPath(key, win.location);
          if (!authServerReady) {
            setStatus("The authentication service is temporarily unavailable.");
            return;
          }
          if (!path || !providerAvailability[key]) {
            setStatus(`${label} sign-in is awaiting its OAuth credentials.`);
            return;
          }
          button.setAttribute("aria-busy", "true");
          setStatus(`Opening ${label} sign-in…`);
          win.location.assign(path);
        });
      });
      logoutButton?.addEventListener("click", async () => {
        logoutButton.disabled = true;
        setStatus("Signing out…");
        try {
          const response = await win.fetch(authApiPath("/logout", win.location), {
            method: "POST",
            credentials: "include",
          });
          if (!response.ok) throw new Error(`logout ${response.status}`);
          renderSession({ authenticated: false, user: null, providers: providerAvailability });
          setStatus("Signed out from this session.");
        } catch (error) {
          setStatus("The session could not be signed out. Please try again.");
        } finally {
          logoutButton.disabled = false;
        }
      });
      renderProviderAvailability();
      const authParams = new URLSearchParams(win.location.search);
      const authResult = authParams.get("auth");
      const emailActionParams = new URLSearchParams(String(win.location.hash || "").replace(/^#/, ""));
      const emailAction = emailActionParams.get("auth");
      const emailToken = emailActionParams.get("token");
      if ((emailAction === "register" || emailAction === "reset") && emailToken) {
        win.history?.replaceState?.(null, "", `${win.location.pathname}${win.location.search}`);
      }
      loadSession().then(async () => {
        if ((emailAction === "register" || emailAction === "reset") && emailToken) {
          showDialog();
          setStatus("Verifying your private link…");
          try {
            const endpoint = emailAction === "register"
              ? "/password/verification"
              : "/password/reset-verification";
            const payload = await postAccountJson(endpoint, { token: emailToken });
            pendingEmail = payload.email || "";
            setAuthView(emailAction === "register" ? "registration-password" : "reset-password");
          } catch (error) {
            setAuthView("login", "This private link is invalid, expired, or has already been used.");
          }
          return;
        }
        if (authResult !== "success" && authResult !== "error") return;
        setStatus(authResultMessage(authResult, authParams.get("provider"), authParams.get("reason")));
        showDialog();
        authParams.delete("auth");
        authParams.delete("provider");
        authParams.delete("reason");
        const cleanSearch = authParams.toString();
        win.history?.replaceState?.(null, "", `${win.location.pathname}${cleanSearch ? `?${cleanSearch}` : ""}${win.location.hash}`);
      });
    }

    function setupBountyLauncher() {
      const dialog = doc.querySelector("[data-bounty-launcher]");
      const openButton = doc.querySelector("[data-bounty-open]");
      if (!dialog || !openButton || typeof dialog.showModal !== "function") return;
      const closeButton = dialog.querySelector("[data-bounty-close]");
      const assistantButtons = Array.from(dialog.querySelectorAll("[data-bounty-assistant]"));
      const promptPreview = dialog.querySelector("[data-bounty-prompt]");
      const status = dialog.querySelector("[data-bounty-launch-status]");
      const customActions = dialog.querySelector("[data-bounty-custom-actions]");
      const copyButton = dialog.querySelector("[data-bounty-copy]");
      const webFallback = dialog.querySelector("[data-bounty-web-fallback]");
      const promptDetails = dialog.querySelector(".bounty-prompt-preview");
      const launcherTitle = dialog.querySelector("#bounty-launcher-title");
      const launcherDescription = dialog.querySelector("#bounty-launcher-description");
      const postingRequest = parseCompetitionPostingRequest(win.location.search);
      let launcherPrompt = BOUNTY_POSTING_PROMPT;
      let contextReady = !postingRequest.requested;
      let contextStatus = postingRequest.requested ? "Verifying the parent competition and reviewed child-bounty brief…" : "";
      let launchTimer = 0;
      let appHandoffObserved = false;

      if (promptPreview) promptPreview.textContent = launcherPrompt;

      const setStatus = (message) => {
        if (status) status.textContent = message;
      };
      const clearLaunchProbe = () => {
        win.clearTimeout(launchTimer);
        launchTimer = 0;
        doc.removeEventListener("visibilitychange", observeAppHandoff);
        win.removeEventListener("blur", observeAppHandoff);
      };
      const observeAppHandoff = () => {
        if (doc.hidden || !doc.hasFocus?.()) appHandoffObserved = true;
      };
      const resetLauncher = () => {
        clearLaunchProbe();
        assistantButtons.forEach((button) => button.removeAttribute("aria-current"));
        if (customActions) customActions.hidden = true;
        if (webFallback) {
          webFallback.hidden = true;
          webFallback.removeAttribute("href");
        }
        promptDetails?.removeAttribute("open");
        assistantButtons.forEach((button) => { button.disabled = !contextReady; });
        if (copyButton) copyButton.disabled = !contextReady;
        setStatus(contextStatus);
      };
      const showDialog = () => {
        resetLauncher();
        dialog.showModal();
        openButton.setAttribute("aria-expanded", "true");
        win.requestAnimationFrame?.(() => assistantButtons[0]?.focus());
      };
      const closeDialog = () => {
        clearLaunchProbe();
        if (dialog.open) dialog.close();
      };
      const copyPrompt = async () => {
        try {
          await win.navigator.clipboard.writeText(launcherPrompt);
          return true;
        } catch (error) {
          const textarea = doc.createElement("textarea");
          textarea.value = launcherPrompt;
          textarea.setAttribute("readonly", "");
          textarea.style.position = "fixed";
          textarea.style.opacity = "0";
          doc.body.append(textarea);
          textarea.select();
          const copied = Boolean(doc.execCommand?.("copy"));
          textarea.remove();
          return copied;
        }
      };
      const attemptDesktop = (links) => {
        clearLaunchProbe();
        appHandoffObserved = false;
        doc.addEventListener("visibilitychange", observeAppHandoff);
        win.addEventListener("blur", observeAppHandoff);
        const anchor = doc.createElement("a");
        anchor.href = links.desktopUrl;
        anchor.hidden = true;
        doc.body.append(anchor);
        anchor.click();
        anchor.remove();
        launchTimer = win.setTimeout(() => {
          clearLaunchProbe();
          if (appHandoffObserved) return;
          setStatus(`${links.label} desktop did not take focus. Opening its web app in this browser…`);
          win.location.assign(links.webUrl);
        }, 2400);
      };

      openButton.addEventListener("click", showDialog);
      closeButton?.addEventListener("click", closeDialog);
      dialog.addEventListener("close", () => {
        clearLaunchProbe();
        openButton.setAttribute("aria-expanded", "false");
      });
      dialog.addEventListener("click", (event) => {
        const rect = dialog.getBoundingClientRect();
        const outside = event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom;
        if (outside) closeDialog();
      });
      assistantButtons.forEach((button) => {
        button.addEventListener("click", async () => {
          clearLaunchProbe();
          const key = String(button.dataset.bountyAssistant || "").toLowerCase();
          const links = bountyAssistantLinks(key, launcherPrompt);
          if (!links) return;
          assistantButtons.forEach((item) => item.removeAttribute("aria-current"));
          button.setAttribute("aria-current", "true");
          if (customActions) customActions.hidden = key !== "custom";
          if (webFallback) webFallback.hidden = true;

          if (key === "custom") {
            const copied = await copyPrompt();
            promptDetails?.setAttribute("open", "");
            setStatus(copied
              ? "Instructions copied. Open your preferred signed-in assistant and paste them into a new chat."
              : "Copy the initialization message above into a new chat with your preferred assistant.");
            return;
          }

          const promptCopy = copyPrompt();
          if (webFallback) {
            webFallback.href = links.webUrl;
            webFallback.textContent = links.webPrefillsPrompt
              ? `Use ${links.label} web instead`
              : `Use ${links.label} web instead — prompt copied`;
            webFallback.hidden = !links.desktopUrl;
          }
          if (!links.desktopUrl) {
            if (!links.webPrefillsPrompt) {
              const copied = await promptCopy;
              setStatus(copied
                ? `Opening your signed-in ${links.label} session. The posting instructions are copied and ready to paste.`
                : `Opening ${links.label}. Copy the initialization message above before continuing.`);
            } else {
              setStatus(`Opening ${links.label} in this browser with the posting instructions prefilled…`);
            }
            win.location.assign(links.webUrl);
            return;
          }
          setStatus(`Opening ${links.label} desktop with the posting instructions prefilled…`);
          attemptDesktop(links);
          void promptCopy;
        });
      });
      copyButton?.addEventListener("click", async () => {
        const copied = await copyPrompt();
        setStatus(copied ? "Initialization message copied." : "Select the message above and copy it manually.");
      });
      if (win.location.hash === "#post-a-bounty") {
        win.requestAnimationFrame?.(showDialog);
      }
      if (postingRequest.requested) {
        void (async () => {
          if (!postingRequest.valid) throw new Error("invalid competition posting context");
          const protocol = await requestJson("protocol.json");
          const apiBase = String(protocol?.api_base_url || "").replace(/\/$/, "");
          if (!/^https:\/\//.test(apiBase)) throw new Error("API discovery is unavailable");
          const projection = await requestJson(`${apiBase}/v1/opportunities?network=${encodeURIComponent(postingRequest.network)}&view=ready_to_earn&source_type=canonical_base&limit=300&live=${Date.now()}`);
          const item = competitionPostingItem(projection, postingRequest);
          launcherPrompt = competitionPostingPrompt(item);
          contextReady = true;
          contextStatus = `Verified ${shortWalletAddress(item.source_id)}. The selected assistant will receive the exact parent contract, UTC window, and reviewed child brief.`;
          if (launcherTitle) launcherTitle.textContent = "Post a qualifying bounty";
          if (launcherDescription) launcherDescription.textContent = `Bound to ${shortWalletAddress(item.source_id)} · ${item.evidence_requirements.scoring_window.starts_at} → ${item.evidence_requirements.scoring_window.ends_at}`;
          if (promptPreview) promptPreview.textContent = launcherPrompt;
          resetLauncher();
        })().catch(() => {
          contextReady = false;
          contextStatus = "This competition could not be verified in the live unified projection. Return to its participation page; no generic posting prompt was substituted.";
          resetLauncher();
        });
      }
    }

    updateSceneTime();
    resizeCanvas();
    if (reducedMotion.matches) root.dataset.motion = "reduced";
    else beginAnimation();
    scheduleCharacters();
    setupAuthDialog();
    setupBountyLauncher();
    refreshMetrics();
    restartMetricsTimer();
    win.setInterval(updateSceneTime, 60000);

    win.addEventListener("resize", resizeCanvas, { passive: true });
    win.addEventListener("online", refreshMetrics);
    root.addEventListener("pointermove", (event) => {
      if (reducedMotion.matches) return;
      const x = clamp(event.clientX / Math.max(1, win.innerWidth), 0, 1) - .5;
      const y = clamp(event.clientY / Math.max(1, win.innerHeight), 0, 1) - .5;
      root.style.setProperty("--scene-shift-x", `${(x * 12).toFixed(2)}px`);
      root.style.setProperty("--scene-shift-y", `${(y * 8).toFixed(2)}px`);
    }, { passive: true });
    doc.addEventListener("visibilitychange", () => {
      visible = !doc.hidden;
      if (visible) {
        updateSceneTime();
        refreshMetrics();
        beginAnimation();
      } else {
        stopAnimation();
      }
    });
    reducedMotion.addEventListener?.("change", () => {
      if (reducedMotion.matches) {
        root.dataset.motion = "reduced";
        stopAnimation();
      } else {
        delete root.dataset.motion;
        beginAnimation();
      }
    });
  }

  return {
    BOUNTY_POSTING_PROMPT,
    accountDashboardView,
    authApiPath,
    authProviderPath,
    authResultMessage,
    bountyAssistantLinks,
    competitionChildBrief,
    competitionPostingItem,
    competitionPostingPrompt,
    clamp,
    flameMotion,
    hashSeed,
    isLocalHost,
    isReadyToEarn,
    marketSnapshot,
    parseSceneTime,
    parseCompetitionPostingRequest,
    sceneBlend,
    sceneInsectKind,
    sceneTimeOverride,
    seededRandom,
    shortWalletAddress,
    smoothstep,
    start,
    utf8Hex,
    walletLinkErrorMessage,
  };
});
