// Saved drafts on the posting page: a signed-in person sees the drafts saved to their
// account and opens any of them here, from any device. Listing and Delete use the same
// account API and rules as the home page's "Your activity": only drafts the server marks
// can_delete (no wallet step) can be deleted, and never the draft open in this tab.
(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root?.document) api.start(root, root.document);
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";
  const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
  const JOURNEY = "agent-bounties.guided-journey.v1", APPROVAL = "agent-bounties.posting-approval.v1";

  // Only first-party posting links are opened; anything else in the payload is dropped.
  function postingHref(value, origin) {
    try {
      const url = new URL(String(value), origin);
      return [new URL(origin).origin, "https://agentizable.ai"].includes(url.origin) && ["/post", "/post.html"].includes(url.pathname)
        && !url.username && !url.password ? url.href : null;
    } catch (_) { return null; }
  }

  function parse(payload, origin, currentId) {
    const available = payload?.status === "available" && Array.isArray(payload.items);
    const current = String(currentId || "").toLowerCase(), seen = new Set(), items = [];
    for (const item of available ? payload.items.slice(0, 300) : []) {
      const operationId = typeof item?.operation_id === "string" && UUID.test(item.operation_id) ? item.operation_id.toLowerCase() : null;
      const href = postingHref(item?.continuation_url, origin), title = String(item?.title || "").trim().slice(0, 160);
      if (!operationId || !href || !title || seen.has(operationId) || !["drafts", "needs_action"].includes(item.group)) continue;
      seen.add(operationId);
      const revision = Number.isSafeInteger(item.revision) && item.revision > 0 ? item.revision : null, open = operationId === current;
      items.push({ operationId, title, href, open, revision,
        status: open ? "Open now" : String(item.status || "Saved draft").slice(0, 100),
        needsAction: item.group === "needs_action",
        deletable: item.group === "drafts" && item.can_delete === true && revision !== null && !open,
        updatedAt: Number.isFinite(Date.parse(item.updated_at)) ? new Date(item.updated_at).toISOString() : null });
    }
    return { available, items, nextOffset: available && Number.isSafeInteger(payload.next_offset) ? payload.next_offset : null };
  }

  function start(win, doc) {
    const panel = doc.querySelector("[data-saved-drafts]"), flow = win.AgentBountiesWorkflow;
    if (!panel || !flow?.apiBase) return;
    const list = panel.querySelector("[data-saved-drafts-list]"), count = panel.querySelector("[data-saved-drafts-count]"), note = panel.querySelector("[data-saved-drafts-status]");
    const base = `${flow.apiBase(win.location)}/v1/site-auth/posting-drafts`;
    const currentId = () => {
      const requested = new URL(win.location.href).searchParams.get("operation_id");
      if (requested && UUID.test(requested)) return requested;
      try { return flow.createClient(win).load()?.id || null; } catch (_) { return null; }
    };
    let items = [], nextOffset = null, loading = 0;
    const request = (path, options = {}) => win.fetch(`${base}${path}`, { credentials: "include", cache: "no-store", referrerPolicy: "no-referrer",
      ...options, headers: { Accept: "application/json", ...(options.body ? { "content-type": "application/json" } : {}) } });
    const updated = (iso) => {
      if (!iso) return "";
      const minutes = Math.round((Date.parse(iso) - Date.now()) / 60000), format = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
      return `Updated ${Math.abs(minutes) < 60 ? format.format(minutes, "minute") : Math.abs(minutes) < 1440 ? format.format(Math.round(minutes / 60), "hour") : format.format(Math.round(minutes / 1440), "day")}`;
    };
    function forgetLocal(operationId) {
      try {
        const local = JSON.parse(win.sessionStorage.getItem(JOURNEY) || "null");
        if (String(local?.id || "").toLowerCase() === operationId) { win.sessionStorage.removeItem(JOURNEY); win.sessionStorage.removeItem(APPROVAL); }
      } catch (_) { /* The server copy is already gone. */ }
    }
    async function remove(item, row, feedback) {
      feedback.textContent = "Deleting…";
      try {
        const response = await request(`/${item.operationId}/delete`, { method: "POST", body: JSON.stringify({ expected_revision: item.revision }) });
        const result = await response.json().catch(() => null);
        if (!response.ok) {
          feedback.textContent = result?.error === "draft_has_wallet_activity" ? "This draft has a recorded wallet step, so it is kept. Open it to check its status."
            : result?.error === "revision_conflict" ? "This draft changed on another device. Reload this page, then try again."
              : "Couldn’t delete this draft. Try again.";
          row.querySelector("[data-saved-draft-delete]")?.removeAttribute("hidden");
          return;
        }
        forgetLocal(item.operationId);
        items = items.filter((entry) => entry.operationId !== item.operationId);
        // The server pages with OFFSET: one fewer row before the next page moves it back by one.
        if (nextOffset !== null) nextOffset = Math.max(0, nextOffset - 1);
        render(`Deleted “${item.title}”.`);
      } catch (_) {
        feedback.textContent = "Couldn’t delete this draft. Try again.";
        row.querySelector("[data-saved-draft-delete]")?.removeAttribute("hidden");
      }
    }
    function render(message = "") {
      list.replaceChildren();
      count.textContent = String(items.length);
      note.textContent = message;
      panel.hidden = !items.length && !message;
      for (const item of items) {
        const row = doc.createElement("li"), head = doc.createElement("div"), link = doc.createElement("a"), chip = doc.createElement("span"), when = doc.createElement("small");
        row.className = "saved-draft"; row.dataset.operationId = item.operationId; head.className = "saved-draft-head";
        link.href = item.href; link.textContent = item.title; if (item.open) link.setAttribute("aria-current", "page");
        chip.className = "saved-draft-chip"; chip.dataset.needsAction = String(item.needsAction); chip.textContent = item.status;
        when.textContent = updated(item.updatedAt);
        head.append(link, chip); row.append(head); if (when.textContent) row.append(when);
        if (item.deletable) {
          const removeButton = doc.createElement("button"), confirm = doc.createElement("div"), question = doc.createElement("span"), yes = doc.createElement("button"), keep = doc.createElement("button"), feedback = doc.createElement("p");
          removeButton.type = yes.type = keep.type = "button";
          removeButton.className = yes.className = "ab-button ab-button--compact ab-button--danger"; keep.className = "ab-button ab-button--compact";
          removeButton.dataset.savedDraftDelete = ""; yes.dataset.savedDraftDeleteConfirm = "";
          removeButton.textContent = "Delete"; removeButton.setAttribute("aria-label", `Delete draft “${item.title}”`);
          question.textContent = "Delete this draft? This can’t be undone."; yes.textContent = "Delete draft"; keep.textContent = "Keep";
          confirm.className = "saved-draft-confirm"; confirm.hidden = true; feedback.setAttribute("role", "status"); feedback.className = "saved-draft-feedback";
          removeButton.addEventListener("click", () => { removeButton.hidden = true; confirm.hidden = false; keep.focus(); });
          keep.addEventListener("click", () => { confirm.hidden = true; removeButton.hidden = false; removeButton.focus(); });
          yes.addEventListener("click", () => { confirm.hidden = true; void remove(item, row, feedback); });
          confirm.append(question, yes, keep); head.append(removeButton); row.append(confirm, feedback);
        }
        list.append(row);
      }
      if (nextOffset !== null) {
        const more = doc.createElement("li"), button = doc.createElement("button");
        button.type = "button"; button.className = "ab-button ab-button--compact"; button.textContent = "Load older drafts";
        button.addEventListener("click", () => { button.disabled = true; void load(nextOffset); });
        more.append(button); list.append(more);
      }
    }
    async function load(offset = 0) {
      const token = ++loading;
      try {
        const response = await request(offset ? `?offset=${offset}` : "");
        if (token !== loading) return;
        // Signed out: nothing to list. The posting form says how to sign in.
        if (response.status === 401) { items = []; nextOffset = null; render(); return; }
        const view = parse(await response.json().catch(() => null), win.location.origin, currentId());
        if (token !== loading) return;
        if (!view.available) { if (!offset) items = []; nextOffset = null; render("Saved drafts are temporarily unavailable. Your saved work has not been removed."); return; }
        const known = new Set(items.map((item) => item.operationId));
        items = offset ? [...items, ...view.items.filter((item) => !known.has(item.operationId))] : view.items;
        nextOffset = view.nextOffset; render();
        // Open the list when this tab has no draft yet; otherwise keep the form first.
        if (!offset && !currentId() && items.length) panel.open = true;
      } catch (_) {
        if (token === loading) render("Saved drafts couldn’t load. Your saved work has not been removed; reload to try again.");
      }
    }
    void load();
  }

  return { parse, postingHref, start };
});
