/* Apply before first paint. Appearance never participates in account state.
   Only Light and Dark exist. A first visit (or an older saved "auto") takes the
   system preference once and saves it; after that the saved choice wins. */
(() => {
  const KEY = "agent-bounties.theme", root = document.documentElement;
  const valid = value => value === "light" || value === "dark";
  const system = () => { try { return window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark"; } catch (_) { return "dark"; } };
  let choice = null;
  try { const saved = localStorage.getItem(KEY); if (valid(saved)) choice = saved; } catch (_) { /* Private browsing can disable storage. */ }
  if (!choice) {
    choice = system();
    try { localStorage.setItem(KEY, choice); } catch (_) { /* Keep the current-tab preference. */ }
  }
  function render() {
    root.dataset.theme = choice;
    root.dataset.themeChoice = choice;
    document.querySelectorAll("button[data-theme-choice]").forEach(button => button.setAttribute("aria-pressed", String(button.dataset.themeChoice === choice)));
    document.querySelector('meta[name="theme-color"]')?.setAttribute("content", choice === "light" ? "#f7f8f2" : "#07110c");
  }
  render();
  document.addEventListener("DOMContentLoaded", render);
  document.addEventListener("click", event => {
    const button = event.target.closest("button[data-theme-choice]");
    if (!button || !valid(button.dataset.themeChoice)) return;
    choice = button.dataset.themeChoice;
    try { localStorage.setItem(KEY, choice); } catch (_) { /* Keep the current-tab preference. */ }
    render();
  });
  window.addEventListener("storage", event => {
    if (event.key !== KEY) return;
    choice = valid(event.newValue) ? event.newValue : system();
    render();
  });
})();
