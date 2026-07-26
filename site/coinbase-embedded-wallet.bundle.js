(() => {
  "use strict";
  window.AgentBountiesCoinbaseWallet = Object.freeze({
    schemaVersion: "agent-bounties/coinbase-embedded-wallet-v1",
    enabled: false,
    reason: "The Coinbase adapter source bundle is generated during CI and GitHub Pages deployment.",
  });
  window.dispatchEvent(new CustomEvent("agentbounties:embedded-wallet-status", {
    detail: Object.freeze({
      adapter: "coinbase-embedded",
      status: "disabled",
      message: "Run the pinned Coinbase adapter build before enabling its CDP Project ID.",
    }),
  }));
})();
