(() => {
  "use strict";

  window.AgentBountiesWalletRuntime = Object.freeze({
    schemaVersion: "agent-bounties/wallet-runtime-v1",
    network: Object.freeze({
      name: "Base",
      chainId: 8453,
      chainIdHex: "0x2105",
      rpcUrl: "https://mainnet.base.org",
    }),
    gasSponsored: true,
    adapters: Object.freeze({
      coinbaseEmbedded: Object.freeze({
        enabled: false,
        projectId: "",
        accountType: "eoa",
        disableAnalytics: true,
        authMethods: Object.freeze(["email", "sms", "oauth:google", "oauth:apple", "oauth:x"]),
      }),
    }),
  });
})();
