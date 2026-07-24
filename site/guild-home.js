(function () {
  const root = document.documentElement;
  const body = document.body;
  if (!body.classList.contains("guild-home")) return;

  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const nav = document.querySelector("[data-guild-nav]");
  const navToggle = document.querySelector("[data-nav-toggle]");
  const navMenu = document.querySelector("[data-nav-menu]");
  const hero = document.querySelector(".guild-hero");
  const heroDepth = document.querySelector("[data-hero-depth]");
  const search = document.querySelector("[data-bounty-search]");
  const searchInput = search && search.querySelector("input[type='search']");

  function setMenu(open) {
    if (!navToggle || !navMenu) return;
    navToggle.setAttribute("aria-expanded", String(open));
    navMenu.classList.toggle("is-open", open);
  }

  if (navToggle && navMenu) {
    navToggle.addEventListener("click", () => {
      setMenu(navToggle.getAttribute("aria-expanded") !== "true");
    });

    navMenu.addEventListener("click", (event) => {
      if (event.target.closest("a")) setMenu(false);
    });

    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape") setMenu(false);
    });

    document.addEventListener("click", (event) => {
      if (!navMenu.classList.contains("is-open")) return;
      if (!event.target.closest("[data-nav-menu]") && !event.target.closest("[data-nav-toggle]")) {
        setMenu(false);
      }
    });
  }

  function updateNav() {
    if (nav) nav.classList.toggle("is-scrolled", window.scrollY > 32);
  }

  updateNav();
  window.addEventListener("scroll", updateNav, { passive: true });

  document.querySelectorAll("[data-search-term]").forEach((button) => {
    button.addEventListener("click", () => {
      if (!searchInput) return;
      searchInput.value = button.dataset.searchTerm || "";
      searchInput.focus();
    });
  });

  if (search) {
    search.addEventListener("submit", (event) => {
      const value = searchInput ? searchInput.value.trim() : "";
      if (!value) {
        event.preventDefault();
        searchInput?.focus();
        searchInput?.setAttribute("aria-invalid", "true");
        window.setTimeout(() => searchInput?.removeAttribute("aria-invalid"), 900);
        return;
      }

      if (window.AgentBountyEntry?.start) {
        event.preventDefault();
        window.AgentBountyEntry.start(value);
      }
    });

    searchInput?.addEventListener("keydown", (event) => {
      if (event.key !== "Enter" || event.isComposing) return;
      event.preventDefault();
      search.requestSubmit();
    });
  }

  const revealNodes = Array.from(document.querySelectorAll("[data-reveal]"));
  if (reducedMotion || !("IntersectionObserver" in window)) {
    revealNodes.forEach((node) => node.classList.add("is-visible"));
  } else {
    const revealObserver = new IntersectionObserver((entries, observer) => {
      entries.forEach((entry) => {
        if (!entry.isIntersecting) return;
        entry.target.classList.add("is-visible");
        observer.unobserve(entry.target);
      });
    }, { rootMargin: "0px 0px -8%", threshold: 0.08 });
    revealNodes.forEach((node) => revealObserver.observe(node));
  }

  if (!reducedMotion && hero && heroDepth) {
    let frame = 0;
    let pointerX = 0;
    let pointerY = 0;

    function paintDepth() {
      frame = 0;
      const scrollDepth = Math.min(18, window.scrollY * 0.035);
      heroDepth.style.transform = `translate3d(${pointerX}px, ${pointerY + scrollDepth}px, 0) scale(1.035)`;
    }

    function scheduleDepth() {
      if (!frame) frame = window.requestAnimationFrame(paintDepth);
    }

    hero.addEventListener("pointermove", (event) => {
      const bounds = hero.getBoundingClientRect();
      pointerX = ((event.clientX - bounds.left) / bounds.width - 0.5) * -8;
      pointerY = ((event.clientY - bounds.top) / bounds.height - 0.5) * -6;
      scheduleDepth();
    }, { passive: true });

    hero.addEventListener("pointerleave", () => {
      pointerX = 0;
      pointerY = 0;
      scheduleDepth();
    }, { passive: true });

    window.addEventListener("scroll", scheduleDepth, { passive: true });
  }

  /*
   * Historical community metrics replace the rolling 30-day paid-solver number.
   * The sources are independently deduplicated and are never added together,
   * because doing so could count the same person or agent twice.
   */
  const COMMUNITY_CACHE_KEY = "agent-bounties-community-metrics-v3";
  const COMMUNITY_CACHE_MS = 60 * 60 * 1_000;
  const COMMUNITY_REFRESH_MS = 30 * 60 * 1_000;
  const GITHUB_PAGE_SIZE = 100;
  const GITHUB_MAX_PAGES = 10;
  const GITHUB_API = "https://api.github.com/repos/NSPG13/agent-bounties";

  function prepareCommunityPresentation() {
    const participantOutput = document.querySelector(".participant-count");
    const oldPaidOutputs = Array.from(document.querySelectorAll("[data-adoption-paid]"));
    oldPaidOutputs.forEach((output) => output.removeAttribute("data-adoption-paid"));

    if (participantOutput) {
      participantOutput.dataset.communityParticipants = "";
      participantOutput.textContent = "--";
      participantOutput.title = "Loading the historical public participant count.";
      const crests = participantOutput.closest(".adventurer-crests");
      if (crests) {
        crests.setAttribute("aria-label", "Historical public participants recorded by Agent Bounties");
      }
    }

    oldPaidOutputs
      .filter((output) => output !== participantOutput)
      .forEach((output) => {
        output.dataset.communityContributors = "";
        output.textContent = "--";
        output.title = "Loading the historical contributor count.";
      });

    const worldContributorRow = document.querySelector(".world-ledger dl > div:last-child");
    if (worldContributorRow) {
      const label = worldContributorRow.querySelector("dt");
      const detail = worldContributorRow.querySelector("dd span");
      if (label) label.textContent = "Contributors";
      if (detail) detail.textContent = "historical public project contributors";
    }

    const impactContributorLabel = document.querySelector(".impact-ledger li:last-child small");
    if (impactContributorLabel) impactContributorLabel.textContent = "Contributors";

    const communityStylesheet = document.querySelector('link[href^="home-community.css"]');
    if (communityStylesheet) communityStylesheet.href = "home-community.css?v=489";

    const missionSprig = document.querySelector(".mission-sprig");
    const charter = missionSprig && missionSprig.closest(".charter-copy");
    if (missionSprig) {
      missionSprig.title = "Quercus alba botanical plate, 1819, public domain";
    }
    if (charter && !charter.querySelector(".botanical-credit")) {
      const credit = document.createElement("a");
      credit.className = "botanical-credit";
      credit.href = "https://commons.wikimedia.org/wiki/File:NAS-001_Quercus_alba.png";
      credit.target = "_blank";
      credit.rel = "noopener noreferrer";
      credit.textContent = "Quercus alba, 1819 · public domain";
      credit.setAttribute("aria-label", "Botanical artwork source: Quercus alba, 1819, public domain on Wikimedia Commons");
      charter.append(credit);
    }
  }

  function publicActorKey(actor) {
    if (!actor || typeof actor !== "object") return null;
    if (actor.id !== undefined && actor.id !== null) return `id:${actor.id}`;
    if (actor.login) return `login:${String(actor.login).toLowerCase()}`;
    if (actor.name || actor.email) {
      return `anonymous:${String(actor.name || "").toLowerCase()}:${String(actor.email || "").toLowerCase()}`;
    }
    return null;
  }

  async function collectGithubPages(path, onItem) {
    let completed = false;
    for (let page = 1; page <= GITHUB_MAX_PAGES; page += 1) {
      const joiner = path.includes("?") ? "&" : "?";
      const response = await fetch(
        `${GITHUB_API}${path}${joiner}per_page=${GITHUB_PAGE_SIZE}&page=${page}`,
        {
          cache: "no-store",
          headers: { Accept: "application/vnd.github+json" },
        },
      );
      if (!response.ok) throw new Error(`GitHub community request failed (${response.status}).`);
      const items = await response.json();
      if (!Array.isArray(items)) throw new Error("GitHub community response is malformed.");
      items.forEach(onItem);
      if (items.length < GITHUB_PAGE_SIZE) {
        completed = true;
        break;
      }
    }
    return completed;
  }

  async function loadGithubCommunityMetrics() {
    const contributors = new Set();
    const participants = new Set();
    const addContributor = (actor) => {
      const key = publicActorKey(actor);
      if (!key) return;
      contributors.add(key);
      participants.add(key);
    };
    const addParticipant = (actor) => {
      const key = publicActorKey(actor);
      if (key) participants.add(key);
    };

    const requests = await Promise.allSettled([
      collectGithubPages("/contributors?anon=true", addContributor),
      collectGithubPages("/issues?state=all", (item) => addContributor(item.user)),
      collectGithubPages("/issues/comments", (item) => addContributor(item.user)),
      collectGithubPages("/pulls/comments", (item) => addContributor(item.user)),
      collectGithubPages("/stargazers", addParticipant),
    ]);
    const successfulSources = requests.filter((request) => request.status === "fulfilled").length;
    if (!successfulSources || !participants.size) {
      throw new Error("No public GitHub community source was available.");
    }
    return {
      contributors: contributors.size || null,
      participants: participants.size,
      complete: requests.every(
        (request) => request.status === "fulfilled" && request.value === true,
      ),
    };
  }

  async function loadPlatformParticipantCount() {
    const protocolResponse = await fetch("protocol.json", { cache: "no-store" });
    if (!protocolResponse.ok) throw new Error("Protocol configuration is unavailable.");
    const protocol = await protocolResponse.json();
    const api = String(protocol.api_base_url || "").replace(/\/$/, "");
    if (!api) throw new Error("Hosted API URL is unavailable.");
    const response = await fetch(`${api}/v1/audience/report`, { cache: "no-store" });
    if (!response.ok) throw new Error(`Historical participant registry is unavailable (${response.status}).`);
    const report = await response.json();
    const total = Number(report.total_members);
    if (!Number.isSafeInteger(total) || total < 0) {
      throw new Error("Historical participant registry returned an invalid count.");
    }
    return total;
  }

  function displayHistoricalCount(selector, count, title, source) {
    document.querySelectorAll(selector).forEach((output) => {
      if (!Number.isSafeInteger(count) || count < 0) {
        output.textContent = "--";
        output.removeAttribute("data-loaded");
        output.title = "Historical count is temporarily unavailable; no estimate is shown.";
        return;
      }
      output.textContent = count.toLocaleString();
      output.dataset.loaded = "true";
      output.dataset.metricSource = source;
      output.title = title;
    });
  }

  function renderCommunityMetrics(metrics) {
    const contributorTitle = metrics.contributorSource === "github-public-history"
      ? metrics.githubComplete
        ? "Distinct historical public contributors found across repository commits, issues, pull requests, and comments."
        : "Distinct historical public contributors found in the available repository history; unavailable sources were not estimated."
      : "Historical public participants recorded by the platform audience registry; repository contributor history was temporarily unavailable.";
    const participantTitle = metrics.participantSource === "platform-audience-registry"
      ? "Historical public participants recorded by the Agent Bounties audience registry."
      : metrics.githubComplete
        ? "Historical public repository participants, including contributors and stargazers."
        : "Historical public participants found in the available repository records; unavailable sources were not estimated.";

    displayHistoricalCount(
      "[data-community-contributors]",
      metrics.contributors,
      contributorTitle,
      metrics.contributorSource,
    );
    displayHistoricalCount(
      "[data-community-participants]",
      metrics.participants,
      participantTitle,
      metrics.participantSource,
    );

    const crests = document.querySelector(".adventurer-crests");
    if (crests && Number.isSafeInteger(metrics.participants)) {
      crests.setAttribute(
        "aria-label",
        `${metrics.participants.toLocaleString()} historical public participants`,
      );
    }
  }

  function readCachedCommunityMetrics() {
    try {
      const cached = JSON.parse(window.localStorage.getItem(COMMUNITY_CACHE_KEY) || "null");
      if (!cached || Date.now() - Number(cached.cachedAt) > COMMUNITY_CACHE_MS) return null;
      if (!Number.isSafeInteger(cached.contributors) || !Number.isSafeInteger(cached.participants)) {
        return null;
      }
      return cached;
    } catch (_error) {
      return null;
    }
  }

  function cacheCommunityMetrics(metrics) {
    try {
      window.localStorage.setItem(
        COMMUNITY_CACHE_KEY,
        JSON.stringify({ ...metrics, cachedAt: Date.now() }),
      );
    } catch (_error) {
      // Metrics remain functional when storage is disabled.
    }
  }

  async function refreshCommunityMetrics() {
    const [platformResult, githubResult] = await Promise.allSettled([
      loadPlatformParticipantCount(),
      loadGithubCommunityMetrics(),
    ]);
    const platformParticipants = platformResult.status === "fulfilled"
      ? platformResult.value
      : null;
    const github = githubResult.status === "fulfilled"
      ? githubResult.value
      : { contributors: null, participants: null, complete: false };
    const githubContributors = Number.isSafeInteger(github.contributors)
      ? github.contributors
      : null;
    const githubParticipants = Number.isSafeInteger(github.participants)
      ? github.participants
      : null;

    const contributors = githubContributors !== null
      ? githubContributors
      : platformParticipants;
    const contributorSource = githubContributors !== null
      ? "github-public-history"
      : platformParticipants !== null
        ? "platform-audience-registry"
        : "unavailable";

    let participants = null;
    let participantSource = "unavailable";
    if (platformParticipants !== null && githubParticipants !== null) {
      if (platformParticipants >= githubParticipants) {
        participants = platformParticipants;
        participantSource = "platform-audience-registry";
      } else {
        participants = githubParticipants;
        participantSource = "github-public-history";
      }
    } else if (platformParticipants !== null) {
      participants = platformParticipants;
      participantSource = "platform-audience-registry";
    } else if (githubParticipants !== null) {
      participants = githubParticipants;
      participantSource = "github-public-history";
    }

    const metrics = {
      contributors,
      participants,
      contributorSource,
      participantSource,
      githubComplete: github.complete,
    };
    if (Number.isSafeInteger(contributors) && Number.isSafeInteger(participants)) {
      cacheCommunityMetrics(metrics);
    }
    renderCommunityMetrics(metrics);
  }

  prepareCommunityPresentation();
  const cachedCommunityMetrics = readCachedCommunityMetrics();
  if (cachedCommunityMetrics) renderCommunityMetrics(cachedCommunityMetrics);
  refreshCommunityMetrics();
  window.setInterval(() => {
    if (!document.hidden) refreshCommunityMetrics();
  }, COMMUNITY_REFRESH_MS);

  root.classList.add("guild-home-ready");
}());
