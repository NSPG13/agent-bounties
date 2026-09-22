(() => {
  "use strict";
  const select = selector => document.querySelector(selector);
  const ADDRESS = /^0x[0-9a-fA-F]{40}$/;
  const BASE_CHAIN_ID = "0x14a34"; // Base mainnet
  const SUPPORTED_ASSETS = new Set(["usdc", "eth"]);
  const BASE_ASSETS = new Set(["usdc"]); // Only USDC is valid for bounty funding on Base

  function buildMoonpayUrl(asset, wallet) {
    const base = asset === "eth" ? "https://www.moonpay.com/buy/eth" : "https://www.moonpay.com/buy/usdc";
    if (wallet && ADDRESS.test(wallet)) {
      return `${base}?walletAddress=${wallet}&currencyCode=${asset === "eth" ? "eth" : "usdc"}&baseCurrencyCode=usd`;
    }
    return base;
  }

  // Validate that the direct MoonPay link can safely open for the current state.
  // Disables the link when we cannot guarantee Base-network delivery to the chosen wallet.
  function canOpenDirectMoonPay() {
    if (!window.AgentBountiesOnramp) return false;
    try {
      const status = window.AgentBountiesOnramp.status();
      if (!status) return false;

      // Wallet must be set and valid
      const wallet = status.wallet_address;
      if (!wallet || !ADDRESS.test(wallet)) return false;

      // Connected wallet must match the address we're sending to (if connected)
      if (status.wallet_connected && status.wallet_address?.toLowerCase() !== status.connection?.address?.toLowerCase()) {
        return false;
      }

      // Must be on Base mainnet (chain_id 8453) if wallet is connected
      if (status.wallet_connected && status.wallet_chain_id !== BASE_CHAIN_ID && status.wallet_chain_id !== 8453) {
        return false;
      }

      // Asset must be supported
      const asset = select("[data-onramp-asset]")?.value;
      if (!asset || !SUPPORTED_ASSETS.has(asset)) return false;

      // Bounty funding requires USDC (not ETH) — ETH is only for gas
      // The direct link is a fallback; we allow both but show guidance
      if (status.ready_for_bounty_review && asset !== "usdc") {
        // Wallet has enough USDC but user selected ETH — redirect them to USDC
        return false;
      }

      // Balance must be fresh (not stale)
      if (!status.fresh) return false;

      // No pending purchase allowed
      if (status.purchase_opened) return false;

      // Shared guard check
      if (!window.AgentBountiesOnramp.canOpenPurchase()) return false;

      return true;
    } catch (e) {
      return false;
    }
  }

  function render() {
    const link = select("[data-direct-moonpay]");
    if (!link) return;
    const asset = select("[data-onramp-asset]")?.value || "usdc";
    let wallet = null;
    try {
      const status = window.AgentBountiesOnramp?.status();
      if (status && status.wallet_address) {
        wallet = status.wallet_address;
      }
    } catch(e) {}
    link.href = buildMoonpayUrl(asset, wallet);
    // Disable when validation cannot guarantee Base-network delivery
    const disabled = !canOpenDirectMoonPay();
    link.setAttribute("aria-disabled", String(disabled));
    link.tabIndex = disabled ? -1 : 0;
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
      window.AgentBountiesOnramp.openDirectCheckout("moonpay");
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
