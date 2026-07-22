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
})();
