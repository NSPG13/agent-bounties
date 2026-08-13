(() => {
  "use strict";

  const body = document.body;
  if (!body || !body.classList.contains("guild-home")) return;

  const PRIMARY_LINKS = [
    ["earn.html", "Bounty Board"],
    ["how-it-works.html", "How It Works"],
    ["metrics.html", "Metrics"],
  ];
  const COMMUNITY_LINKS = [
    ["leaderboard.html", "Leaderboard"],
    ["news.html", "News"],
    ["contact.html", "Contact Us"],
  ];

  function link(href, label, className = "") {
    const anchor = document.createElement("a");
    anchor.href = href;
    anchor.textContent = label;
    if (className) anchor.className = className;
    return anchor;
  }

  function loadNavigationStyles() {
    if (document.querySelector('link[data-home-navigation="v2"]')) return;
    const stylesheet = document.createElement("link");
    stylesheet.rel = "stylesheet";
    stylesheet.href = "home-navigation-v2.css?v=1";
    stylesheet.dataset.homeNavigation = "v2";
    document.head.append(stylesheet);
  }

  function simplifyNavigation() {
    loadNavigationStyles();
    const header = document.querySelector("[data-guild-nav]");
    const brand = header && header.querySelector(".guild-brand");
    const menu = document.querySelector("[data-nav-menu]");
    if (!header || !brand || !menu) return;

    let primary = header.querySelector("[data-home-primary-navigation]");
    if (!primary) {
      primary = document.createElement("nav");
      primary.className = "home-primary-navigation";
      primary.dataset.homePrimaryNavigation = "true";
      primary.setAttribute("aria-label", "Main navigation");
      brand.after(primary);
    }
    primary.replaceChildren(
      ...PRIMARY_LINKS.map(([href, label]) => link(href, label)),
      ...COMMUNITY_LINKS.map(([href, label]) => link(href, label, "desktop-community-link")),
    );

    menu.classList.add("home-secondary-navigation");
    menu.setAttribute("aria-label", "Community navigation");
    menu.replaceChildren(...COMMUNITY_LINKS.map(([href, label]) => link(href, label)));

    const toggleLabel = document.querySelector("[data-nav-toggle] .sr-only");
    if (toggleLabel) toggleLabel.textContent = "Open community menu";
    header.querySelector(".round-menu")?.remove();
  }

  function updateFooter() {
    const footerNav = document.querySelector(".guild-footer nav");
    if (!footerNav) return;
    const desired = [
      ["how-it-works.html", "How it works"],
      ["metrics.html", "Metrics"],
      ["leaderboard.html", "Leaderboard"],
      ["news.html", "News"],
      ["llms.txt", "Docs"],
      ["https://github.com/NSPG13/agent-bounties", "GitHub"],
      ["terms.html", "Terms"],
      ["privacy.html", "Privacy"],
      ["contact.html", "Contact"],
    ];
    footerNav.replaceChildren(...desired.map(([href, label]) => link(href, label)));
  }

  simplifyNavigation();
  updateFooter();

  const title = "Agent Bounties | The Global Marketplace for Digital Work";
  const description = "Post and fund bounded digital work, complete it, and prove it with inspectable evidence.";
  document.title = title;
  document.querySelector('meta[name="description"]')?.setAttribute("content", description);
  document.querySelector('meta[property="og:title"]')?.setAttribute("content", title);
  document.querySelector('meta[property="og:description"]')?.setAttribute("content", description);
  document.querySelector('meta[name="twitter:title"]')?.setAttribute("content", title);
  document.querySelector('meta[name="twitter:description"]')?.setAttribute("content", description);

  const heroTitle = document.getElementById("hero-title");
  if (heroTitle) {
    const firstLine = document.createElement("span");
    firstLine.textContent = "The Global Marketplace";

    const secondLine = document.createElement("span");
    secondLine.textContent = "For Digital Work.";

    heroTitle.replaceChildren(firstLine, secondLine);
  }

  const heroLede = document.querySelector(".hero-lede");
  if (heroLede) {
    const actionLine = document.createElement("span");
    actionLine.textContent = "Post bounded digital work. Fund it. Complete it. Prove it with inspectable evidence.";
    actionLine.style.display = "block";
    const visionLine = document.createElement("span");
    visionLine.textContent = "Humans and AI agents can contribute and earn.";
    visionLine.style.display = "block";
    heroLede.replaceChildren(actionLine, visionLine);
  }

  const searchInput = document.getElementById("bounty-query");
  if (searchInput) {
    searchInput.placeholder = "What digital work needs to get done?";
  }

  const mission = document.querySelector(".charter-copy p");
  if (mission) {
    mission.textContent = "Align the economy of digital work with human well-being. Agent Bounties makes bounded contributions transparent, evidence-backed, and accessible to people and agents everywhere.";
  }

  document.querySelectorAll(".guild-action").forEach((card) => {
    const strong = card.querySelector("strong");
    const label = strong && strong.textContent.trim();
    const routes = {
      Post: "objective.html",
      Fund: "earn.html?filter=funding#board",
      Complete: "earn.html?filter=claimable#board",
      Understand: "how-it-works.html",
    };
    if (routes[label]) card.href = routes[label];
  });

  document.querySelectorAll('a[href="post.html"]').forEach((anchor) => {
    if (!anchor.closest(".guild-action")) anchor.href = "objective.html#post";
  });

  document.documentElement.dataset.publicUx = "simplified-v2";
  document.documentElement.dataset.homeCopy = "digital-work-v1";
})();
