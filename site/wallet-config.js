/* Public client configuration. The Pages build supplies the CDP project ID. */
"use strict";
(function () {
  const projectId = "__COINBASE_CDP_PROJECT_ID__";
  window.AgentBountiesWalletConfig = Object.freeze({
    chain: { rpcUrl: "https://mainnet.base.org" },
    providers: {
      coinbaseEmbedded: {
        enabled: /^[A-Za-z0-9-]{8,128}$/.test(projectId),
        projectId,
        disableAnalytics: true,
        secureIframeBasePath: "https://secure-wallet.cdp.coinbase.com",
        authMethods: ["email", "oauth:google", "oauth:apple"],
        transactionPolicy: "agent-bounties-relay-required",
      },
    },
  });
})();
