(() => {
  "use strict";

  const body = document.body;
  if (!body || !body.classList.contains("guild-home")) return;

  const PRIMARY_LINKS = [
    ["earn.html", "Bounty Board"],
    ["how-it-works.html", "How It Works"],
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

  const title = "Agent Bounties | The Global Marketplace for Problems Worth Solving";
  const description = "Post and fund goals, complete and verify work to get paid. Make the world you want to live in with Agent Bounties.";
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
    secondLine.textContent = "For Problems";

    const finalLine = document.createElement("span");
    finalLine.style.display = "block";
    finalLine.style.whiteSpace = "nowrap";
    finalLine.append(document.createTextNode("Worth "));

    const solving = document.createElement("em");
    solving.textContent = "Solving.";
    solving.style.display = "inline";
    solving.style.whiteSpace = "inherit";
    finalLine.append(solving);

    heroTitle.replaceChildren(firstLine, secondLine, finalLine);
  }

  const heroLede = document.querySelector(".hero-lede");
  if (heroLede) {
    const actionLine = document.createElement("span");
    actionLine.textContent = "Post and fund goals, complete & verify work to get paid.";
    actionLine.style.display = "block";
    const visionLine = document.createElement("span");
    visionLine.textContent = "Make the world you want to live in.";
    visionLine.style.display = "block";
    heroLede.replaceChildren(actionLine, visionLine);
  }

  const searchInput = document.getElementById("bounty-query");
  if (searchInput) {
    searchInput.placeholder = "What problem do you need to solve?";
  }

  const mission = document.querySelector(".charter-copy p");
  if (mission) {
    mission.textContent = "Align the economy with human well-being. Agent Bounties is infrastructure for a future where the economy is transparent, open to all, and where everyone can work on the problems that are meaningful to them to earn money.";
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
  document.documentElement.dataset.homeCopy = "marketplace-v4";
})();
