/* Private GitHub issue snapshots. No provider writes, signatures or funding. */
(function (win, doc) {
  "use strict";
  const root = doc.querySelector("[data-github-origin]"); if (!root) return;
  const api = ["localhost", "127.0.0.1", "[::1]"].includes(win.location.hostname) ? "http://127.0.0.1:3000" : "https://api.agentbounties.app";
  const status = root.querySelector("[data-github-origin-status]"), items = root.querySelector("[data-github-origin-items]"), install = root.querySelector("[data-github-origin-install]"), login = root.querySelector("[data-github-origin-login]");
  login.href = `${api}/v1/site-auth/login/github?return_to=${encodeURIComponent("/install/github/")}`;
  let busy = false;
  async function get(path, privateData) {
    const control = new AbortController(), timer = win.setTimeout(() => control.abort(), 12000);
    try { const reply = await win.fetch(api + path, { credentials: privateData ? "include" : "omit", headers: { Accept: "application/json" }, cache: "no-store", redirect: "error", referrerPolicy: "no-referrer", signal: control.signal }); if (!reply.ok) throw Object.assign(new Error("GitHub draft service unavailable."), { status: reply.status }); return await reply.json(); }
    finally { win.clearTimeout(timer); }
  }
  async function load() {
    if (busy) return; busy = true; items.replaceChildren(); install.hidden = true;
    try {
      const cap = await get("/v1/github-app/capabilities", false);
      if (cap.schema !== "agent-bounties/github-origin-v1" || cap.funding_authority !== false) throw new Error("The GitHub integration could not be verified.");
      if (cap.available && /^https:\/\/github\.com\/apps\/[a-z0-9][a-z0-9-]{0,99}\/installations\/new$/.test(cap.installation_url || "")) { install.href = cap.installation_url; install.hidden = false; status.textContent = `Install the app on selected repositories, then mention ${cap.mention} in an issue comment as an owner, member or collaborator. Your draft will appear here.`; }
      else status.textContent = "The GitHub App is not active yet. The Copilot MCP setup on this page remains available.";
      const inbox = await get("/v1/site-auth/github-origin-drafts", true);
      if (inbox.schema !== cap.schema || inbox.funding_authority !== false || !Array.isArray(inbox.items)) throw new Error("The private draft response could not be verified.");
      login.hidden = true;
      for (const item of inbox.items) {
        const draft = item.plan?.draft;
        if (!/^[0-9a-f-]{36}$/i.test(item.id || "") || !draft || draft.ready_for_publish !== false || typeof draft.title !== "string" || typeof draft.goal !== "string") continue;
        const article = doc.createElement("article"), title = doc.createElement("h3"), goal = doc.createElement("p"), review = doc.createElement("a"), note = doc.createElement("p");
        title.textContent = draft.title; goal.textContent = draft.goal; note.textContent = "Private draft from GitHub. Check the goal, criteria and budget before making any terms public. Nothing is funded or published.";
        review.textContent = "Review this bounty draft"; review.href = `../../creator-open.html?github_origin=${encodeURIComponent(item.id)}`;
        article.append(title, goal, note, review); items.append(article);
      }
      if (!items.childElementCount) { const empty = doc.createElement("p"); empty.textContent = "No current issue drafts for this GitHub account. Snapshots stay available for seven days."; items.append(empty); }
    } catch (error) {
      login.hidden = false;
      const note = doc.createElement("p"); note.textContent = [401, 403].includes(error.status) ? "Sign in with the same GitHub account that mentioned or assigned the app to see your private drafts." : "The GitHub draft service is unavailable. Your existing drafts are unchanged; retry in a moment."; items.append(note);
    } finally { busy = false; }
  }
  root.querySelector("[data-github-origin-refresh]").addEventListener("click", event => { if (event.isTrusted) load(); }); load();
})(window, document);
