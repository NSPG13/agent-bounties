(() => {
  "use strict";

  const projectId = "__COINBASE_CDP_PROJECT_ID__";
  const configured = !projectId.startsWith("__COINBASE_")
    && /^[A-Za-z0-9_-]{8,128}$/.test(projectId);

  window.AgentBountiesWalletConfig = Object.freeze({
    schemaVersion: "agent-bounties/wallet-providers-v1",
    chain: Object.freeze({
      id: 8453,
      idHex: "0x2105",
      name: "Base",
      rpcUrl: "https://mainnet.base.org",
    }),
    providers: Object.freeze({
      coinbaseEmbedded: Object.freeze({
        enabled: configured,
        projectId: configured ? projectId : "",
        accountType: "eoa",
        disableAnalytics: true,
        secureIframeBasePath: "https://secure-wallet.cdp.coinbase.com",
        authMethods: Object.freeze([
          "email",
          "sms",
          "oauth:google",
          "oauth:apple",
          "oauth:x",
          "oauth:telegram",
        ]),
        custody: "user",
        integration: "eip1193-eip6963-adapter",
        transactionPolicy: "agent-bounties-relay-required",
      }),
    }),
  });
})();
