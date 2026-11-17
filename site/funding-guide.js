/* Presentation only. Existing posting code owns consent, requests and receipts. */
(() => {
  "use strict";
  const $ = selector => document.querySelector(selector);
  const show = (selector, visible) => { const node = $(selector); if (node) node.hidden = !visible; };
  const text = (selector, value) => { const node = $(selector); if (node && node.textContent !== value) node.textContent = value; };
  let previous = "";
  function render(s) {
    const dialog = $("#funding-dialog");
    if (!dialog?.open) return;
    const enough = s.balances && s.balances.usdc >= s.balances.required && (s.gasSponsored === true || s.directWallet && s.balances.eth > 0n);
    const done = s.canonical?.creation_confirmed && s.canonical?.funding_confirmed && s.canonical?.claimable && s.canonical?.public_inventory_verified;
    const step = done ? "done" : s.busy || (s.recorded && !s.continuable && !s.resumable) || s.conflict ? "wait" : s.connecting ? "connecting" : !s.connected ? "connect" : s.continuable ? "review" : !s.balances ? "check" : !enough ? "topup" : "review";
    const short = s.balances ? window.AgentBountiesFundingReadiness.formatUnits(s.balances.required > s.balances.usdc ? s.balances.required - s.balances.usdc : 0n) : "";
    const needsUsdc = s.balances && s.balances.usdc < s.balances.required;
    const copy = {
      connect: ["1", "Connect", "Choose your wallet", "Use the wallet you want to pay from."],
      connecting: ["1", "Connect", "Check your wallet", "Approve the connection. No money will move."],
      check: ["2", "Add money", s.tone === "error" ? "Try the balance check again" : "Checking your balance…", s.tone === "error" ? "We could not read your wallet. Your bounty is saved." : "This may take a few seconds."],
      topup: ["2", "Add money", needsUsdc ? "Add money to your wallet" : (s.directWallet ? "Base ETH needed for network fees" : "Waiting for gas sponsorship"), needsUsdc ? "USDC is the digital money used to pay for work." : (s.directWallet ? "Your wallet pays Base ETH for approval and creation. Check its network-fee balance." : "Agentizable pays the network fee. Recheck when sponsorship is available.")],
      review: s.continuable ? ["3", "Continue", "Resume your saved request", "Check the same payment's status. The saved V1 authorization will not be replayed. Do not pay again."]
        : s.resumable ? ["3", "Finish", "Finish creating your bounty", "No USDC has moved yet. Confirm the same bounty creation in your wallet."]
        : ["3", "Review", "Review before you pay", "Check the cost. Then confirm in your wallet."],
      wait: ["3", "Confirm", s.busy ? "Follow the steps in your wallet" : "Checking your saved payment", s.busy ? "Read each request before you approve it." : "Do not pay again. We are checking the same request."],
      done: ["3", "Done", "Your bounty is ready", "People can now claim this bounty."],
    }[step];
    dialog.dataset.fundingView = step;
    text("[data-funding-step]", `Step ${copy[0]} of 3 · ${copy[1]}`);
    text("#funding-dialog-title", copy[2]); text("[data-funding-instruction]", copy[3]);
    show("[data-wallet-panel]", step === "connect");
    show("[data-wallet-readiness]", !["connect", "connecting"].includes(step));
    show("[data-funding-topup]", ["topup", "check"].includes(step));
    show("[data-funding-review]", step === "review");
    show("[data-funding-wait]", ["wait", "done"].includes(step));
    show("[data-change-funding-wallet]", Boolean(s.connected) && !s.busy && !s.recorded);
    show("[data-funding-help]", false);
    if (step === "topup") text("[data-funding-gas-help]", s.directWallet ? "This route requires Base USDC and Base ETH for network fees." : "Agentizable pays the network fee; you only need Base USDC.");
    // Preserve the machine-facing amount even when the extra help is collapsed.
    text("[data-funding-shortfall]", step === "topup" ? needsUsdc ? `${short} USDC needed` : s.directWallet ? "Base ETH needed for network fees" : "Gas sponsorship unavailable" : "");
    const topup = $("[data-funding-topup] [data-onramp-link]");
    topup.hidden = step !== "topup" || !needsUsdc;
    topup.textContent = "Add funds to wallet";
    const check = $("[data-recheck-balance]");
    check.hidden = !["topup", "check", "review"].includes(step);
    check.disabled = step === "check" && s.tone !== "error";
    check.textContent = check.disabled ? "Checking…" : "Check my balance";
    text("[data-funding-total]", `${s.total} USDC · ${s.gasSponsored ? "network fee paid by Agentizable" : s.directWallet ? "network fees paid from your Base ETH" : "waiting for gas sponsorship"}`);
    text("[data-funding-split]", `${s.solver} USDC for the work. ${s.verifier} USDC ${s.creator ? "set aside for your review" : "for the reviewer"}. Platform fee: ${s.fee || "0"} USDC.`);
    const consent = dialog.querySelector("[data-legal-consent-checkbox]");
    show("[data-funding-consent-hint]", !consent?.checked);
    const pay = $("[data-fund-now]");
    if (step === "review") {
      pay.disabled = Boolean(!s.approved || !consent?.checked || s.conflict || s.directWallet && !dialog.querySelector("[data-v2-cost-consent]")?.checked);
      pay.textContent = s.continuable ? "Continue funding" : s.resumable ? "Finish posting" : "Review and post";
    } else pay.disabled = true;
    const feedback = $("[data-funding-feedback]");
    feedback.hidden = !s.message || !["error", "success"].includes(s.tone) && !s.message.startsWith("Saving your bounty");
    feedback.dataset.tone = s.tone;
    // Keep exact diagnostics available without repeating the whole instruction.
    feedback.textContent = s.tone === "error" ? (step === "check" ? "We could not read your balance. Check your connection and try again." : s.message.split(/\n/)[0]) : s.message.startsWith("Saving your bounty") ? "Saving your bounty. Opening the next step…" : /copied|offered/.test(s.message) ? s.message : s.tone === "success" && step === "review" ? "Wallet connected. Money is available." : "";
    if (!feedback.textContent) feedback.hidden = true;
    show("[data-funding-status-details]", Boolean(s.message) && (s.tone === "error" || step === "wait"));
    dialog.setAttribute("aria-busy", String(s.connecting || s.busy || (step === "check" && s.tone !== "error")));
    if (step !== previous) {
      // Focus only after real step changes; balance refreshes must not steal it.
      previous = step;
      if (!document.querySelector(".ab-phone-dialog[open], .wallet-link-dialog[open]")) $("#funding-dialog-title").focus({ preventScroll: true });
      dialog.scrollTop = 0;
    }
  }
  window.AgentBountiesFundingGuide = Object.freeze({ render });
})();
