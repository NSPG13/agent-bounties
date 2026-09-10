(() => {
  "use strict";

  const ADDRESS = /^0x[0-9a-fA-F]{40}$/;
  const DIRECT_BUY_URLS = Object.freeze({
    usdc: "https://www.moonpay.com/buy/usdc",
    eth: "https://www.moonpay.com/buy/eth",
  });

  const select = (selector) => document.querySelector(selector);

  function connectedWallet() {
    const value = String(select("[data-wallet-address]")?.textContent || "").trim();
    return ADDRESS.test(value) ? value.toLowerCase() : "";
  }

  function selectedAsset() {
    return select("[data-onramp-asset]")?.value === "eth" ? "eth" : "usdc";
  }

  function assetLabel(asset) {
    return asset === "eth" ? "ETH on Base (ETH_BASE)" : "USDC on Base (USDC_BASE)";
  }

  function fiatAmount() {
    const value = String(select("[data-fiat-amount]")?.value || "").trim();
    if (!/^\d+(?:\.\d{1,2})?$/.test(value)) return null;
    const amount = Number(value);
    return Number.isFinite(amount) && amount > 0 ? amount : null;
  }

  function amountReady() {
    const amount = fiatAmount();
    return amount !== null;
  }

  function startingAmount() {
    const amount = fiatAmount();
    if (amount === null) return "Enter a USD amount for the provider to quote";
    return `$${amount.toFixed(2)} USD — fees and received amount confirmed by MoonPay`;
  }

  function setDirectOutput(message, tone = "") {
    const output = select("[data-direct-moonpay-output]");
    if (!output) return;
    output.textContent = message;
    output.dataset.tone = tone;
  }

  function setPartnerOutput(message, tone = "") {
    const output = select("[data-onramp-output]");
    if (!output) return;
    output.textContent = message;
    output.dataset.tone = tone;
  }

  function minimumMessage() {
    return "Enter a positive USD amount to quote. MoonPay confirms its purchase minimum for the asset, region and payment method before you pay.";
  }

  function renderDirectFallback() {
    const asset = selectedAsset();
    const wallet = connectedWallet();
    const acknowledged = Boolean(select("[data-onramp-ack]")?.checked);
    const hasValidAmount = amountReady();
    const pending = window.AgentBountiesOnramp?.hasPendingPurchase();
    const link = select("[data-direct-moonpay]");
    const copy = select("[data-copy-direct-wallet]");

    select("[data-direct-asset]").textContent = assetLabel(asset);
    select("[data-direct-asset-instruction]").textContent = asset === "eth" ? "ETH on Base" : "USDC on Base";
    select("[data-direct-amount]").textContent = startingAmount();
    select("[data-direct-wallet]").textContent = wallet || "Connect a Base wallet above";

    link.href = DIRECT_BUY_URLS[asset];
    link.dataset.asset = asset;
    link.setAttribute("aria-disabled", String(!(wallet && acknowledged && hasValidAmount && !pending)));
    copy.disabled = !wallet;

    if (!wallet) {
      setDirectOutput("Connect the destination wallet before opening the manual MoonPay fallback.");
    } else if (!acknowledged) {
      setDirectOutput("Acknowledge that buying crypto and funding the bounty are separate actions.");
    } else if (!hasValidAmount) {
      setDirectOutput(minimumMessage(), "error");
    } else if (pending) {
      setDirectOutput("A purchase may already be in progress. Resolve or resume the existing order above before opening another checkout.", "pending");
    } else {
      setDirectOutput(
        `Ready for manual MoonPay checkout. Select ${assetLabel(asset)}, paste ${wallet}, and verify Base on the final review screen.`,
        "pending",
      );
    }
  }

  async function copyWallet() {
    const wallet = connectedWallet();
    if (!wallet) {
      setDirectOutput("Connect the destination wallet before copying its address.", "error");
      return;
    }
    try {
      await navigator.clipboard.writeText(wallet);
      setDirectOutput(`Copied destination wallet ${wallet}. Verify the same address inside MoonPay.`, "success");
    } catch (_error) {
      const range = document.createRange();
      range.selectNodeContents(select("[data-direct-wallet]"));
      const selection = window.getSelection();
      selection.removeAllRanges();
      selection.addRange(range);
      setDirectOutput("The wallet address is selected. Copy it, then verify the full address inside MoonPay.", "pending");
    }
  }

  function openDirectCheckout(event) {
    event.preventDefault();
    renderDirectFallback();
    const wallet = connectedWallet();
    const acknowledged = Boolean(select("[data-onramp-ack]")?.checked);
    const hasValidAmount = amountReady();
    if (!wallet || !acknowledged || !hasValidAmount) {
      event.preventDefault();
      setDirectOutput(
        !wallet
          ? "Connect the destination wallet before opening MoonPay."
          : (!acknowledged
            ? "Acknowledge the purchase and funding boundary before opening MoonPay."
            : minimumMessage()),
        "error",
      );
      return;
    }
    const asset = selectedAsset();
    try {
      if (!window.AgentBountiesOnramp) throw new Error("Checkout recovery is unavailable. Reload before opening a purchase.");
      window.AgentBountiesOnramp.openDirectCheckout();
    } catch (error) {
      setDirectOutput(error.message || String(error), "error");
      return;
    }
    setDirectOutput(
      `MoonPay is opening in a new tab. Choose ${assetLabel(asset)}, use the starting amount shown here, paste ${wallet}, and stop if the final screen shows another network or address.`,
      "pending",
    );
  }

  function enforceMinimumForPartnerCheckout(event) {
    if (amountReady()) return;
    event.preventDefault();
    event.stopImmediatePropagation?.();
    const message = minimumMessage();
    setPartnerOutput(message, "error");
    setDirectOutput(message, "error");
  }

  function initialize() {
    const link = select("[data-direct-moonpay]");
    const copy = select("[data-copy-direct-wallet]");
    if (!link || !copy) return;

    const amountInput = select("[data-fiat-amount]");

    link.addEventListener("click", openDirectCheckout);
    copy.addEventListener("click", copyWallet);
    select("[data-start-moonpay]")?.addEventListener("click", enforceMinimumForPartnerCheckout, true);
    select("[data-onramp-asset]")?.addEventListener("change", renderDirectFallback);
    amountInput?.addEventListener("input", renderDirectFallback);
    select("[data-onramp-ack]")?.addEventListener("change", renderDirectFallback);
    select("[data-connect-wallet]")?.addEventListener("click", () => setTimeout(renderDirectFallback, 0));
    select("[data-wallet-provider]")?.addEventListener("change", renderDirectFallback);
    window.addEventListener("agent-bounties:onramp-state", renderDirectFallback);

    const walletAddress = select("[data-wallet-address]");
    if (walletAddress) {
      new MutationObserver(renderDirectFallback).observe(walletAddress, {
        childList: true,
        characterData: true,
        subtree: true,
      });
    }
    renderDirectFallback();
  }

  initialize();
})();
