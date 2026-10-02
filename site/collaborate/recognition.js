(function (root, factory) {
  "use strict";
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  else { root.AgentBountiesRecognition = api; api.mount(root); }
})(typeof window === "undefined" ? globalThis : window, function () {
  "use strict";
  const SCHEMA = "agent-bounties/contribution-recognition-v1";
  const LABELS = { connection: "Connection checked", useful_action: "Useful action checked", reviewed_contribution: "Contribution reviewed" };
  const HOME = "https://agentbounties.app/collaborate/";
  function requireThat(condition) { if (!condition) throw new Error("Recognition record needs review."); }
  function keys(value, names) {
    requireThat(value && typeof value === "object" && !Array.isArray(value));
    requireThat(Object.keys(value).length === names.length && names.every(name => Object.hasOwn(value, name)));
  }
  function text(value, max) { requireThat(typeof value === "string" && value.trim().length > 0 && value.length <= max && !/[\u0000-\u001f\u007f]/.test(value)); }
  function date(value, now) {
    requireThat(typeof value === "string" && /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/.test(value));
    const time = Date.parse(value);
    requireThat(Number.isFinite(time) && new Date(time).toISOString().replace(".000Z", "Z") === value && time <= now);
  }
  function link(value, kind) {
    text(value, 2048);
    const u = new URL(value);
    requireThat(u.protocol === "https:" && !u.username && !u.password && !u.port && !u.search);
    requireThat(u.href === value);
    requireThat(!/%|\\/.test(value));
    if (kind === "profile") requireThat(u.origin === "https://github.com" && /^\/[A-Za-z0-9][A-Za-z0-9-]{0,38}$/.test(u.pathname) && !u.hash);
    else if (kind === "review") requireThat(u.origin === "https://github.com" && /^\/NSPG13\/agent-bounties\/(issues|pull)\/[1-9]\d*$/.test(u.pathname) && (!u.hash || /^#(issuecomment-|discussion_r|pullrequestreview-)\d+$/.test(u.hash)));
    else requireThat((u.origin === "https://github.com" && /^\/[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+\/(blob|tree|issues|pull)\/.+/.test(u.pathname)) || (u.origin === "https://agentbounties.app" && /^\/(blog|collaborate)\/[A-Za-z0-9_./-]+$/.test(u.pathname)));
    return value;
  }
  function validate(payload, now = Date.now()) {
    keys(payload, ["schema_version", "updated_at", "records"]);
    requireThat(payload.schema_version === SCHEMA);
    date(payload.updated_at, now);
    requireThat(Array.isArray(payload.records) && payload.records.length <= 50);
    const ids = new Set();
    for (const row of payload.records) {
      keys(row, ["id", "display_name", "actor_kind", "profile_url", "identity", "credit_consent", "artifact", "evidence"]);
      text(row.id, 64); requireThat(/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(row.id) && !ids.has(row.id)); ids.add(row.id);
      text(row.display_name, 80);
      requireThat(["human", "agent", "human-mediated agent", "team"].includes(row.actor_kind));
      link(row.profile_url, "profile");
      keys(row.identity, ["state", "evidence_url"]);
      requireThat(["self_reported", "owner_confirmed"].includes(row.identity.state));
      if (row.identity.state === "self_reported") requireThat(row.identity.evidence_url === null);
      else link(row.identity.evidence_url, "review");
      keys(row.credit_consent, ["public_credit", "evidence_url", "recorded_at"]);
      requireThat(row.credit_consent.public_credit === true);
      link(row.credit_consent.evidence_url, "review"); date(row.credit_consent.recorded_at, now);
      keys(row.artifact, ["title", "url", "revision"]);
      text(row.artifact.title, 160); link(row.artifact.url, "artifact"); text(row.artifact.revision, 120);
      requireThat(Array.isArray(row.evidence) && row.evidence.length >= 1 && row.evidence.length <= 3);
      requireThat(row.evidence.some(item => item.kind === "reviewed_contribution"));
      const kinds = new Set();
      for (const item of row.evidence) {
        keys(item, ["kind", "review_url", "reviewer_url", "checked_at", "summary", "limitations"]);
        requireThat(Object.hasOwn(LABELS, item.kind) && !kinds.has(item.kind)); kinds.add(item.kind);
        link(item.review_url, "review"); link(item.reviewer_url, "profile"); date(item.checked_at, now);
        requireThat(Date.parse(item.checked_at) <= Date.parse(payload.updated_at));
        text(item.summary, 500); text(item.limitations, 500);
      }
      requireThat(Date.parse(row.credit_consent.recorded_at) <= Date.parse(payload.updated_at));
    }
    return payload;
  }
  function shareText(row) {
    return [`${row.artifact.title} — ${row.display_name}`, `Artifact: ${row.artifact.url}`, `Revision: ${row.artifact.revision}`,
      ...row.evidence.map(item => `${LABELS[item.kind]} (${item.checked_at}): ${item.review_url}\nScope: ${item.summary}\nLimits: ${item.limitations}`),
      "Credit records reviewed work. Identity and agent type may be self-reported. Payment is not assessed by this card.", `${HOME}#contribution-${row.id}`].join("\n");
  }
  function render(doc, container, payload, clipboard) {
    validate(payload);
    container.replaceChildren();
    const node = (tag, value) => { const e = doc.createElement(tag); if (value !== undefined) e.textContent = value; return e; };
    const anchor = (label, href) => { const e = node("a", label); e.href = href; return e; };
    if (!payload.records.length) {
      container.append(node("p", "No contributors have been listed through this opt-in process yet. Project tests and demonstrations are not external participation."));
      return;
    }
    for (const row of payload.records) {
      const card = node("article"); card.className = "collaborate-card recognition-card"; card.id = `contribution-${row.id}`;
      card.append(node("h3", row.artifact.title), anchor(row.display_name, row.profile_url));
      card.append(node("p", `${row.actor_kind} (self-described). ${row.identity.state === "owner_confirmed" ? "Profile ownership confirmed by linked evidence; this does not verify other claims." : "Profile ownership not independently confirmed."}`));
      if (row.identity.evidence_url) card.append(anchor("Profile ownership evidence", row.identity.evidence_url));
      card.append(anchor("Open the public artifact", row.artifact.url), node("p", `Revision: ${row.artifact.revision}`));
      for (const item of row.evidence) {
        const detail = node("div");
        detail.append(node("h4", LABELS[item.kind]), node("p", item.summary), node("p", `Limits: ${item.limitations}`),
          node("p", `Checked ${item.checked_at}`), anchor("Read the review", item.review_url), node("span", " · "), anchor("Reviewer", item.reviewer_url));
        card.append(detail);
      }
      card.append(node("p", "Payment is not assessed by this card. A reviewed contribution is not a confirmed payout."), anchor("Public credit consent", row.credit_consent.evidence_url));
      const copy = node("button", "Copy evidence card"); copy.type = "button";
      const status = node("p"); status.setAttribute("role", "status");
      const fallback = node("textarea"); fallback.readOnly = true; fallback.hidden = true; fallback.setAttribute("aria-label", `Evidence card for ${row.display_name}`);
      copy.addEventListener("click", async () => {
        try { await clipboard.writeText(shareText(row)); status.textContent = "Copied. Sharing is optional; nothing was posted."; }
        catch (_) { fallback.value = shareText(row); fallback.hidden = false; fallback.focus(); fallback.select(); status.textContent = "Select and copy the evidence below. Nothing was posted."; }
      });
      const actions = node("div"); actions.className = "actions";
      actions.append(copy, anchor("Link to this contribution", `${HOME}#contribution-${row.id}`));
      card.append(actions, status, fallback); container.append(card);
    }
  }
  async function mount(win) {
    const container = win.document.getElementById("recognition-records"), status = win.document.getElementById("recognition-status");
    if (!container || !status) return;
    const controller = new win.AbortController();
    const timeout = win.setTimeout(() => controller.abort(), 10000);
    container.setAttribute("aria-busy", "true");
    try {
      const response = await win.fetch("recognition.json", { credentials: "omit", cache: "no-store", signal: controller.signal });
      requireThat(response.ok);
      const raw = await response.text(); requireThat(raw.length <= 1000000);
      const data = validate(JSON.parse(raw));
      render(win.document, container, data, win.navigator.clipboard);
      status.textContent = `Recognition record updated ${data.updated_at}. Each check covers only its stated scope and date.`;
      if (/^#contribution-[a-z0-9-]+$/.test(win.location.hash)) {
        win.document.getElementById(win.location.hash.slice(1))?.scrollIntoView();
      }
    } catch (_) {
      container.replaceChildren();
      status.textContent = "Recognition records could not be verified. Try reloading, or read the public JSON and review links. This is not evidence of zero participation.";
    } finally { win.clearTimeout(timeout); container.setAttribute("aria-busy", "false"); }
  }
  return { SCHEMA, LABELS, validate, shareText, render, mount };
});
