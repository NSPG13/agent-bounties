/* Shared ⓘ info tip: "what is this for" help next to a button or control.
   Author it right after the control as <span data-info-tip>Help text.</span>.
   This script turns it into a small circled-i button with the text as its tooltip:
   hover or keyboard focus shows it, a tap or click pins it open, a second tap, a tap
   outside or Esc closes it. The text stays a normal text node, so i18n.js translates
   it (and the button's aria-label) like any other page text. Without JavaScript the
   text simply stays visible next to the control. Real warnings, errors and live
   status messages are never info tips. */
(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root && root.document) api.start(root, root.document);
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";
  const LABEL = "What is this?", MARGIN = 8, GAP = 6;
  const ICON = '<svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true" focusable="false">'
    + '<circle cx="8" cy="8" r="6.75" fill="none" stroke="currentColor" stroke-width="1.5"></circle>'
    + '<path d="M8 7.1v4.4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"></path>'
    + '<circle cx="8" cy="4.7" r="1" fill="currentColor"></circle></svg>';

  // Keep the tip inside the viewport: below the icon when it fits, otherwise above,
  // and shifted sideways so it never runs off a narrow screen.
  function place(icon, tip, viewport) {
    const a = icon.getBoundingClientRect(), t = tip.getBoundingClientRect();
    const width = Math.min(t.width, viewport.width - 2 * MARGIN);
    let left = a.left + a.width / 2 - width / 2;
    left = Math.max(MARGIN, Math.min(left, viewport.width - MARGIN - width));
    let top = a.bottom + GAP;
    if (top + t.height > viewport.height - MARGIN && a.top - GAP - t.height >= MARGIN) top = a.top - GAP - t.height;
    return { left: Math.round(left), top: Math.round(top) };
  }

  function start(win, doc) {
    let count = 0, open = null;
    const viewport = () => ({ width: doc.documentElement.clientWidth || win.innerWidth, height: win.innerHeight });

    function show(tip, pinned) {
      if (open && open.tip !== tip) hide();
      const icon = tip.previousElementSibling;
      tip.hidden = false;
      icon.setAttribute("aria-expanded", "true");
      const at = place(icon, tip, viewport());
      tip.style.left = at.left + "px";
      tip.style.top = at.top + "px";
      open = { tip, icon, pinned: Boolean(pinned) || Boolean(open && open.tip === tip && open.pinned) };
    }
    function hide() {
      if (!open) return;
      open.tip.hidden = true;
      open.icon.setAttribute("aria-expanded", "false");
      open = null;
    }
    // Close on leave or blur unless the person pinned it with a tap or click.
    function release(tip) {
      if (!open || open.tip !== tip || open.pinned) return;
      if (open.icon === doc.activeElement || open.icon.matches(":hover")) return;
      hide();
    }

    function enhance(node) {
      if (!node || node.hasAttribute("data-info-tip-ready")) return;
      node.setAttribute("data-info-tip-ready", "");
      const id = node.id || `ab-info-tip-${++count}`;
      const wrap = doc.createElement("span"), icon = doc.createElement("button");
      wrap.className = "ab-info";
      icon.type = "button";
      icon.className = "ab-info-tip";
      icon.setAttribute("aria-label", node.getAttribute("data-info-tip-label") || LABEL);
      icon.setAttribute("aria-describedby", id);
      icon.setAttribute("aria-expanded", "false");
      icon.innerHTML = ICON;
      node.parentNode.insertBefore(wrap, node);
      wrap.append(icon, node);
      node.id = id;
      node.setAttribute("role", "tooltip");
      node.classList.add("ab-info-tip-text");
      node.hidden = true;
      icon.addEventListener("mouseenter", () => show(node));
      icon.addEventListener("focus", () => show(node));
      icon.addEventListener("blur", () => win.setTimeout(() => release(node), 0));
      wrap.addEventListener("mouseleave", () => win.setTimeout(() => release(node), 0));
      icon.addEventListener("click", (event) => {
        event.preventDefault();
        if (open && open.tip === node && open.pinned) { hide(); return; }
        show(node, true);
      });
    }
    function enhanceAll(scope) {
      if (!scope || scope.nodeType !== 1) return;
      if (scope.matches("[data-info-tip]")) enhance(scope);
      for (const node of scope.querySelectorAll("[data-info-tip]:not([data-info-tip-ready])")) enhance(node);
    }

    function ready() {
      enhanceAll(doc.body);
      doc.addEventListener("pointerdown", (event) => {
        if (open && !open.tip.parentNode.contains(event.target)) hide();
      });
      doc.addEventListener("keydown", (event) => {
        if (event.key !== "Escape" || !open) return;
        const icon = open.icon, inside = icon.parentNode.contains(doc.activeElement);
        hide();
        if (inside) icon.focus({ preventScroll: true });
        // Focus brings the tip back; Esc means closed until the next hover, focus or tap.
        if (open) hide();
      });
      const reposition = () => { if (open) show(open.tip, open.pinned); };
      win.addEventListener("resize", reposition);
      win.addEventListener("scroll", reposition, { passive: true, capture: true });
      // Pages that render help text later (dialogs, lists) get tips too.
      if (typeof win.MutationObserver === "function") {
        new win.MutationObserver((records) => {
          for (const record of records) for (const node of record.addedNodes) enhanceAll(node);
        }).observe(doc.body, { subtree: true, childList: true });
      }
      win.AgentBountiesInfoTip = { enhance: enhanceAll, hide };
    }
    if (doc.readyState === "loading") doc.addEventListener("DOMContentLoaded", ready, { once: true });
    else ready();
  }

  return { start, place, LABEL };
});
