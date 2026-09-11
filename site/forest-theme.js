/* Apply before first paint. Appearance never participates in account state. */
(() => {
  const root = document.documentElement;
  const media = window.matchMedia("(prefers-color-scheme: light)");
  const valid = value => ["light", "dark", "auto"].includes(value);
  let choice = "dark";
  try { const saved = localStorage.getItem("agent-bounties.theme"); if (valid(saved)) choice = saved; } catch (_) { /* Private browsing can disable storage. */ }
  function render() {
    root.dataset.theme = choice === "auto" ? (media.matches ? "light" : "dark") : choice;
    root.dataset.themeChoice = choice;
    document.querySelectorAll("button[data-theme-choice]").forEach(button => button.setAttribute("aria-pressed", String(button.dataset.themeChoice === choice)));
    document.querySelector('meta[name="theme-color"]')?.setAttribute("content", root.dataset.theme === "light" ? "#f7f8f2" : "#07110c");
  }
  render();
  media.addEventListener("change", render);
  document.addEventListener("DOMContentLoaded", render);
  document.addEventListener("click", event => {
    const button = event.target.closest("button[data-theme-choice]");
    if (!button || !valid(button.dataset.themeChoice)) return;
    choice = button.dataset.themeChoice;
    try { localStorage.setItem("agent-bounties.theme", choice); } catch (_) { /* Keep the current-tab preference. */ }
    render();
  });
  window.addEventListener("storage", event => {
    if (event.key !== "agent-bounties.theme") return;
    choice = valid(event.newValue) ? event.newValue : "dark";
    render();
  });
})();
