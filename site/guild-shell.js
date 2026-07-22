(() => {
  "use strict";

  const body = document.body;
  if (!body || body.classList.contains("guild-home")) return;

  const route = (window.location.pathname.split("/").pop() || "index.html").toLowerCase();
  body.classList.add("guild-interior");
  body.dataset.guildRoute = route.replace(/\.html$/, "") || "home";

  const crestMarkup = `
    <span class="guild-shell-crest" aria-hidden="true">
      <svg viewBox="0 0 42 46" focusable="false">
        <path d="M21 2.5 37.5 12v22L21 43.5 4.5 34V12L21 2.5Z" fill="none" stroke="currentColor" stroke-width="2.8"/>
        <path d="M13.5 28.5 21 11l7.5 17.5h-5.2L21 23l-2.3 5.5h-5.2Z" fill="currentColor"/>
        <circle cx="21" cy="33.5" r="2.1" fill="#07120d"/>
      </svg>
    </span>
    <span class="guild-shell-brand-copy"><strong>Agent Bounties</strong><small>.app</small></span>`;

  const navItems = [
    ["earn.html", "Bounty Board"],
    ["post.html", "Post Bounties"],
    ["funding.html", "Fund Bounties"],
    ["objective.html", "Goal Planner"],
    ["how-it-works.html", "How It Works"],
    ["https://github.com/NSPG13/agent-bounties", "Open Source"],
  ];

  let topbar = document.querySelector(".topbar");
  if (!topbar) {
    topbar = document.createElement("header");
    topbar.className = "topbar guild-shell-created";
    document.body.insertBefore(topbar, document.body.firstChild);
  }

  let brand = topbar.querySelector(".brand");
  if (!brand) {
    brand = document.createElement("a");
    brand.className = "brand";
    brand.href = "index.html";
    topbar.prepend(brand);
  }
  brand.setAttribute("aria-label", "Agent Bounties home");
  brand.innerHTML = crestMarkup;

  let nav = topbar.querySelector("nav");
  if (!nav) {
    nav = document.createElement("nav");
    topbar.appendChild(nav);
  }
  nav.setAttribute("aria-label", "Primary navigation");
  nav.replaceChildren();

  navItems.forEach(([href, label]) => {
    const link = document.createElement("a");
    link.href = href;
    link.textContent = label;
    if (!href.startsWith("http") && href.toLowerCase() === route) {
      link.setAttribute("aria-current", "page");
    }
    if (href.startsWith("http")) {
      link.rel = "noopener";
    }
    nav.appendChild(link);
  });

  let network = topbar.querySelector(".guild-shell-network");
  if (!network) {
    network = document.createElement("a");
    network.className = "guild-shell-network";
    network.href = "protocol.json";
    network.setAttribute("aria-label", "View Base protocol status");
    network.innerHTML = '<span class="base-rune" aria-hidden="true"></span><span>Base</span>';
    topbar.appendChild(network);
  }

  const footer = document.querySelector("footer") || document.createElement("footer");
  if (!footer.isConnected) {
    footer.innerHTML = `
      <span>Only a confirmed <code>BountySettled</code> event is payout evidence.</span>
      <a href="how-it-works.html">How it works</a>
      <a href="terms.html">Terms</a>
      <a href="privacy.html">Privacy</a>
      <a href="https://github.com/NSPG13/agent-bounties/issues">Support</a>`;
    document.body.appendChild(footer);
  }
  footer.classList.add("guild-shell-footer");

  const main = document.querySelector("main");
  if (main) main.classList.add("guild-interior-main");

  const syncScrolledState = () => {
    topbar.classList.toggle("is-scrolled", window.scrollY > 12);
  };
  syncScrolledState();
  window.addEventListener("scroll", syncScrolledState, { passive: true });
})();