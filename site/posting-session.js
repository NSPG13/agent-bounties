/* Private account drafts. Saved state never authorizes a wallet or proves payment. */
(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.AgentBountiesPostingSession = api;
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";
  const JOURNAL = "agent-bounties.posting-operation.v1";
  const APPROVAL = "agent-bounties.posting-approval.v1";
  const OWNER = "agent-bounties.posting-owner.v1";
  const SYNC = "agent-bounties.posting-sync.v1";
  const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
  const sessions = new WeakMap();
  function stable(value) {
    if (Array.isArray(value)) return `[${value.map(stable).join(",")}]`;
    if (value && typeof value === "object") return `{${Object.keys(value).sort().filter((key) => value[key] !== undefined).map((key) => `${JSON.stringify(key)}:${stable(value[key])}`).join(",")}}`;
    return JSON.stringify(value);
  }
  function validateHashInput(value) {
    let count = 0;
    const seen = new Set();
    function text(value) {
      for (let i = 0; i < value.length; i++) {
        const point = value.charCodeAt(i);
        if (point >= 0xd800 && point <= 0xdbff) {
          const next = value.charCodeAt(++i);
          if (!(next >= 0xdc00 && next <= 0xdfff)) throw new Error("Draft text contains an unpaired Unicode surrogate. Use valid Unicode text before saving.");
        } else if (point >= 0xdc00 && point <= 0xdfff) throw new Error("Draft text contains an unpaired Unicode surrogate. Use valid Unicode text before saving.");
      }
    }
    function visit(value, depth) {
      if (++count > 2000 || depth > 14) throw new Error("The saved draft is too complex; shorten its nested metadata before saving.");
      if (value === null || typeof value === "boolean") return;
      if (typeof value === "string") { text(value); return; }
      if (typeof value === "number") {
        if (!Number.isFinite(value)) throw new Error("Draft numbers must be finite. Replace non-finite values before saving.");
        if (Number.isInteger(value) && !Number.isSafeInteger(value)) throw new Error("The draft contains an unsafe integer. Preserve exact large integers as strings before saving.");
        return;
      }
      if (!value || typeof value !== "object" || ![Object.prototype, Array.prototype, null].includes(Object.getPrototypeOf(value))) throw new Error("The draft must contain JSON values only.");
      if (seen.has(value)) throw new Error("The saved draft cannot contain circular values.");
      seen.add(value);
      if (Array.isArray(value)) {
        for (let i = 0; i < value.length; i++) visit(value[i], depth + 1);
      } else for (const key of Object.keys(value)) { text(key); if (value[key] !== undefined) visit(value[key], depth + 1); }
      seen.delete(value);
    }
    visit(value, 0);
  }
  function envelope(journey) {
    const original = journey.brief || {};
    const budget = String(original.budget_usdc ?? "").trim(), decimal = /^(\d+)(?:\.(\d{0,6}))?$/.exec(budget);
    const fraction = decimal?.[2]?.replace(/0+$/, "") || "";
    const deadline = original.deadline_at || journey.draft?.delivery_deadline || null;
    const brief = journey.brief ? { goal: String(original.goal || journey.draft?.goal || journey.goal || "").trim(),
      budget_usdc: decimal ? `${BigInt(decimal[1])}${fraction ? `.${fraction}` : ""}` : budget,
      deadline_at: deadline && Number.isFinite(Date.parse(deadline)) ? new Date(deadline).toISOString() : deadline } : null;
    return { schema: "agent-bounties/posting-draft-v1", id: journey.id, role: "post", goal: journey.goal || "", preferences: journey.preferences || "",
      brief, draft: journey.draft || null, draft_stale: journey.draft_stale === true, reference_attachment: journey.reference_attachment || null };
  }
  async function digest(win, value) {
    validateHashInput(value);
    const bytes = new TextEncoder().encode(stable(value));
    if (bytes.byteLength > 65536) throw new Error("The saved draft exceeds 64KiB. Keep attachments as immutable references rather than inline data.");
    return Array.from(new Uint8Array(await win.crypto.subtle.digest("SHA-256", bytes)), (v) => v.toString(16).padStart(2, "0")).join("");
  }
  function create(win) {
    if (sessions.has(win)) return sessions.get(win);
    const flow = win.AgentBountiesWorkflow, client = flow.createClient(win);
    let authenticated = false, owner = null, revision = 0, remoteId = null, lastSaved = null, lastRecovery = null, adopting = false, conflict = false, epoch = 0;
    let status = "local", message = "Saved on this device. Sign in to continue on another device.", timer, queue = Promise.resolve(), remoteApproval = null;
    const read = (key) => { try { return JSON.parse(win.sessionStorage.getItem(key) || "null"); } catch (_) { return null; } };
    const notify = () => {
      const detail = snapshot();
      const output = win.document?.querySelector("[data-draft-sync-status]");
      if (output) output.textContent = message;
      const link = win.document?.querySelector("[data-continue-posting]");
      if (link) { link.hidden = status !== "saved"; if (detail.continuation_url) link.href = detail.continuation_url; }
      win.dispatchEvent?.(new win.CustomEvent("agent-bounties:posting-state", { detail }));
    };
    function snapshot() {
      return { status, message, operation_id: remoteId || client.load()?.id || null, revision, conflict,
        continuation_url: authenticated && remoteId && revision ? new URL(`/post.html?operation_id=${remoteId}#bounty-preview`, win.location.origin).href : null };
    }
    const conflictError = (message) => Object.assign(new Error(message), { status: 409 });
    const staleError = () => Object.assign(new Error("The active account changed. The previous account's draft was preserved."), { stale: true });
    function checkEpoch(expected) { if (expected !== epoch) throw staleError(); }
    function rememberSync(hash) {
      win.sessionStorage.setItem(OWNER, JSON.stringify({ owner, operation_id: remoteId }));
      win.sessionStorage.setItem(SYNC, JSON.stringify({ owner, operation_id: remoteId, revision, hash }));
    }
    function uncertain(value) { return Boolean(value && (value.authorizationIssued || value.transactions?.length || ["signing", "sending", "authorized", "pending", "submitted", "batch_submitted", "creation_confirmed", "funding_confirmed"].includes(value.phase))); }
    function journal(value) {
      if (!value || typeof value !== "object") return null;
      const clean = { ...value }; delete clean.display_context;
      return Object.keys(clean).length ? clean : null;
    }
    function recoveryEnvelope(journey) {
      const value = { ...(journal(read(JOURNAL)) || {}) }, timezone = journey?.brief?.timezone;
      if (typeof timezone === "string" && timezone.length <= 100 && !/[\u0000-\u001f]/.test(timezone)) value.display_context = { timezone };
      return value;
    }
    function mergeRecovery(local, remote, sameOperation) {
      local = journal(local); remote = journal(remote);
      if (!local) return remote;
      if (!sameOperation) {
        if (uncertain(local)) throw conflictError("A wallet request is still recorded for the local draft. Reconcile it before switching posting operations.");
        return remote;
      }
      if (!remote) return local;
      if (local.bounty_id !== remote.bounty_id || String(local.bounty_contract).toLowerCase() !== String(remote.bounty_contract).toLowerCase()) throw conflictError("The devices recorded different bounty contracts. Keep both records and reconcile before continuing.");
      if (stable(local) === stable(remote)) return local;
      if (uncertain(local)) {
        if (uncertain(remote)) throw conflictError("The devices recorded different wallet progress. Reconcile the existing transaction before continuing.");
        return local;
      }
      return remote;
    }
    function archiveAccount(previousOwner) {
      const key = `agent-bounties.account-draft.v1:${encodeURIComponent(previousOwner)}`;
      win.sessionStorage.setItem(key, JSON.stringify({ journey: client.load(), recovery: read(JOURNAL), approval: read(APPROVAL), sync: read(SYNC) }));
    }
    function changeAccount(nextOwner) {
      const binding = read(OWNER), previousOwner = owner || binding?.owner;
      if (previousOwner === nextOwner) return;
      if (previousOwner) archiveAccount(previousOwner);
      const archived = nextOwner ? read(`agent-bounties.account-draft.v1:${encodeURIComponent(nextOwner)}`) : null;
      if (!previousOwner && (!archived?.journey || client.load()?.draft || client.load()?.goal)) return;
      adopting = true;
      try {
        const fresh = { schema: "agent-bounties/guided-journey-v1", id: win.crypto.randomUUID(), role: "post", goal: "", steps: {} };
        client.save(archived?.journey || fresh);
        for (const [key, value] of [[JOURNAL, archived?.recovery], [APPROVAL, archived?.approval], [SYNC, archived?.sync]]) {
          if (value) win.sessionStorage.setItem(key, JSON.stringify(value)); else win.sessionStorage.removeItem(key);
        }
        if (archived?.journey) win.sessionStorage.setItem(OWNER, JSON.stringify({ owner: nextOwner, operation_id: archived.journey.id }));
        else win.sessionStorage.removeItem(OWNER);
        win.dispatchEvent?.(new win.CustomEvent("agent-bounties:posting-restored", { detail: client.load() }));
      } finally { adopting = false; }
    }
    async function request(method, id, body, expectedEpoch = epoch) {
      checkEpoch(expectedEpoch);
      const response = await win.fetch(`${flow.apiBase(win.location)}/v1/site-auth/posting-drafts/${id}`, {
        method, credentials: "include", cache: "no-store", referrerPolicy: "no-referrer",
        headers: { Accept: "application/json", ...(body ? { "content-type": "application/json" } : {}) },
        ...(body ? { body: JSON.stringify(body) } : {}),
      });
      const value = await response.json().catch(() => ({}));
      checkEpoch(expectedEpoch);
      if (!response.ok) throw Object.assign(new Error(value.message || value.error || `Draft service unavailable (${response.status}).`), { status: response.status });
      return value;
    }
    function localApproval() { const value = read(APPROVAL); return value?.owner === owner ? value : null; }
    async function approved() {
      const expectedEpoch = epoch;
      const journey = client.load(), approval = localApproval();
      if (!journey?.draft || journey.draft_stale || !approval || approval.operation_id !== journey.id) return false;
      const terms = stable(envelope(journey));
      const matches = approval.hash === await digest(win, envelope(journey));
      return expectedEpoch === epoch && matches && terms === stable(envelope(client.load()));
    }
    async function approve() {
      const journey = client.load();
      if (!authenticated || !journey?.draft || journey.draft_stale) throw new Error("Sign in and review the complete current draft first.");
      if (conflict) throw conflictError("Reload and review the saved draft before approving.");
      const expectedEpoch = epoch, terms = stable(envelope(journey));
      const hash = await digest(win, envelope(journey));
      checkEpoch(expectedEpoch);
      if (terms !== stable(envelope(client.load()))) throw new Error("The draft changed during approval. Review the updated terms.");
      win.sessionStorage.setItem(APPROVAL, JSON.stringify({ owner, operation_id: journey.id, hash, approved_at: new Date().toISOString() }));
      await flush();
      checkEpoch(expectedEpoch);
      if (!await approved()) throw new Error("The draft changed during approval. Review the updated terms.");
      return hash;
    }
    function invalidate() {
      win.sessionStorage.removeItem(APPROVAL); remoteApproval = null;
      schedule();
    }
    async function saveRemote(required) {
      const expectedEpoch = epoch;
      const journey = client.load();
      if (!authenticated || !journey || journey.role !== "post") {
        if (required) throw new Error("Sign in to save this posting operation before a wallet request.");
        return snapshot();
      }
      if (conflict) throw new Error("This draft changed on another device. Reload its saved version before continuing.");
      if (!UUID.test(journey.id)) throw new Error("The posting operation has no valid identifier.");
      if (remoteId && remoteId !== journey.id) throw new Error("The saved posting operation does not match this draft.");
      remoteId = journey.id;
      const draft = envelope(journey), recovery = recoveryEnvelope(journey), hash = await digest(win, draft), approval = localApproval();
      checkEpoch(expectedEpoch);
      if (stable(envelope(client.load())) !== stable(draft) || stable(recoveryEnvelope(client.load())) !== stable(recovery)) throw new Error("Posting progress changed while preparing the save. Save this same operation again before continuing.");
      const value = await request("POST", remoteId, { draft, expected_revision: revision,
        approved_draft_hash: approval?.operation_id === remoteId && approval.hash === hash ? hash : null,
        recovery_state: recovery }, expectedEpoch);
      if (value.operation_id !== remoteId || value.draft_hash !== hash || stable(value.draft) !== stable(draft)) throw new Error("The saved draft response did not match this operation.");
      revision = value.revision; lastSaved = stable(draft); remoteApproval = value.approved_draft_hash;
      lastRecovery = journal(value.recovery_state);
      rememberSync(hash);
      if (required && (stable(envelope(client.load())) !== lastSaved || stable(recoveryEnvelope(client.load())) !== stable(recovery))) throw new Error("Posting progress changed while saving. Save this same operation again before a wallet request.");
      status = "saved"; message = "Saved to your account. Continue on another device with the same account.";
      notify(); return value;
    }
    function failure(error) {
      if (error.stale) return;
      if (error.status === 409) { conflict = true; status = "conflict"; message = "This draft changed on another device. Reload the saved version before continuing; no wallet action was sent."; }
      else { status = "local"; message = "Saved on this device only. Account sync is unavailable; keep this tab open and retry."; }
      notify();
    }
    function flush(options = {}) {
      win.clearTimeout(timer);
      const expectedEpoch = epoch;
      queue = queue.catch(() => {}).then(() => { checkEpoch(expectedEpoch); return saveRemote(options.requireServer === true); });
      return queue.catch((error) => { failure(error); throw error; });
    }
    function schedule(event) {
      if (adopting) return;
      const journey = event?.detail;
      if (authenticated && remoteId && journey?.id && journey.id !== remoteId && !journal(read(JOURNAL))) {
        const archived = read(`${JOURNAL}.previous`);
        if (archived?.phase === "funding_confirmed") {
          void beginAfterArchive(archived, { preserveDraft: false }, journey).catch(failure);
          return;
        }
      }
      win.clearTimeout(timer);
      if (authenticated) timer = win.setTimeout(() => { void flush().catch(() => {}); }, 500);
      notify();
    }
    async function adopt(value) {
      const expectedEpoch = epoch;
      const initialJourney = stable(client.load()), initialRecovery = stable(read(JOURNAL));
      if (!value?.draft || value.draft.schema !== "agent-bounties/posting-draft-v1" || value.draft.id !== value.operation_id) throw new Error("The saved draft has an unsupported format.");
      if (await digest(win, value.draft) !== value.draft_hash) throw new Error("The saved draft failed its integrity check.");
      checkEpoch(expectedEpoch);
      if (initialJourney !== stable(client.load()) || initialRecovery !== stable(read(JOURNAL))) throw conflictError("This device changed while verifying the saved draft. Your local changes were preserved.");
      const old = client.load();
      const recovery = mergeRecovery(read(JOURNAL), value.recovery_state, old?.id === value.operation_id);
      adopting = true;
      try {
        if (old && old.id !== value.operation_id) win.sessionStorage.setItem("agent-bounties.previous-local-journey.v1", JSON.stringify(old));
        remoteId = value.operation_id; revision = value.revision; lastSaved = stable(value.draft); remoteApproval = value.approved_draft_hash;
        lastRecovery = journal(value.recovery_state);
        const journey = { ...value.draft, schema: "agent-bounties/guided-journey-v1", steps: old?.id === remoteId ? old.steps || {} : {}, updated_at: value.updated_at };
        const timezone = value.recovery_state?.display_context?.timezone;
        if (typeof timezone === "string" && timezone.length <= 100 && !/[\u0000-\u001f]/.test(timezone)) journey.brief = { ...journey.brief, timezone };
        if (recovery) win.sessionStorage.setItem(JOURNAL, JSON.stringify(recovery));
        else win.sessionStorage.removeItem(JOURNAL);
        client.save(journey);
        if (remoteApproval === value.draft_hash && !journey.draft_stale) win.sessionStorage.setItem(APPROVAL, JSON.stringify({ owner, operation_id: remoteId, hash: remoteApproval }));
        else win.sessionStorage.removeItem(APPROVAL);
        rememberSync(value.draft_hash);
        conflict = false; status = "saved"; message = "Saved draft restored. Your unchanged terms approval is preserved; wallet confirmations remain yours.";
        win.dispatchEvent?.(new win.CustomEvent("agent-bounties:posting-restored", { detail: journey }));
      } finally { adopting = false; notify(); }
      return value;
    }
    async function hydrate(account) {
      const nextOwner = account?.authenticated ? String(account.user?.id || "") : null;
      if (owner !== nextOwner) { epoch++; win.clearTimeout(timer); changeAccount(nextOwner); revision = 0; remoteId = null; lastSaved = null; lastRecovery = null; conflict = false; }
      owner = nextOwner; authenticated = Boolean(nextOwner);
      if (!authenticated) { status = "local"; notify(); return null; }
      const requested = new URL(win.location.href).searchParams.get("operation_id");
      if (requested && !UUID.test(requested)) { conflict = true; status = "unavailable"; message = "This continuation link has an invalid posting identifier."; notify(); return null; }
      const journey = client.load();
      const id = requested && UUID.test(requested) ? requested : journey?.id;
      if (!id) { status = "local"; notify(); return null; }
      if (remoteId === id && revision) return snapshot();
      const expectedEpoch = epoch, before = stable(envelope(journey || { id })), localJournal = stable(read(JOURNAL));
      try {
        const value = await request("GET", id, undefined, expectedEpoch);
        if (before !== stable(envelope(client.load() || { id })) || localJournal !== stable(read(JOURNAL))) throw conflictError("This device changed while the saved draft was loading. Your local changes were preserved.");
        const sync = read(SYNC);
        if (journey?.id === id && journey.draft && sync?.owner === owner && sync.operation_id === id) {
          const localHash = await digest(win, envelope(journey)); checkEpoch(expectedEpoch);
          if (localHash !== value.draft_hash && localHash !== sync.hash) {
            if (sync.hash !== value.draft_hash) throw conflictError("Both devices edited this draft. Your local draft is preserved; review the saved version before continuing.");
            remoteId = id; revision = value.revision; lastSaved = stable(value.draft);
            return flush();
          }
        }
        return await adopt(value);
      }
      catch (error) {
        if (error.stale) return null;
        if (error.status === 409) { failure(error); return null; }
        if (error.status === 404 && (!requested || (journey?.id === requested && journey.draft))) { remoteId = id; revision = 0; return flush(); }
        status = "unavailable"; message = error.status === 404 ? "This draft is unavailable to this account or has expired. Your other local draft remains saved." : "Could not restore your account draft. Keep this tab open and retry.";
        conflict = Boolean(requested); notify(); return null;
      }
    }
    async function refresh() {
      if (!authenticated || !remoteId || !revision || conflict || win.document?.hidden) return;
      const current = client.load();
      if (!current || stable(envelope(current)) !== lastSaved) return;
      const expectedEpoch = epoch, before = stable(envelope(current)), localJournal = stable(read(JOURNAL)), startRevision = revision;
      try {
        const value = await request("GET", remoteId, undefined, expectedEpoch);
        if (value.revision <= revision) return;
        if (revision !== startRevision || before !== stable(envelope(client.load())) || localJournal !== stable(read(JOURNAL))) throw conflictError("This device changed while account progress was loading. Your local changes were preserved.");
        await adopt(value);
      } catch (error) { failure(error); if (!error.stale) throw error; }
    }
    async function reconcile() {
      const operation = journal(read(JOURNAL));
      if (!operation) return { creation_confirmed: false, funding_confirmed: false, claimable: false, public_inventory_verified: false };
      if (!flow.ADDRESS.test(operation.bounty_contract || "") || !/^0x[0-9a-f]{64}$/i.test(operation.bounty_id || "")) throw new Error("The saved wallet operation is incomplete. Preserve it for recovery; do not create another transaction.");
      const expectedEpoch = epoch, recorded = stable(operation);
      const [events, inventory] = await Promise.all([
        client.request(`/v1/base/autonomous-bounties/events?network=base-mainnet&bounty_id=${encodeURIComponent(operation.bounty_id)}`), client.inventory(),
      ]);
      if (!Array.isArray(events)) throw new Error("Canonical event evidence is unavailable.");
      checkEpoch(expectedEpoch);
      if (recorded !== stable(journal(read(JOURNAL)))) throw new Error("Wallet progress changed while checking evidence. Recheck this same operation.");
      const matched = events.filter((event) => event.bounty_id?.toLowerCase() === operation.bounty_id.toLowerCase()
        && /^0x[0-9a-f]{64}$/i.test(event.tx_hash || "") && Number.isSafeInteger(event.block_number) && event.block_number >= 0
        && (!event.network || event.network === "base-mainnet")
        && (event.kind === "canonical_bounty_created" ? event.data?.bounty_contract?.toLowerCase() === operation.bounty_contract.toLowerCase()
          : event.contract_address?.toLowerCase() === operation.bounty_contract.toLowerCase()));
      const created = matched.find((event) => event.kind === "canonical_bounty_created"
        && flow.ADDRESS.test(event.contract_address || "") && /^0x[0-9a-f]{64}$/i.test(event.data?.terms_hash || "")
        && (!operation.terms_hash || event.data.terms_hash.toLowerCase() === operation.terms_hash.toLowerCase()));
      const amount = (value) => (typeof value === "string" && /^\d+$/.test(value) || typeof value === "number" && Number.isSafeInteger(value) && value >= 0) ? BigInt(value) : null;
      const funded = matched.find((event) => event.kind === "funding_added" && amount(event.data?.target_amount) > 0n && amount(event.data?.funded_amount) >= amount(event.data?.target_amount));
      const claimable = funded && matched.some((event) => event.kind === "bounty_became_claimable" && amount(event.data?.funded_amount) >= amount(funded.data.target_amount));
      const item = inventory.items.find((entry) => entry.source_id?.toLowerCase() === operation.bounty_contract?.toLowerCase());
      const verified = Boolean(item && flow.ready(item) && created && claimable && /^0x[0-9a-f]{64}$/i.test(item.terms_hash || "") && item.terms_hash.toLowerCase() === created.data.terms_hash.toLowerCase()
        && funded && flow.units(item.funding_target) === amount(funded.data.target_amount));
      const result = { operation_id: remoteId, bounty_contract: operation.bounty_contract,
        creation_confirmed: Boolean(created), funding_confirmed: Boolean(created && funded),
        claimable: Boolean(created && claimable && verified), public_inventory_verified: verified,
        public_url: verified ? new URL(flow.detailUrl(item), win.location.href).href : null, paid: false };
      if (result.creation_confirmed && result.funding_confirmed && result.claimable && result.public_inventory_verified && operation.phase !== "funding_confirmed") flow.createPostingJournal(win).checkpoint("funding_confirmed");
      win.dispatchEvent?.(new win.CustomEvent("agent-bounties:posting-canonical", { detail: result }));
      return result;
    }
    async function beginAfterArchive(archived, options = {}, existingJourney = null) {
      if (!authenticated) throw new Error("Sign in before starting the next posting operation.");
      if (journal(read(JOURNAL))) throw new Error("A wallet operation is still recorded. Reconcile it before starting another.");
      const rejected = ["batch_params_rejected", "batch_wallet_rejected"].includes(archived?.phase);
      // A cached/local rejection cannot retire a shared operation atomically.
      // Another device may have advanced it. Keep that record until server-side
      // retirement and reconciliation are available; never fork a retry here.
      if (rejected && (revision > 0 || lastRecovery)) throw new Error("The saved account attempt requires reconciliation. Starting a replacement across devices is unavailable; its transaction record is preserved.");
      const history = read(`${JOURNAL}.rejected`);
      const exact = rejected ? Array.isArray(history) && stable(history.at(-1)) === stable(archived)
        : archived?.phase === "funding_confirmed" && stable(read(`${JOURNAL}.previous`)) === stable(archived);
      if (!exact || !flow.ADDRESS.test(archived?.bounty_contract || "") || !/^0x[0-9a-f]{64}$/i.test(archived?.bounty_id || "")) throw new Error("Only the exact archived rejection or canonically confirmed posting can start a new operation.");
      if (rejected && (archived.authorizationIssued || archived.transactions?.length)) throw new Error("An authorized or broadcast request cannot be restarted as a rejected batch.");
      if (lastRecovery && (lastRecovery.bounty_id !== archived.bounty_id || String(lastRecovery.bounty_contract).toLowerCase() !== archived.bounty_contract.toLowerCase()
        || rejected && (lastRecovery.authorizationIssued || lastRecovery.transactions?.length))) throw new Error("The saved account operation does not match this rejected or completed attempt. Reconcile it first.");
      const current = client.load();
      if (!current) throw new Error("The original draft is unavailable.");
      const id = existingJourney?.id || win.crypto.randomUUID();
      if (!UUID.test(id) || id === remoteId || (!existingJourney && id === current.id)) throw new Error("The next posting operation needs a new identifier.");
      const preserve = options.preserveDraft === true;
      const next = existingJourney || { schema: "agent-bounties/guided-journey-v1", id, role: "post", steps: {},
        goal: preserve ? current.goal || "" : "", preferences: preserve ? current.preferences || "" : "",
        ...(preserve ? { brief: current.brief || null, draft: current.draft ? { ...current.draft, posting_operation_id: id } : null,
          reference_attachment: current.reference_attachment || null, draft_stale: current.draft_stale === true } : {}), updated_at: new Date().toISOString() };
      win.sessionStorage.setItem("agent-bounties.previous-local-journey.v1", JSON.stringify(current));
      epoch++; win.clearTimeout(timer); revision = 0; remoteId = id; lastSaved = null; lastRecovery = null; remoteApproval = null; conflict = false;
      win.sessionStorage.removeItem(APPROVAL); win.sessionStorage.removeItem(SYNC);
      win.sessionStorage.setItem(OWNER, JSON.stringify({ owner, operation_id: id }));
      adopting = true;
      try {
        client.save(next);
        const url = new URL(win.location.href);
        if (!preserve) for (const key of Array.from(url.searchParams.keys())) if (!["analytics", "from"].includes(key) && !key.startsWith("utm_")) url.searchParams.delete(key);
        url.searchParams.set("operation_id", id);
        win.history?.replaceState(null, "", url.href);
        win.dispatchEvent?.(new win.CustomEvent("agent-bounties:posting-restored", { detail: next }));
      } finally { adopting = false; }
      await flush({ requireServer: true });
      return next;
    }
    win.addEventListener?.("agent-bounties:journey", schedule);
    win.addEventListener?.("agent-bounties:posting-journal", schedule);
    const api = { snapshot, approved, approve, invalidate, hydrate, refresh, flush, reconcile, beginAfterArchive, reload: async () => { try { return await adopt(await request("GET", remoteId)); } catch (error) { failure(error); throw error; } } };
    sessions.set(win, api);
    return api;
  }
  return { stable, envelope, validateHashInput, digest, create };
});
