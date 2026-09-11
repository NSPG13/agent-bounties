(() => {
  "use strict";
  const header = document.querySelector("[data-site-header]");
  if (!header) return;
  const toggle = header.querySelector(".ab-site-menu");
  const nav = header.querySelector(".ab-site-nav");
  if (!toggle || !nav) return;
  const accountLink = header.querySelector(".ab-site-login");
  const signedOutHref = accountLink?.getAttribute("href");
  const renderAccount = (session) => {
    if (!accountLink || typeof session?.authenticated !== "boolean") return;
    const authenticated = session.authenticated && Boolean(session.user?.id);
    accountLink.dataset.authenticated = String(authenticated);
    accountLink.textContent = authenticated ? "Account" : "Sign in";
    accountLink.title = authenticated && session.user.name ? `Account: ${String(session.user.name).slice(0, 100)}` : authenticated ? "Your account" : "Sign in";
    if (authenticated) {
      const destination = new URL(signedOutHref, window.location.href);
      destination.searchParams.delete("postReturn");
      destination.hash = "account";
      accountLink.href = destination.href;
    } else accountLink.setAttribute("href", signedOutHref);
  };
  window.addEventListener("agentbounties:account-session", event => renderAccount(event.detail));
  // The homepage owns its session. Interior pages read the same authenticated
  // endpoint; no email, wallet address, or cached display value proves login.
  if (!document.querySelector("[data-auth-dialog]") && accountLink) {
    let checking = false;
    const refreshAccount = async () => {
      if (checking) return;
      checking = true;
      const controller = new AbortController();
      const timer = window.setTimeout(() => controller.abort(), 8000);
      try {
        const local = ["localhost", "127.0.0.1", "[::1]"].includes(window.location.hostname);
        const response = await window.fetch(local ? "/auth/session" : "https://api.agentbounties.app/v1/site-auth/session", { credentials: "include", headers: { Accept: "application/json" }, signal: controller.signal });
        if (response.ok) renderAccount(await response.json());
      } catch (_) { /* A network failure does not prove sign-out. */ }
      finally { checking = false; window.clearTimeout(timer); }
    };
    refreshAccount();
    window.addEventListener("pageshow", refreshAccount);
    window.addEventListener("focus", refreshAccount);
  }
  const close = (focus = false) => {
    header.removeAttribute("data-menu-open");
    toggle.setAttribute("aria-expanded", "false");
    if (focus) toggle.focus();
  };
  header.classList.add("is-enhanced");
  toggle.hidden = false;
  toggle.addEventListener("click", () => {
    const open = !header.hasAttribute("data-menu-open");
    header.toggleAttribute("data-menu-open", open);
    toggle.setAttribute("aria-expanded", String(open));
  });
  header.addEventListener("keydown", event => {
    if (event.key === "Escape" && header.hasAttribute("data-menu-open")) {
      event.preventDefault(); close(true);
    }
  });
  document.addEventListener("click", event => { if (!header.contains(event.target)) close(); });
  header.addEventListener("focusout", event => { if (event.relatedTarget && !header.contains(event.relatedTarget)) close(); });
  nav.addEventListener("click", event => { if (event.target.closest("a")) close(); });
  window.matchMedia("(min-width: 701px)").addEventListener("change", () => close());

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
