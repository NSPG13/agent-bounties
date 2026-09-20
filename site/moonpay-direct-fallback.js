(() => {
  "use strict";
  const select = selector => document.querySelector(selector);
  const ADDRESS = /^0x[0-9a-fA-F]{40}$/;
  
  function buildMoonpayUrl(asset, wallet) {
    const base = asset === "eth" ? "https://www.moonpay.com/buy/eth" : "https://www.moonpay.com/buy/usdc";
    if (wallet && ADDRESS.test(wallet)) {
      return `${base}?walletAddress=${wallet}&currencyCode=${asset === "eth" ? "eth" : "usdc"}&baseCurrencyCode=usd`;
    }
    return base;
  }
  
  function render() {
    const link = select("[data-direct-moonpay]");
    const asset = select("[data-onramp-asset]").value;
    // Get wallet from onramp state if available
    let wallet = null;
    try {
      const status = window.AgentBountiesOnramp?.status();
      if (status && status.wallet_address) {
        wallet = status.wallet_address;
      }
    } catch(e) {}
    link.href = buildMoonpayUrl(asset, wallet);
    link.setAttribute("aria-disabled", String(!window.AgentBountiesOnramp?.canOpenPurchase()));
  }
  
  select("[data-direct-moonpay]").addEventListener("click", event => {
    event.preventDefault();
    try {
      if (!window.AgentBountiesOnramp) throw new Error("The page is still loading. Try again in a moment.");
      // Use the wallet-aware MoonPay URL with the selected destination address
      const asset = select("[data-onramp-asset]").value;
      const status = window.AgentBountiesOnramp.status();
      const wallet = status?.wallet_address;
      const url = buildMoonpayUrl(asset, wallet);
      window.open(url, "_blank", "noopener,noreferrer");
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
