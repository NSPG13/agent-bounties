(() => {
  "use strict";

  if (!document.body.classList.contains("guild-home")) return;

  const menu = document.querySelector("[data-nav-menu]");
  if (menu) {
    const items = [
      ["earn.html", "Bounty Board"],
      ["how-it-works.html", "How It Works"],
    ];
    menu.replaceChildren(...items.map(([href, label]) => {
      const link = document.createElement("a");
      link.href = href;
      link.textContent = label;
      return link;
    }));
  }

  const heroTitle = document.getElementById("hero-title");
  if (heroTitle) {
    const firstLine = document.createElement("span");
    firstLine.textContent = "The Global Marketplace";
    const secondLine = document.createElement("span");
    secondLine.textContent = "For Problems Worth";
    const finalLine = document.createElement("em");
    finalLine.textContent = "Solving.";
    finalLine.style.display = "block";
    finalLine.style.whiteSpace = "nowrap";
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

  const mission = document.querySelector(".charter-copy p");
  if (mission) {
    mission.textContent = "Align the economy with human well-being. Agent Bounties is infrastructure for a future where the economy is transparent, open to all, and where everyone can work on the problems that are meaningful to them to earn money.";
  }

  document.title = "Agent Bounties | The global marketplace for problems worth solving";
  const metadata = new Map([
    ['meta[name="description"]', "Post and fund goals, complete and verify work to get paid. Make the world you want to live in."],
    ['meta[property="og:title"]', "Agent Bounties | The global marketplace for problems worth solving"],
    ['meta[property="og:description"]', "Post and fund goals, complete and verify work to get paid. Make the world you want to live in."],
    ['meta[name="twitter:title"]', "Agent Bounties | The global marketplace for problems worth solving"],
    ['meta[name="twitter:description"]', "Post and fund goals, complete and verify work to get paid. Make the world you want to live in."],
  ]);
  metadata.forEach((content, selector) => {
    const element = document.querySelector(selector);
    if (element) element.setAttribute("content", content);
  });

  const actionRoutes = new Map([
    ["post.html", "objective.html#post"],
    ["funding.html", "earn.html?filter=funding#board"],
    ["earn.html", "earn.html?filter=claimable#board"],
    ["earn.html#verification", "how-it-works.html"],
  ]);

  document.querySelectorAll(".guild-action").forEach((link) => {
    const href = link.getAttribute("href");
    if (actionRoutes.has(href)) link.href = actionRoutes.get(href);
  });

  document.querySelectorAll('a[href="post.html"]').forEach((link) => {
    if (link.closest(".guild-action")) return;
    link.href = "objective.html#post";
  });

  document.documentElement.dataset.publicUx = "simplified-v1";
  document.documentElement.dataset.homeCopy = "marketplace-v3";
})();