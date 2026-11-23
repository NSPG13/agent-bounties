(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root?.document) api.start(root, root.document);
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";
  const ADDRESS = /^0x[0-9a-f]{40}$/i, HASH = /^0x[0-9a-f]{64}$/i;
  const lower = value => String(value || "").toLowerCase();
  const units = value => /^\d+$/.test(String(value)) && (typeof value !== "number" || Number.isSafeInteger(value)) ? BigInt(value) : null;
  function confirmedItem(items, contract) {
    if (!ADDRESS.test(contract) || !Array.isArray(items)) return null;
    const item = items.find(entry => lower(entry.bounty_contract) === lower(contract));
    const target = units(item?.target_amount), funded = units(item?.funded_amount);
    if (!item || !HASH.test(item.bounty_id) || item.terms_valid !== true || item.validation_errors?.length
      || !["claimable", "claimed", "submitted", "paid"].includes(item.status)
      || target === null || target <= 0n || funded === null || funded < target) return null;
    const events = (item.events || []).filter(event => event.bounty_id === item.bounty_id && event.id && HASH.test(event.tx_hash)
      && Number.isSafeInteger(event.block_number) && event.block_number > 0 && Number.isSafeInteger(event.log_index) && event.log_index >= 0);
    const created = events.find(event => event.kind === "canonical_bounty_created" && lower(event.data?.bounty_contract) === lower(contract));
    const funding = events.find(event => event.kind === "funding_added" && lower(event.contract_address) === lower(contract)
      && units(event.data?.funded_amount) !== null && units(event.data.funded_amount) >= target);
    const claimable = events.find(event => event.kind === "bounty_became_claimable" && lower(event.contract_address) === lower(contract));
    return created && funding && claimable ? { item, funding } : null;
  }
  // The public bounty page is the link to share; it is built only from a validated address.
  function shareUrl(origin, contract) {
    if (!ADDRESS.test(contract)) return "";
    return new URL(`participate.html?bountyContract=${lower(contract)}&network=base-mainnet`, origin).href;
  }
  function formatAmount(value) {
    const n = units(value);
    if (n === null) return "—";
    const fraction = String(n % 1000000n).padStart(6, "0").replace(/0+$/, "").padEnd(2, "0");
    return `${n / 1000000n}.${fraction} USDC`;
  }
  function start(win, doc) {
    const get = selector => doc.querySelector(selector), flow = win.AgentBountiesWorkflow;
    const client = flow.createClient(win), params = new URLSearchParams(win.location.search);
    const contract = lower(params.get("bountyContract")), network = params.get("network") || flow.NETWORK;
    let busy = false;
    // The share link exists only once funding is confirmed; the page stays put so it can be copied.
    const shareStatus = (text) => { get("[data-funded-share-status]").textContent = text; };
    get("[data-funded-share-copy]").addEventListener("click", async () => {
      const input = get("[data-funded-share-url]");
      if (!input.value) return;
      try { await win.navigator.clipboard.writeText(input.value); shareStatus("Link copied."); }
      catch (_) { input.focus(); input.select(); shareStatus("Select the link above and copy it."); }
    });
    get("[data-funded-share-native]").addEventListener("click", async () => {
      const url = get("[data-funded-share-url]").value;
      if (!url) return;
      try { await win.navigator.share({ title: doc.title, url }); } catch (_) { /* Closing the share sheet is not an error. */ }
    });
    async function check() {
      if (busy) return;
      busy = true; get("[data-funded-recheck]").disabled = true;
      try {
        if (!ADDRESS.test(contract) || network !== flow.NETWORK) throw new Error("Open this page from your bounty’s funding review.");
        const workspace = get("[data-funded-workspace]");
        workspace.href = `participate.html?bountyContract=${contract}&network=base-mainnet`; workspace.hidden = false;
        const items = await client.request("/v1/base/autonomous-bounties/feed?network=base-mainnet&claimable_only=false");
        const evidence = confirmedItem(items, contract);
        if (!evidence) {
          get("[data-funded-heading]").textContent = "Funding is still being checked.";
          get("[data-funded-status]").textContent = "Confirmed funding isn’t available yet. Check again or view the bounty’s progress. Don’t fund it a second time.";
          return;
        }
        const { item, funding } = evidence;
        get("[data-funded-symbol]").textContent = "✓";
        get("[data-funded-heading]").textContent = "Bounty funded.";
        get("[data-funded-status]").textContent = item.status === "claimable" && item.verification_ready === true
          ? "Your bounty is ready for someone to take on. Share its link below so the right person finds it."
          : item.status === "claimable" ? `Funding is confirmed, but the bounty is not ready for work. ${item.verification_readiness_reason || "The committed verifier is unavailable."} View its progress for the next step.`
          : "Funding is confirmed. Open the bounty to see its current progress.";
        get("[data-funded-title]").textContent = item.terms?.document?.title || "Funded bounty";
        get("[data-funded-amount]").textContent = formatAmount(item.funded_amount);
        get("[data-funded-split]").textContent = `${formatAmount(item.solver_reward)} solver reward · ${formatAmount(item.verifier_reward)} for review`;
        get("[data-funded-transaction]").href = `https://basescan.org/tx/${funding.tx_hash}`;
        get("[data-funded-share-url]").value = shareUrl(win.location.origin, contract);
        get("[data-funded-share-native]").hidden = typeof win.navigator?.share !== "function";
        get("[data-funded-receipt]").hidden = false;
        get("[data-funded-boundary]").hidden = false;
        get("[data-funded-recheck]").hidden = true;
        const board = `earn.html?posted=${contract}`;
        get("[data-funded-board]").href = board;
        // Journals are only a recovery aid. Evidence above, never storage or a URL, proves success.
        const journal = flow.createPostingJournal(win), operation = journal.load();
        if (operation?.bounty_id === item.bounty_id && lower(operation.bounty_contract) === contract) {
          try { journal.checkpoint("funding_confirmed"); } catch (_) { /* Receipt evidence remains authoritative when session storage is unavailable. */ }
        }
        const parent = params.get("parentBounty");
        if (ADDRESS.test(parent || "")) {
          get("[data-funded-parent]").hidden = false;
          get("[data-funded-parent-link]").href = `participate.html?bountyContract=${lower(parent)}&network=base-mainnet`;
        }
        doc.title = "Bounty funded | Agentizable";
        get("[data-funded-heading]").focus({ preventScroll: true });
      } catch (error) {
        get("[data-funded-heading]").textContent = "We couldn’t check funding.";
        get("[data-funded-status]").textContent = "Your wallet step won’t be repeated. Use Check again to refresh the confirmed record, or return to the board.";
      } finally { busy = false; get("[data-funded-recheck]").disabled = false; }
    }
    get("[data-funded-recheck]").addEventListener("click", check);
    check();
  }
  return { confirmedItem, formatAmount, shareUrl, start };
});
