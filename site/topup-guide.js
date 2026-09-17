/* The guide never buys, signs, or assumes that a purchase succeeded. */
(() => {
  "use strict";
  const $ = selector => document.querySelector(selector);
  let snapshot = null, view = "method", walletStep = 0, previous = "", lastAccount = null;
  function text(selector, value) { const node = $(selector); if (node && node.textContent !== value) node.textContent = value; }
  function feedback(message, tone) { const output = $("[data-topup-feedback]"); output.textContent = message; output.hidden = !message; output.dataset.tone = tone; }
  function asset() { return $("[data-onramp-asset]").value === "eth" ? "ETH" : "USDC"; }
  function render(s = snapshot) {
    if (!s) return;
    snapshot = s;
    if (s.account !== lastAccount) { view = "method"; walletStep = 0; lastAccount = s.account; }
    const units = value => window.AgentBountiesFundingReadiness.formatUnits(value);
    const short = s.usdc === null ? null : s.required > s.usdc ? s.required - s.usdc : 0n;
    const ready = s.usdc !== null && short === 0n && (s.existingBounty || s.eth > 0n);
    const active = s.pending ? "pending" : !s.account ? "wallet" : s.busy ? "checking" : ready ? "ready" : view;
    const buying = asset();
    const steps = [
      ["Open your wallet app", "Tap Buy or Receive in the wallet you connected.", "Keep this page open. Come back when you are done."],
      ["Choose " + buying + " on Base", "Check both the coin and the network.", buying + " · Base network"],
      ["Check the amount and address", "Use this same wallet. Check any fees before you pay.", buying === "USDC" ? short === null ? "Check your balance first." : `Receive at least ${units(short)} USDC after fees.` : "Add ETH on Base for the network fee. Your wallet shows the cost."],
      ["Finish in your wallet", "Approve the purchase or transfer there. Then come back here.", "Already paid? Wait for it to arrive. Do not buy again."],
    ];
    const copy = active === "wallet-buy" ? steps[walletStep] : {
      wallet: ["Which wallet will receive the money?", "Use the same wallet as your bounty."],
      checking: ["Checking your balance…", "This may take a few seconds."],
      method: ["Add money to your wallet", "Choose how you want to add it."],
      card: ["Choose an amount to buy", "You will review the final price with MoonPay."],
      checkout: ["Check before opening MoonPay", "Send the money to your own wallet on Base."],
      pending: ["Check your existing purchase", "Do not pay again while it is pending."],
      ready: ["Your wallet has the money", "Go back to review your bounty payment."],
    }[active];
    $(".topup-guide").dataset.view = active;
    $(".topup-guide").setAttribute("aria-busy", String(s.busy));
    text("#onramp-title", copy[0]); text("[data-topup-instruction]", copy[1]);
    text("[data-topup-step]", active === "wallet-buy" ? `Add money · Step ${walletStep + 1} of 4` : "Step 2 of 3 · Add money");
    for (const panel of document.querySelectorAll("[data-topup-panel]")) panel.hidden = panel.dataset.topupPanel !== active;
    text("[data-topup-needed]", short === null ? "Check your balance below" : short > 0n ? `${units(short)} USDC still needed` : "ETH needed for the network fee");
    text("[data-topup-wallet-detail]", active === "wallet-buy" ? copy[2] : "");
    const destination = $("[data-topup-destination]"); destination.textContent = s.account || ""; destination.hidden = walletStep !== 2;
    $("[data-topup-copy]").hidden = walletStep !== 2;
    text("[data-topup-next]", walletStep === 3 ? "I’m back — check my balance" : "Next");
    const check = $("[data-refresh-balance]"); check.hidden = ["wallet", "checkout", "card", "ready"].includes(active) || active === "wallet-buy" && walletStep < 3;
    check.disabled = s.busy || !s.account; check.textContent = s.busy ? "Checking…" : "Check my balance";
    if (active === "wallet-buy" && walletStep === 3) check.hidden = true;
    $("[data-topup-next]").disabled = s.busy;
    $("[data-topup-back]").hidden = !["wallet-buy", "card", "checkout"].includes(active);
    if (active !== previous) { previous = active; $("#onramp-title").focus({ preventScroll: true }); }
  }
  function selectGasAssetIfNeeded() {
    if (snapshot && snapshot.usdc !== null && snapshot.usdc >= snapshot.required && snapshot.eth === 0n && !snapshot.existingBounty) {
      $("[data-onramp-asset]").value = "eth";
      $("[data-onramp-asset]").dispatchEvent(new Event("change"));
    }
  }
  $("[data-topup-wallet-buy]").addEventListener("click", () => {
    selectGasAssetIfNeeded();
    view = "wallet-buy"; walletStep = 0; feedback("", ""); render(); });
  $("[data-topup-card-buy]").addEventListener("click", () => { selectGasAssetIfNeeded(); view = "card"; feedback("", ""); render(); });
  $("[data-topup-next]").addEventListener("click", () => {
    if (walletStep < 3) { walletStep++; render(); $("#onramp-title").focus({ preventScroll: true }); }
    else { view = "method"; $("[data-refresh-balance]").click(); }
  });
  $("[data-topup-card-next]").addEventListener("click", () => {
    const amount = $("[data-fiat-amount]");
    if (!amount.reportValidity()) return;
    view = "checkout"; feedback("", ""); render();
  });
  $("[data-topup-back]").addEventListener("click", () => {
    if (view === "wallet-buy" && walletStep > 0) walletStep--;
    else view = view === "checkout" ? "card" : "method";
    feedback("", ""); render(); $("#onramp-title").focus({ preventScroll: true });
  });
  $("[data-topup-copy]").addEventListener("click", async () => {
    try { await navigator.clipboard.writeText(snapshot.account); feedback("Wallet address copied.", "success"); }
    catch (_) { feedback("Copy the wallet address shown above.", "error"); }
  });
  window.AgentBountiesTopupGuide = Object.freeze({ render, feedback });
})();
