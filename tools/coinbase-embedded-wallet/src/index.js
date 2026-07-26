import React, { useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import {
  CDPReactProvider,
  LinkAuth,
  LinkAuthError,
  LinkAuthFlow,
  LinkAuthFlowBackButton,
  LinkAuthTitle,
} from "@coinbase/cdp-react";
import { AuthButton } from "@coinbase/cdp-react/components/AuthButton";
import { useCurrentUser, useIsInitialized, useIsSignedIn } from "@coinbase/cdp-hooks";
import {
  createCDPEmbeddedWallet,
  getCurrentUser,
  isSignedIn,
  signOut,
} from "@coinbase/cdp-core";
import { http } from "viem";
import { base } from "viem/chains";

const ADAPTER_ID = "coinbase-embedded";
const PROVIDER_UUID = "16c41c3b-a510-4b72-82f2-9d70f22552c7";
const PROVIDER_RDNS = "app.agentbounties.wallet.coinbase";
const SDK_READY_TIMEOUT_MS = 20_000;
const UNSPONSORED_TRANSACTION_METHODS = new Set([
  "eth_sendTransaction",
  "wallet_sendCalls",
  "wallet_sendTransaction",
]);
const METHOD_LABELS = Object.freeze({
  email: "Email",
  sms: "SMS",
  google: "Google",
  apple: "Apple",
  x: "X",
  twitter: "X",
  telegram: "Telegram",
  coinbase: "Coinbase",
  siwe: "Ethereum wallet",
  jwt: "Application account",
});
const ICON = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 48 48'%3E%3Crect width='48' height='48' rx='14' fill='%230052ff'/%3E%3Cpath fill='white' d='M24 9a15 15 0 1 0 0 30 15 15 0 0 0 13.7-8.9h-8.2a7.8 7.8 0 1 1 0-12.2h8.2A15 15 0 0 0 24 9Z'/%3E%3C/svg%3E";

const runtimeConfig = window.AgentBountiesWalletConfig?.providers?.coinbaseEmbedded;
const registry = window.AgentBountiesWalletAdapters;

let embeddedWallet = null;
let registered = false;
let authRequest = null;
let authResolve = null;
let authReject = null;
let panelControl = null;
let sdkReadyResolve = null;
let sdkReadyReject = null;
const sdkReady = new Promise((resolve, reject) => {
  sdkReadyResolve = resolve;
  sdkReadyReject = reject;
});

function emit(name, payload = null) {
  window.dispatchEvent(new CustomEvent("agentbounties:wallet-adapter-event", {
    detail: Object.freeze({ name, payload }),
  }));
}

function safeError(error) {
  return String(error?.message || error || "Coinbase wallet authentication failed.")
    .replace(/[\w.+-]+@[\w.-]+\.[A-Za-z]{2,}/g, "[email redacted]")
    .replace(/\+\d{7,15}/g, "[phone redacted]")
    .slice(0, 360);
}

function requireConfigured() {
  if (!runtimeConfig?.enabled || !runtimeConfig.projectId) {
    throw new Error("Coinbase embedded wallets are not activated for this deployment.");
  }
}

function accountAddress(user) {
  const address = user?.evmAccountObjects?.[0]?.address || null;
  return /^0x[0-9a-fA-F]{40}$/.test(String(address || "")) ? address.toLowerCase() : null;
}

function methodLabel(key) {
  const normalized = String(key || "").toLowerCase().replace(/^oauth:/, "");
  return METHOD_LABELS[normalized] || normalized.replaceAll("_", " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function linkedAuthMethods(user) {
  const methods = user?.authenticationMethods;
  if (!methods || typeof methods !== "object") return [];
  const found = new Set();
  for (const [key, value] of Object.entries(methods)) {
    if (!value) continue;
    if (key === "oauth" && typeof value === "object") {
      for (const [provider, providerValue] of Object.entries(value)) {
        if (providerValue) found.add(methodLabel(provider));
      }
      continue;
    }
    if (Array.isArray(value)) {
      if (value.length) found.add(methodLabel(key));
      continue;
    }
    if (typeof value === "object" && Object.keys(value).length === 0) continue;
    found.add(methodLabel(key));
  }
  return [...found].sort((left, right) => left.localeCompare(right));
}

function hidePanel() {
  panelControl?.hide();
}

function rejectPendingAuth(error) {
  const rejecter = authReject;
  authRequest = null;
  authResolve = null;
  authReject = null;
  hidePanel();
  if (rejecter) rejecter(error instanceof Error ? error : new Error(String(error)));
}

function resolvePendingAuth(address) {
  const resolver = authResolve;
  authRequest = null;
  authResolve = null;
  authReject = null;
  hidePanel();
  emit("coinbase-embedded-authenticated", { address });
  if (resolver) resolver(address);
}

function shortAddress(address) {
  return address ? `${address.slice(0, 8)}…${address.slice(-6)}` : "Wallet unavailable";
}

function AuthBridge() {
  const { isInitialized } = useIsInitialized();
  const { isSignedIn: signedIn } = useIsSignedIn();
  const { currentUser } = useCurrentUser();
  const [panel, setPanel] = useState({ visible: false, view: "signin", notice: "" });
  const address = accountAddress(currentUser);
  const methods = useMemo(() => linkedAuthMethods(currentUser), [currentUser]);

  useEffect(() => {
    panelControl = Object.freeze({
      showSignIn: () => setPanel({ visible: true, view: "signin", notice: "" }),
      showReview: () => setPanel({ visible: true, view: "review", notice: "" }),
      showLink: () => setPanel({ visible: true, view: "link", notice: "" }),
      hide: () => setPanel((value) => ({ ...value, visible: false, notice: "" })),
    });
    return () => {
      panelControl = null;
      rejectPendingAuth(new Error("Coinbase wallet authentication UI was removed."));
    };
  }, []);

  useEffect(() => {
    if (isInitialized) sdkReadyResolve?.();
  }, [isInitialized]);

  useEffect(() => {
    if (!panel.visible || !signedIn || !address || panel.view !== "signin") return;
    setPanel({ visible: true, view: "review", notice: "Wallet connected. Review recovery access before continuing." });
  }, [panel.visible, panel.view, signedIn, address]);

  if (!panel.visible) return null;

  const close = () => {
    if (authReject) rejectPendingAuth(new Error("Wallet sign-in was cancelled."));
    else hidePanel();
  };
  const continueWithWallet = () => {
    if (!address) return;
    resolvePendingAuth(address);
  };
  const handleLinkSuccess = (method) => {
    const label = methodLabel(method);
    setPanel({
      visible: true,
      view: "review",
      notice: `${label} was linked to this same Coinbase wallet identity.`,
    });
    emit("coinbase-embedded-auth-method-linked", { method: String(method || "unknown") });
  };
  let body;
  if (panel.view === "signin") {
    body = React.createElement(
      React.Fragment,
      null,
      React.createElement(
        "p",
        null,
        "Coinbase provides the non-custodial wallet and maintained sign-in interface. Agent Bounties never receives your one-time code, social password, seed phrase, or private key.",
      ),
      React.createElement(AuthButton, null),
      React.createElement(
        "p",
        { className: "wallet-auth-method-warning" },
        "Use the same sign-in method you used before. An unlinked email, phone number, or social account can create a separate Coinbase user and a different wallet.",
      ),
    );
  } else if (panel.view === "link") {
    body = React.createElement(
      React.Fragment,
      null,
      React.createElement(
        "p",
        null,
        "Add another verified way to reach this same wallet. Coinbase requires you to be signed in before a method can be linked.",
      ),
      React.createElement(
        "p",
        { className: "wallet-auth-method-warning" },
        "Linking does not merge two existing wallet identities. Coinbase may reject a method that already belongs to another user. SMS can help with access, but SIM-swap risk means it should not be your only recovery method for meaningful funds.",
      ),
      React.createElement(
        LinkAuth,
        { onLinkSuccess: handleLinkSuccess },
        (linkState) => React.createElement(
          React.Fragment,
          null,
          React.createElement(
            "div",
            { className: "wallet-auth-link-head" },
            React.createElement(LinkAuthTitle, null),
            React.createElement(LinkAuthFlowBackButton, null),
          ),
          linkState?.methodToLink
            ? React.createElement(
                "p",
                { className: "wallet-auth-notice", role: "status" },
                `Coinbase is verifying ${methodLabel(linkState.methodToLink)} before linking it to this wallet.`,
              )
            : null,
          React.createElement(LinkAuthError, null),
          React.createElement(LinkAuthFlow, null),
        ),
      ),
      React.createElement(
        "button",
        {
          className: "button secondary wallet-auth-back",
          type: "button",
          onClick: () => setPanel({ visible: true, view: "review", notice: "" }),
        },
        "Back to access methods",
      ),
    );
  } else {
    body = React.createElement(
      React.Fragment,
      null,
      React.createElement(
        "p",
        null,
        "These methods currently reach the same Coinbase user wallet. Adding a second method reduces the chance that losing one account locks you out.",
      ),
      React.createElement(
        "div",
        { className: "wallet-auth-account", "aria-label": "Connected Coinbase wallet" },
        React.createElement("span", null, "Wallet"),
        React.createElement("strong", null, shortAddress(address)),
      ),
      React.createElement(
        "div",
        { className: "wallet-auth-linked", "aria-label": "Linked authentication methods" },
        React.createElement("strong", null, "Linked sign-in methods"),
        methods.length
          ? React.createElement(
              "div",
              { className: "wallet-auth-chips" },
              ...methods.map((method) => React.createElement("span", { key: method }, method)),
            )
          : React.createElement("p", null, "Coinbase has not exposed a linked-method summary yet. You can still add another verified method below."),
      ),
      React.createElement(
        "p",
        { className: "wallet-auth-method-warning" },
        "A different method is not automatically the same identity. Link it while signed in here before relying on it for future access. Google and Apple auto-linking, when enabled in the CDP project, is limited to Coinbase's verified matching-email rules.",
      ),
      panel.notice ? React.createElement("p", { className: "wallet-auth-notice", role: "status" }, panel.notice) : null,
      React.createElement(
        "div",
        { className: "wallet-auth-actions" },
        React.createElement(
          "button",
          {
            className: "button secondary",
            type: "button",
            onClick: () => setPanel({ visible: true, view: "link", notice: "" }),
          },
          "Link another sign-in method",
        ),
        React.createElement(
          "button",
          {
            className: "button primary",
            type: "button",
            disabled: !address,
            onClick: continueWithWallet,
          },
          authResolve ? "Continue with this wallet" : "Done",
        ),
      ),
      React.createElement(
        "button",
        {
          className: "wallet-auth-signout",
          type: "button",
          onClick: async () => {
            await signOut();
            rejectPendingAuth(new Error("Wallet was signed out."));
            emit("coinbase-embedded-signed-out", null);
          },
        },
        "Sign out of this wallet",
      ),
    );
  }

  return React.createElement(
    "div",
    { className: "wallet-auth-overlay", role: "presentation" },
    React.createElement(
      "section",
      {
        className: "wallet-auth-panel",
        role: "dialog",
        "aria-modal": "true",
        "aria-labelledby": "coinbase-wallet-auth-title",
      },
      React.createElement(
        "div",
        { className: "wallet-auth-head" },
        React.createElement(
          "div",
          null,
          React.createElement("p", { className: "eyebrow" }, panel.view === "review" ? "Your wallet, your recovery paths" : "No extension or recovery phrase"),
          React.createElement(
            "h2",
            { id: "coinbase-wallet-auth-title" },
            panel.view === "link" ? "Link another way to sign in" : panel.view === "review" ? "Protect access to this wallet" : "Create or access your wallet",
          ),
        ),
        React.createElement(
          "button",
          {
            className: "wallet-auth-close",
            type: "button",
            "aria-label": "Close wallet sign in",
            onClick: close,
          },
          "×",
        ),
      ),
      body,
      React.createElement(
        "p",
        { className: "wallet-auth-boundary" },
        "Authentication and linking never authorize a bounty payment. Agent Bounties sponsors gas only for supported, bounded relay actions; canonical events remain the authority.",
      ),
    ),
  );
}

function mountAuthBridge() {
  requireConfigured();
  let root = document.querySelector("[data-agent-bounties-coinbase-auth-root]");
  if (!root) {
    root = document.createElement("div");
    root.dataset.agentBountiesCoinbaseAuthRoot = "true";
    document.body.append(root);
  }
  const config = {
    projectId: runtimeConfig.projectId,
    disableAnalytics: runtimeConfig.disableAnalytics !== false,
    secureIframeBasePath: runtimeConfig.secureIframeBasePath,
    ethereum: { createOnLogin: "eoa" },
    authMethods: [...runtimeConfig.authMethods],
    appName: "Agent Bounties",
    appLogoUrl: new URL("favicon.svg", document.baseURI).href,
  };
  try {
    createRoot(root).render(
      React.createElement(
        CDPReactProvider,
        { config },
        React.createElement(AuthBridge, null),
      ),
    );
  } catch (error) {
    sdkReadyReject?.(error);
    throw error;
  }
}

async function waitForSdk() {
  requireConfigured();
  return new Promise((resolve, reject) => {
    const timer = window.setTimeout(() => {
      reject(new Error("Coinbase wallet initialization timed out. Reload and try again."));
    }, SDK_READY_TIMEOUT_MS);
    sdkReady.then(
      (value) => {
        window.clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        window.clearTimeout(timer);
        reject(error);
      },
    );
  });
}

async function ensureEmbeddedWallet() {
  await waitForSdk();
  if (!embeddedWallet) {
    embeddedWallet = createCDPEmbeddedWallet({
      chains: [base],
      transports: {
        [base.id]: http(window.AgentBountiesWalletConfig?.chain?.rpcUrl || "https://mainnet.base.org"),
      },
    });
  }
  return embeddedWallet;
}

async function currentAddress() {
  await waitForSdk();
  if (!(await isSignedIn())) return null;
  return accountAddress(await getCurrentUser());
}

async function accessMethods() {
  await waitForSdk();
  if (!(await isSignedIn())) return [];
  return linkedAuthMethods(await getCurrentUser());
}

async function ensureAuthenticated() {
  await waitForSdk();
  if (authRequest) return authRequest;
  const existing = await currentAddress();
  authRequest = new Promise((resolve, reject) => {
    authResolve = resolve;
    authReject = reject;
  });
  if (!panelControl) {
    rejectPendingAuth(new Error("Coinbase wallet authentication UI is not ready. Reload and try again."));
    return authRequest;
  }
  if (existing) panelControl.showReview();
  else panelControl.showSignIn();
  return authRequest;
}

async function manageAccess() {
  await waitForSdk();
  const address = await currentAddress();
  if (!address) return ensureAuthenticated();
  panelControl?.showReview();
  return address;
}

async function innerProvider() {
  return (await ensureEmbeddedWallet()).provider;
}

const provider = {
  async request(args) {
    const method = String(args?.method || "");
    if (!method) throw new TypeError("EIP-1193 method is required.");
    if (UNSPONSORED_TRANSACTION_METHODS.has(method)) {
      throw Object.assign(
        new Error("This embedded wallet permits only Agent Bounties actions routed through an explicit sponsored relay. This action is not sponsored yet; connect an external wallet only after reviewing its gas requirement."),
        { code: 4100 },
      );
    }
    if (method === "eth_accounts") {
      await waitForSdk();
      if (!(await isSignedIn())) return [];
      return (await innerProvider()).request(args);
    }
    if (method === "eth_requestAccounts") {
      await ensureAuthenticated();
      return (await innerProvider()).request(args);
    }
    if (method === "wallet_addEthereumChain") {
      const requested = String(args?.params?.[0]?.chainId || "").toLowerCase();
      if (requested === "0x2105") return null;
      throw Object.assign(new Error("The Agent Bounties embedded wallet currently supports Base mainnet only."), { code: 4902 });
    }
    if (method === "wallet_switchEthereumChain") {
      const requested = String(args?.params?.[0]?.chainId || "").toLowerCase();
      if (requested !== "0x2105") {
        throw Object.assign(new Error("The Agent Bounties embedded wallet currently supports Base mainnet only."), { code: 4902 });
      }
      return null;
    }
    await ensureAuthenticated();
    return (await innerProvider()).request(args);
  },
  on(eventName, listener) {
    ensureEmbeddedWallet().then((wallet) => wallet.provider.on?.(eventName, listener)).catch(() => {});
    return provider;
  },
  removeListener(eventName, listener) {
    embeddedWallet?.provider?.removeListener?.(eventName, listener);
    return provider;
  },
};

async function disconnect() {
  await waitForSdk();
  if (await isSignedIn()) await signOut();
  rejectPendingAuth(new Error("Wallet was disconnected."));
  emit("coinbase-embedded-signed-out", null);
}

function registerAdapter() {
  if (registered || !runtimeConfig?.enabled || !registry) return;
  registry.register({
    id: ADAPTER_ID,
    info: {
      uuid: PROVIDER_UUID,
      name: "Agent Bounties embedded wallet",
      icon: ICON,
      rdns: PROVIDER_RDNS,
    },
    provider,
    capabilities: {
      embedded: true,
      custody: "user",
      vendor: "coinbase-cdp",
      accountType: "eoa",
      chainIds: [8453],
      authMethods: [...runtimeConfig.authMethods],
      authMethodLinking: true,
      typedData: true,
      eip3009: true,
      transactionPolicy: runtimeConfig.transactionPolicy,
      gasSponsoredOnSupportedRelays: true,
      arbitraryTransactionsGasSponsored: false,
      directTransactions: false,
    },
    disconnect,
  });
  registered = true;
}

window.AgentBountiesCoinbaseEmbeddedWallet = Object.freeze({
  id: ADAPTER_ID,
  provider,
  enabled: Boolean(runtimeConfig?.enabled),
  ensureAuthenticated,
  currentAddress,
  accessMethods,
  manageAccess,
  disconnect,
});

if (runtimeConfig?.enabled) {
  mountAuthBridge();
  registerAdapter();
}
