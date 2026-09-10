/* Isolated rollout check. No production journey, approval, or wallet access. */
(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root?.document) api.mount(root);
})(typeof window !== "undefined" ? window : null, function () {
  "use strict";
  const API = "https://api.agentbounties.app";
  const KEY = "agent-bounties.private-draft-canary.v1";
  const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
  const HASH = /^[0-9a-f]{64}$/;
  function envelope(id) {
    if (!UUID.test(id)) throw new Error("Invalid canary operation ID.");
    return { schema: "agent-bounties/posting-draft-v1", id, role: "post",
      goal: "Private posting storage canary. No bounty requested.", preferences: "Canary storage only.",
      brief: null, draft: null, draft_stale: false };
  }
  function stable(value) {
    if (value && typeof value === "object") return Array.isArray(value)
      ? `[${value.map(stable).join(",")}]`
      : `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${stable(value[key])}`).join(",")}}`;
    return JSON.stringify(value);
  }
  function create(win, report = () => {}) {
    let account = null, busy = false;
    function saved() {
      try {
        const value = JSON.parse(win.sessionStorage.getItem(KEY) || "null");
        return value && UUID.test(value.operation_id) && HASH.test(value.account_id) ? value : null;
      } catch (_) { return null; }
    }
    async function request(path, { method = "GET", body, credentials = "include" } = {}) {
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), 15000);
      try {
        const response = await win.fetch(`${API}${path}`, { method, credentials, cache: "no-store",
          signal: controller.signal, headers: body ? { "Content-Type": "application/json" } : {},
          ...(body ? { body: JSON.stringify(body) } : {}) });
        let data = null;
        try { data = await response.json(); } catch (_) { /* Status remains visible. */ }
        if (!String(response.headers.get("cache-control") || "").includes("no-store")) throw new Error("The API response is missing Cache-Control: no-store.");
        return { status: response.status, data };
      } catch (error) {
        if (error.name === "AbortError") throw new Error("The API request timed out. Read the saved canary before starting a new test.");
        throw error;
      } finally { clearTimeout(timer); }
    }
    function expect(response, status, step) {
      if (response.status !== status) throw new Error(`${step}: expected HTTP ${status}, received ${response.status}${/^[a-z_]+$/.test(response.data?.error || "") ? ` (${response.data.error})` : ""}.`);
      return response.data;
    }
    async function refresh() {
      const data = expect(await request("/v1/site-auth/session"), 200, "Account check");
      account = { authenticated: data.authenticated === true, id: HASH.test(data.user?.id || "") ? data.user.id : null,
        enabled: data.posting_drafts_enabled === true };
      report({ kind: "account", account });
      return account;
    }
    function requireAccount(owner) {
      if (!account?.authenticated || !account.id) throw new Error("Sign in to Agent Bounties in this browser, then refresh the account.");
      if (!account.enabled) throw new Error("Private draft storage is not enabled for this account. No test record was written.");
      if (owner && owner !== account.id) throw new Error("This canary belongs to a different account. Its record has been preserved; sign back in to read it.");
    }
    async function hash(draft) {
      const bytes = await win.crypto.subtle.digest("SHA-256", new TextEncoder().encode(stable(draft)));
      return Array.from(new Uint8Array(bytes), (value) => value.toString(16).padStart(2, "0")).join("");
    }
    async function verify(data, record) {
      const draft = envelope(record.operation_id);
      if (data?.operation_id !== record.operation_id || data.revision !== 1 || data.draft_hash !== await hash(draft)
        || stable(data.draft) !== stable(draft) || data.approved_draft_hash !== null || stable(data.recovery_state) !== "{}") {
        throw new Error("The returned canary did not match the exact private, unapproved test record.");
      }
      report({ kind: "record", operation_id: data.operation_id, revision: data.revision, draft_hash: data.draft_hash });
      return data;
    }
    async function locked(action) {
      if (busy) throw new Error("A canary check is already running.");
      busy = true; report({ kind: "busy", busy: true });
      try { return await action(); } finally { busy = false; report({ kind: "busy", busy: false }); }
    }
    async function resume() {
      return locked(async () => {
        const record = saved();
        if (!record) throw new Error("There is no saved canary in this tab.");
        await refresh(); requireAccount(record.account_id);
        const data = await verify(expect(await request(`/v1/site-auth/posting-drafts/${record.operation_id}`), 200, "Read saved canary"), record);
        report({ kind: "pass", text: "Saved canary read and verified under the same account." });
        return data;
      });
    }
    async function run() {
      return locked(async () => {
        await refresh(); requireAccount();
        const record = { account_id: account.id, operation_id: win.crypto.randomUUID() };
        const draft = envelope(record.operation_id);
        // Persist the new isolated ID before sending, so a lost response can be read.
        win.sessionStorage.setItem(KEY, JSON.stringify(record));
        report({ kind: "record", operation_id: record.operation_id });
        const path = `/v1/site-auth/posting-drafts/${record.operation_id}`;
        const body = { draft, expected_revision: 0, approved_draft_hash: null, recovery_state: {} };
        await verify(expect(await request(path, { method: "POST", body }), 200, "Create canary"), record);
        report({ kind: "pass", text: "Created one private record with no bounty, approval, or recovery journal." });
        await verify(expect(await request(path), 200, "Read canary"), record);
        report({ kind: "pass", text: "Read-back matches the exact draft, revision, and SHA-256." });
        await verify(expect(await request(path, { method: "POST", body }), 200, "Replay canary"), record);
        report({ kind: "pass", text: "Exact replay retained the same revision and hash." });
        expect(await request(path, { method: "POST", body: { ...body, draft: { ...draft, goal: "Stale canary edit; must be rejected." } } }), 409, "Stale revision check");
        await verify(expect(await request(path), 200, "Verify stale edit rejected"), record);
        report({ kind: "pass", text: "Stale revision returned HTTP 409 and left the original unchanged." });
        expect(await request(path, { credentials: "omit" }), 401, "Unauthenticated read");
        report({ kind: "pass", text: "Unauthenticated read returned HTTP 401." });
        return record;
      });
    }
    return { refresh, run, resume, saved };
  }
  function mount(win) {
    const doc = win.document, byId = (id) => doc.getElementById(id);
    if (!byId("canary-run")) return;
    let busy = false, account = null;
    const status = (text, failed = false) => {
      byId("canary-status").textContent = text;
      byId("canary-status").dataset.state = failed ? "failed" : "ready";
    };
    const client = create(win, (event) => {
      if (event.kind === "account") {
        account = event.account;
        byId("canary-authenticated").textContent = account.authenticated ? "Yes" : "No";
        byId("canary-account-id").textContent = account.id || "Unavailable";
        byId("canary-enabled").textContent = account.enabled ? "Yes" : "No";
      } else if (event.kind === "busy") busy = event.busy;
      else if (event.kind === "record") {
        byId("canary-operation-id").textContent = event.operation_id;
        byId("canary-revision").textContent = event.revision || "Awaiting response";
        byId("canary-draft-hash").textContent = event.draft_hash || "Awaiting response";
      } else if (event.kind === "pass") {
        const item = doc.createElement("li"); item.textContent = `Passed: ${event.text}`; byId("canary-results").append(item);
      }
      byId("canary-run").disabled = busy || !account?.authenticated || !account?.id || !account?.enabled;
      byId("canary-resume").disabled = busy || !client.saved();
      byId("canary-refresh").disabled = busy;
      byId("canary-reload").disabled = busy;
    });
    async function action(fn, success) {
      status("Checking private draft storage…");
      try { await fn(); status(success); } catch (error) { status(error.message || "The storage check failed.", true); }
    }
    byId("canary-run").onclick = () => {
      byId("canary-results").replaceChildren();
      return action(() => client.run(), "Storage checks passed. Reload the page to verify the saved canary again.");
    };
    byId("canary-resume").onclick = () => action(() => client.resume(), "Saved canary verified. No bounty or payment action occurred.");
    byId("canary-refresh").onclick = () => action(() => client.refresh(), "Account refreshed. Storage checks require an enabled signed-in account.");
    byId("canary-reload").onclick = () => win.location.reload();
    if (client.saved()) action(() => client.resume(), "Reload check passed: the same private canary is available to this account.");
    else action(() => client.refresh(), "Account checked. Run the isolated storage checks when this account is enabled.");
  }
  return { API, KEY, envelope, stable, create, mount };
});
