/* Presentation only: no purchase, signing, or provider-order completion. */
(() => {
  "use strict";
  const $ = selector => document.querySelector(selector);
  let snapshot = null, view = "method", previous = "", lastAccount = null;
  function text(selector, value) { const node = $(selector); if (node && node.textContent !== value) node.textContent = value; }
  function feedback(message, tone) { const output = $("[data-topup-feedback]"); output.textContent = message; output.hidden = !message; output.dataset.tone = tone; }
  function render(s = snapshot) {
    if (!s) return;
    snapshot = s;
    if (s.account !== lastAccount) { view = "method"; lastAccount = s.account; }
    const units = value => window.AgentBountiesFundingReadiness.formatUnits(value);
    const short = s.usdc === null ? null : s.required > s.usdc ? s.required - s.usdc : 0n;
    const ready = s.fresh && short === 0n && (s.existingBounty || s.eth > 0n);
    // Wallet liquidity and provider-order completion are different facts.
    // Keep the uncertain order recorded, but do not trap an adequately funded wallet.
    const active = !s.account ? "wallet" : ready ? "ready" : s.pending ? "pending" : view;
    const buying = $("[data-onramp-asset]").value === "eth" ? "ETH" : "USDC";
    const copy = {
      wallet: ["Which wallet will receive the money?", "Use the same wallet as your bounty."],
      method: ["Add money to your wallet", "Choose how you want to add it."],
      "wallet-buy": ["Open your wallet app", `Buy ${buying} on Base.`],
      card: ["Buy with a card", "Choose where to buy."],
      pending: ["Waiting for money", "Already paid? Keep this page open."],
      ready: ["Your wallet has enough USDC", "Continue to review your bounty payment."],
    }[active];
    $(".topup-guide").dataset.view = active;
    $(".topup-guide").setAttribute("aria-busy", String(s.busy));
    text("#onramp-title", copy[0]); text("[data-topup-instruction]", copy[1]);
    for (const panel of document.querySelectorAll("[data-topup-panel]")) panel.hidden = panel.dataset.topupPanel !== active;
    const need = short === null || !s.fresh ? "Checking your Base balance…" : short > 0n ? `${units(short)} USDC still needed` : "ETH needed for the network fee";
    text("[data-topup-needed]", s.error ? "Balance unavailable" : need);
    text("[data-topup-wallet-detail]", buying === "USDC" ? need : "ETH on Base · for the network fee");
    text("[data-topup-card-asset]", `${buying} on Base`);
    $("[data-topup-wallet]").hidden = !s.account;
    text("[data-topup-address]", s.account || "");
    const connected = s.connection?.connected && s.connection.address?.toLowerCase() === s.account;
    const base = s.connection?.chain_id === "0x2105";
    text("[data-topup-connection]", connected ? base ? "● Wallet connected · Base" : "● Wallet connected · switch to Base for payment" : s.connection?.connected ? "Different wallet connected · saved address below" : "Wallet address saved · not connected");
    $("[data-topup-connection]").dataset.connected = String(Boolean(connected));
    text("[data-topup-balance]", s.usdc !== null && s.fresh ? `${units(s.usdc)} USDC on Base` : "");
    text("[data-topup-ready-amount]", ready ? `${units(s.usdc)} USDC available ✓` : "");
    $("[data-topup-order-note]").hidden = !s.pending;
    const watch = !s.account ? "" : s.error ? "Balance check failed. Trying again…" : s.busy ? "Checking your balance…" : s.received > 0n ? `${units(s.received)} USDC added since the first check.` : "Watching for money · checks every 5 seconds";
    text("[data-topup-watch]", watch);
    const check = $("[data-refresh-balance]"); check.hidden = !s.account; check.disabled = s.busy;
    check.textContent = s.busy ? "Checking…" : "Check my balance";
    $("[data-topup-back]").hidden = !["wallet-buy", "card"].includes(active);
    for (const selector of ["[data-topup-wallet-buy]", "[data-topup-card-buy]"]) $(selector).disabled = !s.fresh;
    if (active !== previous) { previous = active; $("#onramp-title").focus({ preventScroll: true }); }
  }
  function show(method) {
    if (!["method", "wallet", "card"].includes(method)) throw new Error("Choose wallet or card.");
    if (snapshot && snapshot.fresh && snapshot.usdc >= snapshot.required && snapshot.eth === 0n && !snapshot.existingBounty) {
      $("[data-onramp-asset]").value = "eth";
      $("[data-onramp-asset]").dispatchEvent(new Event("change"));
    }
    view = method === "wallet" ? "wallet-buy" : method;
    feedback("", ""); render();
    // User navigation starts the new screen at its wallet/heading, even when
    // the previous choice was below the fold. Background reads never scroll.
    const header = document.querySelector("[data-site-header]")?.getBoundingClientRect().bottom || 0;
    const top = $(".topup-guide").getBoundingClientRect().top + window.scrollY - header - 12;
    window.scrollTo({ top: Math.max(0, top), behavior: "instant" });
    return { view: $(".topup-guide").dataset.view, purchase_opened: false };
  }
  $("[data-topup-wallet-buy]").addEventListener("click", () => show("wallet"));
  $("[data-topup-card-buy]").addEventListener("click", () => show("card"));
  $("[data-topup-back]").addEventListener("click", () => show("method"));
  $("[data-topup-copy]").addEventListener("click", async () => {
    try { await navigator.clipboard.writeText(snapshot.account); feedback("Wallet address copied.", "success"); }
    catch (_) { feedback("Copy the wallet address shown above.", "error"); }
  });
  window.AgentBountiesTopupGuide = Object.freeze({ render, feedback, show });
})();
