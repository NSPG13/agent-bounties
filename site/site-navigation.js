(() => {
  "use strict";
  const header = document.querySelector("[data-site-header]");
  if (!header) return;
  const nav = header.querySelector(".ab-site-nav");
  if (!nav) return;
  header.classList.add("is-enhanced");
  // The header "Log in" and footer "Create an account" links both become "Account".
  const accountLinks = Array.from(document.querySelectorAll("[data-account-link]"), link => ({ link, href: link.getAttribute("href"), label: link.textContent }));
  const renderAccount = (session) => {
    if (typeof session?.authenticated !== "boolean") return;
    const authenticated = session.authenticated && Boolean(session.user?.id);
    for (const { link, href, label } of accountLinks) {
      link.dataset.authenticated = String(authenticated);
      link.textContent = authenticated ? "Account" : label;
      link.title = authenticated && session.user.name ? `Account: ${String(session.user.name).slice(0, 100)}` : authenticated ? "Your account" : label;
      if (authenticated) {
        const destination = new URL(href, window.location.href);
        destination.searchParams.delete("postReturn");
        destination.hash = "account";
        link.href = destination.href;
      } else link.setAttribute("href", href);
    }
  };
  window.addEventListener("agentbounties:account-session", event => renderAccount(event.detail));
  // The homepage owns its session. Interior pages read the same authenticated
  // endpoint; no email, wallet address, or cached display value proves login.
  if (!document.querySelector("[data-auth-dialog]") && accountLinks.length) {
    let checking = false;
    const refreshAccount = async () => {
      if (checking) return;
      checking = true;
      const controller = new AbortController();
      const timer = window.setTimeout(() => controller.abort(), 8000);
      try {
        const local = ["localhost", "127.0.0.1", "[::1]"].includes(window.location.hostname);
        const response = await window.fetch(local ? "/auth/session" : "https://api.agentizable.ai/v1/site-auth/session", { credentials: "include", headers: { Accept: "application/json" }, signal: controller.signal });
        if (response.ok) renderAccount(await response.json());
      } catch (_) { /* A network failure does not prove sign-out. */ }
      finally { checking = false; window.clearTimeout(timer); }
    };
    refreshAccount();
    window.addEventListener("pageshow", refreshAccount);
    window.addEventListener("focus", refreshAccount);
  }
  // Deep links remain useful when the destination is inside an optional detail.
  const revealHash = () => {
    let id;
    try { id = decodeURIComponent(window.location.hash.slice(1)); } catch { return; }
    const target = id && document.getElementById(id);
    if (!target || !target.closest("details")) return;
    let parent = target;
    while (parent) { if (parent.tagName === "DETAILS") parent.open = true; parent = parent.parentElement; }
    target.scrollIntoView({ block: "start" });
  };
  window.addEventListener("hashchange", revealHash);
  revealHash();
})();
