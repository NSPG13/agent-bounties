(() => {
  "use strict";

  const body = document.body;
  if (!body || body.classList.contains("guild-home")) return;

  const route = (window.location.pathname.split("/").pop() || "index.html").toLowerCase();
  body.classList.add("guild-interior");
  body.dataset.guildRoute = route.replace(/\.html$/, "") || "home";

  // Primary navigation is generated from the shared template on every page.
  const topbar = document.querySelector("[data-site-header]");

  const footer = document.querySelector("footer") || document.createElement("footer");
  if (!footer.isConnected) {
    footer.innerHTML = `
      <span>We say “paid” only after confirmed settlement.</span>
      <a href="metrics.html">Metrics</a>
      <a href="terms.html">Terms</a>
      <a href="privacy.html">Privacy</a>
      <a href="https://github.com/NSPG13/agent-bounties/issues">Support</a>`;
    document.body.appendChild(footer);
  }
  footer.classList.add("guild-shell-footer");

  const main = document.querySelector("main");
  if (main) main.classList.add("guild-interior-main");

  const syncScrolledState = () => {
    topbar?.classList.toggle("is-scrolled", window.scrollY > 12);
  };
  syncScrolledState();
  window.addEventListener("scroll", syncScrolledState, { passive: true });
})();
