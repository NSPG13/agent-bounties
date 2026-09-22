(() => {
  "use strict";
  const select = selector => document.querySelector(selector);
  // Validate that the direct MoonPay link can safely open for the current state.
  // Disables the link when the shared readiness check cannot open a purchase.
  // Uses the existing shared readiness check from AgentBountiesOnramp so the button
  // behaves consistently with the rest of the page (no duplicate validation).
  function canOpenDirectMoonPay() {
    if (!window.AgentBountiesOnramp) return false;
    return window.AgentBountiesOnramp.canOpenPurchase();
  }

  function render() {
    const link = select("[data-direct-moonpay]");
    if (!link) return;
    const disabled = !canOpenDirectMoonPay();
    link.setAttribute("aria-disabled", String(disabled));
    link.tabIndex = disabled ? -1 : 0;
    link.removeAttribute("href");
  }

  select("[data-direct-moonpay]").addEventListener("click", event => {
    event.preventDefault();
    try {
      if (!window.AgentBountiesOnramp) throw new Error("The page is still loading. Try again in a moment.");

      // Pre-flight: validate before routing through the guard
      if (!canOpenDirectMoonPay()) {
        throw new Error("The MoonPay link cannot be opened for the current wallet and asset. Connect a Base wallet, refresh your balance, or choose USDC.");
      }

      // Route every purchase through the shared guard so duplicate-pending and stale-balance
      // checks in openDirectCheckout() are always enforced.
      window.AgentBountiesOnramp.openDirectCheckout("moonpay").catch((error) => {
        select("[data-direct-moonpay-output]").textContent = error?.message || String(error);
        window.AgentBountiesTopupGuide?.feedback(error?.message || String(error), "error");
      });
    } catch (error) {
      select("[data-direct-moonpay-output]").textContent = error.message;
      window.AgentBountiesTopupGuide?.feedback(error.message, "error");
    }
  });

  window.addEventListener("agent-bounties:onramp-state", render);
  select("[data-onramp-asset]").addEventListener("change", render);
  render();
})();
