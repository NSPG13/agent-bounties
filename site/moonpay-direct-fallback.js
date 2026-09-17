(() => {
  "use strict";
  const select = selector => document.querySelector(selector);
  function render() {
    const link = select("[data-direct-moonpay]");
    const asset = select("[data-onramp-asset]").value;
    link.href = asset === "eth" ? "https://www.moonpay.com/buy/eth" : "https://www.moonpay.com/buy/usdc";
    link.setAttribute("aria-disabled", String(!window.AgentBountiesOnramp?.canOpenPurchase()));
  }
  select("[data-direct-moonpay]").addEventListener("click", event => {
    event.preventDefault();
    try {
      if (!window.AgentBountiesOnramp) throw new Error("The page is still loading. Try again in a moment.");
      // Only opens a provider. The person chooses amount, signs up, and pays there.
      window.AgentBountiesOnramp.openDirectCheckout();
      select("[data-direct-moonpay-output]").textContent = "";
    } catch (error) {
      select("[data-direct-moonpay-output]").textContent = error.message;
      window.AgentBountiesTopupGuide?.feedback(error.message, "error");
    }
  });
  window.addEventListener("agent-bounties:onramp-state", render);
  select("[data-onramp-asset]").addEventListener("change", render);
  render();
})();
