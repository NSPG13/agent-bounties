(() => {
  "use strict";
  const header = document.querySelector("[data-site-header]");
  if (!header) return;
  const toggle = header.querySelector(".ab-site-menu");
  const nav = header.querySelector(".ab-site-nav");
  if (!toggle || !nav) return;
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
